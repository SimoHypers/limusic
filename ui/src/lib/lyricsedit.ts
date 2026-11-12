// The lyric editor's logic, with no Svelte and no Tauri in it so `lyricsedit.check.ts` can run every
// rule under Node: splitting a line into words, stamping a tap, the loop, dragging a word's edge,
// what is still missing, and the lines that get submitted. The page (`routes/lyrics-editor`) only
// holds state and draws it.
import type { LyricLine, LyricWord } from './api';

/** One word, or one part of a word tapped on its own ("danc" + "ing"). `text` ends in a space when
 *  a space follows it in the line, the way every word-timed provider sends it (`LyricsView` reads
 *  that as "the word ends here"). Times are whole milliseconds, absent until tapped. */
export interface EdWord {
	text: string;
	start?: number;
	end?: number;
	bg?: boolean;
}

export interface EdLine {
	/** Stable across edits: the list's key, and what a late tap is matched against. */
	id: number;
	words: EdWord[];
	/** Line start and end in ms, when known: from the source, the line pass, or its words. */
	time?: number;
	end?: number;
	/** `v1`, `v2`, ... or `v1000` for everyone. */
	agent?: string;
	/** An instrumental break: no words, only a start. The lyrics view draws a music note from
	 *  then until the next line begins. */
	gap?: boolean;
	/** The line as it was loaded. Submitted exactly as it came while `dirty` is unset. */
	orig?: LyricLine;
	dirty?: boolean;
}

export interface Draft {
	v: 1;
	videoId: string;
	lines: EdLine[];
	/** The next free `EdLine.id`. */
	nextId: number;
	/** The server's live revision the draft was started from, `null` when the song had none. */
	baseRevision: number | null;
	/** Whether the starting lyric was already word by word: "Improve" rather than "Sync". */
	improving: boolean;
}

// --- Words ------------------------------------------------------------------------------------

/** Scripts written without spaces between words: kana, Han, Thai, Lao, Myanmar, Khmer. */
const UNSPACED = /[぀-ヿ㐀-䶿一-鿿豈-﫿฀-໿က-႟ក-៿]/;

/** Give every word but the last its trailing space. */
const spaced = (tokens: string[][]): string[] =>
	tokens.flatMap((parts, i) =>
		parts.map((p, j) => (j === parts.length - 1 && i < tokens.length - 1 ? `${p} ` : p))
	);

const tokens = (text: string): string[] => text.trim().split(/\s+/).filter(Boolean);

/** The editor's text field as words: a space ends a word, and a `|` splits one into parts that
 *  are tapped separately without putting a space in the lyric.
 *  ponytail: a lyric cannot hold a literal `|`; pick another mark if one ever needs to. */
export function parseField(text: string): string[] {
	return spaced(tokens(text).map((tok) => tok.split('|').filter(Boolean)).filter((p) => p.length));
}

/** Words as the text field shows them: the reverse of `parseField`. */
export function fieldText(words: EdWord[]): string {
	return words
		.map((w, i) => (/\s$/.test(w.text) || i === words.length - 1 ? w.text : `${w.text}|`))
		.join('')
		.trim();
}

/** A token of an unspaced script as words. The engine's own word breaker where there is one
 *  (WebKit, Chromium and Node all ship it), else one word per character. Punctuation and anything
 *  else that is not a word stays with the word before it. */
function segment(tok: string): string[] {
	let parts: { segment: string; isWordLike?: boolean }[];
	try {
		parts = [...new Intl.Segmenter(undefined, { granularity: 'word' }).segment(tok)];
	} catch {
		parts = [...tok].map((c) => ({ segment: c, isWordLike: /[\p{L}\p{N}]/u.test(c) }));
	}
	const out: string[] = [];
	let lead = '';
	for (const p of parts) {
		if (p.isWordLike) {
			out.push(lead + p.segment);
			lead = '';
		} else if (out.length) out[out.length - 1] += p.segment;
		else lead += p.segment;
	}
	if (lead) out.push(lead);
	return out;
}

/** A line of text as words, for text that arrives without any: a space ends a word, and a run in
 *  a script that has no spaces is broken into its words. The user adjusts it in the text field. */
export function autoSplit(text: string): string[] {
	return spaced(tokens(text).map((tok) => (UNSPACED.test(tok) ? segment(tok) : [tok])));
}

/** The line as sung. */
export const lineText = (line: EdLine): string =>
	line.words
		.map((w) => w.text)
		.join('')
		.trim();

const collapse = (s: string) => s.trim().replace(/\s+/g, ' ');

/** A provider's words with the trailing spaces put back where the line's own text has them. Some
 *  JSON shapes send bare words ("Hello", "world"), which would read as one run-together word. */
function wordsOf(line: LyricLine): EdWord[] {
	const src = line.words ?? [];
	const text = line.text ?? '';
	const joined = src.map((w) => w.text).join('');
	const fix = collapse(joined) !== collapse(text);
	let at = 0;
	return src.map((w, i) => {
		let t = w.text;
		if (fix) {
			const bare = w.text.trim();
			const hit = bare ? text.indexOf(bare, at) : -1;
			if (hit >= 0) {
				at = hit + bare.length;
				t = /\s/.test(text[at] ?? '') && i < src.length - 1 ? `${bare} ` : bare;
			}
		}
		const word: EdWord = { text: t, start: w.start_ms, end: w.end_ms };
		if (w.bg) word.bg = true;
		return word;
	});
}

// --- Starting points --------------------------------------------------------------------------

/** Lyrics from any source as editor lines. A line that has words keeps them and their timings;
 *  one that has none is split into words to be tapped. Blank untimed lines (stanza breaks in
 *  plain lyrics) are dropped, timed ones (instrumental gaps) stay. */
export function fromLyrics(lines: LyricLine[]): EdLine[] {
	const out: EdLine[] = [];
	for (const l of lines) {
		const timedWords = (l.words?.length ?? 0) > 0;
		if (!timedWords && !l.text.trim() && l.time_ms === undefined) continue;
		const line: EdLine = {
			id: out.length + 1,
			words: timedWords ? wordsOf(l) : autoSplit(l.text).map((text) => ({ text })),
			orig: l
		};
		if (!timedWords && !l.text.trim()) line.gap = true;
		if (l.time_ms !== undefined) line.time = l.time_ms;
		else if (timedWords) line.time = l.words![0].start_ms;
		if (l.end_time_ms !== undefined) line.end = l.end_time_ms;
		if (l.agent) line.agent = l.agent;
		out.push(line);
	}
	return out;
}

/** Pasted plain text: one line each, blank ones dropped. */
export function fromText(text: string): EdLine[] {
	return text
		.split(/\r?\n/)
		.map((l) => l.trim())
		.filter(Boolean)
		.map((l, i) => ({ id: i + 1, words: autoSplit(l).map((t) => ({ text: t })) }));
}

/** What a set of lines is, in the source picker's terms. */
export function kindOf(lines: EdLine[]): 'words' | 'synced' | 'plain' {
	const sung = lines.filter((l) => l.words.length);
	if (sung.length && sung.every(wordsTimed)) return 'words';
	return sung.some((l) => l.time !== undefined) ? 'synced' : 'plain';
}

// --- Text edits -------------------------------------------------------------------------------

export const wordsTimed = (line: EdLine): boolean =>
	line.words.length > 0 && line.words.every((w) => w.start !== undefined && w.end !== undefined);

/** `texts` as the line's words. The same number of words keeps each one's timing (fixing a typo
 *  costs nothing); a different number leaves them to be tapped again. */
function reword(line: EdLine, texts: string[]): EdWord[] {
	const keep = texts.length === line.words.length;
	return texts.map((text, i) => {
		const w: EdWord = keep ? { ...line.words[i], text } : { text };
		return w;
	});
}

/** The line with its text field edited. Returns the same object when nothing changed. */
export function setLineText(line: EdLine, field: string): EdLine {
	const texts = parseField(field);
	if (texts.length === line.words.length && texts.every((t, i) => t === line.words[i].text)) {
		return line;
	}
	const out: EdLine = { ...line, words: reword(line, texts), dirty: true };
	// Text typed into an instrumental break makes it a line like any other.
	if (out.words.length) delete out.gap;
	return out;
}

/** The line split into words again by `autoSplit`, dropping any hand-made parts. */
export function resplitLine(line: EdLine): EdLine {
	return setLineText(line, autoSplit(lineText(line)).join(''));
}

/** Split a line in two at `caret`, an offset into its text field. A caret between two words keeps
 *  every word's timing; one inside a word makes two new words to tap. */
export function splitLine(line: EdLine, field: string, caret: number, newId: number): [EdLine, EdLine] {
	const left = parseField(field.slice(0, caret));
	const right = parseField(field.slice(caret));
	const all = parseField(field);
	const clean =
		all.length === line.words.length &&
		left.length + right.length === all.length &&
		all.every((t, i) => t === line.words[i].text);
	const a: EdWord[] = clean ? line.words.slice(0, left.length) : left.map((text) => ({ text }));
	const b: EdWord[] = clean ? line.words.slice(left.length) : right.map((text) => ({ text }));
	// The first half's last word no longer has a space after it.
	if (a.length) a[a.length - 1] = { ...a[a.length - 1], text: a[a.length - 1].text.trimEnd() };
	const first: EdLine = { id: line.id, words: a, dirty: true };
	const second: EdLine = { id: newId, words: b, dirty: true };
	if (line.time !== undefined) first.time = line.time;
	if (line.agent) first.agent = second.agent = line.agent;
	const lastA = a[a.length - 1];
	if (lastA?.end !== undefined) first.end = lastA.end;
	if (b[0]?.start !== undefined) second.time = b[0].start;
	if (line.end !== undefined) second.end = line.end;
	return [first, second];
}

/** Two lines as one, in that order, every timing kept. */
export function joinLines(a: EdLine, b: EdLine): EdLine {
	const head = a.words.map((w, i) =>
		i === a.words.length - 1 && b.words.length && !/\s$/.test(w.text) ? { ...w, text: `${w.text} ` } : w
	);
	const out: EdLine = { id: a.id, words: [...head, ...b.words], dirty: true };
	const time = a.time ?? b.time;
	const end = b.end ?? a.end;
	if (time !== undefined) out.time = time;
	if (end !== undefined) out.end = end;
	const agent = a.agent ?? b.agent;
	if (agent) out.agent = agent;
	if (!out.words.length && (a.gap || b.gap)) out.gap = true;
	return out;
}

// --- Instrumental breaks ----------------------------------------------------------------------

/** Where a line's singing ends, once every word of it is timed. */
export function lineEnd(line: EdLine): number | undefined {
	if (!wordsTimed(line)) return undefined;
	// Every word, not the last one: imported words can overlap, and the last need not end last.
	return Math.max(line.end ?? 0, ...line.words.map((w) => w.end!));
}

/** A new instrumental break. With no `time` it waits for a tap, like any line with no start. */
export function gapLine(id: number, time?: number): EdLine {
	const line: EdLine = { id, words: [], gap: true, dirty: true };
	if (time !== undefined) line.time = Math.round(time);
	return line;
}

/** Singer ids the server takes: `v` and 1 to 4 digits. */
export const validAgent = (a: string | undefined): a is string => !!a && /^v\d{1,4}$/.test(a);

export function setAgent(line: EdLine, agent: string | undefined): EdLine {
	if ((line.agent ?? undefined) === agent) return line;
	const out: EdLine = { ...line, dirty: true };
	if (agent) out.agent = agent;
	else delete out.agent;
	return out;
}

export function toggleBg(line: EdLine, k: number): EdLine {
	const words = line.words.map((w, i) => {
		if (i !== k) return w;
		const out: EdWord = { ...w };
		if (w.bg) delete out.bg;
		else out.bg = true;
		return out;
	});
	return { ...line, words, dirty: true };
}

// --- The clock: where the song was when a key went down --------------------------------------

/** `playback_clock`'s answer: a position read from mpv and the wall-clock time of the read. */
export interface ClockSample {
	/** Seconds. */
	position: number;
	/** Unix ms of the read. */
	atMs: number;
	speed: number;
	paused: boolean;
}

/** The wall-clock time (Unix ms) an input event happened at. The handler runs some time after
 *  the key went down; `event.timeStamp` says when, on `performance.now()`'s clock. A stamp that
 *  is not on that clock, or is in the future, or implausibly old, is ignored and the handler's own
 *  time stands. */
export function eventWallMs(wallNow: number, perfNow: number, eventStamp: number): number {
	const lag = perfNow - eventStamp;
	return wallNow - (lag >= 0 && lag <= 250 ? lag : 0);
}

/** The song position in ms at a tap. The sample was read at `atMs`; the tap was `atMs - tapWallMs`
 *  earlier (or later, when the read came first), and the song moves `speed` ms per ms. `offsetMs`
 *  is the user's tap offset in real time (reaction, output latency), so it scales the same way.
 *  `null` while paused: a tap on a still song stamps nothing. */
export function stampTap(s: ClockSample, tapWallMs: number, offsetMs: number): number | null {
	if (s.paused) return null;
	const ms = s.position * 1000 - (s.atMs - tapWallMs) * s.speed + offsetMs * s.speed;
	return Math.max(0, Math.round(ms));
}

/** Where the song is now, from the last position report: for the playhead, which only has to look
 *  right. Taps never use it. */
export function estimateMs(
	positionSecs: number,
	reportedAtPerf: number,
	perfNow: number,
	speed: number,
	paused: boolean
): number {
	const since = paused ? 0 : Math.max(0, perfNow - reportedAtPerf);
	return positionSecs * 1000 + since * speed;
}

// --- Taps -------------------------------------------------------------------------------------

/** How long a last word runs when nothing says where it ends. */
const LAST_WORD_MIN_MS = 200;
const LAST_WORD_MAX_MS = 2500;

/** One pass of taps as the line's word timings: tap `k` starts word `k` and ends the one before,
 *  and one more tap after the last word ends it. `null` until every word has its tap, so a pass
 *  that stops halfway never replaces what the line had. Without the extra tap the last word runs
 *  to `lineEnd`, within reason. */
export function wordsFromTaps(
	words: EdWord[],
	taps: (number | undefined)[],
	lineEnd: number | undefined
): EdWord[] | null {
	const n = words.length;
	if (!n) return null;
	const starts: number[] = [];
	for (let k = 0; k < n; k++) {
		const t = taps[k];
		if (t === undefined) return null;
		starts.push(Math.max(t, starts[k - 1] ?? 0));
	}
	const last = starts[n - 1];
	const closing = taps[n];
	const end =
		closing !== undefined
			? Math.max(closing, last)
			: Math.min(Math.max(lineEnd ?? 0, last + LAST_WORD_MIN_MS), last + LAST_WORD_MAX_MS);
	return words.map((w, k) => ({ ...w, start: starts[k], end: k < n - 1 ? starts[k + 1] : end }));
}

/** The line with new word timings: its own start and end follow its words. */
export function withWords(line: EdLine, words: EdWord[]): EdLine {
	const out: EdLine = { ...line, words, dirty: true };
	const first = words[0]?.start;
	const last = words[words.length - 1]?.end;
	if (first !== undefined) out.time = first;
	if (last !== undefined) out.end = last;
	return out;
}

/** The line pass: this line starts at `t`. */
export function setLineTime(line: EdLine, t: number): EdLine {
	const out: EdLine = { ...line, time: t, dirty: true };
	delete out.end;
	return out;
}

/** The line pass is owed while a line with words, or an instrumental break, has no start. */
export const needsLineTime = (line: EdLine): boolean =>
	(line.words.length > 0 || !!line.gap) && line.time === undefined;

// --- The loop ---------------------------------------------------------------------------------

/** Song ms played before the line starts and after it ends, so a tap is never on the seam. */
export const LEAD_IN_MS = 1200;
export const LEAD_OUT_MS = 600;
/** A line with nothing to end it (a break follows, or it is the last one). */
const OPEN_LINE_MS = 5000;
const LONGEST_LINE_MS = 15000;

export interface Range {
	start: number;
	end: number;
}

/** The line's own extent in song ms, or `null` before the line pass has given it a start. */
export function lineSpan(lines: EdLine[], i: number, durationMs: number): Range | null {
	const line = lines[i];
	const start = line?.time ?? line?.words[0]?.start;
	if (start === undefined) return null;
	const lastWord = line.words[line.words.length - 1]?.end ?? 0;
	let end = Math.max(line.end ?? 0, lastWord);
	if (end <= start) {
		const next = lines.slice(i + 1).find((l) => l.time !== undefined && l.time > start)?.time;
		end = next === undefined ? start + OPEN_LINE_MS : Math.min(next, start + LONGEST_LINE_MS);
	}
	if (durationMs > 0) end = Math.min(end, durationMs);
	return { start, end: Math.max(end, start + 1) };
}

/** What plays, over and over, while a line is being tapped. */
export function loopRange(lines: EdLine[], i: number, durationMs: number): Range | null {
	const span = lineSpan(lines, i, durationMs);
	if (!span) return null;
	const end = span.end + LEAD_OUT_MS;
	return { start: Math.max(0, span.start - LEAD_IN_MS), end: durationMs > 0 ? Math.min(end, durationMs) : end };
}

/** How long after a seek the position reports may still be the old ones. */
const SEEK_SETTLE_MS = 400;
/** How far before the loop the position may be before it is pulled back in (a seek lands a little
 *  early, on the frame before). */
const BEFORE_SLACK_MS = 500;

/** One step of the loop: seek back to the start, or look again in `wake` real ms. The player has
 *  no A-B loop the app can reach, so the page seeks; a wrap lands within a timer's precision of
 *  the end, which the lead-out absorbs. */
export function loopStep(
	range: Range,
	posMs: number,
	speed: number,
	msSinceSeek: number
): { seek: number } | { wake: number } {
	if (msSinceSeek < SEEK_SETTLE_MS) return { wake: SEEK_SETTLE_MS - msSinceSeek };
	if (posMs >= range.end || posMs < range.start - BEFORE_SLACK_MS) return { seek: range.start };
	return { wake: Math.min(250, Math.max(10, (range.end - posMs) / (speed || 1))) };
}

// --- Dragging a word's edge -------------------------------------------------------------------

/** The shortest a word may be dragged to. */
export const MIN_WORD_MS = 30;

/** Move one edge of word `k` to `t`. Words keep their order and never overlap:
 *  - a word's start moves between the start of the word before it and its own end, and the word
 *    before ends there if it was touching or would now overlap;
 *  - a word's end moves between its own start and the start of the word after it, which opens or
 *    closes a gap and moves nothing else. The last word's end is free up to `max`. */
export function moveEdge(words: EdWord[], k: number, side: 'start' | 'end', t: number, max: number): EdWord[] {
	const w = words[k];
	if (!w || w.start === undefined || w.end === undefined) return words;
	const prev = words[k - 1];
	const next = words[k + 1];
	const out = words.slice();
	if (side === 'start') {
		const lo = prev?.start !== undefined ? prev.start + MIN_WORD_MS : 0;
		const hi = w.end - MIN_WORD_MS;
		if (lo > hi) return words;
		const v = Math.round(Math.min(hi, Math.max(lo, t)));
		out[k] = { ...w, start: v };
		if (prev?.end !== undefined) {
			const touching = prev.end === w.start;
			out[k - 1] = { ...prev, end: touching ? v : Math.min(prev.end, v) };
		}
	} else {
		const lo = w.start + MIN_WORD_MS;
		const hi = next?.start ?? Math.max(max, lo);
		if (lo > hi) return words;
		out[k] = { ...w, end: Math.round(Math.min(hi, Math.max(lo, t))) };
	}
	return out;
}

// --- Submitting -------------------------------------------------------------------------------

const bytes = (s: string) => new TextEncoder().encode(s).length;
const MAX_LINES = 1000;
const MAX_LINE_BYTES = 500;
const MAX_WORDS = 100;
const MAX_WORD_BYTES = 100;

export interface Missing {
	/** No line has any text. */
	empty: boolean;
	/** Ids of lines whose words are not all timed, and of instrumental breaks with no start, in
	 *  order. */
	untimed: number[];
	/** Ids of lines the service would refuse for their size (text over 500 bytes, over 100 words,
	 *  a word over 100 bytes). */
	tooLong: number[];
	/** More than 1000 lines. */
	tooMany: boolean;
}

/** What stands between the draft and a submission. The service takes word-synced lyrics only, and
 *  the editor holds that to every sung line rather than the server's "most of them". */
export function missing(lines: EdLine[]): Missing {
	const sung = lines.filter((l) => l.words.length);
	return {
		empty: sung.length === 0,
		untimed: lines
			.filter((l) => (l.words.length ? !wordsTimed(l) : !!l.gap && l.time === undefined))
			.map((l) => l.id),
		tooLong: sung
			.filter(
				(l) =>
					l.words.length > MAX_WORDS ||
					bytes(lineText(l)) > MAX_LINE_BYTES ||
					l.words.some((w) => bytes(w.text) > MAX_WORD_BYTES)
			)
			.map((l) => l.id),
		tooMany: lines.length > MAX_LINES
	};
}

export const ready = (m: Missing): boolean =>
	!m.empty && !m.tooMany && m.untimed.length === 0 && m.tooLong.length === 0;

const wireWord = (w: LyricWord): LyricWord => {
	const out: LyricWord = { text: w.text, start_ms: w.start_ms, end_ms: w.end_ms };
	if (w.bg) out.bg = true;
	return out;
};

/** A loaded line as the service stores one: its fields and nothing else (a translation another
 *  provider added is not part of the canonical lyric). */
function canon(l: LyricLine): LyricLine {
	const out: LyricLine = { text: l.text, words: (l.words ?? []).map(wireWord) };
	if (l.time_ms !== undefined) out.time_ms = l.time_ms;
	if (l.end_time_ms !== undefined) out.end_time_ms = l.end_time_ms;
	if (validAgent(l.agent)) out.agent = l.agent;
	if (l.romanized) out.romanized = l.romanized;
	if (l.romanized_words?.length) out.romanized_words = l.romanized_words.map(wireWord);
	return out;
}

/** The lines to submit, sorted by start. An untouched line goes out exactly as it was loaded. An
 *  edited one is rebuilt from its words: the line spans them, the singer and background flags are
 *  carried, and the source's romanization stays only while the text is the text it was written
 *  for. An instrumental break goes out as a timed line with no text. Lines with nothing in them
 *  are left out. Call it once `ready(missing(lines))`. */
export function buildLines(lines: EdLine[]): LyricLine[] {
	const out: LyricLine[] = [];
	for (const l of lines) {
		if (l.orig && !l.dirty) {
			const c = canon(l.orig);
			if (c.time_ms === undefined && l.time !== undefined) c.time_ms = l.time;
			if (c.time_ms !== undefined) out.push(c);
			continue;
		}
		if (!l.words.length) {
			if (l.gap && l.time !== undefined) out.push({ time_ms: Math.round(l.time), text: '', words: [] });
			continue;
		}
		const words: LyricWord[] = [];
		let floor = 0;
		for (const w of l.words) {
			if (w.start === undefined || w.end === undefined) continue;
			const start = Math.max(Math.round(w.start), floor);
			floor = start;
			const word: LyricWord = { text: w.text, start_ms: start, end_ms: Math.max(Math.round(w.end), start) };
			if (w.bg) word.bg = true;
			words.push(word);
		}
		const first = words[0]?.start_ms;
		const time = Math.round(Math.min(l.time ?? first ?? 0, first ?? l.time ?? 0));
		const lastEnd = words.length ? Math.max(...words.map((w) => w.end_ms)) : time;
		const text = lineText(l);
		const line: LyricLine = {
			time_ms: time,
			end_time_ms: Math.max(Math.round(l.end ?? lastEnd), lastEnd, time),
			text,
			words
		};
		if (validAgent(l.agent)) line.agent = l.agent;
		if (l.orig && collapse(l.orig.text) === collapse(text)) {
			if (l.orig.romanized) line.romanized = l.orig.romanized;
			if (l.orig.romanized_words?.length) line.romanized_words = l.orig.romanized_words.map(wireWord);
		}
		out.push(line);
	}
	// Stable, so two lines on one instant keep the order they were written in.
	return out
		.map((l, i) => ({ l, i }))
		.sort((a, b) => a.l.time_ms! - b.l.time_ms! || a.i - b.i)
		.map((x) => x.l);
}

// --- Drafts -----------------------------------------------------------------------------------

export const draftKey = (videoId: string) => `lyrics_draft:${videoId}`;

export const packDraft = (d: Draft): string => JSON.stringify(d);

const isMs = (v: unknown): v is number | undefined =>
	v === undefined || (typeof v === 'number' && Number.isFinite(v) && v >= 0);

/** A stored draft, or `null` for anything that is not one for this song: storage is the user's to
 *  edit, and a draft the page cannot draw must not take the page down with it. */
export function unpackDraft(json: string | null, videoId: string): Draft | null {
	if (!json) return null;
	let d: Draft;
	try {
		d = JSON.parse(json);
	} catch {
		return null;
	}
	if (!d || d.v !== 1 || d.videoId !== videoId || !Array.isArray(d.lines)) return null;
	const ids = new Set<number>();
	for (const l of d.lines) {
		if (!l || !Number.isInteger(l.id) || ids.has(l.id) || !Array.isArray(l.words)) return null;
		ids.add(l.id);
		if (!isMs(l.time) || !isMs(l.end)) return null;
		if (l.agent !== undefined && typeof l.agent !== 'string') return null;
		if (l.gap !== undefined && typeof l.gap !== 'boolean') return null;
		for (const w of l.words) {
			if (!w || typeof w.text !== 'string' || !isMs(w.start) || !isMs(w.end)) return null;
		}
	}
	if (!d.lines.length) return null;
	return {
		v: 1,
		videoId,
		lines: d.lines,
		nextId: Math.max(Number.isInteger(d.nextId) ? d.nextId : 0, ...ids) + 1,
		baseRevision: typeof d.baseRevision === 'number' ? d.baseRevision : null,
		improving: d.improving === true
	};
}
