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
import { clearCached, invalidateCached, LIBRARY_SONGS_KEY } from './pagecache';
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

/** videoId → the in-flight or settled loopback URL for its music video (null when it has none).
 *
 *  It lives here, not in `NowPlaying`, because the layout unmounts that component whenever the
 *  player view is closed. Without this, reopening the view re-runs `video_stream`, which is up to
 *  two `/player` round trips to YouTube that the user sits through every single time. Storing the
 *  promise rather than the value also means the prefetch below and the view's own request are the
 *  same request when they overlap.
 *
 *  ponytail: bounded by insertion order, not use. The Rust side keeps eight too, so anything
 *  evicted here is still a cheap answer there. */
const videoUrls = new Map<string, Promise<string | null>>();

/** The picture ladder YouTube publishes, snapped up so the box is never upscaled, capped at 720
 *  because that is already more than the player view's box gets on a 1080p screen. `176` is `11rem`
 *  at the default root font size, the chrome above and below the box (see the `--vid` calc in
 *  NowPlaying.svelte). If the titlebar or the player bar changes height, this changes with it. */
export function wantedVideoHeight() {
	const px = (window.innerHeight - 176) * 0.85;
	return [360, 480, 720].find((h) => h >= px) ?? 720;
}

/** The loopback URL for a track's music video, resolving it at most once. */
export function videoUrlFor(videoId: string): Promise<string | null> {
	let p = videoUrls.get(videoId);
	if (!p) {
		// Silent on failure: a null answer is the ordinary case (the track has no video stream),
		// and the artwork staying put is already the right thing to show.
		p = api.videoStream(videoId, wantedVideoHeight()).catch(() => null);
		if (videoUrls.size >= 8) videoUrls.delete(videoUrls.keys().next().value!);
		videoUrls.set(videoId, p);
	}
	return p;
}

/** Forget a URL the element could not load, here and in Rust, so the next open resolves a fresh
 *  one instead of failing the same way. */
export function forgetVideoUrl(videoId: string) {
	videoUrls.delete(videoId);
	api.forgetVideoStream(videoId).catch(() => {});
}

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
	extrasError: null as string | null,
	// Uploads ▸ Albums. Loaded only when that tab is opened, since most accounts have none.
	uploadAlbums: [] as BrowseItem[],
	uploadAlbumsLoaded: false,
	uploadAlbumsLoading: false,
	uploadAlbumsError: null as string | null
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
	library.uploadAlbums = [];
	library.uploadAlbumsLoaded = false;
	library.uploadAlbumsLoading = false;
	library.uploadAlbumsError = null;
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

/** The user's own uploaded albums, same caching rules as `loadLibrary`. */
export async function loadUploadAlbums(force = false) {
	if (library.uploadAlbumsLoading || (library.uploadAlbumsLoaded && !force)) return;
	const generation = libraryGeneration;
	library.uploadAlbumsLoading = true;
	library.uploadAlbumsError = null;
	try {
		const albums = await api.getUploadAlbums();
		if (generation !== libraryGeneration) return;
		library.uploadAlbums = albums;
		library.uploadAlbumsLoaded = true;
	} catch (e) {
		if (generation === libraryGeneration) library.uploadAlbumsError = String(e);
	} finally {
		if (generation === libraryGeneration) library.uploadAlbumsLoading = false;
	}
}

/** Create a playlist and optimistically prepend it so every view updates immediately. */
export async function createLibraryPlaylist(title: string): Promise<void> {
	const id = await api.createPlaylist(title);
	// YouTube's library browse is eventually-consistent and won't include a brand-new playlist for a
	// few seconds, so surface it immediately instead of refetching.
	const browseId = id.startsWith('VL') ? id : `VL${id}`;
	// The owner line in YouTube's own format ("<owner> • N tracks"), because that is what
	// `ownedByUser` reads to know this one is yours before the library reloads and says so itself.
	const subtitle = auth.account?.name ? `${auth.account.name} \u2022 0 tracks` : undefined;
	library.items = [{ kind: 'playlist', id: browseId, title, subtitle }, ...library.items];
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
	// Liked Music is indexed (it is what fills the heart on a search row) but never named here:
	// the thumbs-up next to the chip already says it.
	const holding = new Set(ids.filter((id) => id !== api.LIKED_MUSIC_ID));
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
 * Put one saved card on the account. `known` is the library's playlist grid when the caller already
 * has it. Answers what happened: 'saved', 'already' when YouTube's own copy made it a no-op, or
 * 'unsupported' for a kind YouTube has nothing to save (a song, which lives in Liked Music).
 */
async function pushSaved(
	item: BrowseItem,
	known?: Set<string>
): Promise<'saved' | 'already' | 'unsupported'> {
	if (item.kind === 'album') {
		// YouTube's own answer, so it can't be liked twice. The album's audio playlist is the like
		// target and only its page carries it, which is what this fetch is for.
		const album = await api.getAlbum(item.id);
		if (album.inLibrary) return 'already';
		if (!album.playlistId) throw new Error('no album playlist');
		await api.setAlbumSaved(album.playlistId, true);
	} else if (item.kind === 'artist') {
		// Subscribing twice is the same subscription. No pre-check: the library's artist grid is
		// built from the songs in your library, not from subscriptions, so it can't answer.
		await api.subscribe(item.id, true);
	} else if (item.kind === 'playlist') {
		// A playlist is liked by its browseId (Rust strips the `VL`), and it lands in the same grid
		// `known` was built from, so a hit there means it is already saved.
		if (known?.has(item.id)) return 'already';
		await api.setAlbumSaved(item.id, true);
	} else {
		return 'unsupported';
	}
	return 'saved';
}

/**
 * Is this card in the library, wherever the library happens to live: saved on this machine, or on
 * the signed-in account. Albums and artists only answer once `loadLibraryExtras` has run, which is
 * why the menus kick it off when they open.
 *
 * Songs are not here: their library is Liked Music (Library ▸ Songs browses `FEmusic_liked_videos`),
 * so `isLiked` is the answer and TrackMenu uses it.
 */
export function inLibrary(item: BrowseItem): boolean {
	if (pl.isSaved(personal, item.id)) return true;
	if (!auth.account?.signedIn) return false;
	const list =
		item.kind === 'album' ? library.albums : item.kind === 'artist' ? library.artists : library.items;
	return list.some((i) => i.id === item.id);
}

/**
 * The menu's "Save to library": the local row always (it is the whole library signed out, and it is
 * what makes the indicator flip everywhere at once), plus the matching write on the account when
 * signed in, flagged `synced` so the Library's sync button has nothing left to push for it.
 */
export async function addToLibrary(item: BrowseItem): Promise<'saved' | 'already'> {
	// Whatever the menu was showing, say so instead of writing again: the card can have landed in
	// the account library from another device since this view was fetched.
	if (inLibrary(item)) return 'already';
	const known = item.kind === 'playlist' ? new Set(library.items.map((i) => i.id)) : undefined;
	pl.toggleSaved(personal, item);
	savePersonal(); // before the write: a rejected push must not lose the local row too
	if (!auth.account?.signedIn) return 'saved';
	const result = await pushSaved(item, known);
	if (result !== 'unsupported') {
		pl.markSynced(personal, [item.id]);
		savePersonal();
	}
	return result === 'already' ? 'already' : 'saved';
}

/**
 * Yours by definition, so there is no "remove from library" to offer: a playlist you made (YTM's
 * library subtitle is "<owner> • N tracks", and on your own it names you), your uploads, Liked
 * Music and the locally-built On Repeat. Everything else in the library got there by being saved,
 * and can be unsaved.
 *
 * ponytail: the owner name, because the grid browse carries no ownership flag (only a playlist's
 * own page does, via `musicEditablePlaylistDetailHeaderRenderer`). If this ever reads wrong, the
 * removal it wrongly offers is a no-op and `loadLibrary(true)` puts the row straight back.
 */
export function ownedByUser(item: BrowseItem): boolean {
	if (item.id === api.LIKED_MUSIC_ID || item.id === api.ON_REPEAT_ID || api.isLocalId(item.id))
		return true;
	if (item.isUpload) return true;
	if (item.kind !== 'playlist') return false;
	const me = auth.account?.name?.trim();
	if (!me) return false;
	// The library's own row wins: a card from a home shelf carries a different subtitle.
	const row = library.items.find((i) => i.id === item.id) ?? item;
	return (row.subtitle ?? '').split('\u2022')[0].trim() === me;
}

/**
 * Take a card back out of the library: the local row, and the account's own copy when signed in
 * (an album's like, an artist's subscription, a saved playlist's like). Optimistic, like every
 * other library write here, and reverted if YouTube says no. Not for the kinds `ownedByUser`
 * covers, which have nothing to undo.
 */
export async function removeFromLibrary(item: BrowseItem): Promise<void> {
	const wasSaved = pl.isSaved(personal, item.id);
	if (wasSaved) {
		pl.toggleSaved(personal, item);
		savePersonal();
	}
	if (!auth.account?.signedIn) return;
	const before = { items: library.items, albums: library.albums, artists: library.artists };
	library.items = library.items.filter((i) => i.id !== item.id);
	library.albums = library.albums.filter((i) => i.id !== item.id);
	library.artists = library.artists.filter((i) => i.id !== item.id);
	try {
		if (item.kind === 'album') {
			// The like sits on the album's audio playlist, which only its page carries.
			const album = await api.getAlbum(item.id);
			if (album.inLibrary && album.playlistId) await api.setAlbumSaved(album.playlistId, false);
		} else if (item.kind === 'artist') {
			await api.subscribe(item.id, false);
		} else if (item.kind === 'playlist') {
			await api.setAlbumSaved(item.id, false);
		}
	} catch (e) {
		library.items = before.items;
		library.albums = before.albums;
		library.artists = before.artists;
		if (wasSaved) {
			pl.toggleSaved(personal, item);
			savePersonal();
		}
		throw e;
	}
}

/**
 * Mirror an account-side save into the local library row: the artist page's Subscribe, the album
 * page's Save. Without it a card's ⋯ menu would keep offering "Save to library" for something the
 * account already holds, since the library's artist grid is built from songs, not subscriptions.
 * Flagged `synced`, so the Library's sync button doesn't count it and no menu offers a local-only
 * removal of a row YouTube owns.
 */
export function noteLibrary(item: BrowseItem, saved: boolean) {
	if (saved !== pl.isSaved(personal, item.id)) pl.toggleSaved(personal, item);
	if (saved) pl.markSynced(personal, [item.id]);
	savePersonal();
}

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
			if ((await pushSaved(item, known)) === 'unsupported') continue;
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

/** Overrides only: a dropped entry falls back to whatever the row was fetched with, which is right.
 *  So a hard clear on overflow is enough (the shape `artcolor.ts` uses) and there is no need for
 *  real LRU book-keeping: past MAX_OVERRIDES entries the whole map goes. */
const MAX_OVERRIDES = 500;
function capOverrides(map: Record<string, unknown>): void {
	if (Object.keys(map).length > MAX_OVERRIDES) for (const k in map) delete map[k];
}

export function ratingOf(song: SongItem): Rating {
	if (playback.now?.videoId === song.video_id) return playback.rating;
	// The saved-in index is the fallback, not an override: a `search` response carries no
	// `likeStatus` on any row (live-checked 2026-08-28), so without it every search result drew
	// itself unrated. A row that does state its rating still wins, since it is fresher than a
	// crawl up to six hours old.
	return (
		ratings[song.video_id] ??
		song.rating ??
		(savedIn.map[song.video_id]?.includes(api.LIKED_MUSIC_ID) ? 'like' : 'indifferent')
	);
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

/** Optimistic rating change, reverted if YouTube rejects it. `msg` overrides the toast, for the
 *  callers that clear a like by another name (out of Library ▸ Songs, which is that same list). */
async function rate(song: SongItem, next: Rating, msg?: string) {
	const prev = ratingOf(song);
	if (prev === next) return;
	const isNow = playback.now?.videoId === song.video_id;
	ratings[song.video_id] = next;
	capOverrides(ratings);
	if (isNow) playback.rating = next;
	try {
		await api.rate(song.video_id, next);
		// Keep the index in step: it outlives this override on a reload, and the crawl that would
		// otherwise correct it runs at most every six hours.
		if (next === 'like') noteSavedIn(api.LIKED_MUSIC_ID, [song.video_id]);
		else noteUnsavedFrom(api.LIKED_MUSIC_ID, song.video_id);
		// Library ▸ Songs *is* the liked-videos browse, and its tab paints from the cache without
		// revalidating, so a like from anywhere else has to drop it or the row is missing for 5 min.
		invalidateCached(LIBRARY_SONGS_KEY);
		toast.success(msg ?? RATED[next]);
		if (next === 'dislike') dropDisliked(song.video_id, isNow);
	} catch (e) {
		ratings[song.video_id] = prev;
		if (isNow) playback.rating = prev;
		toast.error(String(e));
	}
}

/** A disliked track shouldn't keep playing, or sit waiting to. Skip it if it's playing, and drop
 *  every upcoming copy of it from the queue (back to front, so the indices stay valid).
 *  Already-played entries stay: the history is what happened, not what you'd pick again. */
async function dropDisliked(videoId: string, isNow: boolean) {
	const { items, currentIndex } = playback.queue;
	// Removals first, and only above the playing row, so `currentIndex` never shifts under us and
	// the skip lands on a track that survived. Sequential: each one renumbers the backend's queue.
	for (let i = items.length - 1; i > currentIndex; i--) {
		if (items[i]?.video_id === videoId) await api.removeFromQueue(i).catch(() => {});
	}
	if (isNow) api.nextTrack().catch((e) => toast.error(String(e)));
}

// Library membership for songs, overriding what the row was fetched with — the same trick as
// `ratings` above and for the same reason: one song shows up in a list, its ⋯ menu and the queue at
// once, and all three have to agree the moment one of them writes.
const songLibrary = $state<Record<string, boolean>>({});

/** The token for the direction this song can move in, if the row carried one. A menu that offered
 *  only one way (see `LibraryToggle`) runs out after that one write, and the row hides the action
 *  until the list is fetched again — better than a button that answers "token expired". */
export function songLibraryToken(song: SongItem): string | undefined {
	return inSongLibrary(song) ? song.library?.remove_token : song.library?.add_token;
}

/**
 * How a row in Library ▸ Songs can be taken back out, if it can at all.
 *
 * `token` when YouTube sent one with the row, which is the individually-saved case. Otherwise a
 * like is what put the song in that list (it browses `FEmusic_liked_videos`), so clearing the like
 * is the removal. A song that is only there because its album is saved has neither, and gets no
 * option: YouTube sends those rows without a menu, which is its own way of saying the album is
 * what holds them.
 */
export function songLibraryRemoval(song: SongItem): 'token' | 'like' | undefined {
	if (song.library?.remove_token) return 'token';
	return isLiked(song) ? 'like' : undefined;
}

/** Take a song out of Library ▸ Songs by whichever route it has. Answers whether it worked, so the
 *  list only drops the row on success. Toasts either way. */
export async function removeSongFromLibrary(song: SongItem): Promise<boolean> {
	const how = songLibraryRemoval(song);
	if (!how) return false;
	if (how === 'like') {
		await rate(song, 'indifferent', t('toasts.removed_from_library'));
		return ratingOf(song) !== 'like';
	}
	try {
		await api.setSongSaved(song.library!.remove_token!);
		songLibrary[song.video_id] = false;
		capOverrides(songLibrary);
		toast.success(t('toasts.removed_from_library'));
		return true;
	} catch (e) {
		toast.error(String(e));
		return false;
	}
}

/** In Library ▸ Songs? The row's own menu is the fallback; a write in this session wins. */
export function inSongLibrary(song: SongItem): boolean {
	return songLibrary[song.video_id] ?? song.library?.in_library ?? false;
}

/**
 * Add a song to the library, or take it out. Nothing to do with the rating: Library ▸ Songs and
 * Liked Music are separate lists, and this write is a feedback token minted on the row itself, so
 * only songs YouTube sent a menu with can offer it (`song.library`).
 */
export async function toggleSongLibrary(song: SongItem): Promise<void> {
	const next = !inSongLibrary(song);
	const token = songLibraryToken(song);
	if (!token) return;
	songLibrary[song.video_id] = next; // optimistic
	capOverrides(songLibrary);
	try {
		await api.setSongSaved(token);
		invalidateCached(LIBRARY_SONGS_KEY); // the tab paints from cache; this changed what's in it
		toast.success(next ? t('library.saved_to_library') : t('toasts.removed_from_library'));
	} catch (e) {
		delete songLibrary[song.video_id];
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
	theaterOpen: false, // fullscreen theater view (artwork + lyrics)
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
			// Warm the music video now rather than when the view opens: the resolve is a round trip
			// to YouTube, and paid here it overlaps the track starting instead of the user's click.
			// Not in the mini player, which has no player view to show it in.
			if (!mini && prefs.musicVideos && n.isVideo) videoUrlFor(n.videoId);
		}),
		// YouTube's own answer for a track whose row never stated one (issue #93). Into the
		// override map as well as the player bar: the same song is on screen as a list row too,
		// and `ratingOf` reads that map for every row that is not the playing one.
		api.onRating((videoId, rating) => {
			ratings[videoId] = rating;
			capOverrides(ratings);
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
		api.onQueueAppended((q) => {
			const items = [...playback.queue.items, ...q.items];
			if (items.length !== q.len) {
				// Missed an event. Cheaper to refetch once than to guess at what we are missing.
				api.getQueue()
					.then((full) => (playback.queue = full))
					.catch(() => {});
				return;
			}
			playback.queue = {
				...playback.queue,
				items,
				currentIndex: q.currentIndex,
				playedFrom: q.playedFrom
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
