<script lang="ts">
	// "Import from Spotify" (#375). The step on screen is read off the import itself, not kept here,
	// so closing the dialog mid-import and coming back (from the sidebar, after a toast) lands on
	// whatever the import is doing now. Closing never stops anything; Stop does.
	import { untrack } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		AlertCircleIcon,
		Cancel01Icon,
		CheckmarkCircle02Icon,
		CloudIcon,
		ComputerIcon,
		Copy01Icon,
		FileImportIcon,
		Link01Icon,
		MusicNote01Icon,
		PlayIcon,
		Search01Icon,
		SpotifyIcon,
		Tick02Icon
	} from '@hugeicons/core-free-icons';
	import { goto } from '$app/navigation';
	import { open as pickFile } from '@tauri-apps/plugin-dialog';
	import * as Dialog from '$lib/components/ui/dialog';
	import * as Tabs from '$lib/components/ui/tabs';
	import * as Popover from '$lib/components/ui/popover';
	import * as RadioGroup from '$lib/components/ui/radio-group';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import ExperimentalBadge from './ExperimentalBadge.svelte';
	import * as api from '$lib/api';
	import { copyText } from '$lib/clipboard';
	import { thumb } from '$lib/thumb';
	import { parseYtLink } from '$lib/ytlink';
	import { rowScroller } from '$lib/rows.svelte';
	import { rowWindow } from '$lib/rows';
	import { t } from '$lib/i18n.svelte';
	import { auth, toast } from '$lib/player.svelte';
	import {
		clockTime,
		cooldownUntil,
		dismissImport,
		imp,
		importError,
		readFile,
		readLink,
		startImport
	} from '$lib/import.svelte';

	const SPOTIFY_PRIVACY = 'https://www.spotify.com/account/privacy/';

	const signedIn = $derived(!!auth.account?.signedIn);
	// An "Update from Spotify" runs as the same kind of job but never takes the dialog over.
	const job = $derived(imp.snapshot && !imp.snapshot.update ? imp.snapshot : null);
	const view = $derived(job ? job.phase : imp.preview ? 'pick' : 'source');
	const listName = (l: { kind: api.ImportListKind; name: string }) =>
		l.kind === 'liked' ? t('import.liked_songs') : l.name;
	const pct = (done: number, total: number) => (total ? Math.round((done / total) * 100) : 100);
	const mmss = (ms?: number | null) => {
		if (!ms) return '';
		const s = Math.round(ms / 1000);
		return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
	};

	// --- source ----------------------------------------------------------------------------------

	let link = $state('');
	let dragging = $state(false);

	async function chooseFile() {
		const path = await pickFile({
			multiple: false,
			title: t('import.choose_file'),
			filters: [{ name: t('import.file_filter'), extensions: ['zip', 'json', 'csv'] }]
		});
		if (typeof path === 'string') readFile(path);
	}

	function submitLink(e: Event) {
		e.preventDefault();
		if (link.trim()) readLink(link);
	}

	// Only the highlight lives here. The drop itself bubbles out of the portal to the window's
	// handler (`handleImportDrop`, in the root layout), which is what reads it.
	const over = () => {
		if (view === 'source') dragging = true;
	};

	// --- pick ------------------------------------------------------------------------------------

	let picked = $state<boolean[]>([]);
	// Everything ticked whenever a new read lands.
	$effect(() => {
		picked = (imp.preview?.lists ?? []).map(() => true);
	});
	const pickedIdx = $derived(picked.flatMap((p, i) => (p ? [i] : [])));
	const pickedSongs = $derived(
		pickedIdx.reduce((n, i) => n + (imp.preview?.lists[i]?.count ?? 0), 0)
	);

	// --- matching --------------------------------------------------------------------------------

	// When matching started, from this side: the time left is measured, not guessed.
	let clock = $state<{ at: number; done: number } | null>(null);
	$effect(() => {
		if (view !== 'matching') clock = null;
		else if (!clock && job) clock = { at: Date.now(), done: job.done };
	});
	const minutesLeft = $derived.by(() => {
		if (!job || !clock) return null;
		const did = job.done - clock.done;
		if (did < 5) return null;
		return Math.max(1, Math.round((((Date.now() - clock.at) / did) * (job.total - job.done)) / 60000));
	});

	// --- review ----------------------------------------------------------------------------------

	type Tab = 'check' | 'missing' | 'matched';
	let tab = $state<Tab>('check');
	let rows = $state<api.ImportRow[]>([]);
	// Rows the user has settled in this sitting, so they read as done without leaving the tab.
	let settled = $state<Record<string, 'kept' | 'out'>>({});

	async function loadRows(tier: Tab) {
		tab = tier;
		rows = [];
		try {
			rows = await api.importRows(tier);
		} catch (e) {
			toast.error(importError(e));
		}
	}

	// The tab worth opening on: what needs a look first.
	$effect(() => {
		if (view !== 'review') return;
		untrack(() => {
			settled = {};
			loadRows(job?.check ? 'check' : job?.missing ? 'missing' : 'matched');
		});
	});

	async function choose(row: api.ImportRow, song: api.SongItem | null) {
		try {
			imp.snapshot = await api.importPick(row.key, song);
			const i = rows.findIndex((r) => r.key === row.key);
			if (i >= 0) rows[i] = { ...rows[i], pick: song ?? rows[i].pick };
			settled[row.key] = song ? 'kept' : 'out';
			open = null;
		} catch (e) {
			toast.error(importError(e));
		}
	}

	const sc = rowScroller();
	const win = $derived(rowWindow(sc.scrollTop, sc.viewportPx, rows.length, sc.rowPx));

	// The one row whose picker is open, and its search.
	let open = $state<string | null>(null);
	let query = $state('');
	let results = $state<api.SongItem[]>([]);
	let searching = $state(false);
	$effect(() => {
		if (open) {
			const row = untrack(() => rows.find((r) => r.key === open));
			query = row ? `${row.title} ${row.artists}` : '';
			results = [];
		}
	});
	async function search(e: Event) {
		e.preventDefault();
		if (!query.trim()) return;
		// A pasted YouTube link (#441): the track may only be up as a video, which a song search
		// never turns up.
		const link = parseYtLink(query);
		searching = true;
		try {
			results = link?.kind === 'song' ? [await api.song(link.id)] : await api.search(query.trim());
		} catch (e) {
			toast.error(importError(e));
		} finally {
			searching = false;
		}
	}

	// --- create ----------------------------------------------------------------------------------

	let name = $state('');
	let where = $state<'account' | 'device'>('account');
	$effect(() => {
		if (view === 'review') untrack(() => (name = job?.lists.length === 1 ? listName(job.lists[0]) : ''));
	});

	async function create() {
		if (!job) return;
		const names: Record<number, string> = {};
		job.lists.forEach((l, i) => {
			if (l.kind === 'liked') names[i] = t('import.liked_songs_playlist');
		});
		if (job.lists.length === 1 && name.trim()) names[0] = name.trim();
		try {
			await api.importCreate({ names, local: !signedIn || where === 'device' });
		} catch (e) {
			toast.error(importError(e));
		}
	}

	// --- done ------------------------------------------------------------------------------------

	function openResult(r: api.ImportResult) {
		imp.open = false;
		goto(`/playlist/${encodeURIComponent(r.id)}`);
		dismissImport();
	}

	async function copyMissing() {
		const missing = await api.importRows('missing').catch(() => []);
		if (!missing.length) return;
		// Tab-separated, so it pastes into a spreadsheet as two columns.
		await copyText(missing.map((r) => `${r.artists}\t${r.title}`).join('\n'));
		toast.success(t('import.copied', { count: missing.length }));
	}

	function finish() {
		imp.open = false;
		dismissImport();
	}

	// The failed view's clock, so "Try again at 18:40" turns back into "Try again" on its own.
	let now = $state(Date.now());
	$effect(() => {
		if (view !== 'failed') return;
		now = Date.now();
		const timer = setInterval(() => (now = Date.now()), 15_000);
		return () => clearInterval(timer);
	});

	const wide = $derived(view === 'review');
</script>

{#snippet bar(done: number, total: number)}
	<div class="h-2 w-full overflow-hidden rounded-full bg-muted">
		<div class="h-full rounded-full bg-primary transition-[width] duration-300" style="width: {pct(done, total)}%"></div>
	</div>
{/snippet}

{#snippet tierIcon(tier: api.ImportTier)}
	{#if tier === 'matched'}
		<HugeiconsIcon icon={CheckmarkCircle02Icon} class="h-4 w-4 shrink-0 text-primary" />
	{:else if tier === 'check'}
		<HugeiconsIcon icon={AlertCircleIcon} class="h-4 w-4 shrink-0 text-foreground" />
	{:else}
		<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4 shrink-0 text-muted-foreground" />
	{/if}
{/snippet}

{#snippet art(url: string | null | undefined, cls: string)}
	{#if url}
		<img src={thumb(url, 96)} alt="" class="{cls} shrink-0 rounded-md object-cover" loading="lazy" />
	{:else}
		<div class="{cls} flex shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
			<HugeiconsIcon icon={MusicNote01Icon} class="h-4 w-4" />
		</div>
	{/if}
{/snippet}

{#snippet songLine(s: api.SongItem)}
	<div class="flex min-w-0 items-center gap-2">
		{@render art(s.thumbnail, 'h-9 w-9')}
		<div class="min-w-0">
			<div class="truncate text-sm font-medium">{s.title}</div>
			<div class="truncate text-xs text-muted-foreground">
				{[s.artists, s.album, s.duration].filter(Boolean).join(' • ')}
			</div>
		</div>
	</div>
{/snippet}

<Dialog.Root bind:open={imp.open}>
	<!-- One column pinned to the dialog's width. A grid column otherwise grows to its widest
	     content, and a truncated title still reports its full width, so one long song name
	     pushed every view in here past the dialog's edge. -->
	<Dialog.Content
		class="grid-cols-[minmax(0,1fr)] {wide ? 'sm:max-w-3xl' : 'sm:max-w-lg'}"
		ondragover={over}
		ondragleave={() => (dragging = false)}
		ondrop={() => (dragging = false)}
	>
		<Dialog.Header>
			<Dialog.Title class="flex items-center gap-2">
				<HugeiconsIcon icon={SpotifyIcon} class="h-5 w-5 text-primary" />
				{#if view === 'matching'}{t('import.matching_title')}
				{:else if view === 'review'}{t('import.review_title')}
				{:else if view === 'creating'}{t('import.creating_title')}
				{:else if view === 'done'}{t('import.done_title')}
				{:else if view === 'failed'}{t('import.failed_title')}
				{:else if view === 'cancelled'}{t('import.cancelled_title')}
				{:else}{t('import.title')}{/if}
				<ExperimentalBadge />
			</Dialog.Title>
			{#if view === 'source'}
				<Dialog.Description>{t('import.desc')}</Dialog.Description>
			{/if}
		</Dialog.Header>

		{#if view === 'source'}
			<div class="flex flex-col gap-3 {dragging ? 'rounded-2xl ring-2 ring-primary ring-offset-4 ring-offset-background' : ''}">
				<form class="flex flex-col gap-2 rounded-2xl border p-4" onsubmit={submitLink}>
					<div class="flex items-center gap-2 text-sm font-medium">
						<HugeiconsIcon icon={Link01Icon} class="h-4 w-4 text-muted-foreground" />
						{t('import.link_title')}
					</div>
					<p class="text-xs text-muted-foreground">{t('import.link_desc')}</p>
					<div class="flex gap-2">
						<Input bind:value={link} placeholder="https://open.spotify.com/playlist/..." autofocus />
						<Button type="submit" disabled={!link.trim() || imp.reading}>
							{imp.reading ? t('import.reading') : t('import.continue')}
						</Button>
					</div>
				</form>
				<div class="flex flex-col gap-2 rounded-2xl border p-4">
					<div class="flex items-center gap-2 text-sm font-medium">
						<HugeiconsIcon icon={FileImportIcon} class="h-4 w-4 text-muted-foreground" />
						{t('import.data_title')}
					</div>
					<p class="text-xs text-muted-foreground">{t('import.data_desc')}</p>
					<div class="flex flex-wrap gap-2">
						<Button
							variant="outline"
							size="sm"
							onclick={() =>
								api.openExternal(SPOTIFY_PRIVACY).catch((e) =>
									toast.error(t('toasts.browser_failed', { error: String(e) }))
								)}
						>
							{t('import.request_data')}
						</Button>
						<Button size="sm" onclick={chooseFile} disabled={imp.reading}>
							{t('import.choose_file')}
						</Button>
					</div>
					<p class="text-xs text-muted-foreground">{t('import.drop_hint')}</p>
				</div>
			</div>
		{:else if view === 'pick' && imp.preview}
			{@const p = imp.preview}
			{#if p.lists.length === 1}
				{@const l = p.lists[0]}
				<div class="flex items-center gap-4 rounded-2xl border p-4">
					{@render art(l.cover, 'h-20 w-20')}
					<div class="min-w-0">
						<div class="truncate text-lg font-semibold">{listName(l)}</div>
						{#if l.owner}<div class="truncate text-sm text-muted-foreground">{t('import.by', { owner: l.owner })}</div>{/if}
						<div class="text-sm text-muted-foreground">{t('import.songs_count', { count: l.count })}</div>
					</div>
				</div>
				{#if l.truncated}
					<p class="flex items-start gap-2 text-xs text-muted-foreground">
						<HugeiconsIcon icon={AlertCircleIcon} class="mt-px h-3.5 w-3.5 shrink-0" />
						{t('import.truncated')}
					</p>
				{/if}
			{:else}
				<div class="flex items-center justify-between text-xs text-muted-foreground">
					<span>{t('import.lists_found', { count: p.lists.length })}</span>
					<button
						class="cursor-pointer font-medium hover:text-foreground"
						onclick={() => (picked = picked.map(() => !pickedIdx.length))}
					>
						{pickedIdx.length ? t('import.select_none') : t('import.select_all')}
					</button>
				</div>
				<div class="-mx-1 max-h-[45vh] overflow-y-auto px-1">
					{#each p.lists as l, i (i)}
						<label class="flex cursor-pointer items-center gap-3 rounded-xl px-2 py-2 transition-colors hover:bg-accent/10">
							<Checkbox checked={picked[i]} onCheckedChange={(v) => (picked[i] = !!v)} />
							<div class="min-w-0 flex-1">
								<div class="truncate text-sm font-medium">{listName(l)}</div>
								<div class="text-xs text-muted-foreground">{t('import.songs_count', { count: l.count })}</div>
							</div>
						</label>
					{/each}
				</div>
			{/if}
			<Dialog.Footer>
				<Button variant="outline" onclick={() => (imp.preview = null)}>{t('common.back')}</Button>
				<Button onclick={() => startImport(pickedIdx)} disabled={!pickedIdx.length}>
					{t('import.start_count', { count: pickedSongs })}
				</Button>
			</Dialog.Footer>
		{:else if job && view === 'matching'}
			<div class="flex flex-col gap-3">
				<div class="flex items-baseline justify-between">
					<span class="text-2xl font-semibold tabular-nums">{t('import.progress', { done: job.done, total: job.total })}</span>
					<span class="text-xs text-muted-foreground">
						{minutesLeft && !job.waitingUntil ? t('import.eta', { minutes: minutesLeft }) : ''}
					</span>
				</div>
				{@render bar(job.done, job.total)}
				<div class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
					<span class="flex items-center gap-1">{@render tierIcon('matched')}{t('import.matched', { count: job.matched })}</span>
					<span class="flex items-center gap-1">{@render tierIcon('check')}{t('import.to_check', { count: job.check })}</span>
					<span class="flex items-center gap-1">{@render tierIcon('missing')}{t('import.not_found', { count: job.missing })}</span>
				</div>
				<div class="flex flex-col gap-1 rounded-2xl border p-2">
					{#each job.recent as r, i (i)}
						<div class="flex items-center gap-3 rounded-lg px-2 py-1.5">
							{@render art(r.thumbnail, 'h-8 w-8')}
							<div class="min-w-0 flex-1">
								<div class="truncate text-sm">{r.title}</div>
								<div class="truncate text-xs text-muted-foreground">{r.artists}</div>
							</div>
							{@render tierIcon(r.tier)}
						</div>
					{:else}
						<p class="px-2 py-1.5 text-xs text-muted-foreground">{t('import.reading')}</p>
					{/each}
				</div>
				{#if job.waitingUntil}
					<p class="flex items-start gap-2 text-xs text-muted-foreground">
						<HugeiconsIcon icon={AlertCircleIcon} class="mt-px h-3.5 w-3.5 shrink-0" />
						{t('import.waiting', { time: clockTime(job.waitingUntil) })}
					</p>
				{/if}
				<p class="text-xs text-muted-foreground">{t('import.pace_hint')}</p>
				<p class="text-xs text-muted-foreground">{t('import.keep_browsing_hint')}</p>
			</div>
			<Dialog.Footer>
				<Button variant="outline" onclick={() => api.importCancel()}>{t('import.stop')}</Button>
				<Button onclick={() => (imp.open = false)}>{t('import.keep_browsing')}</Button>
			</Dialog.Footer>
		{:else if job && view === 'review'}
			<p class="text-sm text-muted-foreground">
				{t('import.review_desc', { matched: job.matched, total: job.total })}
			</p>
			<Tabs.Root value={tab} onValueChange={(v) => loadRows(v as Tab)}>
				<Tabs.List>
					<Tabs.Trigger value="check">{t('import.tab_check', { count: job.check })}</Tabs.Trigger>
					<Tabs.Trigger value="missing">{t('import.tab_missing', { count: job.missing })}</Tabs.Trigger>
					<Tabs.Trigger value="matched">{t('import.tab_matched', { count: job.matched })}</Tabs.Trigger>
				</Tabs.List>
			</Tabs.Root>
			<div class="h-[38vh] overflow-y-auto rounded-2xl border" {@attach sc.attach}>
				{#if !rows.length}
					<p class="p-4 text-sm text-muted-foreground">{t('import.nothing_here')}</p>
				{/if}
				<div style="height: {win.padTop}px"></div>
				{#each rows.slice(win.start, win.end) as row (row.key)}
					{@const mark = settled[row.key]}
					<div data-row class="flex h-16 items-center gap-3 border-b px-3 last:border-b-0 {mark === 'out' ? 'opacity-50' : ''}">
						<div class="w-[38%] min-w-0">
							<div class="truncate text-sm font-medium">{row.title}</div>
							<div class="truncate text-xs text-muted-foreground">
								{[row.artists, row.album, mmss(row.durationMs)].filter(Boolean).join(' • ')}
							</div>
						</div>
						<div class="min-w-0 flex-1">
							{#if mark === 'out'}
								<span class="text-xs text-muted-foreground">{t('import.left_out')}</span>
							{:else if row.pick}
								{@render songLine(row.pick)}
							{:else}
								<span class="text-xs text-muted-foreground">{t('import.not_found_row')}</span>
							{/if}
						</div>
						<div class="flex shrink-0 items-center gap-1">
							{#if row.pick && mark !== 'out'}
								<Button
									variant="ghost"
									size="icon-sm"
									title={t('import.preview_play')}
									aria-label={t('import.preview_play')}
									onclick={() => row.pick && api.play(row.pick)}
								>
									<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" />
								</Button>
							{/if}
							<Popover.Root open={open === row.key} onOpenChange={(v) => (open = v ? row.key : null)}>
								<Popover.Trigger>
									{#snippet child({ props })}
										<Button {...props} variant="outline" size="sm">
											{row.pick ? t('import.change') : t('import.choose')}
										</Button>
									{/snippet}
								</Popover.Trigger>
								<Popover.Content align="end" class="w-96 p-2">
									<form class="flex gap-2 p-1" onsubmit={search}>
										<Input bind:value={query} class="h-8" aria-label={t('import.search_ytm')} />
										<Button type="submit" size="icon-sm" variant="outline" aria-label={t('import.search_ytm')} disabled={searching}>
											<HugeiconsIcon icon={Search01Icon} class="h-4 w-4" />
										</Button>
									</form>
									<p class="px-1 pb-1 text-xs text-muted-foreground">{t('import.paste_link_hint')}</p>
									<div class="max-h-72 overflow-y-auto">
										{#each results.length ? results : row.candidates as c (c.video_id)}
											<button
												class="flex w-full cursor-pointer items-center rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-accent/10"
												onclick={() => choose(row, c)}
											>
												{@render songLine(c)}
											</button>
										{:else}
											<p class="px-2 py-2 text-xs text-muted-foreground">
												{searching ? t('common.searching') : t('import.no_candidates')}
											</p>
										{/each}
									</div>
								</Popover.Content>
							</Popover.Root>
							{#if mark !== 'out' && row.pick && row.tier === 'check' && mark !== 'kept'}
								<Button
									variant="ghost"
									size="icon-sm"
									title={t('import.keep')}
									aria-label={t('import.keep')}
									onclick={() => choose(row, row.pick ?? null)}
								>
									<HugeiconsIcon icon={Tick02Icon} class="h-4 w-4" />
								</Button>
							{/if}
							{#if mark !== 'out' && row.pick}
								<Button
									variant="ghost"
									size="icon-sm"
									title={t('import.skip')}
									aria-label={t('import.skip')}
									onclick={() => choose(row, null)}
								>
									<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
								</Button>
							{/if}
						</div>
					</div>
				{/each}
				<div style="height: {win.padBottom}px"></div>
			</div>

			<div class="flex flex-col gap-3">
				{#if job.lists.length === 1}
					<Input bind:value={name} aria-label={t('import.name_label')} placeholder={t('import.name_label')} />
				{:else if job.lists.length > 1}
					<p class="text-xs text-muted-foreground">{t('import.many_lists', { count: job.lists.length })}</p>
				{/if}
				{#if signedIn}
					<RadioGroup.Root
						value={where}
						onValueChange={(v) => (where = v === 'device' ? 'device' : 'account')}
						class="grid grid-cols-2 gap-2"
						aria-label={t('dialogs.new_playlist.where')}
					>
						{#each [{ v: 'account', icon: CloudIcon }, { v: 'device', icon: ComputerIcon }] as o (o.v)}
							<label class="flex cursor-pointer items-center gap-3 rounded-2xl border px-3 py-2 transition-colors hover:bg-accent/10 has-[[data-state=checked]]:border-primary has-[[data-state=checked]]:bg-primary/5">
								<RadioGroup.Item value={o.v} />
								<HugeiconsIcon icon={o.icon} class="h-4 w-4 shrink-0 text-muted-foreground" />
								<span class="text-sm">{t(o.v === 'account' ? 'dialogs.new_playlist.account' : 'dialogs.new_playlist.device')}</span>
							</label>
						{/each}
					</RadioGroup.Root>
				{:else}
					<p class="flex items-start gap-2 text-xs text-muted-foreground">
						<HugeiconsIcon icon={ComputerIcon} class="mt-px h-3.5 w-3.5 shrink-0" />
						{t('import.device_only')}
					</p>
				{/if}
			</div>
			<Dialog.Footer>
				<Button variant="outline" onclick={finish}>{t('common.cancel')}</Button>
				<Button onclick={create}>
					{job.lists.length > 1
						? t('import.create_many', { count: job.lists.length })
						: t('import.create')}
				</Button>
			</Dialog.Footer>
		{:else if job && view === 'creating'}
			<div class="flex flex-col gap-3">
				<span class="text-sm text-muted-foreground">{t('import.creating_progress', { done: job.step[0], total: job.step[1] })}</span>
				{@render bar(job.step[0], job.step[1])}
				<p class="text-xs text-muted-foreground">{t('import.keep_browsing_hint')}</p>
			</div>
			<Dialog.Footer>
				<Button variant="outline" onclick={() => api.importCancel()}>{t('import.stop')}</Button>
				<Button onclick={() => (imp.open = false)}>{t('import.keep_browsing')}</Button>
			</Dialog.Footer>
		{:else if job && (view === 'done' || view === 'cancelled')}
			{#if view === 'cancelled'}
				<p class="text-sm text-muted-foreground">{t('import.cancelled_desc')}</p>
			{/if}
			{#if job.results.length}
				<div class="flex flex-col gap-1">
					{#each job.results as r (r.id)}
						<div class="flex items-center gap-3 rounded-xl px-2 py-2">
							<HugeiconsIcon icon={r.local ? ComputerIcon : CloudIcon} class="h-5 w-5 shrink-0 text-muted-foreground" />
							<div class="min-w-0 flex-1">
								<div class="truncate text-sm font-medium">{r.name}</div>
								<div class="text-xs text-muted-foreground">
									{t('import.result_added', { count: r.added })}{r.missing ? ` • ${t('import.result_missing', { count: r.missing })}` : ''}
								</div>
							</div>
							<Button size="sm" variant="outline" onclick={() => openResult(r)}>{t('import.open')}</Button>
						</div>
					{/each}
				</div>
			{/if}
			<Dialog.Footer>
				{#if job.missing}
					<Button variant="outline" class="gap-2" onclick={copyMissing}>
						<HugeiconsIcon icon={Copy01Icon} class="h-4 w-4" />
						{t('import.copy_missing')}
					</Button>
				{/if}
				<Button onclick={finish}>{t('common.done')}</Button>
			</Dialog.Footer>
		{:else if job && view === 'failed'}
			{@const until = cooldownUntil(job.message)}
			{@const waiting = !!until && until * 1000 > now}
			<p class="text-sm">{importError(job.message)}</p>
			<p class="text-xs text-muted-foreground">{t('import.failed_hint')}</p>
			<Dialog.Footer>
				<Button variant="outline" onclick={finish}>{t('common.close')}</Button>
				<!-- Not before the cooldown is over: someone pressing it again and again is exactly the
				     traffic it is there to stop. -->
				<Button onclick={() => startImport(imp.picked)} disabled={waiting}>
					{waiting && until ? t('import.try_again_at', { time: clockTime(until) }) : t('import.try_again')}
				</Button>
			</Dialog.Footer>
		{/if}
	</Dialog.Content>
</Dialog.Root>
