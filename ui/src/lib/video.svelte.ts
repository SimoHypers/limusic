// The music video's state, and the handoff of the one live <video> element.
//
// It lives outside NowPlaying.svelte because +layout unmounts that component whenever the player
// view is closed. Rebuilding the element on every reopen meant opening the stream at byte 0 and
// then seeking minutes into a WebM, which has to fetch the container index first and takes seconds
// that no amount of tuning the sync loop could recover. So the element is owned by VideoSurface,
// which is always mounted, and the view borrows it.
import * as api from './api';
import { playback, prefs, videoReady } from './player.svelte';

/** Session-sticky on purpose: someone who hits "show artwork" wants artwork now and almost
 *  certainly on the next video too, but a permanent no is what the setting is for. */
export const video = $state({
	want: true,
	/** Whether the player view is holding the element, i.e. someone can actually see the picture.
	 *  VideoSurface releases the stream when nobody can and the music is not moving. */
	shown: false,
	/** The loopback proxy URL for the current track, or null. Owned by VideoSurface's fetch. */
	url: null as string | null,
	/** Linux: the hole the page leaves for mpv's picture, while the picture is up (see setHole).
	 *  Viewport CSS pixels. */
	hole: null as Hole | null
});

export type Hole = { x: number; y: number; w: number; h: number };

export const canVideo = () => prefs.musicVideos && !!playback.now?.isVideo;
/** The playing track's other version (#442), read off its queue row: a radio learns it after the
 *  track has started, so the now-playing event can't carry it. */
export const counterpart = () => {
	const row = playback.queue.items[playback.queue.currentIndex];
	return row && row.video_id === playback.now?.videoId ? row.counterpart : undefined;
};
/** Whether the player view offers Song/Video: a video to show or hide, or a version to switch to. */
export const canSwitch = () => prefs.musicVideos && (!!playback.now?.isVideo || !!counterpart());
export const hasVideo = () =>
	canVideo() && (prefs.nativeVideo ? !!videoReady[playback.now!.videoId] : !!video.url);
export const showVideo = () => hasVideo() && video.want;

// The live element and where it waits when nothing is showing it. Plain module lets: this is DOM
// identity, nothing renders off it.
let node: HTMLVideoElement | null = null;
let parking: HTMLElement | null = null;

/** VideoSurface, once, at mount. */
export function registerVideo(v: HTMLVideoElement, park: HTMLElement) {
	node = v;
	parking = park;
	park.appendChild(v);
}

/** The live element, for the ambient light to read frames from. */
export const videoElement = () => node;

/** Put the picture in `box`. Synchronous on purpose: a media element that is out of the document
 *  across a microtask gets paused by the spec's removal steps, and a paused picture is exactly the
 *  desync this whole thing exists to avoid. */
export function claimVideo(box: HTMLElement) {
	if (node) box.appendChild(node);
	video.shown = true;
}

/** Send it back to the parking container. Same rule: synchronous, never from an effect. */
export function parkVideo() {
	if (node && parking) parking.appendChild(node);
	video.shown = false;
}

// --- Linux and Windows: mpv draws the picture under the page ------------------------------------
// (src-tauri/src/nativevideo.rs, nativevideo_windows.rs). The window is transparent, so a hole with
// nothing under it shows the desktop. GTK draws each frame with whatever the webview last
// presented, and a page change takes a frame or two to get there, while Rust moves the picture on
// its very next one. So the hole never gets ahead of the picture: it shrinks at once to what the
// picture covers both before and after, Rust moves the picture only once that has been painted, and
// the hole opens the rest once Rust says the picture is there. Moving them together showed the
// desktop for a frame whenever the picture went away (an opened link, whose video is not resolved
// yet) or moved.

let holeSeq = 0;
let holeSent: string | null = null;

/** Put mpv's picture at `r` (viewport CSS pixels), or take it away. */
export function setHole(r: Hole | null) {
	// Under a pixel there is nothing to show, and a reflow hands one over mid-resize (#321).
	if (r && (r.w < 1 || r.h < 1)) r = null;
	const key = r && `${r.x},${r.y},${r.w},${r.h}`;
	if (key === holeSent) return;
	holeSent = key;
	const seq = ++holeSeq;
	video.hole = r && video.hole && overlap(video.hole, r);
	const send = () => {
		if (seq !== holeSeq) return;
		api.nativeVideoRect(r && [r.x, r.y, r.w, r.h])
			.then((up) => {
				if (seq !== holeSeq || !r) return;
				video.hole = up ? r : null;
				// No GL surface, and there never will be: back to the <video> element.
				if (!up) prefs.nativeVideo = false;
			})
			.catch(() => {});
	};
	// Two frames: the first paints the smaller hole, the second runs once that one is out. A hidden
	// page gets no frames, and nobody can see it anyway.
	// ponytail: a live window resize keeps restarting this, so the picture catches up when the drag
	// stops, with page background at the new edges meanwhile. Throttle instead if that shows.
	if (document.hidden) send();
	else requestAnimationFrame(() => requestAnimationFrame(send));
}

function overlap(a: Hole, b: Hole): Hole | null {
	const x = Math.max(a.x, b.x);
	const y = Math.max(a.y, b.y);
	const w = Math.min(a.x + a.w, b.x + b.w) - x;
	const h = Math.min(a.y + a.h, b.y + b.h) - y;
	return w > 0 && h > 0 ? { x, y, w, h } : null;
}
