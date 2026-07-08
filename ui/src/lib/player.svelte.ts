// Shared reactive app state (playback + auth), set up ONCE by the root layout. Components import
// `playback`/`auth` and read them reactively; the Rust side drives them via Tauri events.
// context/11 UI contract — this module only calls commands / subscribes to events.
import * as api from './api';
import type { Account, BrowseItem, NowPlaying, QueueState } from './api';

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
	account: null as Account | null
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

// Transient UI state for write actions.
export const ui = $state({
	addVideoIds: null as string[] | null, // add-to-playlist picker target(s)
	toast: null as string | null,
	settingsOpen: false // the settings modal
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
		}),
		api.onQueueChanged((q) => (playback.queue = q)),
		api.onPosition((p) => (playback.position = p)),
		api.onDuration((d) => (playback.duration = d)),
		api.onPlaybackState((s) => (playback.paused = s === 'paused')),
		api.onPlaybackError((msg) => (playback.error = msg)),
		api.onAuthChanged((a) => {
			auth.account = a;
			if (a.signedIn) loadLibrary(true);
			else {
				library.items = [];
				library.loaded = false;
			}
		}),
		api.onLoginError((msg) => toast(msg)),
		api.onLoginDone(() => toast('Signed in'))
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
	return () => subs.forEach((u) => u.then((f) => f()));
}
