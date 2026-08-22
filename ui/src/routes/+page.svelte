<script lang="ts">
	import { onMount } from 'svelte';
	import { fade } from 'svelte/transition';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowUpBigIcon, MusicNote01Icon } from '@hugeicons/core-free-icons';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import { Button } from '$lib/components/ui/button';
	import MediaCardSkeleton from '$lib/components/MediaCardSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import HomeHero from '$lib/components/HomeHero.svelte';
	import Shortcuts from '$lib/components/Shortcuts.svelte';
	import RecentRail from '$lib/components/RecentRail.svelte';
	import Shelf from '$lib/components/Shelf.svelte';
	import ForgottenFavourites from '$lib/components/ForgottenFavourites.svelte';
	import FamiliarArtists from '$lib/components/FamiliarArtists.svelte';
	import HomeLayoutDialog from '$lib/components/HomeLayoutDialog.svelte';
	import TrackRowSkeleton from '$lib/components/TrackRowSkeleton.svelte';
	import * as api from '$lib/api';
	import type { BrowseItem, HomeChip, HomePage, HomeSection } from '$lib/api';
	import {
		auth,
		library,
		noteHomeSections,
		personal,
		playback,
		seedOnRepeatPick,
		toast
	} from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import {
		arrangeSections,
		freshen,
		hiddenSections,
		interleave,
		recentItems,
		topArtists
	} from '$lib/personal';
	import { getCached, putCached } from '$lib/pagecache';

	const FORGOTTEN_KEY = 'home:forgotten';

	let home = $state<HomePage | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	// The mood chips + which one is active. Kept out of `home` so the row survives a filter switch's
	// loading state (every home response carries the same chips anyway). Limusic is music-only.
	let chips = $state<HomeChip[]>([]);
	let selected = $state<string | null>(null);
	let loadingMore = $state(false);
	let moreError = $state(false);
	// Anything already on the Shortcuts grid is dropped: a shortcut is something you play, so the two
	// lists otherwise converge on the same handful of items and the top of home shows them twice in
	// two different shapes. Recents earn their space by being what Shortcuts *isn't*. Nine survivors
	// = three full columns; the window is generous because most of it gets filtered away.
	const pinned = $derived(new Set(personal.picks.map((p) => p.id)));
	// Same snapshot problem as the Shortcuts tiles: the stored card is what it looked like when it
	// was last played from, so the live library row wins where there is one (#67).
	const recent = $derived(
		recentItems(personal, 100)
			.filter((r) => !pinned.has(r.id))
			.slice(0, 9)
			.map((r) => freshen(r, library.items))
	);

	// Outlined at rest, filled with the accent when on. Grey-on-grey pills that go black when
	// selected are YouTube Music's chip row exactly, and they carry no colour of the app at all;
	// this way the one active filter is the only saturated thing above the feed.
	const chipClass = (active: boolean) =>
		`shrink-0 cursor-pointer rounded-full border px-3.5 py-1.5 text-sm font-medium transition-colors ${
			active
				? 'border-primary bg-primary text-primary-foreground'
				: 'border-border text-muted-foreground hover:border-foreground/25 hover:text-foreground'
		}`;

	// "Forgotten favourites" is pulled out of the feed and rendered as a list above it (see the
	// markup) — the shelf's cards say nothing about a song, and this one is meant to be read.
	// Songs only: if YouTube ever fills that shelf with something else, it stays a normal card row.
	const isForgotten = (s: HomeSection) =>
		/forgotten/i.test(s.title) && s.items.some((i) => i.kind === 'song');
	// Held separately from `home`, not derived from it: YouTube sends the shelf a page or two into the
	// feed, so it survives the revalidating `home = fresh` that drops back to page one, and a revisit
	// reads it from the cache instead of walking continuations again.
	let forgotten = $state<HomeSection | null>(null);
	let seeking = $state(false); // walking continuations to find it — the slot shows a skeleton
	const feed = $derived(home?.sections.filter((s) => !isForgotten(s)) ?? []);

	// --- the arrangement the user set in the Edit modal (personal.ts) ---------------------------
	// The two sections the app builds itself get reserved keys — a YouTube shelf title can't start
	// with "@" — so they keep their slot even before (or without) any content to show.
	const RECENT = '@recent';
	const FAMILIAR = '@familiar';
	const FORGOTTEN = '@forgotten';
	type Block =
		| { id: string; key: string; title: string; shelf?: undefined }
		| { id: string; key: string; title: string; shelf: HomeSection };
	let editing = $state(false);
	const hidden = $derived(hiddenSections(personal));
	/**
	 * Every section home can show, in the user's order, hidden ones included — the modal lists those
	 * to offer them back. Shelves are keyed on their title (all YouTube gives us that survives a
	 * restart) but rendered under a positional id, because a feed walked far enough does repeat one.
	 */
	const blocks = $derived.by(() => {
		const local: Block[] = selected
			? [] // a mood feed is the chip's: neither of ours belongs in it
			: [
					{ id: RECENT, key: RECENT, title: t('home.jump_back_in') },
					{ id: FAMILIAR, key: FAMILIAR, title: t('home.familiar_artists') },
					{ id: FORGOTTEN, key: FORGOTTEN, title: t('home.forgotten_favourites') }
				];
		const shelves = feed.map((s, i) => ({
			id: `${i}:${s.title}`,
			key: s.title,
			title: s.title,
			shelf: s
		}));
		return arrangeSections([...local, ...shelves], personal);
	});
	const visible = $derived(blocks.filter((b) => !hidden.has(b.key)));
	/**
	 * What the Edit modal lists. Not `blocks`: the feed arrives a page at a time, so `blocks` holds
	 * only the shelves scrolled to so far, and the modal showed five rows before a scroll and
	 * fifteen after one. Every shelf home has ever rendered is remembered (`noteSections`), and the
	 * ones this visit hasn't fetched yet are listed alongside the loaded ones — a section can be
	 * hidden or moved before the page has got to it, which is the whole point of the modal.
	 *
	 * Kept apart from `blocks` deliberately: these carry no shelf, so they must never reach the
	 * feed's renderer. Unranked ones sort to the end, since where they belong is exactly what
	 * hasn't loaded.
	 */
	const known = $derived.by(() => {
		if (selected) return blocks; // a mood feed is the chip's, and its shelves aren't home's
		const have = new Set(blocks.map((b) => b.key));
		const unloaded: Block[] = personal.home.seen
			.filter((t) => !have.has(t))
			.map((t) => ({ id: `seen:${t}`, key: t, title: t }));
		return unloaded.length ? arrangeSections([...blocks, ...unloaded], personal) : blocks;
	});

	// Every page of the feed adds to that memory. Only the unfiltered feed: a mood chip's shelves
	// belong to the chip, not to home's arrangement.
	$effect(() => {
		if (selected) return;
		const titles = feed.map((s) => s.title);
		if (titles.length) noteHomeSections(titles);
	});

	/** Latch the shelf whenever a page turns out to carry it. Called after every `home` change. */
	function noteForgotten() {
		const found = home?.sections.find(isForgotten);
		if (found) {
			forgotten = found;
			putCached(FORGOTTEN_KEY, found);
		}
		return !!found;
	}

	/**
	 * The shelf renders at the top but arrives deep in the feed, so walk a few continuations for it
	 * instead of leaving the slot empty until the user happens to scroll past where YouTube put it.
	 * Bounded — it's a shelf, not worth crawling the whole feed for.
	 */
	async function seekForgotten(params: string | null) {
		if (params || forgotten) return; // a mood feed is the chip's; a cached shelf needs no crawl
		if (hidden.has(FORGOTTEN)) return; // four round-trips for a shelf that won't be rendered
		seeking = true;
		try {
			for (let i = 0; i < 4; i++) {
				if (forgotten || moreError || loadingMore) return;
				if (selected !== params || !home?.continuation) return;
				await loadMore(); // latches the shelf itself if the page carries it
			}
		} finally {
			seeking = false;
		}
	}

	function showMore(section: { title: string; moreBrowseId?: string; moreParams?: string }) {
		const q = new URLSearchParams({ id: section.moreBrowseId!, title: section.title });
		if (section.moreParams) q.set('params', section.moreParams);
		goto(`/list?${q.toString()}`);
	}

	async function load(params: string | null = selected) {
		selected = params;
		const key = params ? `home:${params}` : 'home';
		const hit = getCached<HomePage>(key);
		forgotten = params ? null : getCached<HomeSection>(FORGOTTEN_KEY);
		if (hit) {
			home = hit;
			loading = false;
			noteForgotten();
			cater(hit, params);
		} else {
			loading = true;
		}
		error = null;
		try {
			const fresh = await api.getHome(params ?? undefined);
			// A stale response from a chip the user already clicked away from must not win.
			if (selected !== params) return;
			home = fresh;
			putCached(key, fresh);
			noteForgotten();
			cater(fresh, params);
			seekForgotten(params); // background: the feed is already on screen
		} catch (e) {
			if (!hit) error = String(e);
		} finally {
			loading = false;
		}
	}

	async function loadMore() {
		const token = home?.continuation;
		if (!token || loadingMore) return;
		loadingMore = true;
		moreError = false;
		const params = selected; // guard against chip switches mid-flight
		try {
			const more = await api.getHomeMore(token);
			if (selected !== params || home?.continuation !== token) return; // stale
			home = {
				...home!,
				sections: [...home!.sections, ...more.sections],
				// An empty page would leave the sentinel in view with nothing to show — treat it as the end.
				continuation: more.sections.length ? more.continuation : undefined
			};
			noteForgotten();
		} catch (e) {
			// Stop auto-loading and offer a retry — auto-retrying a visible sentinel would spin.
			moreError = true;
			toast.error('Could not load more');
		} finally {
			loadingMore = false;
		}
	}

	// Home doesn't scroll itself — <main> in the layout is the scroller, so the back-to-top button
	// has to watch the ancestor rather than the window.
	let scroller = $state<HTMLElement | null>(null);
	let scrolled = $state(false);
	function watchScroll(node: HTMLElement) {
		const el = node.closest('main');
		if (!el) return;
		scroller = el;
		const onScroll = () => (scrolled = el.scrollTop > 400);
		el.addEventListener('scroll', onScroll, { passive: true });
		return () => el.removeEventListener('scroll', onScroll);
	}

	// One page per approach to the bottom: the observer only fires when the sentinel *enters* view, so
	// an appended page that pushes it back out is required before the next fetch. rootMargin starts
	// the fetch early enough that the content is usually there by the time you scroll to it.
	function sentinel(node: HTMLElement) {
		const io = new IntersectionObserver(([e]) => e.isIntersecting && loadMore(), {
			rootMargin: '400px 0px'
		});
		io.observe(node);
		return () => io.disconnect();
	}

	/**
	 * YouTube's "From the community" shelf is already account-personalized, but it isn't tied to what
	 * the user actually plays *in Limusic*. Swap its items for community playlists searched from
	 * their top artists, keeping the shelf's title and position. With no listening signal yet — or if
	 * the searches fail — YouTube's own items are left exactly as they came. Best-effort: this can
	 * never fail the page.
	 */
	async function cater(page: HomePage, params: string | null) {
		if (params) return; // a mood-filtered feed is the chip's, not the user's
		if (!page.sections.some((s) => /community/i.test(s.title))) return;
		const artists = topArtists(personal, 3);
		if (!artists.length) return;
		const key = `community:${artists.join('|')}`;
		let items = getCached<BrowseItem[]>(key);
		if (!items) {
			const lists = await Promise.all(
				artists.map((a) => api.searchCards(a, 'playlists').catch(() => [] as BrowseItem[]))
			);
			items = interleave(lists, 20);
			if (!items.length) return;
			putCached(key, items);
		}
		if (selected !== params) return; // the user clicked away to a mood feed
		// Re-locate the shelf instead of patching the page we were handed: `home` has very likely moved
		// on while the searches ran (a revalidation, or the Forgotten favourites crawl appending pages).
		const idx = home?.sections.findIndex((s) => /community/i.test(s.title)) ?? -1;
		if (idx < 0) return;
		home = { ...home!, sections: home!.sections.map((s, i) => (i === idx ? { ...s, items } : s)) };
	}

	// Chips only refresh when a response actually carries them (never blank the row mid-switch).
	$effect(() => {
		if (home?.chips?.length) chips = home.chips.filter((c) => c.title !== 'Podcasts');
	});

	onMount(() => load(null));

	// On Repeat crosses its threshold while you listen, so re-check on every track change rather
	// than once per visit: sitting on home through your fifth song should be enough to see the tile.
	// The check is a local SQLite read, and `seedPick` is what actually decides.
	$effect(() => {
		playback.now?.videoId;
		seedOnRepeatPick();
	});
</script>

<div {@attach watchScroll}>
	<HomeHero />
	<!-- Mood chips filter the whole feed, so they're page-level controls: sticky, they stay reachable
	     while the feed scrolls under them instead of leaving with the header they were pinned to.
	     Opaque rather than blurred — a backdrop-filter repainting on every scroll frame is the one
	     thing WebKitGTK reliably chokes on. -->
	{#if chips.length}
		<div class="sticky top-0 z-20 border-b bg-background px-6 pt-2.5">
			<div class="flex gap-2 overflow-x-auto pb-2">
				<!-- An explicit "All" is the way out of a filter. Clicking the active chip again also
				     clears it, but nobody discovers that, and nothing else on screen says you're filtered. -->
				<button onclick={() => load(null)} class={chipClass(!selected)}>All</button>
				{#each chips as chip (chip.params)}
					<button
						onclick={() => load(selected === chip.params ? null : chip.params)}
						class={chipClass(selected === chip.params)}
					>
						{chip.title}
					</button>
				{/each}
			</div>
		</div>
	{:else if loading}
		<!-- Hold the bar's height on a cold load: chips arrive with the feed, and popping them in
		     afterwards shoves the whole page down under the cursor. -->
		<div class="sticky top-0 z-20 border-b bg-background px-6 pt-2.5" aria-hidden="true">
			<div class="flex gap-2 overflow-hidden pb-2">
				{#each ['w-10', 'w-16', 'w-20', 'w-14', 'w-24', 'w-16'] as w, i (i)}
					<Skeleton class="h-8 shrink-0 rounded-full {w}" />
				{/each}
			</div>
		</div>
	{/if}
	<div class="px-6 pb-6 pt-6">
		<!-- Zone one: what's yours. The grid you arranged, above the rule that separates it from
		     everything the app or YouTube chose. It steps aside entirely while a mood filter is
		     active: none of it is filterable, and neither is the arrangement it edits. -->
		{#if !selected}
			<div class="mb-10 border-b pb-8">
				<Shortcuts onEdit={() => (editing = true)} />
			</div>
		{/if}
		{#snippet shelfSkeletons(n: number)}
			{#each Array(n) as _, s (s)}
				<section aria-hidden="true">
					<Skeleton class="mb-3 h-5 w-40 rounded" />
					<div class="flex gap-2 overflow-hidden pb-2">
						{#each Array(6) as _, i (i)}
							<div class="w-40 shrink-0"><MediaCardSkeleton /></div>
						{/each}
					</div>
				</section>
			{/each}
		{/snippet}
		<!-- One ordered column, so the two sections the app builds itself sit among YouTube's shelves
		     instead of above them, and a drag in the Edit modal can put any of them anywhere.
		     gap-10, not gap-8: with a heading, a row of cards and no rule between them, shelves any
		     closer than this stop reading as separate sections. -->
		<div class="content-in flex flex-col gap-10">
			{#each visible as block (block.id)}
				{#if block.shelf}
					<Shelf
						title={block.shelf.title}
						items={block.shelf.items}
						community={/community/i.test(block.shelf.title)}
						onMore={block.shelf.moreBrowseId ? () => showMore(block.shelf!) : undefined}
					/>
				{:else if block.key === RECENT}
					{#if recent.length}<RecentRail items={recent} />{/if}
				{:else if block.key === FAMILIAR}
					<FamiliarArtists />
				{:else if forgotten}
					<ForgottenFavourites
						section={forgotten}
						onMore={forgotten.moreBrowseId ? () => showMore(forgotten!) : undefined}
					/>
				{:else if seeking}
					<!-- Hold the slot open while the crawl runs, so landing the shelf doesn't shove the feed
					     down under the reader's cursor. -->
					<div aria-hidden="true">
						<Skeleton class="mb-3 h-5 w-48 rounded" />
						<div class="columns-1 gap-x-6 md:columns-2 xl:columns-3">
							{#each Array(15) as _, i (i)}
								<div class="break-inside-avoid"><TrackRowSkeleton /></div>
							{/each}
						</div>
					</div>
				{/if}
			{/each}
			{#if loading}
				{@render shelfSkeletons(3)}
			{:else if error}
				<ErrorState message={error} onRetry={() => load(selected)} />
			{:else if !home?.sections.length}
				<!-- A dead end needs a way out, not a sentence. Signed out, that's the sign-in that fills
				     this page; signed in, an empty feed is a bad response and retrying usually fixes it. -->
				<div class="flex flex-col items-center gap-3 py-20 text-center">
					<HugeiconsIcon icon={MusicNote01Icon} class="h-8 w-8 text-muted-foreground/40" />
					<p class="max-w-sm text-sm text-muted-foreground">
						{auth.account?.signedIn
							? 'Your home feed came back empty this time.'
							: 'Sign in and home fills up with mixes and playlists built from what you listen to.'}
					</p>
					{#if auth.account?.signedIn}
						<Button variant="outline" size="sm" onclick={() => load(selected)}>Try again</Button>
					{:else}
						<Button size="sm" onclick={() => api.loginWebview()}>Sign in with Google</Button>
					{/if}
				</div>
			{:else if home.continuation}
				{#if moreError}
					<div class="p-3 text-center">
						<Button variant="outline" size="sm" onclick={loadMore} disabled={loadingMore}>
							{loadingMore ? 'Loading…' : 'Try again'}
						</Button>
					</div>
				{:else}
					<!-- Skeletons only while a page is actually in flight; the sentinel above them is what
					     triggers the fetch when it scrolls into range. -->
					<div class="flex flex-col gap-10" aria-busy={loadingMore}>
						<div {@attach sentinel}></div>
						{#if loadingMore}{@render shelfSkeletons(2)}{/if}
					</div>
				{/if}
			{/if}
		</div>
	</div>
</div>

{#if scrolled}
	<!-- Clears the player bar when there is one. z-10 keeps it under the queue/lyrics overlays. -->
	<button
		transition:fade={{ duration: 150 }}
		onclick={() => scroller?.scrollTo({ top: 0, behavior: 'smooth' })}
		aria-label="Back to top"
		class="fixed right-6 z-10 flex h-11 w-11 cursor-pointer items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg transition-transform hover:scale-110 {playback.now
			? 'bottom-24'
			: 'bottom-6'}"
	>
		<HugeiconsIcon icon={ArrowUpBigIcon} class="h-5 w-5" />
	</button>
{/if}

<HomeLayoutDialog bind:open={editing} sections={known} />
