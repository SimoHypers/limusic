// The seek bar's fill between position ticks (the logic; `seekfill.svelte.ts` draws it).
//
// `playback.position` arrives at ~4 Hz, so a fill that follows it moves in jumps: a 3-minute track
// in a 1200 px bar is one pixel every 150 ms, and a tick lands every 250. The fill is advanced from
// the last tick at the rate the track plays instead, and corrected when the next one lands.
//
// Percent is the unit throughout, which is why tempo needs no handling: the bar's duration is
// already the shown (tempo-adjusted) one, so `speed` cancels out of position / duration.

/** How long a tick is trusted for. Ticks land 250 ms apart at best and a busy webview delays them
 *  further, so this has to outlast that; past it without a new one (a stall, a buffering network
 *  stream) the fill holds rather than running ahead of audio that is not playing. */
export const MAX_COAST_MS = 750;
/** A tick this far (in shown seconds) *behind* where the fill is can only be a seek back, a loop
 *  or a new track: the fill coasts from an old tick, so it is never ahead of the audio except in a
 *  stall, and jitter alone cannot put it this far ahead. It goes back at once. A tick ahead of
 *  the fill never needs a threshold: the fill only ever moves forward to meet it. */
export const BACK_SECS = 0.5;
/** Redraws per pixel of travel. The track's edge is a 1 px soft ramp (layout.css), so a move of a
 *  fraction of a pixel is a different shade on screen, and a quarter pixel is the step the eye
 *  stops seeing. */
export const STEPS_PER_PX = 4;
/** Longest a redraw can wait: a long track in a narrow bar moves a quarter pixel in seconds. */
export const MAX_GAP_MS = 250;

export type Clock = {
	/** What is drawn, 0..1. Never behind what coasting says, except after a jump. */
	shown: number;
	/** The last tick, 0..1, and when it was taken (`performance.now()`). */
	pct: number;
	at: number;
};

/** A fresh clock, drawn exactly at `pct`: a mount, a resume, the end of a drag. */
export const restart = (pct: number, now: number): Clock => ({ shown: pct, pct, at: now });

/** Where playback is `now`, from the last tick. */
export function coast(c: Clock, now: number, duration: number): number {
	if (!(duration > 0)) return c.pct;
	const dt = Math.min(MAX_COAST_MS, Math.max(0, now - c.at));
	return Math.min(1, c.pct + dt / (duration * 1000));
}

/** A tick landed at `pct`. Ahead of the fill, the fill goes there. A little behind is jitter and
 *  only moves the anchor (the fill never steps backwards for it); far behind is a seek back or a
 *  new track and moves the fill too. */
export function sample(c: Clock, pct: number, now: number, duration: number): Clock {
	const back = !(duration > 0) || (c.shown - pct) * duration > BACK_SECS;
	return { shown: back ? pct : Math.max(c.shown, pct), pct, at: now };
}

/** One redraw: advance the fill to the coast, never back. */
export function frame(c: Clock, now: number, duration: number): Clock {
	const shown = Math.max(c.shown, coast(c, now, duration));
	return shown === c.shown ? c : { ...c, shown };
}

/** Milliseconds between redraws: the time the fill takes to cross a quarter pixel, so 0 (every
 *  animation frame) wherever it moves faster than that, which is any track under a minute or so in
 *  a wide bar, and a few a second on a long one. */
export function drawGap(duration: number, widthPx: number): number {
	const perStep =
		widthPx > 0 && duration > 0 ? (duration * 1000) / (widthPx * STEPS_PER_PX) : MAX_GAP_MS;
	return Math.min(MAX_GAP_MS, perStep);
}
