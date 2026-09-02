<script lang="ts">
	import { untrack } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		PlayIcon,
		ShuffleIcon,
		PencilEdit02Icon,
		Delete02Icon,
		MoreVerticalIcon,
		Radio02Icon,
		ArrowUpNarrowWideIcon,
		ArrowDownWideNarrowIcon,
		DashboardSquare02Icon,
		Share08Icon,
		BookmarkAdd02Icon,
		BookmarkCheck02Icon,
		BookmarkMinus02Icon,
		ListRestartIcon,
		Sorting01Icon,
		ArrowUpDownIcon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import * as RadioGroup from '$lib/components/ui/radio-group';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import EditPlaylistDialog from '$lib/components/EditPlaylistDialog.svelte';
	import TrackFilter, { filterTracks } from '$lib/components/TrackFilter.svelte';
	import TrackRowSkeleton from '$lib/components/TrackRowSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import * as api from '$lib/api';
	import { ON_REPEAT_ID } from '$lib/api';
	import type { BrowseItem, PlaylistPage, SongItem } from '$lib/api';
	import { getCached, putCached, invalidateCached } from '$lib/pagecache';
	import { thumb } from '$lib/thumb';
	import { anchorMenu, fitMenu, NO_ANCHOR } from '$lib/menu';
	import { rowWindow } from '$lib/rows';
	import { rowScroller } from '$lib/rows.svelte';
	import { t } from '$lib/i18n.svelte';
	import {
		SORTS,
		fetchSort,
		persistedSort,
		sortSongs,
		storedExactly,
		type SortKey
	} from '$lib/sort';
	import {
		addPick,
		addToLibrary,
		auth,
		library,
		ownedByUser,
		removeFromLibrary,
		enqueue,
		isSaved,
		isSynced,
		playback,
		openAddToPlaylist,
		openShare,
		playFrom,
		startRadio,
		toast,
		toggleSaved,
		bumpLibraryTrackCount,
		noteUnsavedFrom,
		patchLibraryPlaylist,
		lastPlaylistAdd
	} from '$lib/player.svelte';

	// `$state.raw`, not `$state`: a deep proxy makes every read of a row go through a trap and
	// create a signal, and this list runs to five figures. Measured at 5,000 rows, one filter pass
	// is 0.6ms over a plain array and 6.4ms through the proxy, and both the filter box and a local
	// sort do that pass on every keystroke and on every continuation page that lands. Raw is safe
	// here because every write below reassigns the whole object (`pl = { ...pl, … }`); nothing
	// mutates `pl` in place. Keep it that way, or the page stops updating.
	let pl = $state.raw<PlaylistPage | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let loadingMore = $state(false);
	let moreError = $state(false);
	let inflight: Promise<void> | null = null;
	let confirmingDelete = $state(false);
	// A random song's cover, the hero backdrop when the playlist has no cover of its own.
	let bgImage = $state<string | null>(null);

	// ⋯ options menu, positioned `fixed` at the button so it isn't clipped (matches TrackRow).
	let menuOpen = $state(false);
	let anchor = $state(NO_ANCHOR);

	// "Edit playlist": name, description, visibility and a cover of your own.
	let editing = $state(false);
	// The header's description, clamped to two lines until "More" is pressed.
	let expanded = $state(false);
	// ponytail: character count, not a measured overflow. It only decides whether the More button
	// is worth drawing, and a blurb that short reads fine in full either way; measure the paragraph
	// (and re-measure on resize) only if the button starts showing up under one-liners.
	const longDescription = $derived((pl?.description?.length ?? 0) > 120);
	// The artwork on the page: whatever the user picked on this machine, else YouTube's own.
	const art = $derived(thumb(pl?.cover ?? pl?.thumbnail, 400));

	// YouTube's auto-built 2x2 collage of the first four tracks. It comes off yt3 with an `=s<size>`
	// suffix; every cover somebody actually chose (uploaded here, in YTM, or in Studio) arrives as
	// `=w<n>-h<n>-...` or straight off i.ytimg. Checked against live browse responses, 2026-09-02.
	const COLLAGE = /yt3\.(ggpht|googleusercontent)\.com\/.*=s\d+/;
	// The backdrop: a cover the playlist really has, else a random song's art. The collage isn't a
	// cover anyone picked, and blown up behind the header it just repeats the rows below it.
	const backdrop = $derived(
		thumb(pl?.cover ?? (pl?.thumbnail && !COLLAGE.test(pl.thumbnail) ? pl.thumbnail : null), 1200) ??
			bgImage
	);

	// Header filter box: matches title / artist / album over the rows loaded so far.
	//
	// Two values, not one. `query` is what the input holds, so typing never waits on anything.
	// `applied` is what the list is actually narrowed by, and it lags by `FILTER_DEBOUNCE_MS`,
	// because the two things a query sets off are both expensive: a pass over every loaded row
	// (five figures on Liked Music), and `loadAll()`, which is up to 50 sequential requests for
	// the pages a filter has to cover. Typing "beat" should not start that walk at "b".
	const FILTER_DEBOUNCE_MS = 300;
	let query = $state('');
	let applied = $state('');
	let filterTimer: ReturnType<typeof setTimeout> | undefined;
	$effect(() => {
		const q = query;
		// Clearing the box is instant: there is nothing to compute and nothing to fetch, and a
		// list that stays narrowed for a third of a second after you hit the ✕ reads as broken.
		if (!q.trim()) {
			clearTimeout(filterTimer);
			applied = '';
			return;
		}
		filterTimer = setTimeout(() => (applied = q), FILTER_DEBOUNCE_MS);
		return () => clearTimeout(filterTimer);
	});

	const id = $derived(page.params.id ?? '');
	const nowId = $derived(playback.now?.videoId);
	// The liked-music auto-playlist isn't a user playlist — no rename/delete, but shuffle is fine.
	const isLiked = $derived(id === api.LIKED_MUSIC_ID);
	// On Repeat is built locally from play counts: no artwork, and no radio to seed autoplay from.
	const isOnRepeat = $derived(id === ON_REPEAT_ID);
	// Only offer rename/delete on playlists the signed-in user actually owns (backend `owned` flag).
	// Liked Music reports owned but can't be renamed/deleted, so exclude it explicitly.
	const editable = $derived((pl?.owned ?? false) && !isLiked);
	// Saving someone else's playlist keeps it on this machine, signed in or not: YouTube has no
	// "save" for a playlist that doesn't cost an account, and the local one works offline. Your own
	// playlists, Liked Music and On Repeat are in the library already by definition.
	// Once the sync button has put it on the account, the account owns the save: removing only the
	// local copy would leave it in the library grid, so the entry hides until the user signs out.
	// On the account already (pushed from here, or saved on another device): the local copy is not
	// what holds it in the library any more, so the menu says so instead of offering a second save.
	const inAccount = $derived(
		!!auth.account?.signedIn && (isSynced(id) || library.items.some((i) => i.id === id))
	);
	const savable = $derived(!isOnRepeat && !isLiked && !editable && !inAccount);
	const savedHere = $derived(isSaved(id));
	// YouTube's header count includes rows that never make it into the list (unavailable or
	// region-blocked tracks), so it reads high. Once every page is in, we know the real number, so
	// swap it in. Until then the header's own count is the only estimate of the total there is.
	const subtitle = $derived(
		pl && !pl.continuation && pl.items.length
			? (pl.subtitle ?? '').replace(/^[\d,.]+ songs?/i, `${pl.items.length} songs`)
			: pl?.subtitle
	);
	// --- sorting (`$lib/sort`) ---------------------------------------------------------------
	let sort = $state<SortKey>('default');
	let desc = $state(false);
	let sortOpen = $state(false);
	let sortAnchor = $state(NO_ANCHOR);
	let preparing = $state(false);
	// A YouTube sort is a round trip, so the rows on screen are the previous order until it lands.
	// Dim them meanwhile, or picking a sort looks like it did nothing.
	let resorting = $state(false);

	const sortLabel = $derived(
		sort === 'default' ? t('sort.label') : t(`sort.${sort}`)
	);
	// The local listening history, fetched once and only if "Most played" is ever picked — it is a
	// SQLite read the other five sorts have no use for.
	let plays = $state<Record<string, number>>({});
	let playsInflight: Promise<void> | null = null;
	function loadPlays(): Promise<void> {
		playsInflight ??= api
			.getPlayCounts()
			.then((c) => void (plays = c))
			// An empty map just sorts everything as unplayed, which beats blocking the sort.
			.catch(() => {});
		return playsInflight;
	}

	// YouTube offers a sort menu for this list, so it does the ordering — all of it except "Most
	// played", which is our own listening history and means nothing to YouTube.
	const serverSorted = $derived(!!pl?.sortMenu && sort !== 'plays');
	// …and on a playlist we own the choice is a write, so every other client follows it.
	const storable = $derived(pl?.sortMenu?.editable ?? false);
	// Liked Music has no editable menu, but YouTube remembers whichever order it was last asked
	// for anyway. Someone else's playlist remembers nothing, so that one falls to localStorage.
	const keepsSort = $derived(!!pl?.sortMenu && (storable || isLiked));

	const sortedItems = $derived.by(() => {
		const items = pl?.items ?? [];
		// The rows already arrived in order, reversed ones included. The single order YouTube has
		// no params for is a reversed *manual* order, so that one reverse stays here.
		if (serverSorted) return sort === 'default' && desc ? items.slice().reverse() : items;
		// Liked Music is the one playlist YouTube hands back newest-addition-first.
		return sortSongs(items, sort, isLiked, desc, plays);
	});

	// The rows actually on screen: the sorted list, narrowed by the header's filter box. Identical
	// to `sortedItems` with no query typed.
	const shown = $derived(filterTracks(sortedItems, applied));
	const filtering = $derived(!!applied.trim());

	// A sort has to cover the whole playlist, not the pages scrolled so far, so pull the rest in.
	// Stops on a failed page (`moreError`), on navigation, and on any pass that made no progress.
	// Answers whether it got the lot: a queue built from a short list is missing tracks for good,
	// so the caller has to be able to say so rather than quietly handing over half a playlist.
	async function loadAll(): Promise<boolean> {
		const pid = id;
		moreError = false; // a page that failed earlier gets another go on an explicit action
		while (pl?.continuation && !moreError) {
			const token = pl.continuation;
			await loadMore();
			if (pid !== id) return false;
			if (pl?.continuation === token) break; // no progress, and nothing left to try
		}
		return !pl?.continuation;
	}

	// Sorting rows here is only honest once every page is in. A YouTube sort already covers the
	// whole list (and its continuation token pages on in that same order), so the walk is down to
	// the two orders it cannot produce: our play counts, and a reversed manual order.
	const sorting = $derived(
		serverSorted ? sort === 'default' && desc : sort !== 'default' || desc
	);

	// Everything that hands tracks to the queue goes through here first: a sorted queue is only
	// honest once every page is in.
	async function ready(): Promise<boolean> {
		if (!sorting) return true;
		if (sort === 'plays') await loadPlays(); // queueing before they land would sort by nothing
		if (!pl?.continuation) return true;
		preparing = true;
		try {
			return await loadAll();
		} finally {
			preparing = false;
		}
	}

	// Sorted, but a page never arrived. The queue is a snapshot, so the tracks that did not load
	// are gone from it for good — play them anyway and say so, rather than refusing to play at all
	// over one failed request. The list's own "Try again" sits at the bottom of the page.
	function warnPartial(what: 'queued' | 'added') {
		toast.error(t(what === 'queued' ? 'toasts.partial_playlist_queued' : 'toasts.partial_playlist_added'));
	}

	// One cache entry per order asked for. "No order asked for" keeps the bare key, because the
	// artist page and the community cards cache a playlist under that one too.
	const cacheKey = (pid: string, s: SortKey | null, d: boolean) =>
		!s || (s === 'default' && !d) ? `playlist:${pid}` : `playlist:${pid}:${s}${d ? ':desc' : ''}`;
	// The key the rows on screen came from, so an optimistic mutation writes back to the entry it
	// actually read and never overwrites a different order's. Set by every load.
	let loadedKey = '';

	// Only what YouTube will not hold for us goes in here: "Most played", a reversed
	// Title/Artist/Album, and any sort on a list it stores none for (someone else's playlist, a
	// radio mix). Everything else is read back off the browse response instead, so a sort changed
	// in YouTube Music turns up here too.
	const SORT_STORE = 'playlist_sort';
	type SavedSort = { sort: SortKey; desc: boolean };

	function readSort(pid: string): SavedSort | null {
		try {
			return JSON.parse(localStorage.getItem(SORT_STORE) ?? '{}')[pid] ?? null;
		} catch {
			return null;
		}
	}

	function rememberSort(pid: string, keptByYouTube: boolean) {
		try {
			const all = JSON.parse(localStorage.getItem(SORT_STORE) ?? '{}');
			if (keptByYouTube) delete all[pid];
			else all[pid] = { sort, desc } satisfies SavedSort;
			localStorage.setItem(SORT_STORE, JSON.stringify(all));
		} catch {
			/* a disabled or full store just means this one sort isn't remembered */
		}
	}

	function chooseSort(key: SortKey) {
		sortOpen = false;
		if (key === sort) return;
		sort = key;
		if (key === 'plays') loadPlays(); // the list re-sorts itself when the counts land
		applySort();
	}

	function toggleDesc() {
		desc = !desc;
		applySort();
	}

	// A sort change is a different request, not a different view of the same one: YouTube owns the
	// order, so the rows have to come back from it. Storing the choice first is what makes the sort
	// outlive the visit and show up in YouTube Music.
	async function applySort() {
		const pid = id;
		rememberSort(pid, keepsSort && storedExactly(sort, desc));
		if (!pl?.sortMenu) {
			// Nothing to ask YouTube for. Sort the rows here instead, once they are all here.
			if (sorting) loadAll();
			return;
		}
		// Store it before re-reading, so the page that comes back is the one other clients see too.
		const store = storable ? persistedSort(sort, desc) : null;
		if (store) {
			try {
				await api.setPlaylistSort(pid, store);
			} catch (e) {
				// The order still applies here; only the carry-over to other clients is lost.
				toast.error(t('toasts.sort_not_saved', { error: String(e) }));
			}
			if (pid !== id) return;
		}
		await fetchSorted(pid);
		// "Most played" and a reversed manual order are still ours to do, over the whole list.
		if (pid === id && sorting) loadAll();
	}

	// Ask YouTube for the list in the current order. `key` doubles as the identity of this request:
	// picking a second sort while the first is in the air must not let the first one land on top.
	async function fetchSorted(pid: string) {
		const key = cacheKey(pid, sort, desc);
		const current = () => pid === id && key === cacheKey(id, sort, desc);
		const hit = getCached<PlaylistPage>(key);
		if (hit) {
			loadedKey = key;
			pl = hit;
			return;
		}
		resorting = true;
		try {
			const fresh = await api.getPlaylist(pid, fetchSort(sort), desc);
			putCached(key, fresh); // still the right rows for that order, superseded or not
			if (!current()) return;
			loadedKey = key;
			pl = fresh;
		} catch (e) {
			if (current()) toast.error(t('toasts.sort_failed', { error: String(e) }));
		} finally {
			// Only the newest pick clears it; an older one finishing late must not un-dim the list.
			if (current()) resorting = false;
		}
	}

	// Right-anchored, unlike the ⋯ menu: this button sits at the far end of the header, so a menu
	// wider than it would run off the page opening leftwards from its left edge.
	function openSort(e: MouseEvent) {
		sortAnchor = anchorMenu(e, { align: 'right' });
		sortOpen = true;
	}

	async function load(pid: string) {
		// A sort YouTube keeps is read back off the response below; this store only holds the ones
		// it cannot (see `rememberSort`), so an entry here means "ask for exactly this".
		const saved = readSort(pid);
		sort = saved?.sort ?? 'default';
		desc = saved?.desc ?? false;
		const key = cacheKey(pid, saved && sort, desc);
		loadedKey = key;
		const hit = getCached<PlaylistPage>(key);
		confirmingDelete = false;
		editing = false;
		expanded = false;
		sortOpen = false;
		query = '';
		applied = '';
		clearTimeout(filterTimer);
		// A page that failed on the last playlist would otherwise keep this one's retry state
		// showing, and block the filter's own walk (`loadAll` bails while it's set).
		moreError = false;
		if (hit) {
			pl = hit;
			if (!saved) sort = hit.sortMenu?.selected ?? 'default';
			bgImage = pickCover(hit.items);
			loading = false;
		} else {
			loading = true;
			pl = null;
			bgImage = null;
		}
		error = null;
		const [askedSort, askedDesc] = [sort, desc];
		try {
			// Nothing saved means asking for no order at all: what comes back is the order the
			// account already has this list in, which is the one YouTube Music would show.
			const fresh = await api.getPlaylist(pid, saved ? fetchSort(sort) : undefined, desc);
			// Superseded by navigation, or by a sort picked off the cached rows while this was in
			// the air — either way `fetchSorted` owns the page now, so drop this response.
			if (pid !== id || sort !== askedSort || desc !== askedDesc) return;
			pl = fresh;
			if (!saved) sort = fresh.sortMenu?.selected ?? 'default';
			bgImage = pickCover(fresh.items);
			putCached(key, fresh);
		} catch (e) {
			if (pid !== id) return;
			if (!hit) error = String(e);
		} finally {
			if (pid === id) loading = false;
		}
	}

	// Reload whenever the route param changes (playlist → playlist navigation), and *only* then.
	// untrack: `load` both reads and writes `sort`/`desc` — it adopts the order YouTube has the list
	// in — so tracking them would make every finished load re-run this effect and fetch the playlist
	// again, forever, on any list not sitting on Default.
	$effect(() => {
		const pid = id;
		if (pid) untrack(() => load(pid));
	});

	// Songs added to THIS playlist via the picker (e.g. from the queue) appear immediately.
	// Epoch-guarded so an add is applied once; adds to other playlists are just marked seen.
	let seenAddEpoch = lastPlaylistAdd.epoch;
	$effect(() => {
		if (lastPlaylistAdd.epoch === seenAddEpoch) return;
		seenAddEpoch = lastPlaylistAdd.epoch;
		if (!pl || lastPlaylistAdd.playlistId !== id) return;
		pl = { ...pl, items: [...pl.items, ...lastPlaylistAdd.songs] };
		cacheCurrent();
		fillSetVideoIds();
	});

	// Optimistic rows lack set_video_id, so "Remove from playlist" is hidden on them. Refetch and
	// patch the real ids into place (merge, not replace — keeps loadMore pages and any row YouTube
	// hasn't reflected yet). Retries because the add is eventually-consistent on YouTube's side.
	async function fillSetVideoIds() {
		if (isLiked) return;
		const pid = id;
		for (const delay of [0, 2000, 4000]) {
			if (delay) await new Promise((r) => setTimeout(r, delay));
			if (pid !== id || !pl) return;
			try {
				// Same order the rows on screen are in, so the two lists line up row for row.
				const fresh = await api.getPlaylist(pid, fetchSort(sort), desc);
				if (pid !== id || !pl) return;
				const used = new Set(pl.items.map((t) => t.set_video_id).filter(Boolean));
				pl = {
					...pl,
					subtitle: fresh.subtitle, // header track count catches up too
					items: pl.items.map((t) => {
						if (t.set_video_id) return t;
						const match = fresh.items.find(
							(f) => f.video_id === t.video_id && f.set_video_id && !used.has(f.set_video_id)
						);
						if (!match) return t;
						used.add(match.set_video_id);
						return { ...t, set_video_id: match.set_video_id };
					})
				};
				cacheCurrent();
				if (pl.items.every((t) => t.set_video_id)) return;
			} catch {
				/* retry on the next pass */
			}
		}
	}

	// Keep the page cache in step with optimistic mutations so a revisit within the TTL never
	// resurrects pre-mutation data (the optimistic-UI contract). context: plans/007.
	function cacheCurrent() {
		if (pl && loadedKey) putCached(loadedKey, pl);
	}

	// One page at a time, shared: the scroll sentinel and the "load the rest before playing" walk
	// both go through here, so they can never fire overlapping requests for the same token.
	function loadMore(): Promise<void> {
		inflight ??= fetchPage().finally(() => (inflight = null));
		return inflight;
	}

	async function fetchPage() {
		const token = pl?.continuation;
		if (!token) return;
		loadingMore = true;
		moreError = false;
		try {
			const more = await api.getPlaylistMore(token);
			if (pl?.continuation !== token) return; // stale (navigated or mutated mid-flight)
			pl = {
				...pl,
				items: [...pl.items, ...more.items],
				// An empty page would leave the sentinel in view with nothing to show — that's the end.
				continuation: more.items.length ? more.continuation : undefined
			};
			cacheCurrent();
		} catch {
			// Stop auto-loading and offer a retry — auto-retrying a visible sentinel would spin.
			moreError = true;
		} finally {
			loadingMore = false;
		}
	}

	// Only the rows around the viewport are rendered; the rest are two padded boxes (`rows.ts`).
	// A Liked Songs list runs to five figures, and `content-visibility` spares the layout and the
	// paint but not the DOM node, the style or the component.
	const sc = rowScroller();
	// The header scrolls away with the rows, so the window is measured from where row 0 sits
	// rather than from the top of the scroller.
	const win = $derived(
		rowWindow(sc.scrollTop - sc.offsetPx, sc.viewportPx, shown.length, sc.rowPx)
	);

	// One page per approach to the bottom: the observer only fires when the sentinel *enters* view,
	// so an appended page that pushes it back out is required before the next fetch. rootMargin
	// starts the fetch early enough that the rows are usually there by the time you reach them.
	function sentinel(node: HTMLElement) {
		const io = new IntersectionObserver(([e]) => e.isIntersecting && loadMore(), {
			rootMargin: '600px 0px'
		});
		io.observe(node);
		return () => io.disconnect();
	}

	// A filter can only match rows that have arrived, and a narrowed list never pushes the sentinel
	// back out of view, so the observer fires once and stops. Same walk a sort needs, for the same
	// reason: the search has to cover the whole playlist, not the pages scrolled so far. The flag
	// keeps one walk running rather than starting a fresh one on every page it lands.
	//
	// Keyed on the playlist id rather than a bare boolean because this component is reused across
	// playlist-to-playlist navigation (that's why `load` above is itself driven by an `$effect` on
	// `id`, not a fresh mount). A walk started on playlist A can still be in flight when you
	// navigate to B and filter there; a plain `walking` flag would still read true from A, block
	// B's walk from ever starting, and then A's `finally` would clear a flag B never got to set,
	// so B's filter would search only whatever it happened to have loaded and could wrongly show
	// "No tracks match" with real matches still unfetched. Comparing against the current `id`
	// means a different playlist is never blocked by someone else's walk, and capturing the id a
	// walk started for (`pid`) into its own `finally` means a stale walk can only ever clear its
	// own marker, never a fresher walk's.
	let walkingFor: string | null = null;
	$effect(() => {
		if (!filtering || !pl?.continuation || walkingFor === id) return;
		const pid = id;
		walkingFor = pid;
		loadAll().finally(() => {
			if (walkingFor === pid) walkingFor = null;
		});
	});

	// This playlist as a card, for the sidebar's last-played sort and the Shortcuts grid.
	// Saving goes through the shared path (local row, plus the account write when signed in), so a
	// playlist saved here is in the library everywhere and not only until the next sync. Removal
	// stays local: undoing YouTube's own copy is what `inAccount` hides this row for.
	async function saveToLibrary() {
		if (savedHere) {
			toggleSaved(asItem());
			toast.success(t('library.removed_from_library'));
			return;
		}
		try {
			const result = await addToLibrary(asItem());
			toast.success(
				result === 'already' ? t('toasts.already_in_library') : t('library.saved_to_library')
			);
		} catch (e) {
			toast.error(String(e));
		}
	}

	async function unsaveFromLibrary() {
		try {
			await removeFromLibrary(asItem());
			toast.success(t('library.removed_from_library'));
		} catch (e) {
			toast.error(String(e));
		}
	}

	const asItem = (): BrowseItem => ({
		kind: 'playlist',
		id,
		title: pl?.title ?? t('common.playlist_singular'),
		subtitle,
		// On Repeat stays artwork-free wherever it's rendered (shortcuts, recents) so it always
		// draws its icon rather than one of its songs' covers.
		thumbnail: isOnRepeat ? undefined : (pl?.cover ?? pl?.thumbnail ?? bgImage ?? undefined)
	});

	// `sourceId` points autoplay at that playlist's radio. On Repeat has no YouTube id, so pass
	// none and let autoplay seed off the last video instead. The queue is the whole playlist, not
	// the pages scrolled so far, but waiting for it here is what made long playlists take forever
	// to start: YouTube hands out tracks 100 at a time and the tokens are chained, so the backend
	// takes the token and walks the rest into the queue while page 1 is already playing.
	//
	// A YouTube sort keeps the token: the pages behind it continue that same order, so the backend
	// walking them is exactly right. The two orders it cannot produce (our play counts, a reversed
	// manual order) do drop it, because there the token would walk YouTube's order in behind a
	// queue sorted here; `ready()` has walked those pages into `pl.items` instead. The queue is a
	// snapshot either way, so a later sort change never touches one that is already playing.
	//
	// A filter narrows which rows are on screen but never what gets queued: it finds a track, it
	// doesn't decide what plays after it, so playing a match leaves the same queue behind as
	// scrolling to that row would.
	async function playAll(start: number | null) {
		if (!pl) return;
		const pid = id;
		// Resolve the clicked row to a track first: awaiting the walk can grow and re-sort the list
		// under it, which would leave the index pointing at a different song.
		const pick = start === null ? null : shown[start];
		const whole = await ready();
		// Navigating while that walk ran would otherwise play the playlist you left for.
		if (!pl || pid !== id) return;
		if (!whole) warnPartial('queued');
		const items = sortedItems;
		const at = pick ? items.indexOf(pick) : -1;
		playFrom(
			asItem(),
			items,
			at >= 0 ? at : null,
			isOnRepeat ? undefined : id,
			undefined,
			sorting ? undefined : pl.continuation
		);
	}

	// Random cover from the songs, picked once per load so it stays stable while browsing
	// (loadMore appends tracks without changing it).
	function pickCover(items: SongItem[]): string | null {
		const withThumb = items.filter((t) => t.thumbnail);
		if (!withThumb.length) return null;
		const url = withThumb[Math.floor(Math.random() * withThumb.length)].thumbnail!;
		return hiRes(url);
	}

	// List thumbnails come at a small size; YouTube/Google encode the size in the URL, so bump it
	// for a crisp full-width backdrop.
	function hiRes(url: string): string {
		return url.replace(/=w\d+-h\d+/, '=w1200-h1200').replace(/=s\d+/, '=s1200');
	}

	// Same deal as `playAll` for a long playlist: the loaded pages go in now and the token hands
	// the rest to the backend to walk in behind them.
	// A "Play next" block stays capped at the loaded tracks either way (see `enqueue`), so only
	// "Add to queue" is worth waiting on the rest of the playlist for.
	async function queue(next: boolean) {
		if (!pl?.items.length) return;
		const pid = id;
		const whole = next || (await ready());
		if (!pl || pid !== id) return;
		if (!whole) warnPartial('added');
		enqueue(sortedItems, next, pl.title, sorting ? undefined : pl.continuation);
	}

	// Untouched by the sort: the backend shuffles the whole playlist (continuation pages included),
	// so what order it was handed is irrelevant.
	function shufflePlay() {
		if (!pl?.items.length) return;
		// Real order + shuffle flag — the backend owns shuffling, so the shuffle toggle can
		// restore the true playlist order and every re-shuffle is fresh. It also mixes each page
		// it walks into the unplayed tail, so this stays a shuffle of the whole playlist rather
		// than of the pages that happen to be loaded.
		playFrom(asItem(), pl.items, null, isOnRepeat ? undefined : id, true, pl.continuation);
	}

	function openMenu(e: MouseEvent) {
		anchor = anchorMenu(e);
		menuOpen = true;
	}
	function run(action: () => void) {
		menuOpen = false;
		action();
	}

	// The dialog hands over what it changed (and hands the old values back if YouTube refused it),
	// so the page never waits on a refetch to show an edit. The sidebar/Library row follows.
	function applyEdit(patch: {
		title?: string;
		description?: string;
		privacy?: string;
		cover?: string;
		thumbnail?: string;
	}) {
		if (!pl) return;
		pl = { ...pl, ...patch };
		cacheCurrent();
		if ('title' in patch || 'cover' in patch || 'thumbnail' in patch) {
			// A row has to keep a name: a page whose header never gave us a title would otherwise
			// blank the sidebar entry on a cover change.
			patchLibraryPlaylist(id, {
				...(pl.title ? { title: pl.title } : {}),
				thumbnail: pl.cover ?? pl.thumbnail
			});
		}
	}

	// The liked-music auto-playlist can't be edited like a normal one — removing = un-liking.
	async function removeTrack(track: SongItem) {
		if (!pl) return;
		if (!isLiked && !track.set_video_id) return;
		const prev = pl.items;
		// Reassign `pl` (not mutate `pl.items`) so the list re-renders immediately. Match by the
		// per-instance setVideoId on normal playlists (duplicates), by videoId on liked music.
		const kept = pl.items.filter((t) =>
			isLiked ? t.video_id !== track.video_id : t.set_video_id !== track.set_video_id
		);
		pl = { ...pl, items: kept };
		try {
			if (isLiked) {
				await api.rate(track.video_id, 'indifferent');
				toast.success(t('toasts.removed_from_liked'));
			} else {
				await api.removeFromPlaylist(id, track.video_id, track.set_video_id!);
				bumpLibraryTrackCount(id, -1);
				noteUnsavedFrom(id, track.video_id);
				toast.success(t('toasts.removed_from_playlist'));
			}
			cacheCurrent();
		} catch (e) {
			pl = { ...pl, items: prev }; // revert
			cacheCurrent();
			toast.error(String(e));
		}
	}

	async function deleteThisPlaylist() {
		try {
			await api.deletePlaylist(id);
			invalidateCached(`playlist:${id}`);
			toast.success(t('toasts.playlist_deleted'));
			goto('/library');
		} catch (e) {
			toast.error(String(e));
			confirmingDelete = false;
		}
	}
</script>

<div class="flex h-full flex-col">
	{#if loading}
		<div class="flex items-end gap-6 border-b p-6">
			<Skeleton class="h-40 w-40 shrink-0 rounded-xl" />
			<div class="flex-1 space-y-3">
				<Skeleton class="h-3 w-16 rounded" />
				<Skeleton class="h-10 w-2/3 rounded-lg" />
				<Skeleton class="h-4 w-40 rounded" />
				<Skeleton class="h-9 w-24 rounded-4xl" />
			</div>
		</div>
		<div class="p-4">
			{#each Array(8) as _, i (i)}
				<TrackRowSkeleton />
			{/each}
		</div>
	{:else if error}
		<div class="p-6"><ErrorState message={error} onRetry={() => load(id)} /></div>
	{:else if pl}
		<!-- One scroller for the whole page: the header scrolls away above the rows, same as the
		     album page. -->
		<div class="content-in min-h-0 flex-1 overflow-y-auto" {@attach sc.attach}>
			<div class="relative flex min-h-[38vh] shrink-0 items-end gap-6 overflow-hidden border-b p-6">
				{#if backdrop}
					<img
						src={backdrop}
						alt=""
						class="pointer-events-none absolute inset-0 h-full w-full object-cover object-center"
					/>
				{/if}
				<!-- Fade the cover into the page so the text stays readable: solid at the bottom and on the
				     left (behind the title), the image itself visible toward the top-right. -->
				<div
					class="absolute inset-0 bg-gradient-to-t from-background via-background/60 to-background/20"
				></div>
				<div class="absolute inset-0 bg-gradient-to-r from-background via-background/50 to-transparent"></div>
				{#if isOnRepeat}
					<div
						class="relative flex h-40 w-40 items-center justify-center rounded-xl bg-primary/10 text-primary shadow-lg"
					>
						<HugeiconsIcon icon={ListRestartIcon} class="h-20 w-20" />
					</div>
				{:else if art}
					<img src={art} alt="" class="relative h-40 w-40 rounded-xl object-cover shadow-lg" />
				{:else}
					<div class="relative h-40 w-40 rounded-xl bg-muted"></div>
				{/if}
				<div class="relative min-w-0 flex-1">
					<div class="flex items-center gap-2 text-xs font-medium uppercase text-muted-foreground">
						Playlist
						{#if pl.collaborative}
							<span
								class="rounded-full bg-primary/15 px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-primary"
								title={t('library.collab_tooltip')}>{t('library.collab')}</span
							>
						{/if}
					</div>
					<h1 class="mt-1 font-heading text-4xl font-bold tracking-tight drop-shadow-lg">
						{pl.title ?? t('common.playlist_singular')}
					</h1>
					{#if subtitle}<p class="mt-2 text-sm text-muted-foreground">{subtitle}</p>{/if}
					{#if pl.description}
						<!-- Two lines, then More/Less, same as the album page. -->
						<div class="mt-2 max-w-2xl">
							<p class="whitespace-pre-line text-sm text-foreground/80 {expanded ? '' : 'line-clamp-2'}">
								{pl.description}
							</p>
							{#if longDescription}
								<button
									class="mt-1 cursor-pointer text-xs font-semibold uppercase text-muted-foreground hover:text-foreground"
									onclick={() => (expanded = !expanded)}
								>
									{expanded ? t('common.less') : t('common.more')}
								</button>
							{/if}
						</div>
					{/if}
					<div class="mt-4 flex items-center justify-between gap-2">
						<div class="flex items-center gap-2">
							<Button
								class="gap-2"
								onclick={() => playAll(null)}
								disabled={!pl.items.length || preparing || resorting}
							>
								<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" />
								{preparing || resorting ? t('common.sorting') : t('player.play')}
							</Button>
							{#if confirmingDelete}
								<div class="flex items-center gap-2 rounded-lg border border-destructive/40 px-2 py-1">
									<span class="text-xs text-muted-foreground">{t('library.delete_playlist_confirm')}</span>
									<Button variant="destructive" size="sm" onclick={deleteThisPlaylist}>{t('common.delete')}</Button>
									<Button variant="ghost" size="sm" onclick={() => (confirmingDelete = false)}>
										Cancel
									</Button>
								</div>
							{:else}
								<Button
									variant="ghost"
									size="icon"
									aria-label={t('a11y.playlist_options')}
									onclick={openMenu}
								>
									<HugeiconsIcon icon={MoreVerticalIcon} class="h-5 w-5 text-muted-foreground" />
								</Button>
							{/if}
						</div>
						<!-- Pushed to the far end of the header, away from the play controls. -->
						<div class="flex items-center gap-1">
							<Button
								variant="ghost"
								size="sm"
								class="gap-2 {sort === 'default' ? 'text-muted-foreground' : ''}"
								onclick={openSort}
								disabled={!pl.items.length}
							>
								<HugeiconsIcon icon={Sorting01Icon} class="h-4 w-4" />
								{sortLabel}
							</Button>
							<Button
								variant="ghost"
								size="icon"
								class={desc ? '' : 'text-muted-foreground'}
								aria-label={t('sort.direction', { dir: desc ? t('sort.descending') : t('sort.ascending') })}
								onclick={toggleDesc}
								disabled={!pl.items.length}
							>
								<HugeiconsIcon icon={ArrowUpDownIcon} class="h-4 w-4" />
							</Button>
						</div>
					</div>
				</div>
				<div class="absolute right-6 top-6">
					<TrackFilter bind:value={query} placeholder={t('common.search_this_playlist')} />
				</div>
			</div>
			<div
				class="p-4 transition-opacity {resorting ? 'opacity-50' : ''}"
				aria-busy={resorting}
			>
				{#if shown.length}
					<!-- The padding stands in for the rows outside the window, so the scrollbar is the
					     length of the whole playlist even though only ~30 rows exist.
					     data-rows: what the scroller measures row 0's position from. -->
					<div data-rows style="padding-top:{win.padTop}px;padding-bottom:{win.padBottom}px">
						{#each shown.slice(win.start, win.end) as item, i (item.video_id + (win.start + i))}
							{@const n = win.start + i}
							<!-- data-row: what the scroller measures a row's real height from. -->
							<div data-row>
								<TrackRow
									song={item}
									index={n}
									showPlayCount
									active={item.video_id === nowId}
									onplay={() => playAll(n)}
									onAdd={() => openAddToPlaylist(item)}
									onRemove={isLiked || item.set_video_id
										? () => removeTrack(item)
										: undefined}
								/>
							</div>
						{/each}
					</div>
				{:else if filtering}
					<p class="p-4 text-sm text-muted-foreground">
						{t('library.no_tracks_match_loading', {
							query: applied.trim(),
							loading: pl.continuation && !moreError ? t('library.still_loading') : ''
						})}
					</p>
				{:else}
					<p class="p-4 text-sm text-muted-foreground">{t('library.empty_playlist')}</p>
				{/if}
				{#if pl.continuation}
					{#if moreError}
						<div class="p-3 text-center">
							<Button variant="outline" size="sm" onclick={loadMore} disabled={loadingMore}>
								{loadingMore ? t('common.loading') : t('common.try_again')}
							</Button>
						</div>
					{:else}
						<!-- The sentinel sits above the skeletons: it triggers the next page as it scrolls
						     into range, so the rest of a long playlist arrives without a button. -->
						<div aria-busy={loadingMore}>
							<div {@attach sentinel}></div>
							{#if loadingMore}
								{#each Array(4) as _, i (i)}
									<TrackRowSkeleton />
								{/each}
							{/if}
						</div>
					{/if}
				{/if}
			</div>
		</div>
	{/if}
</div>

{#if sortOpen}
	<button
		class="fixed inset-0 z-40 cursor-default"
		onclick={() => (sortOpen = false)}
		aria-label={t('a11y.close_menu')}
	></button>
	<div
		class="fixed z-50 min-w-44 animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style={sortAnchor.style}
		{@attach fitMenu(sortAnchor)}
	>
		<RadioGroup.Root
			value={sort}
			onValueChange={(v) => chooseSort(v as SortKey)}
			class="gap-0"
		>
			{#each SORTS as key (key)}
				<label
					class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				>
					<RadioGroup.Item value={key} />
					{t(`sort.${key}`)}
				</label>
			{/each}
		</RadioGroup.Root>
	</div>
{/if}

{#if menuOpen}
	<button
		class="fixed inset-0 z-40 cursor-default"
		onclick={() => (menuOpen = false)}
		oncontextmenu={(e) => {
			e.preventDefault();
			menuOpen = false;
		}}
		aria-label={t('a11y.close_menu')}
	></button>
	<div
		class="fixed z-50 min-w-52 animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style={anchor.style}
		{@attach fitMenu(anchor)}
	>
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={() => run(shufflePlay)}
			disabled={!pl?.items.length}
		>
			<HugeiconsIcon icon={ShuffleIcon} class="h-4 w-4" /> {t('player.shuffle_play')}
		</button>
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={() => run(() => queue(true))}
			disabled={!pl?.items.length}
		>
			<HugeiconsIcon icon={ArrowUpNarrowWideIcon} class="h-4 w-4" /> {t('player.play_next')}
		</button>
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={() => run(() => queue(false))}
			disabled={!pl?.items.length}
		>
			<HugeiconsIcon icon={ArrowDownWideNarrowIcon} class="h-4 w-4" /> {t('player.add_to_queue')}
		</button>
		<!-- On Repeat is built from local play counts — there is no YouTube playlist to seed a
		     radio from. -->
		{#if !isOnRepeat}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={() => run(() => startRadio('playlist', id, pl?.title))}
			>
				<HugeiconsIcon icon={Radio02Icon} class="h-4 w-4" /> {t('player.start_radio')}
			</button>
		{/if}
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={() => run(() => addPick(asItem()))}
		>
			<HugeiconsIcon icon={DashboardSquare02Icon} class="h-4 w-4" /> {t('player.add_to_shortcuts')}
		</button>
		{#if !isOnRepeat}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={() => run(() => openShare(asItem()))}
			>
				<HugeiconsIcon icon={Share08Icon} class="h-4 w-4" /> {t('player.share')}
			</button>
		{/if}
		{#if savable}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={() => run(saveToLibrary)}
			>
				<!-- altIcon/showAlt, not a ternary: `icon` is read once at mount. -->
				<HugeiconsIcon
					icon={BookmarkAdd02Icon}
					altIcon={BookmarkMinus02Icon}
					showAlt={savedHere}
					class="h-4 w-4"
				/>
				{savedHere ? t('library.remove_from_library') : t('library.save_to_library')}
			</button>
		{:else if inAccount && !isOnRepeat && !isLiked && !editable}
			{#if ownedByUser(asItem())}
				<div class="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm text-muted-foreground">
					<HugeiconsIcon icon={BookmarkCheck02Icon} class="h-4 w-4" /> {t('library.in_library')}
				</div>
			{:else}
				<button
					class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
					onclick={() => run(unsaveFromLibrary)}
				>
					<HugeiconsIcon icon={BookmarkMinus02Icon} class="h-4 w-4" />
					{t('library.remove_from_library')}
				</button>
			{/if}
		{/if}
		{#if editable}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={() => run(() => (editing = true))}
			>
				<HugeiconsIcon icon={PencilEdit02Icon} class="h-4 w-4" /> {t('player.edit_playlist')}
			</button>
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-destructive hover:bg-destructive/10"
				onclick={() => run(() => (confirmingDelete = true))}
			>
				<HugeiconsIcon icon={Delete02Icon} class="h-4 w-4" /> {t('player.delete_playlist')}
			</button>
		{/if}
	</div>
{/if}

{#if editable}
	<EditPlaylistDialog
		bind:open={editing}
		{id}
		title={pl?.title}
		description={pl?.description}
		privacy={pl?.privacy}
		cover={pl?.cover}
		fallback={pl?.thumbnail}
		onchange={applyEdit}
	/>
{/if}
