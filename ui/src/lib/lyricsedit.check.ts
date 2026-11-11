// Self-check for the lyric editor's logic in `lyricsedit.ts`. Same shape as `timecode.check.ts`:
//
//     node --experimental-strip-types ui/src/lib/lyricsedit.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant. Not imported by the app.
import {
	LEAD_IN_MS,
	LEAD_OUT_MS,
	MIN_WORD_MS,
	autoSplit,
	buildLines,
	draftKey,
	estimateMs,
	eventWallMs,
	fieldText,
	fromLyrics,
	fromText,
	joinLines,
	kindOf,
	lineSpan,
	lineText,
	loopRange,
	loopStep,
	missing,
	moveEdge,
	needsLineTime,
	packDraft,
	parseField,
	ready,
	resplitLine,
	setAgent,
	setLineText,
	setLineTime,
	splitLine,
	stampTap,
	toggleBg,
	unpackDraft,
	withWords,
	wordsFromTaps,
	wordsTimed,
	type Draft,
	type EdLine,
	type EdWord
} from './lyricsedit.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

// --- Words ------------------------------------------------------------------------------------
ok(same(parseField('I was dancing'), ['I ', 'was ', 'dancing']), 'a space ends a word');
ok(same(parseField('  I   was  '), ['I ', 'was']), 'extra spaces are not words');
ok(same(parseField('danc|ing in'), ['danc', 'ing ', 'in']), 'a | splits a word without a space');
ok(same(parseField('a|| b|'), ['a ', 'b']), 'stray | make no empty words');
ok(same(parseField(''), []), 'nothing is no words');
const w = (text: string, start?: number, end?: number): EdWord =>
	start === undefined ? { text } : { text, start, end };
ok(fieldText([w('danc'), w('ing '), w('in')]) === 'danc|ing in', 'the field shows the parts');
for (const f of ['I was danc|ing in the rain', '雨|の|中', 'one']) {
	ok(fieldText(parseField(f).map((t) => w(t))) === f, `the field round-trips: ${f}`);
}

ok(same(autoSplit('I was dancing'), ['I ', 'was ', 'dancing']), 'Latin text splits on spaces');
const ja = autoSplit('雨の中で踊っていた');
ok(ja.length > 1, 'a Japanese line is more than one word');
ok(ja.join('') === '雨の中で踊っていた', 'and loses nothing');
ok(ja.every((t) => !/\s/.test(t)), 'and gains no spaces');
const mixed = autoSplit('君の love、ずっと');
ok(mixed.join('') === '君の love、ずっと', 'mixed scripts keep their text and their spaces');
ok(mixed.includes('love、'), 'a Latin word stays whole, with the comma after it');
ok(autoSplit('สวัสดีครับ').join('') === 'สวัสดีครับ', 'Thai keeps its text');

// --- Starting points --------------------------------------------------------------------------
const wordLine = {
	time_ms: 1000,
	end_time_ms: 2500,
	text: 'Baby love',
	agent: 'v1',
	romanized: 'beibi',
	romanized_words: [{ text: 'beibi', start_ms: 1000, end_ms: 2500 }],
	translation: 'not ours',
	words: [
		{ text: 'Baby ', start_ms: 1000, end_ms: 1500 },
		{ text: 'love', start_ms: 1500, end_ms: 2500, bg: true }
	]
};
const src = [
	wordLine,
	{ time_ms: 3000, text: '', words: [] },
	{ time_ms: 4000, text: 'Line two here' },
	{ text: '' }
];
const ed = fromLyrics(src);
ok(ed.length === 3, 'a blank untimed line is dropped, a timed gap is kept');
ok(same(ed[0].words, [w('Baby ', 1000, 1500), { ...w('love', 1500, 2500), bg: true }]), 'words keep timings and bg');
ok(ed[0].agent === 'v1' && ed[0].time === 1000 && ed[0].end === 2500, 'the line keeps singer and extent');
ok(same(ed[2].words, [w('Line '), w('two '), w('here')]), 'a line without words is split, untimed');
ok(ed[2].time === 4000 && !wordsTimed(ed[2]) && wordsTimed(ed[0]), 'and keeps its line time');
ok(new Set(ed.map((l) => l.id)).size === 3, 'ids are unique');

const bare = fromLyrics([
	{
		time_ms: 0,
		text: 'Hello big world',
		words: [
			{ text: 'Hello', start_ms: 0, end_ms: 5 },
			{ text: 'big', start_ms: 5, end_ms: 9 },
			{ text: 'world', start_ms: 9, end_ms: 12 }
		]
	}
]);
ok(lineText(bare[0]) === 'Hello big world', 'bare words get their spaces back from the line text');
const syl = fromLyrics([
	{
		time_ms: 0,
		text: 'dancing in',
		words: [
			{ text: 'danc', start_ms: 0, end_ms: 5 },
			{ text: 'ing ', start_ms: 5, end_ms: 9 },
			{ text: 'in', start_ms: 9, end_ms: 12 }
		]
	}
]);
ok(same(syl[0].words.map((x) => x.text), ['danc', 'ing ', 'in']), 'words that already agree with the text are left alone');

const pasted = fromText('First line\r\n\r\n  Second line  \n');
ok(pasted.length === 2 && lineText(pasted[1]) === 'Second line', 'pasted text: one line each, blanks dropped');
ok(kindOf(pasted) === 'plain' && kindOf(ed) === 'synced' && kindOf([ed[0], ed[1]]) === 'words', 'kinds');

// --- Text edits -------------------------------------------------------------------------------
ok(setLineText(ed[0], 'Baby love') === ed[0], 'an unchanged field changes nothing');
const typo = setLineText(ed[0], 'Babe love');
ok(typo.dirty === true && typo.words[0].start === 1000 && typo.words[1].bg === true, 'a fixed word keeps every timing');
const longer = setLineText(ed[0], 'Baby my love');
ok(longer.words.length === 3 && !wordsTimed(longer) && longer.time === 1000, 'a new word count is tapped again');
const parts = setLineText(ed[0], 'Ba|by love');
ok(same(parts.words.map((x) => x.text), ['Ba', 'by ', 'love']), 'a | makes parts');
ok(same(resplitLine(parts).words.map((x) => x.text), ['Baby ', 'love']), 'resplit drops them');

const four: EdLine = {
	id: 7,
	time: 100,
	end: 900,
	agent: 'v2',
	orig: { text: 'a b c d' },
	words: [w('a ', 100, 200), w('b ', 200, 400), w('c ', 400, 600), w('d', 600, 900)]
};
const [s1, s2] = splitLine(four, fieldText(four.words), 'a b '.length, 8);
ok(same(s1.words, [w('a ', 100, 200), w('b', 200, 400)]), 'a split between words keeps the first half');
ok(same(s2.words, [w('c ', 400, 600), w('d', 600, 900)]), 'and the second');
ok(s1.id === 7 && s2.id === 8 && s1.time === 100 && s1.end === 400 && s2.time === 400 && s2.end === 900, 'each half spans its words');
ok(s1.agent === 'v2' && s2.agent === 'v2' && !s1.orig && !s2.orig, 'both halves keep the singer and are new lines');
const [m1, m2] = splitLine(four, 'a b c d', 'a b c'.length - 1 + 1, 9);
ok(lineText(m1) === 'a b c' && lineText(m2) === 'd' && wordsTimed(m1), 'a caret after a word is still between words');
const [x1, x2] = splitLine({ id: 1, words: [w('hello ', 0, 5), w('there', 5, 9)] }, 'hello there', 3, 2);
ok(lineText(x1) === 'hel' && lineText(x2) === 'lo there' && !wordsTimed(x1) && !wordsTimed(x2), 'a split inside a word makes new words');
const joined = joinLines(s1, s2);
ok(same(joined.words, four.words) && joined.time === 100 && joined.end === 900, 'join undoes split, timings kept');
ok(lineText(joinLines({ id: 1, words: [w('a')] }, { id: 2, words: [] })) === 'a', 'joining an empty line adds no space');

ok(setAgent(four, 'v2') === four, 'the same singer changes nothing');
ok(setAgent(four, undefined).agent === undefined && setAgent(four, 'v1').dirty === true, 'singer set and cleared');
const bgd = toggleBg(four, 1);
ok(bgd.words[1].bg === true && toggleBg(bgd, 1).words[1].bg === undefined && bgd.dirty === true, 'bg toggles');

// --- The clock --------------------------------------------------------------------------------
ok(eventWallMs(10_000, 500, 488) === 9_988, 'the handler ran 12 ms after the key');
ok(eventWallMs(10_000, 500, 600) === 10_000, 'a stamp in the future is ignored');
ok(eventWallMs(10_000, 500, 100) === 10_000, 'so is one from another clock');

const clock = (position: number, atMs: number, speed = 1, paused = false) => ({ position, atMs, speed, paused });
ok(stampTap(clock(12.5, 1000), 1000, 0) === 12_500, 'read at the tap: the position itself');
ok(stampTap(clock(12.5, 1040), 1000, 0) === 12_460, 'read 40 ms after the tap: 40 ms earlier in the song');
ok(stampTap(clock(12.5, 1040, 0.75), 1000, 0) === 12_470, 'at 0.75x those 40 ms are 30 ms of song');
ok(stampTap(clock(12.5, 1040, 0.5), 1000, 0) === 12_480, 'at 0.5x, 20 ms');
ok(stampTap(clock(12.5, 990), 1000, 0) === 12_510, 'a read from before the tap goes forward');
ok(stampTap(clock(12.5, 1000), 1000, -80) === 12_420, 'the tap offset moves every tap');
ok(stampTap(clock(12.5, 1000, 0.5), 1000, -80) === 12_460, 'and is real time, so it scales with the tempo');
ok(stampTap(clock(0.01, 1000), 1000, -80) === 0, 'never before the song starts');
ok(stampTap(clock(12.5, 1000, 1, true), 1000, 0) === null, 'a paused song stamps nothing');
// The same tap, however late the read: within a millisecond at every tempo.
for (const speed of [0.5, 0.75, 1, 1.5]) {
	const at = (delay: number) => stampTap(clock(20 + (delay * speed) / 1000, 5000 + delay, speed), 5000, 0)!;
	ok(Math.abs(at(3) - at(180)) <= 1, `the read's latency cancels out at ${speed}x`);
}

ok(estimateMs(10, 1000, 1200, 0.75, false) === 10_150, 'the playhead runs on from the last report');
ok(estimateMs(10, 1000, 1200, 0.75, true) === 10_000, 'and holds while paused');

// --- Taps -------------------------------------------------------------------------------------
const three = [w('I '), w('was '), { ...w('here'), bg: true }];
ok(wordsFromTaps(three, [100, 300], 2000) === null, 'a pass that stops halfway is nothing');
ok(wordsFromTaps(three, [100, undefined, 500], 2000) === null, 'and so is one with a hole');
ok(wordsFromTaps([], [], 0) === null, 'no words, no take');
const take = wordsFromTaps(three, [100, 300, 500], 2000)!;
ok(same(take.map((x) => [x.start, x.end]), [[100, 300], [300, 500], [500, 2000]]), 'each tap starts a word and ends the one before');
ok(take[2].bg === true && take[2].text === 'here', 'text and bg are kept');
ok(wordsFromTaps(three, [100, 300, 500, 800], 2000)![2].end === 800, 'one more tap ends the last word');
ok(wordsFromTaps(three, [100, 300, 500], 9000)![2].end === 3000, 'a far line end does not stretch the last word');
ok(wordsFromTaps(three, [100, 300, 500], undefined)![2].end === 700, 'no line end: a short last word');
ok(wordsFromTaps(three, [100, 300, 500], 400)![2].end === 700, 'a line end before the last tap is ignored');
const crooked = wordsFromTaps(three, [300, 100, 500, 400], 2000)!;
ok(same(crooked.map((x) => [x.start, x.end]), [[300, 300], [300, 500], [500, 500]]), 'taps never go backwards');

const retimed = withWords(four, take);
ok(retimed.time === 100 && retimed.end === 2000 && retimed.dirty === true, 'the line follows its words');
const untimedLine: EdLine = { id: 3, words: [w('a')] };
ok(needsLineTime(untimedLine) && !needsLineTime({ id: 4, words: [] }), 'the line pass is owed to lines with words');
const lt = setLineTime({ ...untimedLine, end: 5 }, 4200);
ok(lt.time === 4200 && lt.end === undefined && !needsLineTime(lt), 'a line tap sets the start and drops a stale end');

// --- The loop ---------------------------------------------------------------------------------
const song: EdLine[] = [
	{ id: 1, time: 10_000, words: [w('a')] },
	{ id: 2, time: 13_000, words: [w('b ', 13_100, 13_400), w('c', 13_400, 14_800)] },
	{ id: 3, words: [w('untimed')] },
	{ id: 4, time: 60_000, words: [w('d')] },
	{ id: 5, time: 200_000, end: 203_000, words: [w('e')] }
];
ok(same(lineSpan(song, 0, 210_000), { start: 10_000, end: 13_000 }), 'a line runs to the next one');
ok(same(lineSpan(song, 1, 210_000), { start: 13_000, end: 14_800 }), 'or to its last word');
ok(lineSpan(song, 2, 210_000) === null && loopRange(song, 2, 210_000) === null, 'no start, no loop');
ok(same(lineSpan(song, 3, 210_000), { start: 60_000, end: 75_000 }), 'a break after it does not stretch the line');
ok(same(lineSpan(song, 4, 210_000), { start: 200_000, end: 203_000 }), 'an end of its own wins');
ok(same(lineSpan([song[0]], 0, 0), { start: 10_000, end: 15_000 }), 'the last line gets a few seconds');
ok(same(lineSpan([song[0]], 0, 12_000), { start: 10_000, end: 12_000 }), 'but not past the end of the song');
ok(same(loopRange(song, 1, 210_000), { start: 13_000 - LEAD_IN_MS, end: 14_800 + LEAD_OUT_MS }), 'the loop adds a lead-in and a lead-out');
ok(loopRange([{ id: 1, time: 300, words: [w('a')] }], 0, 0)!.start === 0, 'never before zero');
ok(loopRange(song, 4, 203_200)!.end === 203_200, 'never past the end');

const range = { start: 1000, end: 5000 };
ok(same(loopStep(range, 5000, 1, 9999), { seek: 1000 }), 'at the end: back to the start');
ok(same(loopStep(range, 7000, 1, 9999), { seek: 1000 }), 'past it: back');
ok(same(loopStep(range, 100, 1, 9999), { seek: 1000 }), 'well before it: in');
ok('wake' in loopStep(range, 600, 1, 9999), 'a seek that landed a little early is left alone');
ok(same(loopStep(range, 4900, 1, 9999), { wake: 100 }), 'near the end: look again when it gets there');
ok(same(loopStep(range, 4900, 0.5, 9999), { wake: 200 }), 'which is later at half speed');
ok(same(loopStep(range, 2000, 1, 9999), { wake: 250 }), 'far from it: no long sleeps, the tempo may change');
ok(same(loopStep(range, 4999, 1, 9999), { wake: 10 }), 'and no busy loop');
ok(same(loopStep(range, 9000, 1, 150), { wake: 250 }), 'right after a seek the old position is not believed');
// Driven by a clock: it wraps once per pass and never seeks twice for one wrap.
{
	let pos = 1000;
	let sinceSeek = 9999;
	let seeks = 0;
	for (let t = 0; t < 20_000; ) {
		const step = loopStep(range, pos, 0.75, sinceSeek);
		if ('seek' in step) {
			seeks++;
			sinceSeek = 0;
			// The seek takes 120 ms to land; until then the reports are the old position.
			t += 120;
			sinceSeek += 120;
			pos = step.seek;
			continue;
		}
		t += step.wake;
		sinceSeek += step.wake;
		pos += step.wake * 0.75;
		ok(pos < range.end + 250 * 0.75 + 1, 'the loop never runs far past its end');
	}
	// A 4 s range at 0.75x is 5.33 s a pass (plus the landing): three wraps in 20 s.
	ok(seeks === 3, `one seek per pass, got ${seeks}`);
}

// --- Dragging a word's edge -------------------------------------------------------------------
const row = [w('a ', 100, 200), w('b ', 200, 400), w('c', 500, 700)];
const d1 = moveEdge(row, 1, 'start', 150, 9999);
ok(d1[1].start === 150 && d1[0].end === 150, 'touching words share the boundary');
const d2 = moveEdge(row, 1, 'start', 260, 9999);
ok(d2[1].start === 260 && d2[0].end === 260, 'in both directions');
ok(moveEdge(row, 1, 'start', 0, 9999)[1].start === 100 + MIN_WORD_MS, 'a start stops at the word before it');
ok(moveEdge(row, 1, 'start', 9999, 9999)[1].start === 400 - MIN_WORD_MS, 'and before its own end');
const d3 = moveEdge(row, 2, 'start', 450, 9999);
ok(d3[2].start === 450 && d3[1].end === 400, 'across a gap the word before is left alone');
const d4 = moveEdge(row, 2, 'start', 300, 9999);
ok(d4[2].start === 300 && d4[1].end === 300, 'until it would overlap');
const d5 = moveEdge(row, 0, 'end', 150, 9999);
ok(d5[0].end === 150 && d5[1].start === 200, 'an end opens a gap and moves nothing else');
ok(moveEdge(row, 0, 'end', 900, 9999)[0].end === 200, 'and stops at the next word');
ok(moveEdge(row, 1, 'end', 480, 9999)[1].end === 480, 'or closes a gap');
ok(moveEdge(row, 2, 'end', 5000, 3000)[2].end === 3000, 'the last word ends by the end of the song');
ok(moveEdge(row, 2, 'end', 0, 3000)[2].end === 500 + MIN_WORD_MS, 'and after its own start');
ok(moveEdge(row, 0, 'start', -50, 9999)[0].start === 0, 'the first word starts at zero or later');
ok(moveEdge(row, 1, 'start', 150.6, 9999)[1].start === 151, 'whole milliseconds');
ok(moveEdge([w('a')], 0, 'start', 5, 9999)[0].start === undefined, 'an untimed word has no edge');
ok(moveEdge(row, 9, 'end', 5, 9999) === row, 'no such word');
// Whatever is dragged wherever, order and non-overlap hold.
{
	let words = row;
	let seed = 7;
	const rnd = () => (seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648;
	for (let i = 0; i < 2000; i++) {
		words = moveEdge(words, Math.floor(rnd() * 3), rnd() < 0.5 ? 'start' : 'end', rnd() * 1200 - 100, 1000);
		for (let k = 0; k < words.length; k++) {
			const cur = words[k];
			ok(cur.start! >= 0 && cur.start! < cur.end!, 'a word never ends before it starts');
			if (k) ok(words[k - 1].end! <= cur.start! && words[k - 1].start! < cur.start!, 'words never overlap or swap');
		}
		ok(words[2].end! <= 1000, 'nothing ends past the limit');
	}
}

// --- Submitting -------------------------------------------------------------------------------
const m = missing(ed);
ok(!m.empty && same(m.untimed, [ed[2].id]) && !ready(m), 'the line without word timings is what is missing');
ok(missing([]).empty && missing([{ id: 1, words: [] }]).empty && !ready(missing([])), 'nothing to submit');
ok(ready(missing([ed[0], ed[1]])), 'every sung line timed: ready (a gap needs nothing)');
const big: EdLine = { id: 1, words: Array.from({ length: 101 }, (_, i) => w('la ', i, i + 1)) };
ok(same(missing([big]).tooLong, [1]), 'over 100 words');
ok(same(missing([{ id: 2, words: [w('é'.repeat(51), 0, 1)] }]).tooLong, [2]), 'a word over 100 bytes, not characters');
ok(same(missing([{ id: 3, words: Array.from({ length: 60 }, () => w('abcdefgh ', 0, 1)) }]).tooLong, [3]), 'a line over 500 bytes');
ok(missing(Array.from({ length: 1001 }, (_, i) => ({ id: i, words: [] }))).tooMany, 'over 1000 lines');

// Untouched lines go out as they were loaded, minus what is not the service's.
const built = buildLines(ed.slice(0, 2));
ok(
	same(built[0], {
		text: 'Baby love',
		words: wordLine.words,
		time_ms: 1000,
		end_time_ms: 2500,
		agent: 'v1',
		romanized: 'beibi',
		romanized_words: wordLine.romanized_words
	}),
	'an untouched line is passed through, without the translation'
);
ok(same(built[1], { text: '', words: [], time_ms: 3000 }), 'a gap keeps its place, with an empty word list');

// A retimed line: rebuilt, romanization kept because the text is the same.
const re = withWords(ed[0], moveEdge(ed[0].words, 1, 'start', 1400, 9999));
const rb = buildLines([re])[0];
ok(rb.time_ms === 1000 && rb.end_time_ms === 2500 && rb.agent === 'v1', 'a retimed line keeps singer and extent');
ok(same(rb.words, [{ text: 'Baby ', start_ms: 1000, end_ms: 1400 }, { text: 'love', start_ms: 1400, end_ms: 2500, bg: true }]), 'its words carry the new times and bg');
ok(rb.romanized === 'beibi' && same(rb.romanized_words, wordLine.romanized_words), 'romanization stays with unchanged text');
ok(!('translation' in rb), 'no translation');
// A reworded line loses the romanization written for the old text.
const rw = buildLines([setLineText(ed[0], 'Babe love')])[0];
ok(rw.text === 'Babe love' && rw.romanized === undefined && rw.romanized_words === undefined, 'romanization goes with changed text');
ok(buildLines([setAgent(ed[0], 'v1000')])[0].agent === 'v1000', 'a new singer is carried');
ok(buildLines([{ ...re, agent: 'voice1' }])[0].agent === undefined, 'a singer id the service would refuse is dropped');
ok(buildLines([setAgent(ed[0], undefined)])[0].agent === undefined, 'a cleared singer is not sent');

// Sorted by start, words inside their line, whole numbers, empty lines left out.
const scrambled: EdLine[] = [
	{ id: 1, time: 5000, end: 5100, dirty: true, words: [w('late ', 5000.4, 5200.6), w('line', 5200.6, 5900.2)] },
	{ id: 2, dirty: true, words: [] },
	{ id: 3, time: 2000, dirty: true, words: [w('early', 1900, 2400)] },
	{ id: 4, time: 5000, dirty: true, words: [w('same', 5000, 5001)] }
];
const sorted = buildLines(scrambled);
ok(same(sorted.map((l) => l.text), ['early', 'late line', 'same']), 'sorted by start, ties in written order, empties dropped');
ok(sorted[0].time_ms === 1900 && sorted[0].end_time_ms === 2400, 'a line starts no later than its first word');
ok(sorted[1].end_time_ms === 5900 && sorted[1].words![0].end_ms === 5201, 'ends no earlier than its last, in whole ms');
for (const l of sorted) {
	ok(Number.isInteger(l.time_ms) && Number.isInteger(l.end_time_ms), 'line times are integers');
	let prev = 0;
	for (const x of l.words!) {
		ok(Number.isInteger(x.start_ms) && Number.isInteger(x.end_ms) && x.start_ms <= x.end_ms && x.start_ms >= prev, 'word times are ordered integers');
		ok(x.start_ms >= l.time_ms! && x.end_ms <= l.end_time_ms!, 'words sit inside their line');
		prev = x.start_ms;
	}
}
// A line loaded without a line time takes the one it was given since.
ok(buildLines([{ id: 1, time: 700, orig: { text: 'x', words: [{ text: 'x', start_ms: 700, end_ms: 900 }] }, words: [w('x', 700, 900)] }])[0].time_ms === 700, 'a loaded line with no time of its own gets one');

// --- Drafts -----------------------------------------------------------------------------------
const draft: Draft = { v: 1, videoId: 'FkxEE7wZ83c', lines: [re, ed[1], ed[2]], nextId: 9, baseRevision: 12, improving: true };
const back = unpackDraft(packDraft(draft), 'FkxEE7wZ83c')!;
ok(same(back.lines, draft.lines) && back.baseRevision === 12 && back.improving, 'a draft comes back as it was saved');
ok(same(buildLines(back.lines), buildLines(draft.lines)), 'and builds the same lyric');
ok(back.nextId > Math.max(...draft.lines.map((l) => l.id)), 'new lines never reuse an id');
ok(unpackDraft(packDraft(draft), 'otherVideo00') === null, "another song's draft is not this one's");
ok(unpackDraft(null, 'a') === null && unpackDraft('', 'a') === null && unpackDraft('{', 'a') === null, 'nothing and junk');
const broken = (patch: (d: any) => void) => {
	const d = JSON.parse(packDraft(draft));
	patch(d);
	return unpackDraft(JSON.stringify(d), 'FkxEE7wZ83c');
};
ok(broken((d) => (d.v = 2)) === null, 'a version this build does not know');
ok(broken((d) => (d.lines = [])) === null, 'no lines');
ok(broken((d) => (d.lines[0].words = 'x')) === null, 'words that are not a list');
ok(broken((d) => (d.lines[0].words[0].start = -5)) === null, 'a negative time');
ok(broken((d) => (d.lines[0].words[0].text = 5)) === null, 'text that is not text');
ok(broken((d) => (d.lines[1].id = d.lines[0].id)) === null, 'two lines with one id');
ok(broken((d) => delete d.baseRevision)!.baseRevision === null, 'a missing base is no base');
ok(draftKey('abc') === 'lyrics_draft:abc', 'one key per song');

console.log('ok');
