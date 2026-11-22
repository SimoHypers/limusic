// The seek bar's fill between position ticks (the logic; `seekfill.svelte.ts` draws it).
//
// `playback.position` arrives at ~4 Hz, so a fill that follows it moves in jumps: a 3-minute track
// in a 1200 px bar is one pixel every 150 ms, and a tick lands every 250. The fill is advanced from
// the last tick at the rate the track plays instead, and corrected when the next one lands.
//
// Percent is the unit throughout, which is why tempo needs no handling: the bar's duration is
// already the shown (tempo-adjusted) one, so `speed` cancels out of position / duration.

/** How long a tick is trusted for. Past this without a new one (a stall, a buffering network
 *  stream) the fill holds rather than running ahead of audio that is not playing. */
export const MAX_COAST_MS = 500;
/** A tick this far (in shown seconds) from where the fill expected it is a seek or a track change,
 *  not timer jitter: the fill goes there at once, backwards included. */
export const JUMP_SECS = 1;
/** Fastest and slowest redraw. One pixel of travel is the useful rate, which on any real window
 *  is a few draws a second; the cap keeps a short track in a wide bar from asking for 60. */
export const MIN_GAP_MS = 1000 / 30;
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

/** A tick landed at `pct`. Small disagreement with the coast is jitter and only moves the anchor
 *  (the fill never steps backwards for it); a big one is a seek and moves the fill too. */
export function sample(c: Clock, pct: number, now: number, duration: number): Clock {
	const jumped = !(duration > 0) || Math.abs(pct - coast(c, now, duration)) * duration > JUMP_SECS;
	return { shown: jumped ? pct : c.shown, pct, at: now };
}

/** One redraw: advance the fill to the coast, never back. */
export function frame(c: Clock, now: number, duration: number): Clock {
	const shown = Math.max(c.shown, coast(c, now, duration));
	return shown === c.shown ? c : { ...c, shown };
}

/** Milliseconds between redraws: the time the fill takes to cross one pixel, clamped. */
export function drawGap(duration: number, widthPx: number): number {
	const perPx = widthPx > 0 && duration > 0 ? (duration * 1000) / widthPx : MAX_GAP_MS;
	return Math.min(MAX_GAP_MS, Math.max(MIN_GAP_MS, perPx));
}
