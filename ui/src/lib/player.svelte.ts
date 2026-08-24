// Shared reactive app state (playback + auth), set up ONCE by the root layout. Components import
// `playback`/`auth` and read them reactively; the Rust side drives them via Tauri events.
// context/11 UI contract — this module only calls commands / subscribes to events.
import { browser } from '$app/environment';
import * as api from './api';
import type {
	Account,
	AccountIdentity,
	BrowseItem,
	NowPlaying,
	QueueState,
	Rating,
	SongItem
} from './api';
import { applyLtState, lt } from './lt.svelte';
import { clearCached } from './pagecache';
import * as pl from './personal';
import type { Personal } from './personal';
import { appearance } from './theme.svelte';
import { t } from './i18n.svelte';

export const playback = $state({
	now: null as NowPlaying | null,
	queue: { items: [], currentIndex: 0 } as QueueState,
	paused: false,
	position: 0,
	/** `performance.now()` when `position` last arrived. Ticks land at ~4 Hz, so `position` on its
	 *  own is a sample up to 250 ms old; anything that has to line up with the audio *right now*
	 *  (the player view's music video) extrapolates from this instead of trusting it directly. */
	positionAt: 0,
	duration: 0,
	volume: 100,
	// Tempo + pitch ("Advanced"). Frontend-owned because nothing persists them: mpv starts at
	// 1.0 / 0 every launch and so does this, so the two can't drift apart.
	speed: 1,
	semitones: 0,
	// Rating of the current track — seeded from its real `likeStatus` on each change, then
	// optimistic on toggle. Owned here rather than in `ratings` below because the mini player is a
	// separate webview with its own module instance: the backend reseed is what keeps them agreeing.
	rating: 'indifferent' as Rating
});

/**
 * The full-window now-playing view (NowPlaying.svelte): big artwork, plus the Queue/Lyrics tabs.
 * It lives here rather than in the layout because starting something playing opens it, and every
 * "play this" path already goes through this module. The open has to happen at the click: a
 * gapless advance looks exactly like a user play from the `now-playing` event alone.
 */
export const np = $state({ open: false, tab: 'queue' as 'queue' | 'lyrics' });

/**
 * Backend settings the app has to know outside the settings modal (which holds the rest in its own
 * local state). Hydrated once in `initApp`; the modal writes here too, so a toggle takes effect
 * without a reload.
 */
export const prefs = $state({ musicVideos: false });

/** No-op when the user has turned the auto-open off (#64): playback starts, the view stays put. */
export const openPlayer = () => {
	if (appearance.openPlayerOnPlay) np.open = true;
};

/** Play one track (a search row, a song card, a shelf), and show it. */
export function playSong(song: SongItem) {
	openPlayer();
	return api.play(song);
}

export const auth = $state({
	account: null as Account | null,
	// Bumped on every sign-in/out. The root layout keys the page on it, so the current route
	// remounts and refetches — home/browse data is per-account and otherwise stays stale until
	// the user navigates away and back.
	epoch: 0
});

// The signed-in user's library (playlists + liked), shared by the sidebar list and the Library page
// so a create reflects in both instantly (context/11 UI contract, optimistic updates).
export const library = $state({
	items: [] as BrowseItem[],
	loaded: false,
	loading: false,
	error: null as string | null,
	// Saved albums and artists. Only the Library page renders them, but they live here rather than in
	// that page's local state so leaving and coming back paints the cached grid instead of a skeleton
	// while three requests go out again.
	albums: [] as BrowseItem[],
	artists: [] as BrowseItem[],
	extrasLoaded: false,
	extrasLoading: false,
	extrasError: null as string | null
});

// Account switches can happen while a library request is still in flight. A generation lets the
// old response finish harmlessly instead of overwriting the newly selected channel's data.
let libraryGeneration = 0;

function resetLibraryForAccount() {
	libraryGeneration++;
	library.items = [];
	savedIn.map = {};
	library.loaded = false;
	library.loading = false;
	library.error = null;
	library.albums = [];
	library.artists = [];
	library.extrasLoaded = false;
	library.extrasLoading = false;
	library.extrasError = null;
}

/** Fetch the library once (or force a refresh). No-op while a load is in flight. */
export async function loadLibrary(force = false) {
	if (library.loading || (library.loaded && !force)) return;
	const generation = libraryGeneration;
	library.loading = true;
	library.error = null;
	try {
		const items = await api.getLibrary();
		if (generation !== libraryGeneration) return;
		library.items = items;
		library.loaded = true;
	} catch (e) {
		if (generation === libraryGeneration) library.error = String(e);
	} finally {
		if (generation === libraryGeneration) library.loading = false;
	}
}

/** Saved albums + artists, same caching rules as `loadLibrary`. */
export async function loadLibraryExtras(force = false) {
	if (library.extrasLoading || (library.extrasLoaded && !force)) return;
	const generation = libraryGeneration;
	library.extrasLoading = true;
	library.extrasError = null;
	try {
		const [albums, artists] = await Promise.all([
			api.getLibraryAlbums(),
			api.getLibraryArtists()
		]);
		if (generation !== libraryGeneration) return;
		library.albums = albums;
		library.artists = artists;
		library.extrasLoaded = true;
	} catch (e) {
		if (generation === libraryGeneration) library.extrasError = String(e);
	} finally {
		if (generation === libraryGeneration) library.extrasLoading = false;
	}
}

/** Create a playlist and optimistically prepend it so every view updates immediately. */
export async function createLibraryPlaylist(title: string): Promise<void> {
	const id = await api.createPlaylist(title);
	// YouTube's library browse is eventually-consistent and won't include a brand-new playlist for a
	// few seconds, so surface it immediately instead of refetching.
	const browseId = id.startsWith('VL') ? id : `VL${id}`;
	library.items = [{ kind: 'playlist', id: browseId, title }, ...library.items];
}

/** Optimistically apply an edit to a library playlist's row (sidebar + Library grid), so a rename
 *  or a new cover shows up everywhere without a refetch. */
export function patchLibraryPlaylist(playlistId: string, patch: Partial<BrowseItem>) {
	library.items = library.items.map((it) => (it.id === playlistId ? { ...it, ...patch } : it));
}

/** Optimistically bump the "N tracks" count in a library playlist's subtitle (sidebar + Library). */
export function bumpLibraryTrackCount(playlistId: string, delta: number) {
	library.items = library.items.map((it) => {
		if (it.id !== playlistId || !it.subtitle) return it;
		const subtitle = it.subtitle.replace(/\d+\s+tracks?/, (m) => {
			const n = Math.max(0, parseInt(m) + delta);
			return `${n} track${n === 1 ? '' : 's'}`;
		});
		return { ...it, subtitle };
	});
}

// --- Saved-in-playlists index (Rust commands::playlist_index) ---------------------------------

/**
 * videoId → the ids of your own playlists holding it, mirrored from SQLite. Track rows read it to
 * draw the "saved" checkmark, so every add and remove patches it in place: waiting for the next
 * crawl would leave the row lying about a playlist the user just put it in.
 */
export const savedIn = $state({ map: {} as Record<string, string[]> });

/**
 * The playlists holding `videoId`, in library order. Resolved against `library.items` rather than
 * carrying titles of its own, so a playlist deleted or unsaved anywhere stops being named here the
 * moment the library list drops it.
 */
export function savedPlaylists(videoId: string): BrowseItem[] {
	const ids = savedIn.map[videoId];
	if (!ids?.length) return [];
	const holding = new Set(ids);
	return library.items.filter((it) => holding.has(it.id));
}

// Module-level, so the whole list shares one answer: a per-row `Object.keys` over a five-figure
// map would be paid a thousand times a paint.
const indexHasAnything = $derived(Object.keys(savedIn.map).length > 0);

/** Whether anything is indexed at all, which is what decides if a row reserves the mark's slot. */
export const anySaved = () => indexHasAnything;

/**
 * Load the index: the stored one first so rows mark up immediately, then whatever a refresh crawl
 * turns up. Generation-guarded like the library itself, so an account switch mid-flight can't
 * paint the previous channel's playlists.
 */
export async function loadSavedIndex() {
	const generation = libraryGeneration;
	const apply = (map: Record<string, string[]>) => {
		if (generation === libraryGeneration) savedIn.map = map;
	};
	await api
		.playlistIndex()
		.then(apply)
		.catch(() => {});
	await api
		.syncPlaylistIndex()
		.then(apply)
		.catch(() => {});
}

/** Every one of `videoIds` is now in `playlistId`, refused duplicates included: YouTube saying the
 *  playlist already holds a track is the same fact the mark shows. */
export function noteSavedIn(playlistId: string, videoIds: string[]) {
	for (const videoId of videoIds) {
		const ids = savedIn.map[videoId] ?? [];
		if (!ids.includes(playlistId)) savedIn.map[videoId] = [...ids, playlistId];
	}
}

export function noteUnsavedFrom(playlistId: string, videoId: string) {
	const ids = savedIn.map[videoId];
	if (ids?.includes(playlistId)) savedIn.map[videoId] = ids.filter((id) => id !== playlistId);
}

// --- Local music (Rust local.rs) --------------------------------------------------------------
// Shared like `library` is: the Library page renders it, and the app rescans at startup so tiles
// pointing at deleted files disappear before anyone clicks one.

export const local = $state({
	folders: [] as string[],
	albums: [] as BrowseItem[],
	artists: [] as BrowseItem[],
	songs: [] as SongItem[],
	loading: false,
	scanned: false,
	error: null as string | null
});

/**
 * Music that is no longer on disk, from a scan or from a play attempt that found nothing there.
 * Everything holding those ids drops them in the same tick: the Local tab's lists, the Shortcuts
 * grid, sidebar pins, recents. Nothing waits for a refetch, and nothing is left to fail later.
 */
export function forgetLocal(removed: string[]) {
	if (!removed.length) return;
	const gone = new Set(removed);
	local.songs = local.songs.filter((s) => !gone.has(s.video_id));
	local.albums = local.albums.filter((a) => !gone.has(a.id));
	local.artists = local.artists.filter((a) => !gone.has(a.id));
	const dropped = pl.forgetIds(personal, removed);
	savePersonal();
	if (dropped)
		toast(
			dropped === 1
				? t('toasts.shortcuts_dropped_one')
				: t('toasts.shortcuts_dropped', { count: dropped })
		);
}

/** Take a scan result: replace the library, then prune whatever it reports as gone. */
function applyLocal(lib: api.LocalLibrary) {
	local.folders = lib.folders;
	local.albums = lib.albums;
	local.artists = lib.artists;
	local.songs = lib.songs;
	local.scanned = true;
	local.error = null;
	forgetLocal(lib.removed);
}

async function runLocal(call: () => Promise<api.LocalLibrary>) {
	local.loading = true;
	try {
		applyLocal(await call());
	} catch (e) {
		local.error = String(e);
	} finally {
		local.loading = false;
	}
}

/** No-op while a scan is already running: the startup scan and opening the Local tab overlap. */
export const scanLocal = () =>
	local.loading ? Promise.resolve() : runLocal(api.getLocalLibrary);
export const addLocalFolder = (path: string) => runLocal(() => api.addLocalFolder(path));
export const removeLocalFolder = (path: string) => runLocal(() => api.removeLocalFolder(path));

// --- Personalization: the Shortcuts grid, sidebar pins, play recency (see personal.ts) ----------
// The Shortcuts grid holds what the user puts in it, plus the one tile the app suggests (On
// Repeat, via `seedOnRepeatPick`). See `personal.ts`.
// localStorage rather than SQLite: only the webview ever reads this, so a table + commands + a
// `UI_SETTINGS` allowlist entry would buy nothing. Loaded at module scope (guarded like the layout's
// `initTheme`) so the sidebar and home grid render sorted on the very first paint.
// ponytail: move to db.rs if it ever needs to be account-scoped or readable outside the webview.
const PERSONAL_KEY = 'limusic:personal';

export const personal = $state<Personal>(pl.empty());

if (browser) {
	try {
		Object.assign(personal, pl.hydrate(JSON.parse(localStorage.getItem(PERSONAL_KEY) ?? 'null')));
	} catch {
		// Unreadable blob — start clean rather than break startup.
	}
}

function savePersonal() {
	if (!browser) return;
	try {
		localStorage.setItem(PERSONAL_KEY, JSON.stringify(personal));
	} catch {
		// Quota or a locked store: personalization is best-effort, never fatal.
	}
}

/** Add to Shortcuts (evicting the tile gone longest unplayed when the grid is full). */
export function addPick(item: BrowseItem) {
	const added = pl.addPick(personal, item);
	savePersonal();
	toast.success(added ? 'Added to shortcuts' : 'Already in shortcuts');
}

/** Drop landed: move (or add) a tile so it sits before `beforeId` — null appends. No toast: the
 *  grid rearranging under the cursor is its own feedback. */
export function placePick(item: BrowseItem, beforeId: string | null) {
	pl.placePick(personal, item, beforeId);
	savePersonal();
}

export function removePick(id: string) {
	pl.removePick(personal, id);
	savePersonal();
}

/** How many songs On Repeat needs before it's worth a tile on the grid. */
const ON_REPEAT_SEED_MIN = 5;

/**
 * Put On Repeat on the Shortcuts grid once it has enough songs to be useful: the one tile the app
 * adds by itself. Called on every home visit; `seedPick` owns the "should this go on" decision, so
 * removing the tile is permanent no matter how many times this runs. Cheap to repeat: On Repeat is
 * built from local SQLite, so the fetch never touches the network.
 */
export async function seedOnRepeatPick() {
	try {
		const onRepeat = await api.getPlaylist(api.ON_REPEAT_ID);
		if (onRepeat.items.length < ON_REPEAT_SEED_MIN) return;
		const added = pl.seedPick(personal, {
			kind: 'playlist',
			id: api.ON_REPEAT_ID,
			title: onRepeat.title ?? 'On Repeat',
			// Not a track count: the tile is stored as-is, so a number here would go stale the next
			// time the playlist re-ranks itself.
			subtitle: 'Your most played'
		});
		if (added) savePersonal();
	} catch {
		// No tile this time; the next home visit tries again.
	}
}

/**
 * Home's arrangement, as set in the Edit modal. `order` is every section key the modal listed, in
 * display order, hidden ones included — a hidden section that keeps its slot comes back where it was.
 */
export function saveHomeLayout(order: string[], hidden: string[]) {
	// Spread, not a fresh object: `seen` is written by the feed, not by the modal, and rebuilding
	// `home` from the two lists the modal owns used to drop it.
	personal.home = { ...personal.home, order, hidden };
	savePersonal();
}

/** Remember the shelves this page of the feed carried, for Edit home. See `pl.noteSections`. */
export function noteHomeSections(titles: string[]) {
	if (pl.noteSections(personal, titles)) savePersonal();
}

/** Called from every card click app-wide, so only persist when the id was actually on the grid. */
export function touchPick(id: string) {
	if (pl.touchPick(personal, id)) savePersonal();
}

/**
 * Save a playlist/album/artist to the library from this machine, or take it back out. Returns the
 * new state. Not account-scoped and never cleared on sign-in: what a signed-out user saved is still
 * theirs afterwards, sitting next to whatever YouTube says their library holds.
 */
export function toggleSaved(item: BrowseItem): boolean {
	const saved = pl.toggleSaved(personal, item);
	savePersonal();
	return saved;
}

export const isSaved = (id: string): boolean => pl.isSaved(personal, id);

/** Saved here and pushed to the account: while signed in, unsaving it belongs on the item's page,
 *  where the button knows which write to send. Signed out, this row is the only library there is. */
export const isSynced = (id: string): boolean => pl.isSynced(personal, id);

/**
 * Push everything saved on this machine into the signed-in account. The local rows stay put and are
 * only flagged `synced`: they are the whole library again the moment the user signs out, and
 * `mergeSaved` dedupes the two copies into one card while signed in. Sequential, like every other
 * bulk write here (a library is a handful of requests, don't hammer). Anything that fails keeps its
 * flag off, so pressing the button again retries exactly what's left.
 */
export async function syncSavedToYouTube(): Promise<{ synced: number; failed: number }> {
	// Fresh: "is this already in the account" is the whole duplicate check.
	await loadLibrary(true);
	const known = new Set(library.items.map((i) => i.id));
	const done: string[] = [];
	let failed = 0;
	for (const item of pl.unsynced(personal)) {
		try {
			if (item.kind === 'album') {
				// YouTube's own answer, so it can't be liked twice. The album's audio playlist is the
				// like target and only its page carries it, which is what this fetch is for.
				const album = await api.getAlbum(item.id);
				if (!album.inLibrary) {
					if (!album.playlistId) throw new Error('no album playlist');
					await api.setAlbumSaved(album.playlistId, true);
				}
			} else if (item.kind === 'artist') {
				// Subscribing twice is the same subscription. No pre-check: the library's artist grid
				// is built from the songs in your library, not from subscriptions, so it can't answer.
				await api.subscribe(item.id, true);
			} else if (item.kind === 'playlist') {
				// A playlist is liked by its browseId (Rust strips the `VL`), and it lands in the same
				// grid `known` was built from, so a hit there means it is already saved.
				if (!known.has(item.id)) await api.setAlbumSaved(item.id, true);
			} else {
				continue; // ponytail: songs can't be saved today, so there is nothing to push
			}
			done.push(item.id);
		} catch {
			failed++;
		}
	}
	if (done.length) {
		pl.markSynced(personal, done);
		savePersonal();
		// No refetch: YouTube's library browse is eventually consistent and won't list a just-liked
		// album for a few seconds (same reason `createLibraryPlaylist` prepends). The local rows are
		// still on screen through `mergeSaved`, so there is nothing to bridge.
	}
	return { synced: done.length, failed };
}

export function togglePin(id: string) {
	const result = pl.togglePin(personal, id);
	if (result === 'full') toast.error(t('toasts.pins_full', { max: pl.MAX_PINS }));
	else savePersonal();
	return result;
}

// Rating state that outlives one row. A song's `rating` is a snapshot from whenever its page was
// fetched, and the same song shows up in several places at once (a list row, its ⋯ menu, the player
// bar). One override map keyed by videoId keeps them all telling the same story; the current track
// stays owned by `playback.rating`, which the Rust side reseeds on every track change.
const ratings = $state<Record<string, Rating>>({});

export function ratingOf(song: SongItem): Rating {
	if (playback.now?.videoId === song.video_id) return playback.rating;
	return ratings[song.video_id] ?? song.rating ?? 'indifferent';
}

export const isLiked = (song: SongItem): boolean => ratingOf(song) === 'like';

/** Like/unlike whatever is playing. Thin wrapper so the player bar and the mini player share one
 *  implementation (and one optimistic path) with every list row. */
export function toggleNowPlayingLike(): Promise<void> {
	const n = playback.now;
	if (!n) return Promise.resolve();
	return toggleRating({ video_id: n.videoId, title: n.title, artists: n.artists }, 'like');
}

// --- Volume ------------------------------------------------------------------------------------
// Shared by the player bar and the mini player, which means there is one behaviour to get right
// instead of two to keep in step.

// Live while dragging (the user hears it), coalesced to one update per frame so a drag doesn't
// flood IPC. One *frame*, not a 100ms throttle, because mpv has no volume ramp: `ao_apply_gain`
// (audio/out/ao.c) multiplies the next output buffer by the new gain and that's it, so every
// update is a step discontinuity in the waveform and the bigger the step the louder the click.
// At 100ms a drag landed as a handful of ~12dB jumps, which popped audibly; a frame keeps each
// step small enough to be masked by the music.
// ponytail: smaller steps, not a real ramp. If a fast drag still pops, slew toward the target in
// Rust (~30ms of small steps, cancelled by the next set_volume) rather than shrinking this again.
let volFrame: number | null = null;

export function dragVolume(v: number) {
	playback.volume = v;
	if (volFrame !== null) return;
	volFrame = requestAnimationFrame(() => {
		volFrame = null;
		api.setVolume(playback.volume);
	});
}

/** Pointer released: always send the final value, pending frame or not. */
export function commitVolume(v: number) {
	if (volFrame !== null) {
		cancelAnimationFrame(volFrame);
		volFrame = null;
	}
	playback.volume = v;
	api.setVolume(v);
	// Persisted here rather than in Rust's `set_volume`: a drag calls that once per frame and every
	// settings write is an fsync. A commit is one per gesture, and it's the level to reopen at.
	api.setSetting('volume', String(v)).catch(() => {});
}

/**
 * One keyboard step of volume (Ctrl+> / Ctrl+<). Live like a drag, so a held key gets one IPC per
 * frame instead of one per repeat, and persisted only once the presses stop: a settings write is an
 * fsync, and a run of taps is one gesture the same way a drag is.
 */
let volSettle: ReturnType<typeof setTimeout> | undefined;

export function nudgeVolume(delta: number) {
	dragVolume(Math.min(100, Math.max(0, playback.volume + delta)));
	clearTimeout(volSettle);
	volSettle = setTimeout(() => commitVolume(playback.volume), 400);
}

/**
 * Mouse wheel over the volume slider. Same gesture as a run of key presses, so it reuses the nudge
 * path (live per frame, persisted once the scrolling stops). preventDefault keeps the page from
 * scrolling underneath: Svelte only forces passive listeners on touch events, not wheel.
 */
export function wheelVolume(e: WheelEvent) {
	e.preventDefault();
	nudgeVolume(e.deltaY < 0 ? 5 : -5);
}

/**
 * Tempo + pitch (the "Advanced" dialog). Applied live, reverted if mpv rejects it: the pitch
 * filter needs a libmpv built with librubberband, and Rust applies pitch first so a rejection
 * leaves neither of them set.
 */
export function setTempoPitch(speed: number, semitones: number) {
	const previous = { speed: playback.speed, semitones: playback.semitones };
	playback.speed = speed;
	playback.semitones = semitones;
	api.setPlaybackParams(speed, semitones).catch((e) => {
		Object.assign(playback, previous);
		toast.error(String(e));
	});
}

// Mute *is* volume 0 — no separate flag, so dragging the slider off zero un-mutes for free and the
// icon can't disagree with what you hear. Remembers the level to come back to; falls back to 100
// when the user dragged to zero themselves (nothing was remembered).
let preMute = 100;

export function toggleMute() {
	const muted = playback.volume === 0;
	if (!muted) preMute = playback.volume;
	commitVolume(muted ? preMute || 100 : 0);
}

/** Hand over to the floating widget (Rust `mini.rs`); the app hides to the tray behind it. */
export function openMiniPlayer() {
	api.openMini().catch((e) => toast.error(String(e)));
}

/** Advance the repeat mode: off → all → one → off. */
export function cycleRepeat(): Promise<void> {
	const r = playback.queue.repeat ?? 'off';
	return api.setRepeat(r === 'off' ? 'all' : r === 'all' ? 'one' : 'off');
}

const RATED: Record<Rating, string> = {
	like: 'Added to liked songs',
	dislike: 'Disliked',
	indifferent: 'Rating removed'
};

/** Optimistic rating change, reverted if YouTube rejects it. */
async function rate(song: SongItem, next: Rating) {
	const prev = ratingOf(song);
	if (prev === next) return;
	const isNow = playback.now?.videoId === song.video_id;
	ratings[song.video_id] = next;
	if (isNow) playback.rating = next;
	try {
		await api.rate(song.video_id, next);
		toast.success(RATED[next]);
	} catch (e) {
		ratings[song.video_id] = prev;
		if (isNow) playback.rating = prev;
		toast.error(String(e));
	}
}

/** Click the rating you already have to clear it, the way YouTube Music's own buttons work.
 *  One call either way: YouTube's states are exclusive, so a dislike un-likes on its own. */
export function toggleRating(song: SongItem, want: 'like' | 'dislike') {
	return rate(song, ratingOf(song) === want ? 'indifferent' : want);
}

/**
 * Play a playlist/album/artist and record that it was played, which is what sorts the sidebar and
 * seeds Shortcuts. Every "play these tracks from somewhere" call site goes through this.
 * `sourceId` (playlist/album pages only) points autoplay at that context's radio.
 * `continuation` (the playlist page's next-page token) hands the rest of a long playlist to the
 * backend to walk in the background, so playback starts on the tracks already loaded.
 */
export function playFrom(
	source: BrowseItem,
	items: SongItem[],
	start: number | null,
	sourceId?: string,
	shuffle?: boolean,
	continuation?: string
) {
	pl.noteRecent(personal, source);
	pl.touchPick(personal, source.id);
	savePersonal();
	openPlayer();
	return api.playPlaylist(items, start, sourceId, source.title, shuffle, continuation);
}

/**
 * "Play next" / "Add to queue" from any surface (song menus, card menus, page headers). One
 * implementation so the wording is the same everywhere. Guests get their toast from the session
 * flow instead ("Added to the session queue."), so this one stays quiet for them.
 */
export async function enqueue(
	items: SongItem[],
	next: boolean,
	from?: string,
	continuation?: string
) {
	if (!items.length) return;
	try {
		// A "Play next" block is capped at the tracks the page has loaded: shoving 5000 in front of
		// what's playing isn't what anyone means by "next". "Add to queue" walks the rest.
		await (next ? api.playNext(items, from) : api.addToQueue(items, from, continuation));
	} catch (e) {
		toast.error(String(e));
		return;
	}
	if (lt.role === 'guest') return;
	const n = items.length;
	if (next) toast.success(n === 1 ? 'Playing next' : `${n} songs play next`);
	else toast.success(n === 1 ? 'Added to queue' : `Added ${n} songs to the queue`);
}

/**
 * Start a radio from any surface (song menus, card menus, page headers). One implementation so the
 * feedback is the same everywhere: radio is a network round trip before anything audibly happens,
 * so it says so up front rather than looking like the click was swallowed.
 */
export async function startRadio(
	kind: 'song' | 'artist' | 'album' | 'playlist',
	id: string,
	name?: string
) {
	toast(t('toasts.starting_radio'));
	openPlayer();
	try {
		await api.startRadio(kind, id, name);
	} catch (e) {
		toast.error(String(e));
	}
}

// Transient UI state for write actions.
export const ui = $state({
	addSongs: null as SongItem[] | null, // add-to-playlist picker target(s), full items for optimistic appends
	share: null as BrowseItem | null, // the share modal's target
	toast: null as Toast | null,
	settingsOpen: false, // the settings modal
	ltOpen: false, // the Listen Together modal
	linkOpen: false, // the "open a pasted link" modal
	paletteOpen: false, // the Ctrl+K search palette
	shortcutsOpen: false, // the Ctrl+H keyboard-shortcuts list
	channelPickerOpen: false,
	channelPickerRequired: false, // true while a multi-channel login is not finalized yet
	channelIdentities: [] as AccountIdentity[],
	// Manual sidebar collapse, lg and up (below that the rail is already collapsed by the
	// breakpoint). Here rather than in Sidebar because the now-playing view and the fullscreen
	// lyrics panel are overlays that offset themselves by the sidebar's width.
	sidebarCollapsed: browser && localStorage.getItem('sidebar_collapsed') === '1'
});

export function openChannelPicker(required = false) {
	ui.channelPickerRequired = required;
	ui.channelIdentities = [];
	ui.channelPickerOpen = true;
}

export function toggleSidebar() {
	ui.sidebarCollapsed = !ui.sidebarCollapsed;
	localStorage.setItem('sidebar_collapsed', ui.sidebarCollapsed ? '1' : '0');
}

export type Toast = { msg: string; kind: 'info' | 'success' | 'error' };

// A counter, not the toast itself: $state proxies the stored object, so `ui.toast === t` is never
// true and the toast would never clear. It also means a repeated message can't cut its own retry short.
let seq = 0;

function show(msg: string, kind: Toast['kind']) {
	const id = ++seq;
	ui.toast = { msg, kind };
	setTimeout(() => {
		if (seq === id) ui.toast = null;
	}, 2500);
}

/** Sonner-shaped. Bare `toast(msg)` is a neutral notice; .success/.error pick the icon. */
export const toast = Object.assign((msg: string) => show(msg, 'info'), {
	info: (msg: string) => show(msg, 'info'),
	success: (msg: string) => show(msg, 'success'),
	error: (msg: string) => show(msg, 'error')
});

export function openShare(item: BrowseItem) {
	ui.share = item;
}

export function openAddToPlaylist(song: SongItem) {
	ui.addSongs = [song];
}

/** Open the picker to add several tracks at once (e.g. a whole album). */
export function openAddManyToPlaylist(songs: SongItem[]) {
	ui.addSongs = songs.length ? songs : null;
}

// Last successful add-to-playlist — the open playlist page appends these optimistically.
export const lastPlaylistAdd = $state({ playlistId: '', songs: [] as SongItem[], epoch: 0 });

export function notePlaylistAdd(playlistId: string, songs: SongItem[]) {
	lastPlaylistAdd.playlistId = playlistId;
	// Strip per-context fields: set_video_id belongs to the source playlist, the queue markers to
	// the queue — none apply to the row's new home.
	lastPlaylistAdd.songs = songs.map((s) => ({
		...s,
		set_video_id: undefined,
		added_by: undefined,
		added_by_avatar: undefined,
		autoplay: undefined,
		queued: undefined,
		queued_end: undefined,
		queued_from: undefined,
		queued_by: undefined
	}));
	lastPlaylistAdd.epoch++;
}

let started = false;

/**
 * Wire the Tauri event listeners once and seed initial state. Returns a teardown fn.
 *
 * `mini` is the floating-widget window (mini.rs): it runs this same module, and the events are
 * emitted app-wide so it gets playback for free — but it has no library, no local tab, no account
 * menu and no Listen Together UI, so it skips those fetches rather than duplicating the app's.
 */
export function initApp(mini = false): () => void {
	if (started) return () => {};
	started = true;
	const subs = [
		api.onNowPlaying((n) => {
			playback.now = n;
			playback.rating = n.rating ?? 'indifferent'; // the track's real rating when known
			// Feeds Shortcuts recency and the community shelf's artist seed. Every play lands here,
			// gapless advances included, so it's the one hook that sees them all.
			pl.touchPick(personal, n.videoId);
			if (n.artists) pl.noteArtist(personal, n.artistId ?? n.artists, pl.firstArtist(n.artists));
			savePersonal();
		}),
		// YouTube's own answer for a track whose row never stated one (issue #93). Into the
		// override map as well as the player bar: the same song is on screen as a list row too,
		// and `ratingOf` reads that map for every row that is not the playing one.
		api.onRating((videoId, rating) => {
			ratings[videoId] = rating;
			if (playback.now?.videoId === videoId) playback.rating = rating;
		}),
		api.onQueueChanged((q) => (playback.queue = q)),
		// The items did not change, so keep the array we already hold and patch the rest. Splice
		// the playing row back in: `start_current` backfills its duration and artists after the
		// stream resolves, and that repair rides on this event rather than a whole new queue.
		api.onQueueIndex((q) => {
			const items = playback.queue.items;
			if (q.current && items[q.currentIndex]) items[q.currentIndex] = q.current;
			playback.queue = {
				...playback.queue,
				items,
				currentIndex: q.currentIndex,
				playedFrom: q.playedFrom,
				shuffle: q.shuffle,
				repeat: q.repeat,
				sourceName: q.sourceName
			};
		}),
		api.onPosition((p) => {
			playback.position = p;
			playback.positionAt = performance.now();
		}),
		api.onDuration((d) => (playback.duration = d)),
		api.onPlaybackState((s) => (playback.paused = s === 'paused')),
		api.onVolume((v) => {
			// Not while our own drag is in flight: the echo is a value the pointer has already
			// moved past, and applying it would yank the thumb backwards mid-drag.
			if (volFrame === null) playback.volume = v;
		}),
		api.onPlaybackError((msg) => toast.error(msg)),
		api.onPlaybackNotice((msg) => toast(msg)), // auto-skipped an unplayable track
		api.onCoverError((msg) => toast.error(msg)), // playlist artwork YouTube wouldn't take
		api.onLocalChanged(forgetLocal), // a local file turned out to be gone — drop it everywhere
		api.onAuthChanged((a) => {
			auth.account = a;
			resetLibraryForAccount();
			// Signing out doesn't empty the library: On Repeat and anything saved on this machine
			// are still there, and the backend answers both without touching YouTube.
			if (!mini) {
				loadLibrary(true);
				loadSavedIndex();
			}
			if (!a.signedIn) {
				ui.channelPickerOpen = false;
				ui.channelPickerRequired = false;
				ui.channelIdentities = [];
			}
			clearCached();
			auth.epoch++;
		}),
		api.onAccountSelectionRequired(() => openChannelPicker(true)),
		api.onLoginError((msg) => toast.error(msg)),
		api.onLoginDone(() => toast.success(t('toasts.signed_in'))),
		// Listen Together (context/19): mirror the Rust session state; surface notices as toasts.
		api.onLtState((s) => {
			// A room is a shared clock, so tempo is off while one is on (the stepper hides itself).
			// Dropping back to 1x here too, or a speed set before joining strands you off the beat
			// with no visible control to undo it.
			if (s.role !== 'none' && playback.speed !== 1) setTempoPitch(1, playback.semitones);
			applyLtState(s);
		}),
		api.onLtNotice((msg) => toast(msg))
	];
	const teardown = () => subs.forEach((u) => u.then((f) => f()));
	api.getQueue()
		.then((q) => (playback.queue = q))
		.catch(() => {});
	// The events above are fire-and-forget, and this window missed every one that already fired:
	// on a cold start the backend restores the queue before the UI subscribes, and the mini player
	// is created mid-song. Ask for the current state once rather than guessing at it.
	api.getPlayback()
		.then((s) => {
			playback.volume = s.volume; // before the guard below: the slider is stale either way
			if (playback.now) return; // a real now-playing event beat us to it
			playback.now = s.now;
			playback.rating = s.now?.rating ?? 'indifferent';
			playback.paused = s.paused;
			playback.position = s.position;
			playback.positionAt = performance.now();
			playback.duration = s.duration;
		})
		.catch(() => {});
	if (mini) return teardown;
	api.getSettings()
		.then((s) => (prefs.musicVideos = s.music_videos === 'true'))
		.catch(() => {});
	api.getAccount()
		.then((a) => {
			auth.account = a;
			if (a.signedIn && a.selectionRequired) {
				openChannelPicker(true);
				return;
			}
			loadLibrary();
			if (a.signedIn) {
				// The crawl behind this is the app's only bulk request, so it runs once here (and
				// on a sign-in), never on navigation. It settles into the background while the
				// first page paints from the stored index.
				loadSavedIndex();
			}
		})
		.catch(() => {});
	// Scan the local folders once at startup: it seeds the Library's Local tab and, more to the
	// point, prunes shortcuts for music that was deleted while the app was closed.
	scanLocal();
	// Seed the Listen Together state (server URL, any active room after a UI reload).
	api.ltGetState().then(applyLtState).catch(() => {});
	return teardown;
}
