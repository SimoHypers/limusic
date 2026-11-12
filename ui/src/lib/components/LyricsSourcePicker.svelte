<script lang="ts">
	// The lyrics footer's source switch (#23): every provider's answer for this song side by side,
	// one click to keep one, a timing nudge, and a search for when YouTube's title is what went
	// wrong. A pick lands on the lyrics behind the popover straight away, so they're the preview.
	import { tick, untrack } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		ArrowRight01Icon,
		ArrowTurnBackwardIcon,
		ArrowUp01Icon,
		Flag02Icon,
		LinkSquare02Icon,
		Loading03Icon,
		MagicWand01Icon,
		MinusSignIcon,
		PencilEdit02Icon,
		PinIcon,
		PlusSignIcon,
		Search01Icon,
		Settings02Icon,
		StarIcon,
		Tap01Icon,
		ThumbsDownIcon,
		ThumbsUpIcon,
		Tick02Icon,
		Timer01Icon,
		TranslateIcon
	} from '@hugeicons/core-free-icons';
	import * as Popover from '$lib/components/ui/popover';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import * as api from '$lib/api';
	import { playback, toast, ui } from '$lib/player.svelte';
	import { t, currentLocale } from '$lib/i18n.svelte';
	import { KIND_CLASS, KIND_LABEL, lyricsKind } from '$lib/lyricsSources';
	import { goto } from '$app/navigation';
	import { limusic } from '$lib/limusic.svelte';
	import { REPORT_REASONS, creditNames, feedbackErrorKey, nextVote, withVote } from '$lib/limusic';
	import { link } from '$lib/link.svelte';
	import { draftKey, unpackDraft, wordsTimed } from '$lib/lyricsedit';

	let {
		lyrics,
		track,
		open = $bindable(false),
		onchange
	}: {
		lyrics: api.Lyrics | null;
		track: api.LyricsTrack;
		open?: boolean;
		/** New lyrics for `videoId`; the view drops them if the song has moved on. */
		onchange: (lyrics: api.Lyrics | null, videoId: string) => void;
	} = $props();

	/** One step of the timing nudge. A quarter second is under what anyone hears as early. */
	const STEP_MS = 250;
	const MAX_OFFSET_MS = 15_000;

	type Result =
		| { status: 'idle' | 'loading' | 'error' }
		| { status: 'done'; lyrics: api.Lyrics | null };

	let providers = $state<api.LyricsProvider[]>([]);
	let results = $state<Record<string, Result>>({});
	/** Which song and query `results` belong to. A late answer for another one is dropped. */
	let resultsFor = '';
	/** A title and artist typed by hand, in place of the song's own. */
	let query = $state<{ title: string; artists: string } | null>(null);
	let searchOpen = $state(false);
	let draftTitle = $state('');
	let draftArtists = $state('');
	let titleInput = $state<HTMLInputElement | null>(null);
	/** The row being kept ('auto' for Automatic) while Rust stores the choice. */
	let busy = $state<string | null>(null);

	const pinned = $derived(!!lyrics?.pinned);
	const offset = $derived(lyrics?.offset_ms ?? 0);
	// A typed title is a different song as far as the providers know, so the album goes; the length
	// stays, it's still the recording that's playing.
	const request = $derived(query ? { ...track, ...query, album: undefined, searched: true } : track);
	/** A community lyric is credited to the people who synced it; anything else to its source. */
	const attribution = $derived(
		!lyrics
			? t('lyrics.find')
			: lyrics.authors?.length
				? t('lyrics.synced_by', { names: creditNames(lyrics.authors, currentLocale.id) })
				: lyrics.source.startsWith('Source:')
					? lyrics.source
					: t('lyrics.source', { source: lyrics.source })
	);
	const offsetLabel = $derived(
		t('lyrics.timing_value', {
			value: new Intl.NumberFormat(currentLocale.id, {
				minimumFractionDigits: 2,
				maximumFractionDigits: 2,
				signDisplay: 'exceptZero'
			}).format(offset / 1000)
		})
	);

	// A new song closes the picker and forgets the last one's answers and search.
	let shownFor = '';
	$effect(() => {
		const id = track.videoId;
		if (id === shownFor) return;
		shownFor = id;
		open = false;
		query = null;
		searchOpen = false;
		resultsFor = '';
		ratingFor = '';
		rating = null;
	});

	// Opened on a song nothing was found for, every provider has already said no: the title
	// search is the useful part, so it opens with it.
	$effect(() => {
		if (!open) return;
		untrack(() => {
			refresh();
			readDraft();
			if (!lyrics && !query) openSearch();
		});
	});

	/** Ask every provider that's on, once per song and query. The one on screen already answered. */
	async function refresh() {
		providers = await api.lyricsProviders();
		const key = `${request.videoId}\n${query?.title ?? ''}\n${query?.artists ?? ''}`;
		if (key === resultsFor) return;
		resultsFor = key;
		results = {};
		for (const p of providers) {
			if (!query && lyrics?.provider === p.id) results[p.id] = { status: 'done', lyrics };
			else results[p.id] = { status: 'idle' };
			if (p.on && results[p.id].status === 'idle') check(p.id);
		}
	}

	function check(id: string) {
		const key = resultsFor;
		results[id] = { status: 'loading' };
		api
			.getLyrics({ ...request, source: id })
			.then((l) => {
				if (resultsFor === key) results[id] = { status: 'done', lyrics: l };
			})
			.catch(() => {
				if (resultsFor === key) results[id] = { status: 'error' };
			});
	}

	/** Keep a provider's lyrics for this song (`null`: back to Automatic). Shown before Rust has
	 *  stored it: the preview already is those lyrics. */
	async function choose(id: string | null) {
		const videoId = track.videoId;
		const r = id ? results[id] : undefined;
		if (id && (r?.status !== 'done' || !r.lyrics)) return;
		// A nudge still waiting to be saved belongs to the lyrics being replaced.
		clearTimeout(offsetTimer);
		if (r?.status === 'done' && r.lyrics) {
			onchange({ ...r.lyrics, pinned: true, offset_ms: 0 }, videoId);
		}
		busy = id ?? 'auto';
		try {
			// Automatic is the song's own details; a pick is refetched with what found it.
			const l = await api.chooseLyricsSource({ ...(id ? request : track), source: id });
			if (l || !id) onchange(l, videoId);
		} finally {
			busy = null;
		}
	}

	let offsetTimer: ReturnType<typeof setTimeout> | undefined;
	/** Nudge the timing (`by = 0` resets it). Saved once the clicking stops. Not while a pick is
	 *  being stored: that write would land after this one and put the timing back. */
	function nudge(by: number) {
		if (!lyrics || busy) return;
		const ms = by === 0 ? 0 : Math.max(-MAX_OFFSET_MS, Math.min(MAX_OFFSET_MS, offset + by));
		const videoId = track.videoId;
		onchange({ ...lyrics, offset_ms: ms, pinned: true }, videoId);
		clearTimeout(offsetTimer);
		offsetTimer = setTimeout(() => api.setLyricsOffset(videoId, ms), 400);
	}

	async function openSearch() {
		draftTitle = query?.title ?? track.title;
		draftArtists = query?.artists ?? track.artists;
		searchOpen = true;
		await tick();
		titleInput?.select();
	}

	function search(e: SubmitEvent) {
		e.preventDefault();
		const title = draftTitle.trim();
		if (!title) return;
		query = { title, artists: draftArtists.trim() };
		searchOpen = false;
		refresh();
	}

	function clearSearch() {
		query = null;
		searchOpen = false;
		refresh();
	}

	/** The lyric editor, for songs the service can file: not a local file or a broadcast, not while
	 *  the bar controls another device, and not from a typed search, whose lyrics may be another
	 *  song's. Navigating closes the player view and theater mode by themselves. */
	const canEdit = $derived(
		limusic.available &&
			!query &&
			!api.isLocalId(track.videoId) &&
			!playback.now?.live &&
			!link.remote
	);
	const improving = $derived(!!lyrics && lyricsKind(lyrics) === 'words');
	/** A draft of this song left in the editor: how far it got. Read when the picker opens. */
	let draft = $state<{ done: number; total: number } | null>(null);
	function readDraft() {
		let d = null;
		try {
			d = unpackDraft(localStorage.getItem(draftKey(track.videoId)), track.videoId);
		} catch {
			// No storage: no draft to offer.
		}
		const sung = d?.lines.filter((l) => l.words.length) ?? [];
		draft = d ? { done: sung.filter(wordsTimed).length, total: sung.length } : null;
	}
	function openEditor() {
		open = false;
		goto('/lyrics-editor');
	}

	// --- Votes and reports ------------------------------------------------------------------------
	// For the lyric Limusic has live, when that is the one on screen. No sign-in: the first vote or
	// report makes an anonymous account, silently, as the first submit does.
	const rateable = $derived(
		lyrics?.provider === 'limusic' && (limusic.available || limusic.signedIn) && !api.isLocalId(track.videoId)
	);
	/** The votes, mine among them. Null while they load, and when there is nothing to vote on. */
	let rating = $state<api.LimusicLyricState | null>(null);
	let ratingLoading = $state(false);
	/** The song and revision `rating` belongs to. A late answer for another one is dropped. */
	let ratingFor = '';
	let voting = false;
	let reportOpen = $state(false);
	let reportReason = $state<api.LimusicReportReason | null>(null);
	let reportNote = $state('');
	let reported = $state(false);

	$effect(() => {
		if (!open || !rateable) return;
		const videoId = track.videoId;
		const key = `${videoId} ${lyrics?.revision ?? ''}`;
		untrack(() => loadRating(key, videoId));
	});

	async function loadRating(key: string, videoId: string) {
		if (key === ratingFor) return;
		ratingFor = key;
		rating = null;
		reportOpen = false;
		reported = false;
		reportReason = null;
		reportNote = '';
		ratingLoading = true;
		try {
			const s = await api.limusicLyricState(videoId);
			if (ratingFor === key) rating = s;
		} catch {
			// The service is away: nothing to vote on, and nothing to say about it here.
		} finally {
			if (ratingFor === key) ratingLoading = false;
		}
	}

	/** The thumb shows pressed and the count moves at once; a refusal puts both back and says why. */
	async function vote(clicked: -1 | 1) {
		if (!rating || voting) return;
		const before = rating;
		const key = ratingFor;
		const value = nextVote(before.my_vote, clicked);
		rating = withVote(before, value);
		voting = true;
		try {
			const r = await api.limusicVote(track.videoId, value);
			if (ratingFor === key) rating = { ...before, ...r };
		} catch (e) {
			// The service knows better than the picker whose lyric this is: then the row goes.
			if (ratingFor === key) rating = String(e) === 'own' ? { ...before, own: true } : before;
			toast.error(t(feedbackErrorKey(String(e))));
		} finally {
			voting = false;
		}
	}

	// ponytail: "reported" lasts as long as the picker has this lyric. The service keeps one report
	// per account and revision and accepts a repeat silently, so nothing needs remembering here.
	async function sendReport(e: SubmitEvent) {
		e.preventDefault();
		if (!reportReason) return;
		const key = ratingFor;
		reported = true;
		reportOpen = false;
		try {
			await api.limusicReport(track.videoId, reportReason, reportNote.trim());
		} catch (err) {
			if (ratingFor === key) {
				reported = false;
				reportOpen = true;
			}
			toast.error(t(feedbackErrorKey(String(err))));
		}
	}

	function openSettings() {
		open = false;
		ui.settingsFocus = 'lyrics';
		ui.settingsOpen = true;
	}
</script>

<Popover.Root bind:open>
	<Popover.Trigger
		class="group -ml-2 flex min-w-0 cursor-pointer items-center gap-1.5 rounded-full py-0.5 pl-2 pr-1.5 transition-colors hover:bg-foreground/10 hover:text-foreground data-[state=open]:bg-foreground/10 data-[state=open]:text-foreground"
		title={lyrics?.pinned ? t('lyrics.chosen') : lyrics?.authors?.length ? attribution : undefined}
	>
		{#if !lyrics}
			<HugeiconsIcon icon={Search01Icon} class="h-3.5 w-3.5 shrink-0" />
		{/if}
		<span class="truncate">{attribution}</span>
		{#if lyrics?.pinned}
			<HugeiconsIcon icon={PinIcon} class="h-3 w-3 shrink-0 text-primary" />
		{/if}
		{#if offset}
			<span class="shrink-0 font-medium tabular-nums text-primary">{offsetLabel}</span>
		{/if}
		<HugeiconsIcon
			icon={ArrowUp01Icon}
			class="h-3 w-3 shrink-0 opacity-60 transition-transform group-data-[state=open]:rotate-180"
		/>
	</Popover.Trigger>
	<Popover.Content
		side="top"
		align="start"
		sideOffset={8}
		class="max-h-[min(40rem,var(--bits-popover-content-available-height))] w-[22rem] max-w-[calc(100vw-2rem)] gap-0 overflow-hidden p-0"
	>
		<div class="flex items-start gap-3 border-b px-4 pb-3 pt-3.5">
			<div class="min-w-0 flex-1">
				<p class="font-heading text-sm font-semibold">{t('lyrics.sources_title')}</p>
				<p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">{t('lyrics.sources_hint')}</p>
			</div>
			<Button
				variant="ghost"
				size="icon-sm"
				class="-mr-1.5 -mt-1 shrink-0 text-muted-foreground"
				aria-label={t('lyrics.open_settings')}
				title={t('lyrics.open_settings')}
				onclick={openSettings}
			>
				<HugeiconsIcon icon={Settings02Icon} class="h-4 w-4" />
			</Button>
		</div>

		<div
			class="min-h-0 flex-1 overflow-y-auto p-1.5"
			role="radiogroup"
			aria-label={t('lyrics.sources_title')}
		>
			{#if query}
				<div class="mx-1 mb-1.5 flex items-center gap-2 rounded-lg bg-primary/8 px-2.5 py-1.5 text-xs">
					<HugeiconsIcon icon={Search01Icon} class="h-3.5 w-3.5 shrink-0 text-primary" />
					<span class="min-w-0 flex-1 truncate">
						{t('lyrics.search_results', {
							query: query.artists ? `${query.title} · ${query.artists}` : query.title
						})}
					</span>
					<button
						class="shrink-0 cursor-pointer font-medium text-primary hover:text-primary/80"
						onclick={clearSearch}
					>
						{t('lyrics.search_reset')}
					</button>
				</div>
			{:else}
				<button
					role="radio"
					aria-checked={!pinned}
					onclick={() => pinned && choose(null)}
					class="flex w-full cursor-pointer items-center gap-3 rounded-xl px-2.5 py-2 text-left transition-colors hover:bg-muted/60 {pinned
						? ''
						: 'bg-muted/50'}"
				>
					<span
						class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg {pinned
							? 'bg-muted text-muted-foreground'
							: 'bg-primary/12 text-primary'}"
					>
						<HugeiconsIcon icon={MagicWand01Icon} class="h-4 w-4" />
					</span>
					<span class="min-w-0 flex-1">
						<span class="block text-sm font-medium">{t('lyrics.automatic')}</span>
						<span class="block truncate text-xs text-muted-foreground">
							{!pinned && lyrics
								? t('lyrics.automatic_showing', { source: lyrics.source })
								: t('lyrics.automatic_hint')}
						</span>
					</span>
					{#if busy === 'auto'}
						<HugeiconsIcon icon={Loading03Icon} class="h-4 w-4 shrink-0 animate-spin text-muted-foreground" />
					{:else if !pinned}
						<HugeiconsIcon icon={Tick02Icon} class="h-4 w-4 shrink-0 text-primary" />
					{/if}
				</button>
				<div class="mx-2.5 my-1.5 h-px bg-border/70"></div>
			{/if}

			<div>
				{#each providers as p (p.id)}
					{@const r = results[p.id] ?? { status: 'idle' }}
					{@const hit = r.status === 'done' ? r.lyrics : null}
					{@const showing = lyrics?.provider === p.id}
					{@const chosen = showing && pinned}
					{@const dead = r.status === 'done' && !hit}
					<button
						role="radio"
						aria-checked={chosen}
						aria-disabled={dead || r.status === 'loading'}
						title={!p.on && r.status === 'idle' ? t('lyrics.off_hint') : undefined}
						onclick={() => {
							if (r.status === 'idle' || r.status === 'error') check(p.id);
							else if (hit && !chosen) choose(p.id);
						}}
						class="group flex w-full items-center gap-3 rounded-xl px-2.5 py-2 text-left transition-colors {dead
							? 'cursor-default'
							: 'cursor-pointer hover:bg-muted/60'} {chosen ? 'bg-muted/50' : ''}"
					>
						<!-- The dot is what's on screen, whether Automatic put it there or you did. -->
						<span
							class="flex h-4 w-4 shrink-0 items-center justify-center rounded-full border-2 transition-colors {showing
								? 'border-primary'
								: dead
									? 'border-border/60'
									: 'border-muted-foreground/40 group-hover:border-muted-foreground/70'}"
						>
							{#if showing}<span class="h-1.5 w-1.5 rounded-full bg-primary"></span>{/if}
						</span>
						<span
							class="min-w-0 flex-1 truncate text-sm {dead || !p.on
								? 'text-muted-foreground'
								: ''} {showing ? 'font-medium' : ''}"
						>
							{p.name}
						</span>
						{#if busy === p.id}
							<HugeiconsIcon icon={Loading03Icon} class="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />
						{/if}
						{#if r.status === 'loading'}
							<span class="flex shrink-0 items-center gap-1.5">
								<span class="h-4 w-14 animate-pulse rounded-full bg-muted"></span>
							</span>
						{:else if r.status === 'idle'}
							<span class="shrink-0 text-[11px] text-muted-foreground">
								{t('lyrics.off')} ·
								<span class="font-medium text-primary group-hover:underline">{t('lyrics.check')}</span>
							</span>
						{:else if r.status === 'error'}
							<span class="shrink-0 text-[11px] text-muted-foreground">
								{t('lyrics.unreachable')} ·
								<span class="font-medium text-primary group-hover:underline">{t('lyrics.retry')}</span>
							</span>
						{:else if hit}
							{@const kind = lyricsKind(hit)}
							{#if hit.lines.some((l) => l.translation)}
								<span title={t('lyrics.translated')} class="shrink-0 text-muted-foreground">
									<HugeiconsIcon icon={TranslateIcon} class="h-3.5 w-3.5" />
								</span>
							{/if}
							<span
								class="shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide {KIND_CLASS[
									kind
								]}"
							>
								{t(KIND_LABEL[kind])}
							</span>
						{:else}
							<span class="shrink-0 text-[11px] text-muted-foreground/70">{t('lyrics.not_found')}</span>
						{/if}
					</button>
				{/each}
			</div>
		</div>

		{#if lyrics?.synced}
			<div class="flex items-center gap-3 border-t px-4 py-3">
				<HugeiconsIcon icon={Timer01Icon} class="h-4 w-4 shrink-0 text-muted-foreground" />
				<div class="min-w-0 flex-1">
					<p class="text-sm font-medium">{t('lyrics.timing')}</p>
					<p class="text-xs leading-snug text-muted-foreground">{t('lyrics.timing_hint')}</p>
				</div>
				<div class="flex shrink-0 items-center rounded-full border bg-muted/40">
					<button
						class="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
						disabled={busy !== null}
						aria-label={t('lyrics.timing_earlier')}
						title={t('lyrics.timing_earlier')}
						onclick={() => nudge(-STEP_MS)}
					>
						<HugeiconsIcon icon={MinusSignIcon} class="h-3.5 w-3.5" />
					</button>
					<span
						class="w-14 text-center text-xs font-medium tabular-nums {offset ? 'text-primary' : ''}"
						aria-live="polite"
					>
						{offsetLabel}
					</span>
					<button
						class="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
						disabled={busy !== null}
						aria-label={t('lyrics.timing_later')}
						title={t('lyrics.timing_later')}
						onclick={() => nudge(STEP_MS)}
					>
						<HugeiconsIcon icon={PlusSignIcon} class="h-3.5 w-3.5" />
					</button>
				</div>
				<!-- Held in place when there's nothing to reset, so the stepper doesn't jump. -->
				<button
					class="flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground {offset
						? ''
						: 'invisible'}"
					aria-label={t('lyrics.timing_reset')}
					title={t('lyrics.timing_reset')}
					onclick={() => nudge(0)}
				>
					<HugeiconsIcon icon={ArrowTurnBackwardIcon} class="h-3.5 w-3.5" />
				</button>
			</div>
		{/if}

		{#if rateable && (ratingLoading || (rating && !rating.own))}
			<!-- Votes and reports on Limusic's lyric. The same row and pill as Timing above it. -->
			<div class="border-t">
				<div class="flex items-center gap-3 px-4 py-3">
					<HugeiconsIcon icon={StarIcon} class="h-4 w-4 shrink-0 text-muted-foreground" />
					<div class="min-w-0 flex-1">
						<p class="text-sm font-medium">{t('lyrics.rate')}</p>
						<p class="text-xs leading-snug text-muted-foreground">{t('lyrics.rate_hint')}</p>
					</div>
					{#if rating}
						<div class="flex shrink-0 items-center rounded-full border bg-muted/40">
							{#each [{ v: 1, icon: ThumbsUpIcon, n: rating.up, label: 'lyrics.vote_up' }, { v: -1, icon: ThumbsDownIcon, n: rating.down, label: 'lyrics.vote_down' }] as const as b (b.v)}
								{@const mine = rating.my_vote === b.v}
								<button
									class="flex h-7 min-w-11 cursor-pointer items-center justify-center gap-1.5 rounded-full px-2.5 text-xs font-medium tabular-nums transition-colors {mine
										? 'bg-primary/12 text-primary'
										: 'text-muted-foreground hover:bg-foreground/10 hover:text-foreground'}"
									aria-pressed={mine}
									aria-label={t(b.label)}
									title={t(b.label)}
									onclick={() => vote(b.v)}
								>
									<HugeiconsIcon icon={b.icon} class="h-3.5 w-3.5 {mine ? 'fill-current' : ''}" />
									{b.n}
								</button>
							{/each}
						</div>
					{:else}
						<span class="h-7 w-[5.75rem] shrink-0 animate-pulse rounded-full bg-muted"></span>
					{/if}
				</div>
				{#if rating}
					{#if reportOpen}
						<form class="space-y-2.5 px-4 pb-3.5" onsubmit={sendReport}>
							<p class="text-xs font-medium" id="report-title">{t('lyrics.report_title')}</p>
							<div class="grid grid-cols-2 gap-1.5" role="radiogroup" aria-labelledby="report-title">
								{#each REPORT_REASONS as r (r.id)}
									{@const on = reportReason === r.id}
									<button
										type="button"
										role="radio"
										aria-checked={on}
										onclick={() => (reportReason = r.id)}
										class="flex cursor-pointer items-center gap-2 rounded-lg border px-2.5 py-2 text-left text-xs transition-colors {on
											? 'border-primary/60 bg-primary/8 font-medium'
											: 'hover:bg-muted/60'}"
									>
										<span
											class="flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-full border-2 {on
												? 'border-primary'
												: 'border-muted-foreground/40'}"
										>
											{#if on}<span class="h-1.5 w-1.5 rounded-full bg-primary"></span>{/if}
										</span>
										{t(r.label)}
									</button>
								{/each}
							</div>
							<Input
								bind:value={reportNote}
								maxlength={300}
								placeholder={t('lyrics.report_note')}
								aria-label={t('lyrics.report_note')}
								class="h-8"
							/>
							<div class="flex justify-end gap-2">
								<Button type="button" variant="ghost" size="sm" onclick={() => (reportOpen = false)}>
									{t('common.cancel')}
								</Button>
								<Button type="submit" size="sm" disabled={!reportReason}>
									<HugeiconsIcon icon={Flag02Icon} class="h-3.5 w-3.5" />
									{t('lyrics.report_send')}
								</Button>
							</div>
						</form>
					{:else if reported}
						<p class="flex items-center gap-2 px-4 pb-3 pl-11 text-xs text-muted-foreground" role="status">
							<HugeiconsIcon icon={Tick02Icon} class="h-3.5 w-3.5 shrink-0 text-primary" />
							{t('lyrics.reported')}
						</p>
					{:else}
						<button
							class="-mt-1 mb-2 ml-9 flex cursor-pointer items-center gap-1.5 rounded-full px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground"
							onclick={() => (reportOpen = true)}
						>
							<HugeiconsIcon icon={Flag02Icon} class="h-3.5 w-3.5" />
							{t('lyrics.report')}
						</button>
					{/if}
				{/if}
			</div>
		{/if}

		{#if lyrics?.provider === 'unison'}
			<!-- Unison votes are signed with a Better Lyrics key, so they happen on its site (#342). -->
			<button
				class="flex w-full cursor-pointer items-center gap-2 border-t px-4 py-2.5 text-xs text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
				onclick={() =>
					api.openExternal(`https://unison.betterlyrics.org/song/${encodeURIComponent(track.videoId)}`)}
			>
				<HugeiconsIcon icon={LinkSquare02Icon} class="h-3.5 w-3.5" />
				{t('lyrics.unison_rate')}
			</button>
		{/if}

		{#if canEdit}
			<!-- The way into the lyric editor: the one row here with a filled icon and a second line,
			     so it does not pass for another setting. It knows about a draft. -->
			<button
				class="flex w-full cursor-pointer items-center gap-3 border-t px-3.5 py-3 text-left transition-colors hover:bg-muted/50"
				onclick={openEditor}
			>
				<span class="flex size-9 shrink-0 items-center justify-center rounded-[10px] bg-primary text-primary-foreground">
					<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
					<HugeiconsIcon icon={Tap01Icon} altIcon={PencilEdit02Icon} showAlt={!!draft || improving} class="size-[18px]" />
				</span>
				<span class="min-w-0 flex-1">
					<span class="block text-sm font-semibold">
						{draft ? t('lyrics.draft') : improving ? t('lyrics.improve') : t('lyrics.sync')}
					</span>
					<span class="block text-[12.5px] leading-snug text-muted-foreground">
						{draft ? t('lyrics.draft_hint', draft) : improving ? t('lyrics.improve_hint') : t('lyrics.sync_hint')}
					</span>
				</span>
				<HugeiconsIcon icon={ArrowRight01Icon} class="size-3.5 shrink-0 text-muted-foreground" />
			</button>
		{/if}

		{#if searchOpen}
			<form class="space-y-2 border-t px-4 py-3" onsubmit={search}>
				<Input
					bind:ref={titleInput}
					bind:value={draftTitle}
					placeholder={t('lyrics.search_title')}
					aria-label={t('lyrics.search_title')}
					class="h-8"
				/>
				<div class="flex gap-2">
					<Input
						bind:value={draftArtists}
						placeholder={t('lyrics.search_artist')}
						aria-label={t('lyrics.search_artist')}
						class="h-8 flex-1"
					/>
					<Button type="submit" size="sm" disabled={!draftTitle.trim()}>
						<HugeiconsIcon icon={Search01Icon} class="h-3.5 w-3.5" />
						{t('lyrics.search_go')}
					</Button>
				</div>
			</form>
		{:else}
			<button
				class="flex w-full cursor-pointer items-center gap-2 border-t px-4 py-2.5 text-xs text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
				onclick={openSearch}
			>
				<HugeiconsIcon icon={Search01Icon} class="h-3.5 w-3.5" />
				{t('lyrics.search_open')}
				<HugeiconsIcon icon={ArrowRight01Icon} class="ml-auto h-3.5 w-3.5" />
			</button>
		{/if}
	</Popover.Content>
</Popover.Root>
