<script lang="ts">
	// Review lyrics: what is waiting for a reviewer, oldest first. Opening one plays the song and
	// shows the submitted lyric with the same view the editor previews in, so the sweep is judged
	// as listeners will see it. The queue stays beside it, and a decision moves on to the next one.
	// The player bar is the only transport. Rules are in `$lib/review.ts`.
	import { onMount } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Add01Icon,
		ArrowLeft01Icon,
		Cancel01Icon,
		Delete02Icon,
		Flag02Icon,
		InformationCircleIcon,
		LockIcon,
		PencilEdit02Icon,
		TaskDone01Icon,
		Tick02Icon,
		UserIcon
	} from '@hugeicons/core-free-icons';
	import LogoMark from '$lib/components/LogoMark.svelte';
	import { Button } from '$lib/components/ui/button';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import * as Tabs from '$lib/components/ui/tabs';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import LimusicNotice from '$lib/components/LimusicNotice.svelte';
	import LyricsView from '$lib/components/LyricsView.svelte';
	import * as api from '$lib/api';
	import { playback, seekTo, toast } from '$lib/player.svelte';
	import { limusic, refreshAccount } from '$lib/limusic.svelte';
	import { currentLocale, t } from '$lib/i18n.svelte';
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

	/** Who this is, then the queue. Also the retry. */
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
	}
	onMount(start);

	// --- One submission ---------------------------------------------------------------------------
	type Side = 'proposed' | 'current' | 'changes';
	let open = $state<api.LimusicQueueItem | null>(null);
	let detail = $state.raw<api.LimusicRevision | null>(null);
	let detailError = $state('');
	/** What is live now, for an edit. `undefined` while it loads, null when there is nothing. */
	let current = $state.raw<api.Lyrics | null | undefined>(undefined);
	let side = $state<Side>('proposed');
	let rejecting = $state(false);
	let reason = $state('');
	let busy = $state(false);

	const shown = $derived(side === 'current' ? (current ?? null) : (detail?.lyrics ?? null));
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

	async function openItem(item: api.LimusicQueueItem) {
		open = item;
		detail = null;
		detailError = '';
		current = item.kind === 'edit' ? undefined : null;
		side = 'proposed';
		rejecting = false;
		reason = '';
		notice = '';
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
		if (!row || busy) return;
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
		<span class="flex h-5 shrink-0 items-center gap-1 rounded-full bg-primary/16 px-2 text-[11px] font-semibold">
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
							{@render cover(item, '2.625rem', 'rounded-md')}
							<span class="min-w-0 flex-1">
								<span class="block truncate text-sm font-semibold">{item.title}</span>
								<span class="mt-0.5 flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
									{@render kind(item)}<span class="truncate">{when(item)}</span>
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
				<div class="relative flex shrink-0 items-center gap-4 px-6 pt-4 pb-3">
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
					<div class="relative min-w-0 flex-1">
						<h2 class="truncate font-heading text-2xl leading-tight font-bold tracking-tight">{open.title}</h2>
						<p class="truncate text-sm text-muted-foreground">
							{open.artist}{#if open.duration}<span class="ml-2 tabular-nums">{length(open.duration)}</span>{/if}
						</p>
						<div class="mt-2 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[13px] text-muted-foreground">
							{@render kind(open)}
							<span class="text-foreground">{@render who(open.author)}</span>
							<span>{t('review.sent_ago', { when: when(open) })}</span>
						</div>
					</div>
				</div>
				{#if open.kind === 'reported'}
					<p class="px-6 pb-2.5 text-[13px] text-muted-foreground">{t('review.reported_hint', { count: open.reports })}</p>
				{/if}
				{#if detail?.note}
					<p class="mx-6 mb-2.5 rounded-r-lg border-l-[3px] bg-muted/40 px-3 py-2 text-[13.5px]">
						<span class="text-muted-foreground">{t('review.note')}</span>
						{detail.note}
					</p>
				{/if}
				{#if open.kind === 'edit'}
					<div class="flex flex-wrap items-center gap-x-3 gap-y-1.5 px-6 pb-2.5">
						<Tabs.Root value={side} onValueChange={(v) => (side = v as Side)}>
							<Tabs.List>
								<Tabs.Trigger value="proposed" class="px-3">{t('review.proposed')}</Tabs.Trigger>
								<Tabs.Trigger value="current" class="px-3">{t('review.current')}</Tabs.Trigger>
								<Tabs.Trigger value="changes" class="px-3">{t('review.changes')}</Tabs.Trigger>
							</Tabs.List>
						</Tabs.Root>
						{#if summary}<span class="text-[13px] text-muted-foreground">{summary}</span>{/if}
					</div>
				{/if}

				<div class="mx-6 flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border bg-card">
					{#if detailError}
						<div class="p-4"><LimusicNotice icon={InformationCircleIcon} title={detailError} class="border-0 bg-transparent p-0" /></div>
					{:else if !detail || (side !== 'proposed' && current === undefined)}
						<div class="space-y-3.5 p-6">
							{#each [60, 78, 52, 70] as w (w)}<Skeleton class="h-6 rounded" style="width:{w}%" />{/each}
						</div>
					{:else if side === 'changes'}
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
					{:else if shown}
						<div class="flex min-h-0 flex-1 flex-col pt-2"><LyricsView expanded preview={shown} /></div>
					{:else}
						<p class="p-6 text-sm text-muted-foreground">
							{side === 'current' ? t('review.no_current') : t('review.no_lyrics')}
						</p>
					{/if}
				</div>

				<!-- The decision: a fixed bar, never scrolled away. -->
				<div class="mt-3 flex shrink-0 flex-wrap items-center gap-2.5 border-t bg-card px-6 py-3">
					{#if rejecting}
						<form
							class="flex min-w-0 flex-1 items-center gap-2.5"
							onsubmit={(e) => {
								e.preventDefault();
								decide('reject');
							}}
						>
							<!-- svelte-ignore a11y_autofocus -->
							<input
								bind:value={reason}
								autofocus
								maxlength="300"
								placeholder={t('review.reason')}
								aria-label={t('review.reason')}
								class="min-w-0 flex-1 border-b border-border bg-transparent py-1 text-sm outline-none focus:border-primary"
							/>
							<Button type="button" variant="ghost" onclick={() => (rejecting = false)}>{t('common.cancel')}</Button>
							<Button type="submit" size="lg" variant="destructive" disabled={busy}>
								<HugeiconsIcon icon={Cancel01Icon} class="size-4" />
								{open.kind === 'reported' ? t('review.take_down') : t('review.reject')}
							</Button>
						</form>
					{:else}
						<span class="min-w-0 flex-1 truncate text-[13px] text-muted-foreground">{t('review.playing_hint')}</span>
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
