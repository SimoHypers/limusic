<script lang="ts">
	// Review lyrics: what is waiting for a reviewer, oldest first. Opening one plays the song and
	// shows the submitted lyric: line by line with each word's reading under it (the editor's word
	// track), or as listeners will see it. The queue stays beside it, and a decision moves on to the
	// next one. A submission that is nearly right is fixed in the editor before it is approved, and
	// stays its author's. The player bar is the only transport. Rules are in `$lib/review.ts`.
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Add01Icon,
		ArrowLeft01Icon,
		Cancel01Icon,
		Delete02Icon,
		Flag02Icon,
		InformationCircleIcon,
		LockIcon,
		MusicNote01Icon,
		PencilEdit02Icon,
		TaskDone01Icon,
		Tick02Icon,
		UserGroupIcon,
		UserIcon
	} from '@hugeicons/core-free-icons';
	import LogoMark from '$lib/components/LogoMark.svelte';
	import { Button } from '$lib/components/ui/button';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import { Switch } from '$lib/components/ui/switch';
	import * as Tabs from '$lib/components/ui/tabs';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import LimusicNotice from '$lib/components/LimusicNotice.svelte';
	import LyricsView, { prefs as lyricsPrefs, toggle as toggleLyricsPref } from '$lib/components/LyricsView.svelte';
	import * as api from '$lib/api';
	import { playback, seekTo, toast } from '$lib/player.svelte';
	import { limusic, refreshAccount } from '$lib/limusic.svelte';
	import { currentLocale, t } from '$lib/i18n.svelte';
	import { keyOf, typing } from '$lib/shortcuts';
	import {
		ago,
		coverOf,
		diffLyrics,
		isGone,
		kindKey,
		restored,
		reviewErrorKey,
		waitingLabel,
		without,
		type Change
	} from '$lib/review';

	let allowed = $state(true);
	let items = $state.raw<api.LimusicQueueItem[]>([]);
	let next = $state<number | null>(null);
	let loading = $state(true);
	let loadError = $state('');
	/** What the service said to the last decision that didn't go through, until the next one. */
	let notice = $state('');
	const admin = $derived(limusic.account?.role === 'admin' && !limusic.anonymous);

	async function load(more = false) {
		loadError = '';
		try {
			const q = await api.limusicReviewQueue(more ? (next ?? undefined) : undefined);
			const seen = new Set(items.map((i) => i.revision));
			items = more ? [...items, ...q.items.filter((i) => !seen.has(i.revision))] : q.items;
			next = q.next;
			count();
		} catch (e) {
			if (String(e) === 'forbidden') allowed = false;
			else loadError = t(reviewErrorKey(String(e)));
		} finally {
			loading = false;
		}
	}
	/** The card in Settings shows the same number. */
	const count = () => (limusic.waiting = { count: items.length, more: next !== null });

	/** Who this is, then the queue. Also the retry. Back from the editor, the row it was on opens
	 *  again: the one after a fix that went live, the same one after a fix that was dropped. */
	async function start() {
		loadError = '';
		loading = true;
		if (!limusic.account) await refreshAccount();
		if (!limusic.account && limusic.signedIn) {
			// The service couldn't say who this is, which is not the same as not being a reviewer.
			loadError = t(reviewErrorKey('unavailable'));
			loading = false;
			return;
		}
		if (!limusic.account?.can_review) {
			allowed = false;
			loading = false;
			return;
		}
		await load();
		const at = limusic.reviewAt;
		limusic.reviewAt = null;
		if (at !== null && at >= 0 && items.length) openItem(items[Math.min(at, items.length - 1)]);
	}
	onMount(() => {
		start();
		window.addEventListener('keydown', onKey);
		return () => window.removeEventListener('keydown', onKey);
	});

	// --- One submission ---------------------------------------------------------------------------
	type View = 'lines' | 'player' | 'changes' | 'current';
	let open = $state<api.LimusicQueueItem | null>(null);
	let detail = $state.raw<api.LimusicRevision | null>(null);
	let detailError = $state('');
	/** What is live now, for an edit. `undefined` while it loads, null when there is nothing. */
	let current = $state.raw<api.Lyrics | null | undefined>(undefined);
	let view = $state<View>('lines');
	let rejecting = $state(false);
	let reason = $state('');
	let busy = $state(false);

	const raw = $derived(view === 'current' ? (current ?? null) : (detail?.lyrics ?? null));
	/** `raw` with the romanization the player would add, so both views read as listeners will see
	 *  them. The raw lines until it answers. */
	let shown = $state.raw<api.Lyrics | null>(null);
	$effect(() => {
		const l = raw;
		shown = l;
		if (!l) return;
		let stale = false;
		api.lyricsRomanize(l)
			.then((r) => !stale && (shown = r))
			.catch(() => {});
		return () => (stale = true);
	});
	/** Each word's reading, under it in the Lines view: the engine the editor's word track uses. */
	let readings = $state.raw<(string | null)[][]>([]);
	$effect(() => {
		const ls = detail?.lyrics?.lines;
		readings = [];
		if (!ls?.length) return;
		let stale = false;
		api.lyricsWordReadings(ls.map((l) => (l.words ?? []).map((w) => w.text)))
			.then((r) => !stale && (readings = r))
			.catch(() => {});
		return () => (stale = true);
	});
	/** The romanization switch is offered when there is something to romanize. */
	const canRead = $derived(readings.some((r) => r.some(Boolean)) || !!shown?.lines.some((l) => l.romanized));

	/** What an edit changes, once both versions are here. */
	const changes = $derived<Change[] | null>(
		open?.kind === 'edit' && detail?.lyrics && current ? diffLyrics(current.lines, detail.lyrics.lines) : null
	);
	const summary = $derived.by(() => {
		if (!changes) return '';
		const n = (k: Change['kind']) => changes.filter((c) => c.kind === k).length;
		const parts = (
			[
				['reworded', 'review.sum_reworded'],
				['retimed', 'review.sum_retimed'],
				['added', 'review.sum_added'],
				['removed', 'review.sum_removed']
			] as const
		)
			.filter(([k]) => n(k) > 0)
			.map(([k, key]) => t(key, { count: n(k) }));
		return parts.length
			? new Intl.ListFormat(currentLocale.id, { style: 'narrow', type: 'unit' }).format(parts)
			: t('review.no_changes');
	});

	/** The song plays from the start; one already playing is taken back to it. It starts as a song
	 *  radio, like a timecode link does, because that is what fetches its cover and album: a bare
	 *  id has neither. The player view is not opened, this page stays in front. */
	function play(item: api.LimusicQueueItem) {
		if (playback.now?.videoId === item.video_id) return seekTo(0);
		api.startRadio('song', item.video_id, item.title, 0).catch((e) => toast.error(String(e)));
	}
	const onSong = $derived(!!open && playback.now?.videoId === open.video_id);

	async function openItem(item: api.LimusicQueueItem) {
		open = item;
		detail = null;
		detailError = '';
		current = item.kind === 'edit' ? undefined : null;
		if (view !== 'player') view = 'lines';
		rejecting = false;
		reason = '';
		notice = '';
		heldAt = 0;
		play(item);
		const id = item.revision;
		if (item.kind === 'edit') {
			api.limusicLiveLyrics(item.video_id)
				.then((l) => open?.revision === id && (current = l))
				.catch(() => open?.revision === id && (current = null));
		}
		try {
			const r = await api.limusicReviewRevision(id);
			if (open?.revision === id) detail = r;
		} catch (e) {
			if (open?.revision !== id) return;
			detailError = t(reviewErrorKey(String(e)));
			if (isGone(String(e))) items = without(items, id), count();
		}
	}

	function close() {
		open = null;
		detail = null;
	}

	/** The row leaves the list at once and the one after it opens, so a queue is worked through
	 *  without going back to it. If the service refuses, the row comes back with the reason. */
	async function decide(verdict: 'approve' | 'reject') {
		const row = open;
		if (!row || busy || !detail) return;
		busy = true;
		const why = verdict === 'reject' ? reason.trim() : '';
		const at = items.findIndex((i) => i.revision === row.revision);
		items = without(items, row.revision);
		count();
		const following = items[at] ?? items[at - 1];
		if (following) openItem(following);
		else close();
		try {
			const status = await api.limusicReview(row.revision, row.video_id, verdict, why || undefined);
			toast.success(t(verdict === 'approve' ? 'review.approved' : status === 'reverted' ? 'review.taken_down' : 'review.rejected'));
		} catch (e) {
			const code = String(e);
			notice = t(reviewErrorKey(code));
			if (!isGone(code)) {
				items = restored(items, row);
				count();
			}
		} finally {
			busy = false;
		}
	}

	/** Into the editor with the song that is playing. A pending submission goes with it, and the
	 *  editor's send becomes the approval; a reported live lyric is edited like any other, and the
	 *  edit replacing it takes it out of the queue. */
	const canFix = $derived(!!open && !!detail?.lyrics && onSong && !busy);
	function fix() {
		if (!open || !detail?.lyrics || !canFix) return;
		limusic.reviewAt = items.findIndex((i) => i.revision === open!.revision);
		limusic.fixing =
			open.kind === 'reported'
				? null
				: { revision: open.revision, videoId: open.video_id, author: open.author, lines: detail.lyrics.lines };
		goto('/lyrics-editor');
	}

	/** Reasons a reviewer gives most, one click each, in the reviewer's language. */
	const PRESETS = ['review.preset_timing', 'review.preset_words', 'review.preset_missing', 'review.preset_wrong_song'] as const;

	// --- Keys -------------------------------------------------------------------------------------
	// None collide with the app's own: every global shortcut but Space and `;` hangs off Ctrl or Cmd.
	function step(by: number) {
		const at = open ? items.findIndex((i) => i.revision === open!.revision) : -1;
		const to = items[at < 0 ? 0 : at + by];
		if (to) openItem(to);
	}
	function onKey(e: KeyboardEvent) {
		if (!open || e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
		if (typing(e.target)) return;
		if (e.target instanceof HTMLElement && e.target.closest('[role=dialog],[role=alertdialog],[role=menu],[role=listbox]')) return;
		const k = keyOf(e);
		if (k === 'ArrowDown') step(1);
		else if (k === 'ArrowUp') step(-1);
		// A held key is one decision: its repeats would approve the next one unread.
		else if (e.repeat || rejecting || busy || !detail) return;
		else if (k === 'a') decide('approve');
		else if (k === 'r') rejecting = true;
		else if (k === 'e') fix();
		else return;
		e.preventDefault();
	}

	// --- The Lines view ---------------------------------------------------------------------------
	/** Song time in ms while this song plays, else -1. ponytail: the 4 Hz position tick, no frame
	 *  clock: the list only marks the line and the words begun, the Player view sweeps. */
	const posMs = $derived(onSong ? playback.position * 1000 : -1);
	const at = $derived.by(() => {
		const ls = shown?.lines ?? [];
		let i = -1;
		for (let j = 0; j < ls.length && (ls[j].time_ms ?? Infinity) <= posMs; j++) i = j;
		return i;
	});
	/** Two singers or more: each line says whose it is. */
	const voices = $derived(new Set(shown?.lines.map((l) => l.agent ?? 'v1')).size > 1);
	const agentLabel = (a: string) =>
		a === 'v1000' ? t('lyrics_editor.singer_all') : t('lyrics_editor.singer_n', { n: Number(a.slice(1)) || 1 });
	const stamp = (ms?: number) => {
		if (ms === undefined) return '';
		const s = ms / 1000;
		return `${Math.floor(s / 60)}:${(s % 60).toFixed(2).padStart(5, '0')}`;
	};
	const isBreak = (l: api.LyricLine) => !l.text.trim() && !l.words?.length;
	function breakSecs(i: number) {
		const ls = shown?.lines ?? [];
		const from = ls[i]?.time_ms ?? 0;
		const to = ls[i + 1]?.time_ms ?? ls[i]?.end_time_ms ?? from;
		return new Intl.NumberFormat(currentLocale.id, { maximumFractionDigits: 1 }).format(Math.max(0, to - from) / 1000);
	}
	function seekLine(l: api.LyricLine) {
		if (l.time_ms === undefined || !open) return;
		if (onSong) seekTo(l.time_ms / 1000);
		else play(open);
	}

	/** The list follows the line being sung, unless the reviewer scrolled it in the last few
	 *  seconds. The list's own scroller, never `scrollIntoView`, which scrolls every ancestor (#168). */
	let linesBox = $state<HTMLElement | null>(null);
	let heldAt = 0;
	const hold = () => (heldAt = performance.now());
	$effect(() => {
		const el = linesBox;
		if (!el) return;
		// Passive: a wheel handler that could cancel the scroll makes the engine wait for it.
		el.addEventListener('wheel', hold, { passive: true });
		el.addEventListener('touchmove', hold, { passive: true });
		return () => {
			el.removeEventListener('wheel', hold);
			el.removeEventListener('touchmove', hold);
		};
	});
	$effect(() => {
		const i = at;
		if (view !== 'lines' || !linesBox || i < 0 || performance.now() - heldAt < 4000) return;
		const row = linesBox.querySelector<HTMLElement>(`[data-line="${i}"]`);
		if (!row) return;
		const a = row.getBoundingClientRect();
		const b = linesBox.getBoundingClientRect();
		if (a.top < b.top || a.bottom > b.bottom) linesBox.scrollTop += a.top - b.top - (b.height - a.height) / 3;
	});

	// --- Drawing ---------------------------------------------------------------------------------
	/** The page's own width (a ResizeObserver): wide, the queue stays beside the open submission. */
	let pageW = $state(0);
	const wide = $derived(pageW === 0 || pageW >= 860);
	/** Covers that did not load: the row shows a note instead. */
	let noCover = $state<Record<string, true>>({});
	const when = (item: api.LimusicQueueItem) => ago(item.created_at, Date.now() / 1000, currentLocale.id);
	const length = (secs: number | null) =>
		secs ? `${Math.floor(secs / 60)}:${String(Math.round(secs % 60)).padStart(2, '0')}` : '';
	const shift = (ms: number) =>
		t(ms > 0 ? 'review.change_later' : 'review.change_earlier', {
			secs: new Intl.NumberFormat(currentLocale.id, { minimumFractionDigits: 2, maximumFractionDigits: 2 }).format(Math.abs(ms) / 1000)
		});
	const KEY =
		'inline-flex h-5 min-w-[22px] items-center justify-center rounded-[6px] border border-b-[3px] bg-muted px-1.5 text-[11px] font-semibold text-foreground';
</script>

{#snippet cover(item: api.LimusicQueueItem, size: string, radius = 'rounded-lg')}
	{@const src = coverOf(item.video_id)}
	<span class="flex shrink-0 items-center justify-center overflow-hidden bg-muted text-muted-foreground/20 {radius}" style="width:{size};height:{size}">
		{#if src && !noCover[item.video_id]}
			<img {src} alt="" class="size-full object-cover" draggable="false" loading="lazy" onerror={() => (noCover[item.video_id] = true)} />
		{:else}
			<LogoMark class="size-3/4" />
		{/if}
	</span>
{/snippet}

{#snippet kind(item: api.LimusicQueueItem)}
	<!-- A shape as well as a word: three blocks keyed on the kind, so each icon is fixed. -->
	{#if item.kind === 'new'}
		<span class="flex h-5 shrink-0 items-center gap-1 rounded-full bg-primary/16 px-2 text-[11px] font-semibold text-foreground">
			<HugeiconsIcon icon={Add01Icon} class="size-3" />{t('review.kind_new')}
		</span>
	{:else if item.kind === 'reported'}
		<span class="flex h-5 shrink-0 items-center gap-1 rounded-full bg-destructive/15 px-2 text-[11px] font-semibold text-destructive">
			<HugeiconsIcon icon={Flag02Icon} class="size-3" />{t('review.kind_reported_n', { count: item.reports })}
		</span>
	{:else}
		<span class="flex h-5 shrink-0 items-center gap-1 rounded-full bg-muted px-2 text-[11px] font-semibold text-muted-foreground">
			<HugeiconsIcon icon={PencilEdit02Icon} class="size-3" />{t(kindKey(item.kind))}
		</span>
	{/if}
{/snippet}

{#snippet who(author: string | null)}
	<span class="flex min-w-0 items-center gap-2">
		<span class="flex size-[22px] shrink-0 items-center justify-center rounded-full bg-muted text-[11px] font-bold uppercase">
			{#if author}{author[0]}{:else}<HugeiconsIcon icon={UserIcon} class="size-[13px] text-muted-foreground" />{/if}
		</span>
		<span class="truncate {author ? '' : 'text-muted-foreground'}">{author ?? t('review.anonymous')}</span>
	</span>
{/snippet}

{#snippet lineRows(lyrics: api.Lyrics)}
	<!-- Every line with its time, its words as they are split and timed, and under each word how
	     it is read. No transitions on the rows (docs/UI-PERFORMANCE.md). -->
	<div bind:this={linesBox} class="min-h-0 flex-1 overflow-y-auto p-2">
		{#each lyrics.lines as l, i (i)}
			<button
				type="button"
				data-line={i}
				class="flex w-full cursor-pointer items-start gap-3.5 rounded-[10px] px-3 py-2.5 text-left {i === at ? 'bg-primary/10' : 'hover:bg-muted/40'}"
				onclick={() => seekLine(l)}
			>
				<span class="w-16 shrink-0 pt-1 text-xs tabular-nums {i === at ? 'text-primary' : 'text-muted-foreground'}">{stamp(l.time_ms)}</span>
				<span class="min-w-0 flex-1">
					{#if isBreak(l)}
						<span class="flex items-center gap-2 pt-0.5 text-[13px] text-muted-foreground">
							<HugeiconsIcon icon={MusicNote01Icon} class="size-4" />{t('review.break', { secs: breakSecs(i) })}
						</span>
					{:else if l.words?.length}
						<span class="flex flex-wrap items-end gap-x-[3px] gap-y-0.5">
							{#each l.words as w, k (k)}
								<span class="flex flex-col items-center rounded-md px-[3px] py-px">
									<span
										class="font-heading leading-tight font-bold {w.bg ? 'text-base italic opacity-80' : 'text-xl'} {i === at && w.start_ms > posMs
											? 'text-muted-foreground'
											: ''}">{w.text}</span
									>
									{#if lyricsPrefs.romanized && readings[i]?.[k]}
										<span class="text-[11px] leading-tight font-medium text-muted-foreground">{readings[i][k]}</span>
									{/if}
								</span>
							{/each}
						</span>
					{:else}
						<span class="font-heading text-xl leading-tight font-bold">{l.text}</span>
					{/if}
					{#if lyricsPrefs.romanized && l.romanized}
						<span class="mt-1 block text-[13px] text-muted-foreground">{l.romanized}</span>
					{/if}
				</span>
				{#if voices && !isBreak(l)}
					<span class="mt-1 shrink-0 rounded-full bg-muted px-2 py-px text-[11px] font-semibold text-muted-foreground">{agentLabel(l.agent ?? 'v1')}</span>
				{/if}
			</button>
		{/each}
	</div>
{/snippet}

<div class="flex h-full flex-col" bind:clientWidth={pageW}>
	{#if !open || wide}
		<header class="flex shrink-0 items-center gap-3.5 px-6 pt-5 pb-4">
			<Button variant="ghost" size="icon" class="-ml-2" aria-label={t('common.back')} title={t('common.back')} onclick={() => history.back()}>
				<HugeiconsIcon icon={ArrowLeft01Icon} class="size-[18px]" />
			</Button>
			<div class="min-w-0 flex-1">
				<h1 class="truncate font-heading text-2xl leading-tight font-bold tracking-tight">{t('review.title')}</h1>
				{#if allowed && !loading && !loadError && items.length}
					<p class="truncate text-sm text-muted-foreground">
						{t('review.waiting', { count: waitingLabel(items.length, next !== null) })}
					</p>
				{/if}
			</div>
			{#if admin}
				<Button variant="outline" onclick={() => goto('/team')}>
					<HugeiconsIcon icon={UserGroupIcon} class="size-4" />
					{t('team.title')}
				</Button>
			{/if}
		</header>
	{/if}

	{#if notice}
		<div class="px-6 pb-3">
			<LimusicNotice icon={InformationCircleIcon} title={notice}>
				{#snippet actions()}
					<Button size="sm" variant="ghost" onclick={() => (notice = '')}>{t('common.close')}</Button>
				{/snippet}
			</LimusicNotice>
		</div>
	{/if}

	{#if !allowed}
		<div class="flex flex-1 flex-col items-center justify-center gap-1.5 border-t px-6 text-center">
			<span class="mb-1 flex size-12 items-center justify-center rounded-[13px] bg-muted"><HugeiconsIcon icon={LockIcon} class="size-[22px]" /></span>
			<p class="font-heading text-base font-semibold">{t('review.not_allowed_title')}</p>
			<p class="text-[13px] text-muted-foreground">{t('review.not_allowed')}</p>
		</div>
	{:else if loading}
		<div class="border-t">
			{#each Array(5) as _, i (i)}
				<div class="flex items-center gap-3.5 border-b border-border/60 px-6 py-2.5">
					<Skeleton class="size-12 rounded-lg" />
					<div class="flex-1 space-y-2"><Skeleton class="h-4 w-1/3 rounded" /><Skeleton class="h-3 w-1/5 rounded" /></div>
					<Skeleton class="h-5 w-16 rounded-full" />
				</div>
			{/each}
		</div>
	{:else if loadError}
		<div class="border-t px-6"><ErrorState message={loadError} onRetry={start} /></div>
	{:else if !items.length && !open}
		<div class="flex flex-1 flex-col items-center justify-center gap-1.5 border-t px-6 text-center">
			<span class="mb-1 flex size-12 items-center justify-center rounded-[13px] bg-muted"><HugeiconsIcon icon={TaskDone01Icon} class="size-[22px]" /></span>
			<p class="font-heading text-base font-semibold">{t('review.empty_title')}</p>
			<p class="text-[13px] text-muted-foreground">{t('review.empty')}</p>
		</div>
	{:else if !open}
		<!-- The queue. No transitions on the rows (docs/UI-PERFORMANCE.md). -->
		<div class="min-h-0 flex-1 overflow-y-auto border-t">
			{#each items as item (item.revision)}
				<button
					type="button"
					class="flex w-full cursor-pointer items-center gap-3.5 border-b border-border/60 px-6 py-2.5 text-left hover:bg-muted/50"
					onclick={() => openItem(item)}
				>
					{@render cover(item, '3rem')}
					<span class="min-w-0 flex-1">
						<span class="block truncate text-[15px] font-semibold">{item.title}</span>
						<span class="block truncate text-[13px] text-muted-foreground">{item.artist}</span>
					</span>
					<span class="flex shrink-0 {wide ? 'w-28' : ''}">{@render kind(item)}</span>
					{#if wide}
						<span class="w-40 shrink-0 text-[13px]">{@render who(item.author)}</span>
						<span class="w-28 shrink-0 text-right text-[13px] text-muted-foreground">{when(item)}</span>
					{/if}
				</button>
			{/each}
			{#if next !== null}
				<div class="px-6 py-3"><Button size="sm" variant="outline" onclick={() => load(true)}>{t('review.load_more')}</Button></div>
			{/if}
		</div>
	{:else}
		<div class="flex min-h-0 flex-1 border-t">
			{#if wide}
				<!-- The queue stays in reach while one is open. -->
				<div class="w-[300px] shrink-0 overflow-y-auto border-r p-1.5">
					{#each items as item (item.revision)}
						<button
							type="button"
							aria-current={item.revision === open.revision}
							class="flex w-full cursor-pointer items-center gap-2.5 rounded-[10px] p-2 text-left {item.revision === open.revision
								? 'bg-primary/16'
								: 'hover:bg-muted/50'}"
							onclick={() => openItem(item)}
						>
							{@render cover(item, '2.75rem', 'rounded-md')}
							<span class="min-w-0 flex-1">
								<span class="block truncate text-sm font-semibold">{item.title}</span>
								<span class="mt-0.5 flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
									{@render kind(item)}<span class="truncate">{item.author ?? t('review.anonymous')} · {when(item)}</span>
								</span>
							</span>
						</button>
					{/each}
					{#if next !== null}
						<div class="p-2"><Button size="sm" variant="outline" onclick={() => load(true)}>{t('review.load_more')}</Button></div>
					{/if}
				</div>
			{/if}
			<div class="flex min-w-0 flex-1 flex-col">
				<!-- The song. Same wash as the album page: a small thumbnail blurred once, with its fade. -->
				<div class="relative flex shrink-0 flex-wrap items-center gap-x-4 gap-y-3 px-6 pt-4 pb-3">
					{#if coverOf(open.video_id) && !noCover[open.video_id]}
						<div class="art-wash absolute inset-0 overflow-hidden">
							<img src={coverOf(open.video_id)} alt="" class="absolute inset-0 h-full w-full scale-125 object-cover opacity-50 blur-2xl" />
							<div class="absolute inset-0 bg-gradient-to-t from-background via-background/75 to-background/40"></div>
						</div>
					{/if}
					{#if !wide}
						<Button variant="ghost" size="icon" class="relative -ml-2" aria-label={t('common.back')} onclick={close}>
							<HugeiconsIcon icon={ArrowLeft01Icon} class="size-[18px]" />
						</Button>
					{/if}
					<span class="relative shadow-lg">{@render cover(open, wide ? '5.25rem' : '3.5rem', 'rounded-[10px]')}</span>
					<div class="relative min-w-60 flex-1">
						<h2 class="truncate font-heading text-2xl leading-tight font-bold tracking-tight">{open.title}</h2>
						<p class="truncate text-sm text-muted-foreground">
							{open.artist}{#if open.duration}<span class="ml-2 tabular-nums">{length(open.duration)}</span>{/if}
						</p>
						<div class="mt-2 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[13px] text-muted-foreground">
							{@render kind(open)}
							<span class="text-foreground">{@render who(open.author)}</span>
							{#if detail?.author_record}
								<span>{t('review.record', { approved: detail.author_record.approved, rejected: detail.author_record.rejected })}</span>
							{/if}
							<span>{t('review.sent_ago', { when: when(open) })}</span>
						</div>
					</div>
					<div class="relative flex flex-wrap items-center gap-x-4 gap-y-2">
						<Tabs.Root value={view} onValueChange={(v) => (view = v as View)}>
							<Tabs.List>
								<Tabs.Trigger value="lines" class="px-3">{t('review.view_lines')}</Tabs.Trigger>
								<Tabs.Trigger value="player" class="px-3">{t('review.view_player')}</Tabs.Trigger>
								{#if open.kind === 'edit'}
									<Tabs.Trigger value="changes" class="px-3">{t('review.changes')}</Tabs.Trigger>
									<Tabs.Trigger value="current" class="px-3">{t('review.current')}</Tabs.Trigger>
								{/if}
							</Tabs.List>
						</Tabs.Root>
						{#if canRead && view !== 'changes'}
							<label class="flex cursor-pointer items-center gap-2 text-[13px] select-none" title={t('lyrics.romanize_hint')}>
								<Switch checked={lyricsPrefs.romanized} onCheckedChange={() => toggleLyricsPref('romanized')} />
								{t('lyrics.romanize')}
							</label>
						{/if}
					</div>
				</div>
				{#if open.kind === 'reported'}
					<p class="px-6 pb-2.5 text-[13px] text-muted-foreground">{t('review.reported_hint', { count: open.reports })}</p>
				{/if}
				{#if detail?.note}
					<p class="mx-6 mb-2.5 rounded-lg bg-muted/40 px-3 py-2 text-[13.5px]">
						<span class="text-muted-foreground">{t('review.note')}</span>
						{detail.note}
					</p>
				{/if}
				{#if open.kind === 'edit' && view === 'changes' && summary}
					<p class="px-6 pb-2.5 text-[13px] text-muted-foreground">{summary}</p>
				{/if}

				<div class="mx-6 flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border bg-card">
					{#if detailError}
						<div class="p-4"><LimusicNotice icon={InformationCircleIcon} title={detailError} class="border-0 bg-transparent p-0" /></div>
					{:else if !detail || (view === 'current' && current === undefined) || (view === 'changes' && current === undefined)}
						<div class="space-y-3.5 p-6">
							{#each [60, 78, 52, 70] as w (w)}<Skeleton class="h-6 rounded" style="width:{w}%" />{/each}
						</div>
					{:else if view === 'changes'}
						{#if changes?.length}
							<!-- Only the lines that differ. -->
							<div class="min-h-0 flex-1 overflow-y-auto p-2 text-sm">
								{#each changes as c (c.kind + c.n)}
									<div class="grid grid-cols-[1.75rem_1fr_auto] items-baseline gap-x-2.5 rounded-lg px-2 py-1.5 odd:bg-muted/40">
										<span class="text-right text-xs text-muted-foreground tabular-nums">{c.n}</span>
										<span class="min-w-0">
											{#if c.kind === 'reworded'}
												<del class="block text-muted-foreground decoration-destructive">{c.old}</del>
												<ins class="rounded bg-primary/20 px-1 font-semibold no-underline">{c.text}</ins>
											{:else if c.kind === 'added'}
												<ins class="rounded bg-primary/20 px-1 font-semibold no-underline">{c.text}</ins>
											{:else if c.kind === 'removed'}
												<del class="text-muted-foreground decoration-destructive">{c.text}</del>
											{:else}
												{c.text}
											{/if}
										</span>
										<span class="text-xs text-muted-foreground">
											{c.kind === 'retimed'
												? c.shiftMs !== undefined
													? shift(c.shiftMs)
													: t('review.change_words')
												: t(`review.change_${c.kind}`)}
										</span>
									</div>
								{/each}
							</div>
						{:else}
							<p class="p-6 text-sm text-muted-foreground">{current ? t('review.no_changes') : t('review.no_current')}</p>
						{/if}
					{:else if shown && view === 'lines'}
						{@render lineRows(shown)}
					{:else if shown}
						<div class="flex min-h-0 flex-1 flex-col pt-2"><LyricsView expanded preview={shown} /></div>
					{:else}
						<p class="p-6 text-sm text-muted-foreground">
							{view === 'current' ? t('review.no_current') : t('review.no_lyrics')}
						</p>
					{/if}
				</div>
				{#if view === 'lines' && shown && !detailError}
					<p class="mx-6 mt-2 text-xs text-muted-foreground">{t('review.lines_hint')}</p>
				{/if}

				<!-- The decision: a fixed bar, never scrolled away. -->
				<div class="mt-3 flex shrink-0 flex-wrap items-center gap-2.5 border-t bg-card px-6 py-3">
					{#if rejecting}
						<form
							class="flex min-w-0 flex-1 flex-col gap-2.5"
							onsubmit={(e) => {
								e.preventDefault();
								decide('reject');
							}}
						>
							<div class="flex flex-wrap items-center gap-1.5">
								{#each PRESETS as key (key)}
									<button
										type="button"
										class="h-7 cursor-pointer rounded-full border px-3 text-xs font-medium hover:bg-muted {reason === t(key) ? 'border-primary bg-primary/12' : ''}"
										onclick={() => (reason = t(key))}
									>
										{t(key)}
									</button>
								{/each}
							</div>
							<div class="flex min-w-0 items-center gap-2.5">
								<!-- svelte-ignore a11y_autofocus -->
								<input
									bind:value={reason}
									autofocus
									maxlength="300"
									placeholder={t('review.reason')}
									aria-label={t('review.reason')}
									class="min-w-0 flex-1 border-b border-border bg-transparent py-1 text-sm outline-none focus:border-primary"
									onkeydown={(e) => e.key === 'Escape' && (rejecting = false)}
								/>
								<Button type="button" variant="ghost" onclick={() => (rejecting = false)}>{t('common.cancel')}</Button>
								<Button type="submit" size="lg" variant="destructive" disabled={busy}>
									<HugeiconsIcon icon={Cancel01Icon} class="size-4" />
									{open.kind === 'reported' ? t('review.take_down') : t('review.reject')}
								</Button>
							</div>
						</form>
					{:else}
						<div class="flex min-w-0 flex-1 flex-wrap items-center gap-x-3.5 gap-y-1 text-[12.5px] text-muted-foreground">
							{#if wide}
								<span class="flex items-center gap-1.5"><kbd class={KEY}>A</kbd>{open.kind === 'reported' ? t('review.keep_live') : t('review.approve')}</span>
								<span class="flex items-center gap-1.5"><kbd class={KEY}>R</kbd>{open.kind === 'reported' ? t('review.take_down') : t('review.reject')}</span>
								<span class="flex items-center gap-1.5"><kbd class={KEY}>E</kbd>{t('review.key_fix')}</span>
								<span class="flex items-center gap-1.5"><kbd class={KEY}>↑</kbd><kbd class={KEY}>↓</kbd>{t('review.key_next')}</span>
							{:else}
								<span class="truncate">{t('review.playing_hint')}</span>
							{/if}
						</div>
						<Button
							size="lg"
							variant="outline"
							disabled={!canFix}
							title={onSong ? undefined : t('review.fix_wait')}
							onclick={fix}
						>
							<HugeiconsIcon icon={PencilEdit02Icon} class="size-4" />
							{open.kind === 'reported' ? t('review.fix_live') : t('review.fix')}
						</Button>
						<!-- The buttons say what they do: for a reported lyric that is take down or keep. -->
						{#if open.kind === 'reported'}
							<Button size="lg" variant="destructive" disabled={busy || !detail} onclick={() => (rejecting = true)}>
								<HugeiconsIcon icon={Delete02Icon} class="size-4" />
								{t('review.take_down')}
							</Button>
							<Button size="lg" disabled={busy || !detail} onclick={() => decide('approve')}>
								<HugeiconsIcon icon={Tick02Icon} class="size-4" />
								{t('review.keep_live')}
							</Button>
						{:else}
							<Button size="lg" variant="outline" disabled={busy || !detail} onclick={() => (rejecting = true)}>
								<HugeiconsIcon icon={Cancel01Icon} class="size-4" />
								{t('review.reject')}
							</Button>
							<Button size="lg" disabled={busy || !detail} onclick={() => decide('approve')}>
								<HugeiconsIcon icon={Tick02Icon} class="size-4" />
								{t('review.approve')}
							</Button>
						{/if}
					{/if}
				</div>
			</div>
		</div>
	{/if}
</div>
