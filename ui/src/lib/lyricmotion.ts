// How a synced lyric moves, shared by the lyrics view and the line under the now-playing cover.
// The numbers are Better Lyrics' defaults (@braccato/core, MIT), which is the feel this aims at.
// Only the word being sung is rewritten per frame, and only its sweep. A held note's halo and crest
// are classes its letters take once (`.lyric-halo`, `.lyric-held` in layout.css).
import type { LyricWord } from './api';

type Timed = Pick<LyricWord, 'start_ms' | 'end_ms'>;

/** The soft edge of the sweep, in em: a short word and a long one light at the same softness. */
const FEATHER = 0.5;
const DIM = 'var(--muted-foreground)';
const LIT = 'var(--foreground)';
const UNSUNG = `background-image:linear-gradient(${DIM},${DIM})`;
const SUNG = `background-image:linear-gradient(${LIT},${LIT})`;

/** How far the singer has got through a word, 0 to 1. */
export function progress(w: Timed, ms: number): number {
	const span = w.end_ms - w.start_ms;
	if (span <= 0) return ms >= w.start_ms ? 1 : 0;
	return Math.min(1, Math.max(0, (ms - w.start_ms) / span));
}

/** The word's inline style: lit up to the singer, a soft edge, dim after it. A word before or
 *  after its time gets the same string every frame, so only the one being sung is rewritten.
 *  No word rises as it is sung (Apple's lift, Better Lyrics' wobble): moving every word cost the
 *  lyrics tab 40% to 49% of a core on WebKitGTK 2.52 as a CSS transition and 46% to 57% written per
 *  frame, for a movement of two pixels. Held notes still crest (`letters`). */
export function sweep(w: Timed, ms: number): string {
	const p = progress(w, ms);
	if (p <= 0) return UNSUNG;
	if (p >= 1) return SUNG;
	// The lit stop runs from one feather before the word to its end, the dim stop from its start
	// to one feather after, so the edge is always FEATHER wide and nothing shows lit at p = 0.
	const at = (p * 100).toFixed(2);
	const lit = ((1 - p) * FEATHER).toFixed(3);
	const dim = (p * FEATHER).toFixed(3);
	return `background-image:linear-gradient(90deg,${LIT} calc(${at}% - ${lit}em),${DIM} calc(${at}% + ${dim}em))`;
}

/** A held note's letter glows from when it is reached until a moment after the whole note ends,
 *  so the glow builds across the note and lets go of it at once. */
export function glowing(letter: Timed, note: Timed, ms: number): boolean {
	return ms >= letter.start_ms && ms < note.end_ms + 150;
}

/** A held note: long enough that its letters can light and lift one by one. */
const HELD_MS = 1000;
const MAX_LETTERS = 12;
// Scripts whose letters stand alone. Splitting a joined script (Arabic, Devanagari, Thai...) into
// boxes would break its shaping, so those words light whole.
const SEPARABLE = /^[\p{Script=Latin}\p{Script=Greek}\p{Script=Cyrillic}\p{M}\p{N}\p{P}]+$/u;
const graphemes = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

/** A held word's letters, each with its share of the word's time, or null for a word that lights
 *  whole. */
export function letters(w: LyricWord): LyricWord[] | null {
	const text = w.text.trimEnd();
	const span = w.end_ms - w.start_ms;
	if (span < HELD_MS || !SEPARABLE.test(text)) return null;
	const parts = Array.from(graphemes.segment(text), (s) => s.segment);
	if (parts.length < 2 || parts.length > MAX_LETTERS) return null;
	return parts.map((text, i) => ({
		text,
		start_ms: w.start_ms + (span * i) / parts.length,
		end_ms: w.start_ms + (span * (i + 1)) / parts.length
	}));
}

/** The lyrics' scroll easing: a slow start, a quick middle, a long settle. */
export const SCROLL_EASE = 'cubic-bezier(0.86, 0, 0.2, 1)';

/** How long a line `rel` lines from the one scrolled to takes to get there. The lines ahead of the
 *  motion arrive first, the ones trailing it later the further back they are, so the column moves
 *  with some give instead of as one block. */
export function scrollMs(rel: number, ahead: boolean): number {
	const n = Math.abs(rel);
	if (ahead && n > 0) return 750 + Math.min(n, 6) * 20;
	return 750 + Math.log(n + 2) * 80 + (n + 1) * 20;
}
