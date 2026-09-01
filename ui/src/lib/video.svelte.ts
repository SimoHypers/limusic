// The music video's state, and the handoff of the one live <video> element.
//
// It lives outside NowPlaying.svelte because +layout unmounts that component whenever the player
// view is closed. Rebuilding the element on every reopen meant opening the stream at byte 0 and
// then seeking minutes into a WebM, which has to fetch the container index first and takes seconds
// that no amount of tuning the sync loop could recover. So the element is owned by VideoSurface,
// which is always mounted, and the view borrows it.
import { playback, prefs } from './player.svelte';

/** Session-sticky on purpose: someone who hits "show artwork" wants artwork now and almost
 *  certainly on the next video too, but a permanent no is what the setting is for. */
export const video = $state({
	want: true,
	/** Whether the player view is holding the element, i.e. someone can actually see the picture.
	 *  VideoSurface releases the stream when nobody can and the music is not moving. */
	shown: false,
	/** The loopback proxy URL for the current track, or null. Owned by VideoSurface's fetch. */
	url: null as string | null
});

export const canVideo = () => prefs.musicVideos && !!playback.now?.isVideo;
export const hasVideo = () => canVideo() && !!video.url;
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
