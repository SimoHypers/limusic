// Singers and background vocals as the lyrics view lays them out: `agent` on a line (Limusic's
// lyrics only), `bg` on a word (Limusic's, and Apple's through Boidu, LyricsPlus and Unison). Pure,
// so `lyricvoices.check.ts` can run it.
import type { LyricLine, LyricWord } from './api';

/** A stretch of a line's words that are all lead vocals, or all background ones. */
export interface VoiceRun {
	bg: boolean;
	words: LyricWord[];
}

export interface Voice {
	/** The line sits on the opposite side: it belongs to the other singer of a duet. */
	far: boolean;
	/** The lead's words, then the background's, which go under them. Null when the line has no
	 *  background words, so it is drawn whole. */
	runs: VoiceRun[] | null;
	/** When its last background word ends, 0 without any. They are often sung over the start of
	 *  the next line, and the line stays lit until then. */
	until: number;
}

/** `v2` -> 2. Anything that isn't a singer's id (the group's `v1000` included) is null. */
function singer(agent: string | undefined): number | null {
	const n = agent && /^v\d{1,4}$/.test(agent) ? Number(agent.slice(1)) : NaN;
	return Number.isNaN(n) || n === 1000 ? null : n;
}

/** Background vocals have their own timings, sung over the lead rather than between its words, so
 *  they are one line under it wherever they were written in the lyric (Apple's layout): a "(Yeah)"
 *  in the middle no longer cuts the lead in two. */
function runsOf(words: LyricWord[] | undefined): VoiceRun[] | null {
	if (!words?.some((w) => w.bg)) return null;
	const lead = words.filter((w) => !w.bg);
	const bg = { bg: true, words: words.filter((w) => w.bg) };
	return lead.length ? [{ bg: false, words: lead }, bg] : [bg];
}

/** One entry per line, or null when the lyric has no second singer and no background word: the
 *  view then draws exactly what it drew before either existed, and pays nothing for them.
 *
 *  Sides go by the singer's number, not by who sings first: the lowest is on the near side, the
 *  next on the far side, and so on in turn, so "Singer 2" in the editor is always the other side.
 *  Lines for everyone together, and lines nobody is named on, stay on the near side. One singer
 *  alone is not a duet and moves nothing. */
export function voices(lines: LyricLine[]): Voice[] | null {
	const singers = [...new Set(lines.map((l) => singer(l.agent)).filter((n) => n !== null))].sort(
		(a, b) => a - b
	);
	const duet = singers.length > 1;
	let any = false;
	const out = lines.map((l) => {
		const n = singer(l.agent);
		const until = Math.max(0, ...(l.words ?? []).filter((w) => w.bg).map((w) => w.end_ms));
		const v = { far: duet && n !== null && singers.indexOf(n) % 2 === 1, runs: runsOf(l.words), until };
		any ||= v.far || v.runs !== null;
		return v;
	});
	return any ? out : null;
}
