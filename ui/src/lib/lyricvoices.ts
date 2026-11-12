// Singers and background vocals as the lyrics view lays them out. Only Limusic's lyrics carry
// either (`agent` on a line, `bg` on a word). Pure, so `lyricvoices.check.ts` can run it.
import type { LyricLine, LyricWord } from './api';

/** A stretch of a line's words that are all lead vocals, or all background ones. */
export interface VoiceRun {
	bg: boolean;
	words: LyricWord[];
}

export interface Voice {
	/** The line sits on the opposite side: it belongs to the other singer of a duet. */
	far: boolean;
	/** The line's words split where lead and background vocals meet. Null when it has no
	 *  background words, so the line is drawn whole. */
	runs: VoiceRun[] | null;
}

/** `v2` -> 2. Anything that isn't a singer's id (the group's `v1000` included) is null. */
function singer(agent: string | undefined): number | null {
	const n = agent && /^v\d{1,4}$/.test(agent) ? Number(agent.slice(1)) : NaN;
	return Number.isNaN(n) || n === 1000 ? null : n;
}

function runsOf(words: LyricWord[] | undefined): VoiceRun[] | null {
	if (!words?.some((w) => w.bg)) return null;
	const runs: VoiceRun[] = [];
	for (const w of words) {
		const last = runs[runs.length - 1];
		if (last && last.bg === !!w.bg) last.words.push(w);
		else runs.push({ bg: !!w.bg, words: [w] });
	}
	return runs;
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
		const v = { far: duet && n !== null && singers.indexOf(n) % 2 === 1, runs: runsOf(l.words) };
		any ||= v.far || v.runs !== null;
		return v;
	});
	return any ? out : null;
}
