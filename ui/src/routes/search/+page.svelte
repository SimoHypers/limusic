<script module lang="ts">
	// Survive remounts (module scope), so coming back to /search, from a result you clicked or from
	// the sidebar, shows the last search on the tab it was on instead of a blank page. The results
	// themselves come back from the page cache, so the rerun paints instantly and just revalidates.
	type Cat = 'all' | 'songs' | 'videos' | 'albums' | 'artists' | 'playlists';
	let lastQuery = '';
	let lastCat: Cat = 'all';
</script>

<script lang="ts">
	import { onMount, untrack } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Cancel01Icon,
		Clock01Icon,
		GuitarIcon,
		MusicNote01Icon,
		Search01Icon,
		SmileIcon,
		SparklesIcon,
		StarAward02Icon,
		Video01Icon
	} from '@hugeicons/core-free-icons';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import MediaCardSkeleton from '$lib/components/MediaCardSkeleton.svelte';
	import SearchSuggest from '$lib/components/SearchSuggest.svelte';
	import SectionHeading from '$lib/components/SectionHeading.svelte';
	import Shelf from '$lib/components/Shelf.svelte';
	import TopResult from '$lib/components/TopResult.svelte';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import TrackRowSkeleton from '$lib/components/TrackRowSkeleton.svelte';
	import TrackSelectionBar from '$lib/components/TrackSelectionBar.svelte';
	import TrackSelectButton from '$lib/components/TrackSelectButton.svelte';
	import { trackSelection, type TrackSelection } from '$lib/selection.svelte';
	import * as api from '$lib/api';
	import type { BrowseItem, Mood, MoodSection, SearchResults, SongItem } from '$lib/api';
	import { getCached, putCached } from '$lib/pagecache';
	import {
		auth,
		clearSearches,
		forgetSearch,
		noteSearch,
		openAddToPlaylist,
		personal,
		playSong,
		playback
	} from '$lib/player.svelte';
	import { asSong } from '$lib/browse';
	import { appIcon } from '$lib/appicon.svelte';
	import { thumb } from '$lib/thumb';
	import { currentLocale, t } from '$lib/i18n.svelte';

	type Cached = { res: SearchResults; songs: SongItem[]; videos: SongItem[] };
	type CardCat = 'albums' | 'artists' | 'playlists';

	let query = $state(lastQuery);
	let res = $state<SearchResults | null>(null);
	// The Songs list comes from the songs-filtered search, not from `res.songs`: an unfiltered
	// response gives a song row either its artist or its length, never both, so those rows land
	// duration-less. The filtered endpoint returns "Artist • Album • 3:58" on every row.
	let songs = $state<SongItem[]>([]);
	// Videos have no unfiltered fallback: the mixed response's video rows are the ones YouTube
	// already folded into `res.songs`, so an empty list here just hides the section and its chip.
	let videos = $state<SongItem[]>([]);
	let searched = $state('');
	let searching = $state(false);
	let error = $state<string | null>(null);
	let cat = $state<Cat>(lastCat);

	// The query of the most recent runSearch call, so an older in-flight one can't clobber it.
	let latest = '';

	async function runSearch() {
		if (!query.trim()) return;
		const q = query;
		latest = q;
		lastQuery = q;
		noteSearch(q);
		const key = `search:${q}`;
		const hit = getCached<Cached>(key);
		if (hit) {
			res = hit.res;
			songs = hit.songs;
			videos = hit.videos;
			searched = q;
			searching = false;
		} else {
			searching = true;
		}
		error = null;
		try {
			// In parallel, and the filtered one may fail on its own: the list falls back to the
			// unfiltered rows rather than the whole search erroring out.
			const [fresh, freshSongs, freshVideos] = await Promise.all([
				// The one search the user actually asked for, so this is the one that goes to
				// YouTube signed in and lands in their search history (#203).
				api.searchAll(q, true),
				// Not recorded: the unfiltered search above already wrote this query to the
				// account's history, and recording it twice is two entries for one search.
				api.search(q, false).catch(() => [] as SongItem[]),
				api.searchVideos(q).catch(() => [] as SongItem[])
			]);
			if (latest !== q) return; // a newer search superseded this one
			res = fresh;
			songs = freshSongs;
			videos = freshVideos;
			searched = q;
			putCached(key, { res: fresh, songs: freshSongs, videos: freshVideos });
		} catch (e) {
			if (latest !== q) return;
			if (!hit) error = String(e);
		} finally {
			if (latest === q) searching = false;
		}
	}

	function searchFor(q: string) {
		query = q;
		runSearch();
	}

	/** Back to the browse page. `latest` is reset too, so a search still in flight lands nowhere. */
	function clearSearch(input?: HTMLInputElement | null) {
		latest = '';
		lastQuery = '';
		query = '';
		searched = '';
		res = null;
		songs = [];
		videos = [];
		error = null;
		searching = false;
		setCat('all');
		input?.focus();
	}

	function setCat(c: Cat) {
		cat = lastCat = c;
	}

	// A chip or a "See all": the tab starts at its top, not wherever the click was on the one before.
	// Only from a click: arriving by URL must leave the layout's scroll restore alone.
	let root = $state<HTMLElement | null>(null);
	function pickCat(c: Cat) {
		setCat(c);
		root?.closest('main')?.scrollTo({ top: 0 });
	}

	// Run the search when arriving with a ?q= (e.g. from the Home search box). Keyed on the URL
	// alone: typing a new query in the field must not look like a URL change and bounce us back.
	const urlQuery = $derived(page.url.searchParams.get('q') ?? '');
	let lastUrlQuery = '';
	$effect(() => {
		if (urlQuery && urlQuery !== lastUrlQuery) {
			lastUrlQuery = urlQuery;
			query = urlQuery;
			untrack(() => {
				setCat('all');
				runSearch();
			});
		}
	});

	// Arriving without a ?q= (back from a result, or the sidebar link): rerun whatever was last
	// searched. onMount, not the effect above, so a ?q= arrival still wins.
	onMount(() => {
		if (!urlQuery && query) runSearch();
	});

	// --- Albums / Artists / Playlists tabs: a filtered search of their own, fetched on open ----------
	let cards = $state<BrowseItem[]>([]);
	let cardsLoading = $state(false);
	let cardsError = $state<string | null>(null);

	async function loadCards(q: string, c: CardCat) {
		const key = `searchcat:${c}:${q}`;
		const hit = getCached<BrowseItem[]>(key);
		cards = hit ?? [];
		cardsLoading = !hit;
		cardsError = null;
		try {
			const fresh = await api.searchCards(q, c);
			if (q !== searched || c !== cat) return; // the tab or the query moved on
			cards = fresh;
			putCached(key, fresh);
		} catch (e) {
			if (q === searched && c === cat && !hit) cardsError = String(e);
		} finally {
			if (q === searched && c === cat) cardsLoading = false;
		}
	}

	$effect(() => {
		const q = searched;
		const c = cat;
		if (q && (c === 'albums' || c === 'artists' || c === 'playlists')) {
			untrack(() => loadCards(q, c));
		}
	});

	// --- Moods & Genres, the browse page under an empty field ---------------------------------------
	// Keyed by language: YouTube names the tiles in the `hl` it was asked in.
	const moodsKey = () => `moods:${currentLocale.id}`;
	let moods = $state<MoodSection[] | null>(getCached<MoodSection[]>(moodsKey()));
	let moodsAsked = false;
	$effect(() => {
		// Only once the browse page is actually on screen: a return to a search never needs them.
		if (res || searching || moods || moodsAsked) return;
		moodsAsked = true;
		const key = moodsKey();
		api
			.getMoods()
			.then((m) => {
				moods = m;
				putCached(key, m);
			})
			.catch(() => (moods = [])); // the tiles just don't show; recent searches still do
	});

	// Which section is which, by position: the titles are in the user's language, and "For you"
	// only exists signed in. YouTube ends on Moods & moments, then Genres; anything before is ours.
	type MoodKind = 'for-you' | 'moods' | 'genres';
	const moodKind = (i: number, n: number): MoodKind =>
		i === n - 1 ? 'genres' : i === n - 2 ? 'moods' : 'for-you';
	const MOOD_ICON = { 'for-you': StarAward02Icon, moods: SmileIcon, genres: GuitarIcon };

	// Tile covers. localStorage rather than SQLite, like `personal`: only the webview reads them.
	// A category's first playlist changes over weeks, so a stale cover still shows while its
	// replacement is fetched, and a fresh one is never asked for twice.
	const ART_KEY = 'limusic:mood-art';
	const ART_TTL_MS = 7 * 24 * 3600_000;
	type ArtStore = Record<string, { url: string; at: number }>;
	let art = $state<Record<string, string>>({});
	let artAsked = false;

	async function loadArt(sections: MoodSection[]) {
		let store: ArtStore = {};
		try {
			store = JSON.parse(localStorage.getItem(ART_KEY) ?? '{}') ?? {};
		} catch {
			// unreadable or blocked: every cover is a miss this time
		}
		const now = Date.now();
		for (const [p, v] of Object.entries(store)) art[p] = v.url;
		// A section at a time, top first, so the covers on screen land before the ones below.
		for (const sec of sections) {
			const due = sec.items
				.map((m) => m.params)
				.filter((p) => !store[p] || now - store[p].at > ART_TTL_MS);
			if (!due.length) continue;
			try {
				const got = await api.getMoodArt(due);
				for (const [p, url] of Object.entries(got)) {
					art[p] = url;
					store[p] = { url, at: now };
				}
			} catch {
				continue; // offline: the tiles keep their tint, and the next visit asks again
			}
			// Written back without anything past its TTL, so tiles YouTube dropped age out.
			const fresh = Object.fromEntries(
				Object.entries(store).filter(([, v]) => now - v.at <= ART_TTL_MS)
			);
			try {
				localStorage.setItem(ART_KEY, JSON.stringify(fresh));
			} catch {
				// quota or a locked store: covers are decoration
			}
		}
	}

	$effect(() => {
		if (!moods?.length || artAsked) return;
		artAsked = true;
		const sections = moods;
		untrack(() => loadArt(sections));
	});

	function openMood(m: Mood) {
		const q = new URLSearchParams({ id: api.MOODS_CATEGORY_ID, params: m.params, title: m.title });
		goto(`/list?${q.toString()}`);
	}

	// --- Results ----------------------------------------------------------------------------------
	const songRows = $derived(songs.length ? songs : (res?.songs ?? []).map(asSong));
	const top = $derived(res?.top[0]);
	// On "All", Songs shares the row with the top result, and five rows is about its height.
	const songList = $derived(cat === 'songs' ? songRows : songRows.slice(0, 5));
	const videoList = $derived(cat === 'videos' ? videos : videos.slice(0, 4));
	const selection = trackSelection(
		() => songList,
		() => songList,
		() => `${auth.epoch}:${cat}:${searched}`
	);
	// A second list means a second selection: one shared scope would let a bulk action from the
	// Songs bar act on video rows the user never checked.
	const videoSelection = trackSelection(
		() => videoList,
		() => videoList,
		() => `${auth.epoch}:videos:${cat}:${searched}`
	);
	const nothing = $derived(
		!!res &&
			!top &&
			!songRows.length &&
			!videos.length &&
			!res.albums.length &&
			!res.artists.length &&
			!res.playlists.length
	);

	const CATS: Cat[] = ['all', 'songs', 'videos', 'albums', 'artists', 'playlists'];
	const LABEL = $derived<Record<Cat, string>>({
		all: t('common.all'),
		songs: t('common.songs'),
		videos: t('common.videos'),
		albums: t('common.albums'),
		artists: t('common.artists'),
		playlists: t('common.playlists')
	});
	const chipClass = (active: boolean) =>
		`shrink-0 cursor-pointer rounded-full px-3.5 py-1.5 text-sm font-medium transition-colors ${
			active
				? 'bg-primary text-primary-foreground'
				: 'bg-foreground/5 text-foreground/75 hover:bg-foreground/10 hover:text-foreground'
		}`;

	// The field sits on the header's wash until it pins to the top, and only then needs a
	// background of its own (home's chip bar does the same).
	let stuck = $state(false);
	function stickWatch(node: HTMLElement) {
		const io = new IntersectionObserver(
			([e]) => (stuck = !e.isIntersecting && e.boundingClientRect.top < (e.rootBounds?.top ?? 0)),
			{ root: node.closest('main') }
		);
		io.observe(node);
		return () => io.disconnect();
	}
</script>

{#snippet rows(list: SongItem[], sel: TrackSelection)}
	<TrackSelectionBar selection={sel} />
	{#each list as song, i (JSON.stringify([song.video_id, i]))}
		<TrackRow
			{song}
			selection={sel}
			selectionKey={sel.visibleKeys[i]}
			showPlayCount
			active={playback.now?.videoId === song.video_id}
			onplay={() => playSong(song)}
			onAdd={() => openAddToPlaylist(song)}
		/>
	{:else}
		<p class="text-sm text-muted-foreground">{t('common.nothing_found')}</p>
	{/each}
{/snippet}

{#snippet cover(params: string)}
	{@const url = art[params]}
	{#if url}
		<!-- 200 for 1x and 400 for 2x, the two sizes MediaCard verified against the CDN. A cover that
		     still fails is dropped for this visit, leaving the frame's tint. -->
		<img
			class="mood-art"
			src={thumb(url, 200)}
			srcset="{thumb(url, 200)} 1x, {thumb(url, 400)} 2x"
			alt=""
			loading="lazy"
			draggable="false"
			onload={(e) => e.currentTarget.classList.add('ready')}
			onerror={() => (art[params] = '')}
		/>
	{/if}
{/snippet}

<!-- Scrolls <main> like every other page, so the layout's scroll snapshot brings you back to where
     you were when you return from a result. isolate: the wash below sits at -z-10 inside it. -->
<div class="relative isolate" bind:this={root}>
	<div
		class="pointer-events-none absolute inset-x-0 top-0 -z-10 h-72 overflow-hidden"
		aria-hidden="true"
	>
		<div
			class="absolute inset-0 opacity-[0.18]"
			style="background:radial-gradient(90% 75% at 10% 0%, var(--primary) 0%, transparent 70%)"
		></div>
		<div class="absolute inset-0 bg-gradient-to-b from-transparent to-background"></div>
	</div>

	<h1 class="px-6 pt-10 font-heading text-4xl font-bold tracking-tight">{t('common.search')}</h1>

	<div class="h-px" {@attach stickWatch}></div>
	<div
		class="sticky top-0 z-20 px-6 pb-3 pt-4 transition-colors duration-200 {stuck
			? 'bg-background'
			: ''}"
	>
		<form
			id="search-form"
			class="relative max-w-2xl"
			onsubmit={(e) => {
				e.preventDefault();
				runSearch();
			}}
		>
			<HugeiconsIcon
				icon={Search01Icon}
				class="pointer-events-none absolute left-4 top-1/2 z-10 h-5 w-5 -translate-y-1/2 text-muted-foreground"
			/>
			<SearchSuggest
				bind:value={query}
				placeholder={t('common.search_placeholder')}
				inputClass="h-12 rounded-full bg-card/80 pl-12 text-base shadow-sm md:text-base"
				onpick={() => {
					lastQuery = query;
					noteSearch(query);
				}}
			/>
			{#if query}
				<button
					type="button"
					class="absolute right-2 top-1/2 z-10 flex h-8 w-8 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground"
					aria-label={t('a11y.clear_search')}
					title={t('a11y.clear_search')}
					onclick={(e) => clearSearch(e.currentTarget.form?.querySelector('input'))}
				>
					<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
				</button>
			{/if}
		</form>
		{#if res || searching}
			<div class="mt-3 flex items-center gap-2">
				<div class="rail flex min-w-0 flex-1 gap-2 overflow-x-auto" role="group" aria-label={t('search.filters')}>
					{#each CATS as c (c)}
						<!-- Hidden with music videos off: there is no such thing as a video search then. -->
						{#if c !== 'videos' || videos.length}
							<button
								class={chipClass(cat === c)}
								aria-pressed={cat === c}
								onclick={() => pickCat(c)}
							>
								{LABEL[c]}
							</button>
						{/if}
					{/each}
				</div>
				{#if cat === 'songs'}
					<TrackSelectButton {selection} />
				{:else if cat === 'videos'}
					<TrackSelectButton selection={videoSelection} />
				{/if}
			</div>
		{/if}
		{#if stuck}
			<div
				class="pointer-events-none absolute inset-x-0 top-full h-5 bg-gradient-to-b from-background to-transparent"
			></div>
		{/if}
	</div>

	<div class="px-6 pb-10 pt-4">
		{#if error}
			<ErrorState message={error} onRetry={runSearch} />
		{:else if searching}
			<div class="flex flex-col gap-10" aria-hidden="true">
				<section>
					<Skeleton class="mb-4 h-6 w-40 rounded" />
					{#each Array(5) as _, i (i)}
						<TrackRowSkeleton />
					{/each}
				</section>
				<section>
					<Skeleton class="mb-4 h-6 w-32 rounded" />
					<div class="flex gap-2 overflow-hidden pb-2">
						{#each Array(5) as _, i (i)}
							<div class="w-40 shrink-0"><MediaCardSkeleton /></div>
						{/each}
					</div>
				</section>
			</div>
		{:else if res && nothing}
			<p class="text-sm text-muted-foreground">{t('common.no_results', { query: searched })}</p>
		{:else if res && cat === 'all'}
			<div class="content-in @container flex flex-col gap-10">
				{#if top || songList.length}
					<div
						class="grid gap-x-8 gap-y-10 {top && songList.length
							? '@4xl:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]'
							: ''}"
					>
						{#if top}
							<section class="min-w-0">
								<SectionHeading title={t('common.top_result')} icon={SparklesIcon} />
								<TopResult item={top} related={res.top.slice(1)} />
							</section>
						{/if}
						{#if songList.length}
							<section class="min-w-0">
								<SectionHeading
									title={t('common.songs')}
									icon={MusicNote01Icon}
									onMore={() => pickCat('songs')}
								>
									<TrackSelectButton {selection} />
								</SectionHeading>
								{@render rows(songList, selection)}
							</section>
						{/if}
					</div>
				{/if}
				{#if res.albums.length}
					<Shelf title={t('common.albums')} items={res.albums} onMore={() => pickCat('albums')} />
				{/if}
				{#if res.artists.length}
					<Shelf title={t('common.artists')} items={res.artists} onMore={() => pickCat('artists')} />
				{/if}
				{#if videoList.length}
					<section>
						<SectionHeading
							title={t('common.videos')}
							icon={Video01Icon}
							onMore={() => pickCat('videos')}
						>
							<TrackSelectButton selection={videoSelection} />
						</SectionHeading>
						{@render rows(videoList, videoSelection)}
					</section>
				{/if}
				{#if res.playlists.length}
					<Shelf
						title={t('common.playlists')}
						items={res.playlists}
						onMore={() => pickCat('playlists')}
					/>
				{/if}
			</div>
		{:else if res && cat === 'songs'}
			<div class="content-in">{@render rows(songList, selection)}</div>
		{:else if res && cat === 'videos'}
			<div class="content-in">{@render rows(videoList, videoSelection)}</div>
		{:else if res}
			{#if cardsLoading}
				<div class="card-grid">
					{#each Array(12) as _, i (i)}
						<MediaCardSkeleton />
					{/each}
				</div>
			{:else if cardsError}
				<ErrorState message={cardsError} onRetry={() => loadCards(searched, cat as CardCat)} />
			{:else if cards.length}
				<div class="card-grid content-in">
					{#each cards as item (item.id + item.title)}
						<MediaCard {item} />
					{/each}
				</div>
			{:else}
				<p class="text-sm text-muted-foreground">{t('common.nothing_found')}</p>
			{/if}
		{:else}
			<div class="content-in flex flex-col gap-10">
				{#if personal.searches.length}
					<section>
						<SectionHeading title={t('search.recent')} icon={Clock01Icon}>
							<button
								class="shrink-0 cursor-pointer text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
								onclick={clearSearches}
							>
								{t('search.clear_recent')}
							</button>
						</SectionHeading>
						<div class="flex flex-wrap gap-2">
							{#each personal.searches as q (q)}
								<div
									class="flex items-center rounded-full bg-foreground/5 text-sm transition-colors hover:bg-foreground/10"
								>
									<button
										class="flex cursor-pointer items-center gap-2 py-1.5 pl-3.5 pr-1.5 font-medium text-foreground/80 hover:text-foreground"
										onclick={() => searchFor(q)}
									>
										<HugeiconsIcon icon={Search01Icon} class="h-3.5 w-3.5 text-muted-foreground" />
										{q}
									</button>
									<button
										class="mr-1 flex h-6 w-6 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground"
										aria-label={t('search.forget', { query: q })}
										title={t('search.forget', { query: q })}
										onclick={() => forgetSearch(q)}
									>
										<HugeiconsIcon icon={Cancel01Icon} class="h-3 w-3" />
									</button>
								</div>
							{/each}
						</div>
					</section>
				{/if}
				{#if moods === null}
					<section aria-hidden="true">
						<Skeleton class="mb-4 h-6 w-44 rounded" />
						<div class="mood-grid moods">
							{#each Array(12) as _, i (i)}
								<Skeleton class="h-28 rounded-2xl" />
							{/each}
						</div>
					</section>
				{:else}
					{#each moods as sec, i (sec.title)}
						{@const kind = moodKind(i, moods.length)}
						<section>
							<SectionHeading title={sec.title} icon={MOOD_ICON[kind]} />
							<div class="mood-grid {kind}">
								{#each sec.items as m (m.params)}
									<button class="mood" style="--mood: {m.color}" onclick={() => openMood(m)}>
										<span class="mood-title">
											{#if kind === 'genres'}<span class="mood-dot"></span>{/if}{m.title}
										</span>
										{#if kind === 'for-you'}
											<span class="mood-logo"><img src={appIcon.src} alt="" /></span>
											{@render cover(m.params)}
										{:else if kind === 'moods'}
											<span class="mood-pill"></span>
											<span class="mood-disc"><span class="mood-spin">{@render cover(m.params)}</span></span>
										{:else}
											<span class="mood-sleeve">{@render cover(m.params)}</span>
										{/if}
									</button>
								{/each}
							</div>
						</section>
					{/each}
					{#if !moods.length && !personal.searches.length}
						<p class="text-sm text-muted-foreground">{t('common.search_prompt')}</p>
					{/if}
				{/if}
			</div>
		{/if}
	</div>
</div>
