<script lang="ts">
	// Review lyrics: what is waiting for a reviewer, oldest first. Opening one plays the song and
	// shows the submitted lyric with the same view the editor previews in, so the sweep is judged
	// as listeners will see it. The player bar is the only transport. Rules are in `$lib/review.ts`.
	import { onMount } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Alert02Icon, ArrowLeft01Icon, Tick02Icon, Cancel01Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import * as Tabs from '$lib/components/ui/tabs';
	import LyricsView from '$lib/components/LyricsView.svelte';
	import * as api from '$lib/api';
	import { playback, seekTo, toast } from '$lib/player.svelte';
	import { limusic, refreshAccount } from '$lib/limusic.svelte';
	import { currentLocale, t } from '$lib/i18n.svelte';
	import { ago, isGone, kindKey, restored, reviewErrorKey, waitingLabel, without } from '$lib/review';

	const CARD = 'rounded-xl border bg-card';
	const BADGE = 'shrink-0 rounded-full border bg-muted/50 px-2 py-0.5 text-[11px] font-medium text-muted-foreground';

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

	onMount(async () => {
		if (!limusic.account) await refreshAccount();
		if (!limusic.account?.can_review) {
			allowed = false;
			loading = false;
			return;
		}
		await load();
	});

	// --- One submission ---------------------------------------------------------------------------
	let open = $state<api.LimusicQueueItem | null>(null);
	let detail = $state.raw<api.LimusicRevision | null>(null);
	let detailError = $state('');
	/** What is live now, for an edit. `undefined` while it loads, null when there is nothing. */
	let current = $state.raw<api.Lyrics | null | undefined>(undefined);
	let side = $state<'proposed' | 'current'>('proposed');
	let rejecting = $state(false);
	let reason = $state('');
	let busy = $state(false);

	const shown = $derived(side === 'current' ? (current ?? null) : (detail?.lyrics ?? null));

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

	/** The row leaves the list at once. If the service refuses, it comes back with the reason. */
	async function decide(verdict: 'approve' | 'reject') {
		const row = open;
		if (!row || busy) return;
		busy = true;
		const why = verdict === 'reject' ? reason.trim() : '';
		items = without(items, row.revision);
		count();
		close();
		try {
			const status = await api.limusicReview(row.revision, row.video_id, verdict, why || undefined);
			notice = '';
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
</script>

<div class="mx-auto flex h-full max-w-5xl flex-col gap-4 p-6">
	<header class="flex flex-wrap items-center gap-x-3 gap-y-2">
		<Button variant="ghost" size="sm" onclick={() => (open ? close() : history.back())}>
			<HugeiconsIcon icon={ArrowLeft01Icon} class="h-4 w-4" />
			{t('common.back')}
		</Button>
		<div class="min-w-0 flex-1">
			<h1 class="truncate font-heading text-lg font-semibold">
				{open ? open.title : t('review.title')}
			</h1>
			<p class="truncate text-sm text-muted-foreground">
				{#if open}
					{open.artist}
				{:else if allowed && !loading && !loadError}
					{t('review.waiting', { count: waitingLabel(items.length, next !== null) })}
				{/if}
			</p>
		</div>
	</header>

	{#if notice}
		<div class="{CARD} flex items-center gap-3 px-4 py-3 text-sm" role="status">
			<HugeiconsIcon icon={Alert02Icon} class="h-4 w-4 shrink-0 text-muted-foreground" />
			<p class="min-w-0 flex-1 leading-relaxed">{notice}</p>
			<Button size="sm" variant="ghost" onclick={() => (notice = '')}>{t('common.close')}</Button>
		</div>
	{/if}

	{#if !allowed}
		<p class="text-sm text-muted-foreground">{t('review.not_allowed')}</p>
	{:else if open}
		<div class="flex flex-wrap items-center gap-x-3 gap-y-2 text-sm text-muted-foreground">
			<span class={BADGE}>{t(kindKey(open.kind))}</span>
			<span>{t('review.sent_by', { who: open.author ?? t('review.anonymous') })}</span>
			<span>{ago(open.created_at, Date.now() / 1000, currentLocale.id)}</span>
		</div>
		{#if open.kind === 'reported'}
			<p class="text-sm text-muted-foreground">{t('review.reported_hint', { count: open.reports })}</p>
		{/if}
		{#if detail?.note}
			<p class="text-sm"><span class="text-muted-foreground">{t('review.note')}</span> {detail.note}</p>
		{/if}

		<div class="flex flex-wrap items-center gap-3">
			{#if open.kind === 'edit'}
				<Tabs.Root value={side} onValueChange={(v) => (side = v as 'proposed' | 'current')}>
					<Tabs.List>
						<Tabs.Trigger value="proposed" class="px-3">{t('review.proposed')}</Tabs.Trigger>
						<Tabs.Trigger value="current" class="px-3">{t('review.current')}</Tabs.Trigger>
					</Tabs.List>
				</Tabs.Root>
			{/if}
			<div class="flex-1"></div>
			{#if rejecting}
				<form
					class="flex min-w-0 flex-1 items-center justify-end gap-2"
					onsubmit={(e) => {
						e.preventDefault();
						decide('reject');
					}}
				>
					<input
						bind:value={reason}
						maxlength="300"
						placeholder={t('review.reason')}
						aria-label={t('review.reason')}
						class="min-w-0 max-w-xs flex-1 border-b border-border bg-transparent py-0.5 text-sm outline-none focus:border-primary"
					/>
					<Button type="submit" size="sm" variant="destructive" disabled={busy}>{t('review.reject')}</Button>
					<Button type="button" size="sm" variant="ghost" onclick={() => (rejecting = false)}>
						{t('common.cancel')}
					</Button>
				</form>
			{:else}
				<Button size="sm" variant="outline" disabled={busy || !detail} onclick={() => (rejecting = true)}>
					<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
					{t('review.reject')}
				</Button>
				<Button size="sm" disabled={busy || !detail} onclick={() => decide('approve')}>
					<HugeiconsIcon icon={Tick02Icon} class="h-4 w-4" />
					{t('review.approve')}
				</Button>
			{/if}
		</div>

		{#if detailError}
			<p class="text-sm text-muted-foreground">{detailError}</p>
		{:else if !detail || (side === 'current' && current === undefined)}
			<p class="text-sm text-muted-foreground">{t('common.loading')}</p>
		{:else if shown}
			<div class="{CARD} flex min-h-0 flex-1 flex-col overflow-hidden pt-2">
				<LyricsView expanded preview={shown} />
			</div>
		{:else}
			<p class="text-sm text-muted-foreground">
				{side === 'current' ? t('review.no_current') : t('review.no_lyrics')}
			</p>
		{/if}
	{:else if loading}
		<p class="text-sm text-muted-foreground">{t('common.loading')}</p>
	{:else if loadError}
		<div class="flex items-center gap-3 text-sm text-muted-foreground">
			<p>{loadError}</p>
			<Button size="sm" variant="outline" onclick={() => load()}>{t('common.retry')}</Button>
		</div>
	{:else if !items.length}
		<p class="text-sm text-muted-foreground">{t('review.empty')}</p>
	{:else}
		<p class="text-sm text-muted-foreground">{t('review.intro')}</p>
		<div class="{CARD} min-h-0 divide-y divide-border/60 overflow-y-auto">
			{#each items as item (item.revision)}
				<button
					type="button"
					class="flex w-full cursor-pointer items-center gap-3 px-4 py-3 text-left hover:bg-muted/50"
					onclick={() => openItem(item)}
				>
					<span class="min-w-0 flex-1">
						<span class="block truncate text-sm font-medium">{item.title}</span>
						<span class="block truncate text-xs text-muted-foreground">{item.artist}</span>
					</span>
					<span class={BADGE}>{t(kindKey(item.kind))}</span>
					<span class="hidden w-36 shrink-0 truncate text-sm text-muted-foreground sm:block">
						{item.author ?? t('review.anonymous')}
					</span>
					<span class="w-28 shrink-0 text-right text-xs text-muted-foreground">
						{ago(item.created_at, Date.now() / 1000, currentLocale.id)}
					</span>
				</button>
			{/each}
		</div>
		{#if next !== null}
			<div>
				<Button size="sm" variant="outline" onclick={() => load(true)}>{t('review.load_more')}</Button>
			</div>
		{/if}
	{/if}
</div>
