// Shared reactive app state (playback + auth), set up ONCE by the root layout. Components import
// `playback`/`auth` and read them reactively; the Rust side drives them via Tauri events.
// context/11 UI contract — this module only calls commands / subscribes to events.
import * as api from './api';
import type { Account, NowPlaying, QueueState } from './api';

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

// Transient UI state for write actions.
export const ui = $state({
	addVideoIds: null as string[] | null, // add-to-playlist picker target(s)
	toast: null as string | null
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
		api.onAuthChanged((a) => (auth.account = a)),
		api.onLoginError((msg) => toast(msg)),
		api.onLoginDone(() => toast('Signed in'))
	];
	api.getQueue().then((q) => (playback.queue = q)).catch(() => {});
	api.getAccount().then((a) => (auth.account = a)).catch(() => {});
	return () => subs.forEach((u) => u.then((f) => f()));
}
