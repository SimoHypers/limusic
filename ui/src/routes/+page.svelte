<script lang="ts">
	import { onMount, untrack } from 'svelte';
	import { fade } from 'svelte/transition';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		ArrowUpBigIcon,
		DashboardSquareEditIcon,
		MusicNote01Icon,
		ViewOffSlashIcon
	} from '@hugeicons/core-free-icons';
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
		adoptHomeKinds,
		personal,
		playback,
		seedOnRepeatPick,
		toast
	} from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import {
		HOME_KINDS,
		arrangeSections,
		freshen,
		hiddenSections,
		interleave,
		recentItems
	} from '$lib/personal';
	import { getCached, putCached } from '$lib/pagecache';

	const FORGOTTEN_KEY = 'home:forgotten';

	let home = $state<HomePage | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	// The mood chips + which one is active. Kept out of `home` so the row survives a filter switch's
	// loading state (every home response carries the same chips anyway).
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

	// A faint tint at rest, the accent when on, so the one active filter is the only saturated thing
	// above the feed. Tinted rather than outlined: a row of outlines was another dozen lines on a page
	// that had too many (#319), and a tint of the foreground reads on the artwork and on a plain page.
	const chipClass = (active: boolean) =>
		`shrink-0 cursor-pointer rounded-full px-3.5 py-1.5 text-sm font-medium transition-colors ${
			active
				? 'bg-primary text-primary-foreground'
				: 'bg-foreground/5 text-foreground/75 hover:bg-foreground/10 hover:text-foreground'
		}`;

	// The chips stay while a filter is on even when Edit home switched them off: "All" is the way out.
	const showChips = $derived(chips.length > 0 && (personal.home.chips || !!selected));
	/** Anything in the chip row besides Edit Home: the chips, or their skeletons on a cold load. */
	const chipRow = $derived(showChips || (loading && personal.home.chips));
	// The chip bar sits on the header's artwork until it pins to the top, and only then needs a
	// background of its own. Opaque at rest, it cut a hard band across the artwork right under the
	// greeting. The sentinel is the line the bar leaves behind when it sticks.
	let stuck = $state(false);
	function stickWatch(node: HTMLElement) {
		const io = new IntersectionObserver(
			([e]) => (stuck = !e.isIntersecting && e.boundingClientRect.top < (e.rootBounds?.top ?? 0)),
			{ root: node.closest('main') }
		);
		io.observe(node);
		return () => io.disconnect();
	}

	// "Forgotten favourites" is pulled out of the feed and rendered as a list above it (see the
	// markup) — the shelf's cards say nothing about a song, and this one is meant to be read.
	// Songs only: if YouTube ever fills that shelf with something else, it stays a normal card row.
	const isForgotten = (s: HomeSection) =>
		s.kind === 'forgotten' && s.items.some((i) => i.kind === 'song');
	// Held separately from `home`, not derived from it: YouTube sends the shelf a page or two into the
	// feed, so it survives the revalidating `home = fresh` that drops back to page one, and a revisit
	// reads it from the cache instead of walking continuations again.
	let forgotten = $state<HomeSection | null>(null);
	let seeking = $state(false); // walking continuations to find it — the slot shows a skeleton
	// Home shows only the shelves it knows (`HOME_KINDS`); a mood chip's feed shows whatever it sends.
	// So does a signed-out home: it carries five of the kinds and seven general shelves (charts,
	// moods), and without those it is half empty. It is also the same feed every visit, which is
	// what makes listing whatever it sent harmless there.
	const homeKinds = new Set<string>(HOME_KINDS);
	const everything = $derived(!!selected || auth.account?.signedIn === false);
	const feed = $derived(
		home?.sections.filter((s) => !isForgotten(s) && (everything || homeKinds.has(s.kind!))) ?? []
	);

	// --- the arrangement the user set in Edit home (personal.ts) --------------------------------
	// The sections the app builds itself get reserved keys (a YouTube shelf title can't start with
	// "@"), so they keep their slot even before (or without) any content to show.
	const SHORTCUTS = '@shortcuts';
	const RECENT = '@recent';
	const FAMILIAR = '@familiar';
	const FORGOTTEN = '@forgotten';
	type Block =
		| { id: string; key: string; title: string; shelf?: undefined }
		| { id: string; key: string; title: string; shelf: HomeSection };
	let editing = $state(false);
	const hidden = $derived(hiddenSections(personal));
	/** Ours, in the order they sit on a home nobody has arranged. */
	const local = $derived<Block[]>([
		{ id: SHORTCUTS, key: SHORTCUTS, title: t('home.shortcuts') },
		{ id: RECENT, key: RECENT, title: t('home.jump_back_in') },
		{ id: FAMILIAR, key: FAMILIAR, title: t('home.familiar_artists') },
		{ id: FORGOTTEN, key: FORGOTTEN, title: t('home.forgotten_favourites') }
	]);
	/**
	 * Every section home can show, in the user's order, hidden ones included. Shelves are keyed on
	 * their kind, the same in every language, but rendered under a positional id, because a feed
	 * walked far enough does repeat one.
	 *
	 * A mood feed is the chip's: YouTube's order, none of ours, and none of home's arrangement.
	 */
	const blocks = $derived.by(() => {
		const shelves: Block[] = feed.map((s, i) => ({
			id: `${i}:${s.title}`,
			key: s.kind ?? s.title,
			title: s.title,
			shelf: s
		}));
		return selected ? shelves : arrangeSections([...local, ...shelves], personal);
	});
	const visible = $derived(selected ? blocks : blocks.filter((b) => !hidden.has(b.key)));
	/**
	 * What Edit home lists: ours and every kind home knows, whether or not the feed has sent it yet
	 * (it arrives a page at a time, and a section can be hidden or moved before the page gets to it).
	 * Under our own labels, so the list reads the same before anything has loaded. Signed out, the
	 * other shelves loaded so far join them, under YouTube's title.
	 */
	const known = $derived.by(() => {
		const rows = new Map<string, { key: string; title: string }>();
		for (const b of local) rows.set(b.key, b);
		for (const k of HOME_KINDS) rows.set(k, { key: k, title: t(`home.kinds.${k}`) });
		if (!selected) for (const b of blocks) if (!rows.has(b.key)) rows.set(b.key, b);
		return arrangeSections([...rows.values()], personal);
	});

	// Older arrangements are keyed by shelf title; move them over as the feed names the kinds.
	// Untracked: it reads the arrangement it rewrites, which would re-run this effect.
	$effect(() => {
		const sections = !selected && home?.sections;
		if (sections && sections.length) untrack(() => adoptHomeKinds(sections));
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

	/** Forgotten favourites renders at the top but arrives deep in the feed. */
	const wantForgotten = () => !forgotten && !hidden.has(FORGOTTEN);

	/**
	 * A custom arrangement can only be honoured for the shelves that have loaded, so a section the
	 * user dragged upwards stayed missing until they scrolled to wherever YouTube actually put it.
	 * True while some section ranked *above* one already on screen hasn't arrived yet — the ones
	 * ranked below it land at the bottom regardless, which is what scrolling is for.
	 */
	function missingRanked() {
		const order = personal.home.order;
		if (!order.length) return false;
		const rank = new Map(order.map((k, i) => [k, i]));
		const here = new Set([SHORTCUTS, RECENT, FAMILIAR, FORGOTTEN, ...feed.map((s) => s.kind)]);
		let deepest = -1;
		for (const [k, r] of rank) if (here.has(k) && r > deepest) deepest = r;
		// Kinds only: a title key left from another language would never arrive.
		for (const [k, r] of rank)
			if (r < deepest && homeKinds.has(k) && !here.has(k) && !hidden.has(k)) return true;
		return false;
	}

	/**
	 * Walk a few continuations up front rather than leaving those slots empty until the reader
	 * happens to scroll past them. Bounded — the feed is long and this is a nicety.
	 */
	async function seekForgotten(params: string | null) {
		if (params) return; // a mood feed is the chip's, and its shelves aren't home's
		seeking = true;
		try {
			for (let i = 0; i < 6; i++) {
				if (moreError || loadingMore) return;
				if (!wantForgotten() && !missingRanked()) return;
				if (selected !== params || !home?.continuation) return;
				await loadMore(); // latches the forgotten shelf itself if the page carries it
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

	// One key per view of the feed — the base feed, or this mood chip's filter of it.
	const homeKey = () => (selected ? `home:${selected}` : 'home');

	async function load(params: string | null = selected) {
		selected = params;
		const key = homeKey();
		const hit = getCached<HomePage>(key);
		forgotten = params ? null : getCached<HomeSection>(FORGOTTEN_KEY);
		if (hit) {
			home = hit;
			loading = false;
			noteForgotten();
			cater(hit, params);
			// Show the cached feed and leave it alone for this visit. A background revalidation
			// replaces the whole feed on return and re-lays out every section under the restored
			// scroll position — trading freshness for a stable page. The five minutes runs from
			// the last write, not the visit: putCached re-stamps the entry on every write, and
			// loadMore writes here too, so a session that keeps scrolling home can sit on the
			// same page 1 content well past five minutes.
			return;
		}
		loading = true;
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
			// The appended pages live only in the response otherwise — cache the whole feed, or a
			// back-navigation off the first page shows a stub of home and the restore clamps early.
			putCached(homeKey(), home);
			noteForgotten();
		} catch (e) {
			// Stop auto-loading and offer a retry — auto-retrying a visible sentinel would spin.
			moreError = true;
			toast.error(t('toasts.could_not_load_more'));
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
		if (!page.sections.some((s) => s.kind === 'community')) return;
		const artists = (await api.topArtists(3).catch(() => [])).map((a) => a.name);
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
		const idx = home?.sections.findIndex((s) => s.kind === 'community') ?? -1;
		if (idx < 0) return;
		home = { ...home!, sections: home!.sections.map((s, i) => (i === idx ? { ...s, items } : s)) };
	}

	// Chips only refresh when a response actually carries them (never blank the row mid-switch).
	$effect(() => {
		if (home?.chips?.length) chips = home.chips;
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

<!-- isolate: the header's artwork canvas sits at -z-10 and must stay inside this page, above the
     window's background rather than behind it. -->
<div class="relative isolate" {@attach watchScroll}>
	<HomeHero />
	<!-- The feed's control row: Edit Home, then the mood chips. Sticky, so both
	     stay reachable while the feed scrolls under them instead of leaving with the header.
	     Always rendered, even with the chips switched off in Edit home: the button that switches them
	     back on lives here, so this row is the one part of the page that can't be hidden.
	     Opaque when pinned rather than blurred: a backdrop-filter repainting on every scroll frame
	     is the one thing WebKitGTK reliably chokes on. Under it, a short fade instead of a rule. -->
	<div class="h-px" {@attach stickWatch}></div>
	<div
		class="sticky top-0 z-20 flex items-start gap-2 px-6 py-2.5 transition-colors duration-200 {stuck
			? 'bg-background'
			: ''}"
	>
		<!-- Leads the row, ahead of the chips, and never scrolls away with them: the only way into
		     arranging home. An icon beside the chips (the tooltip names it); labelled once the chips
		     are switched off, when it is alone in the row and a bare glyph would say nothing.
		     Sized by the same padding and line height as a chip, plus its outline. Top-aligned with the
		     chips, not centred on the row: an overflowing chip row reserves 4px under itself for the
		     scrollbar, which put the button below them. -mt-px splits the outline's 2px. -->
		<button
			onclick={() => (editing = true)}
			title={chipRow ? t('home.edit_home') : undefined}
			aria-label={chipRow ? t('home.edit_home') : undefined}
			class="-mt-px flex shrink-0 cursor-pointer items-center justify-center gap-1.5 rounded-full border border-border py-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground {chipRow
				? 'px-1.5'
				: 'pl-3 pr-3.5 text-sm font-medium'}"
		>
			<HugeiconsIcon icon={DashboardSquareEditIcon} class="h-5 w-5" />
			{#if !chipRow}{t('home.edit_home')}{/if}
		</button>
		<div class="min-w-0 flex-1">
			{#if showChips}
				<div class="rail flex gap-2 overflow-x-auto">
					<!-- An explicit "All" is the way out of a filter. Clicking the active chip again also
					     clears it, but nobody discovers that, and nothing else on screen says you're filtered. -->
					<button onclick={() => load(null)} class={chipClass(!selected)}>{t('common.all')}</button>
					{#each chips as chip (chip.params)}
						<button
							onclick={() => load(selected === chip.params ? null : chip.params)}
							class={chipClass(selected === chip.params)}
						>
							{chip.title}
						</button>
					{/each}
				</div>
			{:else if loading && personal.home.chips}
				<!-- Hold the chips' place on a cold load: they arrive with the feed. -->
				<div class="flex gap-2 overflow-hidden" aria-hidden="true">
					{#each ['w-10', 'w-16', 'w-20', 'w-14', 'w-24', 'w-16'] as w, i (i)}
						<Skeleton class="h-8 shrink-0 rounded-full {w}" />
					{/each}
				</div>
			{/if}
		</div>
		{#if stuck}
			<div
				class="pointer-events-none absolute inset-x-0 top-full h-5 bg-gradient-to-b from-background to-transparent"
			></div>
		{/if}
	</div>
	<div class="px-6 pb-8 pt-4">
		{#snippet shelfSkeletons(n: number)}
			{#each Array(n) as _, s (s)}
				<section aria-hidden="true">
					<Skeleton class="mb-4 h-6 w-44 rounded" />
					<div class="flex gap-2 overflow-hidden pb-2">
						{#each Array(6) as _, i (i)}
							<div class="w-40 shrink-0"><MediaCardSkeleton /></div>
						{/each}
					</div>
				</section>
			{/each}
		{/snippet}
		<!-- One ordered column, so the sections the app builds itself sit among YouTube's shelves
		     instead of above them, and a drag in Edit home can put any of them anywhere.
		     Space is the only thing between sections, no rules (#319), so it has to be generous:
		     any closer than gap-12 and a heading reads as the caption of the row above it. -->
		<div class="content-in flex flex-col gap-12">
			{#each visible as block (block.id)}
				{#if block.shelf}
					<Shelf
						title={block.shelf.title}
						items={block.shelf.items}
						queueAll={false}
						size={personal.home.cards}
						community={block.shelf.kind === 'community'}
						onMore={block.shelf.moreBrowseId ? () => showMore(block.shelf!) : undefined}
					/>
				{:else if block.key === SHORTCUTS}
					<Shortcuts />
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
						<Skeleton class="mb-4 h-6 w-52 rounded" />
						<div class="columns-1 gap-x-6 md:columns-2 xl:columns-3">
							{#each Array(15) as _, i (i)}
								<div class="break-inside-avoid"><TrackRowSkeleton /></div>
							{/each}
						</div>
					</div>
				{/if}
			{/each}
			{#if !selected && !visible.length && home?.sections.length && !loading && !loadingMore && !seeking}
				<!-- Everything switched off. Say so, and hand back the way in, rather than a blank page
				     under the chips that looks like a feed that failed to load. -->
				<div class="flex flex-col items-center gap-3 py-20 text-center">
					<HugeiconsIcon icon={ViewOffSlashIcon} class="h-8 w-8 text-muted-foreground/40" />
					<p class="max-w-sm text-sm text-muted-foreground">{t('home.all_hidden')}</p>
					<Button variant="outline" size="sm" onclick={() => (editing = true)}>
						{t('home.edit_home')}
					</Button>
				</div>
			{/if}
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
							? t('home.feed_empty')
							: t('home.signed_out_hint')}
					</p>
					{#if auth.account?.signedIn}
						<Button variant="outline" size="sm" onclick={() => load(selected)}>{t('common.try_again')}</Button>
					{:else}
						<Button size="sm" onclick={() => api.loginWebview()}>{t('common.sign_in_google')}</Button>
					{/if}
				</div>
			{:else if home.continuation}
				{#if moreError}
					<div class="p-3 text-center">
						<Button variant="outline" size="sm" onclick={loadMore} disabled={loadingMore}>
							{loadingMore ? t('common.loading') : t('common.try_again')}
						</Button>
					</div>
				{:else}
					<!-- Skeletons only while a page is actually in flight; the sentinel above them is what
					     triggers the fetch when it scrolls into range. -->
					<div class="flex flex-col gap-12" aria-busy={loadingMore}>
						<!-- Remounted per page: a page whose shelves home doesn't show leaves the sentinel
						     in view, and the observer only fires on entering it. -->
						{#key home.sections.length}<div {@attach sentinel}></div>{/key}
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
		aria-label={t('a11y.back_to_top')}
		class="fixed right-6 z-10 flex h-11 w-11 cursor-pointer items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg transition-transform hover:scale-110 {playback.now
			? 'bottom-24'
			: 'bottom-6'}"
	>
		<HugeiconsIcon icon={ArrowUpBigIcon} class="h-5 w-5" />
	</button>
{/if}

<HomeLayoutDialog bind:open={editing} sections={known} />
