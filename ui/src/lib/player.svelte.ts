// Shared reactive app state (playback + auth), set up ONCE by the root layout. Components import
// `playback`/`auth` and read them reactively; the Rust side drives them via Tauri events.
// context/11 UI contract — this module only calls commands / subscribes to events.
import { browser } from '$app/environment';
import * as api from './api';
import type { Account, BrowseItem, NowPlaying, QueueState, SongItem } from './api';
import { applyLtState } from './lt.svelte';
import { clearCached } from './pagecache';
import * as pl from './personal';
import type { Personal } from './personal';

export const playback = $state({
	now: null as NowPlaying | null,
	queue: { items: [], currentIndex: 0 } as QueueState,
	paused: false,
	position: 0,
	duration: 0,
	volume: 100,
	error: null as string | null,
	// Like state for the current track — seeded from the track's real `likeStatus` on each change,
	// then optimistic on toggle.
	liked: false
});

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
	error: null as string | null
});

/** Fetch the library once (or force a refresh). No-op while a load is in flight. */
export async function loadLibrary(force = false) {
	if (library.loading || (library.loaded && !force)) return;
	library.loading = true;
	library.error = null;
	try {
		library.items = await api.getLibrary();
		library.loaded = true;
	} catch (e) {
		library.error = String(e);
	} finally {
		library.loading = false;
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

// --- Personalization: Quick Picks, sidebar pins, play recency (see personal.ts) -----------------
// The Quick Picks grid holds only what the user puts in it — nothing is ever auto-added.
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

/** Add to Quick Picks (evicting the tile gone longest unplayed when the grid is full). */
export function addPick(item: BrowseItem) {
	const added = pl.addPick(personal, item);
	savePersonal();
	toast(added ? 'Added to Quick Picks' : 'Already in Quick Picks');
}

export function removePick(id: string) {
	pl.removePick(personal, id);
	savePersonal();
}

/** Called from every card click app-wide, so only persist when the id was actually on the grid. */
export function touchPick(id: string) {
	if (pl.touchPick(personal, id)) savePersonal();
}

export function togglePin(id: string) {
	const result = pl.togglePin(personal, id);
	if (result === 'full') toast(`Unpin one first — ${pl.MAX_PINS} pins max`);
	else savePersonal();
	return result;
}

/**
 * Play a playlist/album/artist and record that it was played, which is what sorts the sidebar and
 * seeds Quick Picks. Every "play these tracks from somewhere" call site goes through this.
 */
export function playFrom(source: BrowseItem, items: SongItem[], start: number | null) {
	pl.noteRecent(personal, source);
	pl.touchPick(personal, source.id);
	savePersonal();
	return api.playPlaylist(items, start);
}

// Transient UI state for write actions.
export const ui = $state({
	addVideoIds: null as string[] | null, // add-to-playlist picker target(s)
	toast: null as string | null,
	settingsOpen: false, // the settings modal
	ltOpen: false // the Listen Together modal
});

export function toast(msg: string) {
	ui.toast = msg;
	setTimeout(() => {
		if (ui.toast === msg) ui.toast = null;
	}, 2500);
}

export function openAddToPlaylist(videoId: string) {
	ui.addVideoIds = [videoId];
}

/** Open the picker to add several tracks at once (e.g. a whole album). */
export function openAddManyToPlaylist(videoIds: string[]) {
	ui.addVideoIds = videoIds.length ? videoIds : null;
}

let started = false;

/** Wire the Tauri event listeners once and seed initial state. Returns a teardown fn. */
export function initApp(): () => void {
	if (started) return () => {};
	started = true;
	const subs = [
		api.onNowPlaying((n) => {
			playback.now = n;
			playback.liked = n.liked ?? false; // reflect the track's real like status when known
			playback.error = null; // a track started → clear any stale dead-end banner
			// Feeds Quick Picks recency and the community shelf's artist seed. Every play lands here,
			// gapless advances included, so it's the one hook that sees them all.
			pl.touchPick(personal, n.videoId);
			if (n.artists) pl.noteArtist(personal, n.artistId ?? n.artists, pl.firstArtist(n.artists));
			savePersonal();
		}),
		api.onQueueChanged((q) => (playback.queue = q)),
		api.onPosition((p) => (playback.position = p)),
		api.onDuration((d) => (playback.duration = d)),
		api.onPlaybackState((s) => (playback.paused = s === 'paused')),
		api.onPlaybackError((msg) => (playback.error = msg)),
		api.onPlaybackNotice((msg) => toast(msg)), // auto-skipped an unplayable track
		api.onAuthChanged((a) => {
			auth.account = a;
			if (a.signedIn) loadLibrary(true);
			else {
				library.items = [];
				library.loaded = false;
			}
			clearCached();
			auth.epoch++;
		}),
		api.onLoginError((msg) => toast(msg)),
		api.onLoginDone(() => toast('Signed in')),
		// Listen Together (context/19): mirror the Rust session state; surface notices as toasts.
		api.onLtState((s) => applyLtState(s)),
		api.onLtNotice((msg) => toast(msg))
	];
	api.getQueue()
		.then((q) => {
			playback.queue = q;
			// On a cold start the backend restores the queue (paused) before the UI subscribes, so
			// the now-playing event is missed. Seed the player-bar card from the restored current
			// item; hitting play resolves it for real and re-emits now-playing.
			if (!playback.now) {
				const cur = q.items[q.currentIndex];
				if (cur) {
					playback.now = {
						videoId: cur.video_id,
						title: cur.title,
						artists: cur.artists,
						artistId: cur.artist_id,
						thumbnail: cur.thumbnail,
						duration: cur.duration,
						streamClient: 'restored',
						liked: null
					};
					playback.paused = true;
				}
			}
		})
		.catch(() => {});
	api.getAccount()
		.then((a) => {
			auth.account = a;
			if (a.signedIn) loadLibrary();
		})
		.catch(() => {});
	// Seed the Listen Together state (server URL, any active room after a UI reload).
	api.ltGetState().then(applyLtState).catch(() => {});
	return () => subs.forEach((u) => u.then((f) => f()));
}
