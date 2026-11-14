// Self-check for `lyricmotion.ts`: `node --experimental-strip-types ui/src/lib/lyricmotion.check.ts`
import { glowing, letters, scrollMs, sweep } from './lyricmotion.ts';

function ok(cond: boolean, what: string) {
	if (!cond) throw new Error(`broken: ${what}`);
}

const w = { text: 'glow ', start_ms: 1000, end_ms: 1400 };
ok(sweep(w, 0) === sweep(w, 999), 'before its time a word is one string, so it is not rewritten');
ok(sweep(w, 1400) === sweep(w, 9000), 'after its time too');
ok(sweep(w, 1000) === sweep(w, 0), 'at its start nothing is lit yet');
ok(sweep(w, 1200).includes('calc(50.00% - 0.250em)'), 'half way, the edge straddles the middle');
ok(sweep(w, 1200).includes('calc(50.00% + 0.250em)'), 'and is half an em wide');
ok(sweep({ ...w, end_ms: 1000 }, 1000) === sweep(w, 9000), 'a word with no length is lit once reached');

const note = { text: 'night', start_ms: 0, end_ms: 1500 };
const last = { text: 't', start_ms: 1200, end_ms: 1500 };
ok(!glowing(last, note, 1199), 'a letter not reached has no glow');
ok(glowing(last, note, 1200) && glowing(last, note, 1649), 'glows once reached, a moment past the note');
ok(glowing({ ...last, start_ms: 0, end_ms: 300 }, note, 1400), 'a first letter holds its glow to the end');
ok(!glowing(last, note, 1650), 'then the whole note lets go');

ok(letters(w) === null, 'a short word lights whole');
const held = letters({ text: 'night ', start_ms: 0, end_ms: 1500 })!;
ok(held.map((l) => l.text).join('') === 'night', 'a held word splits into its letters, gap dropped');
ok(held[0].start_ms === 0 && held[4].end_ms === 1500, 'the letters share the word time end to end');
ok(held[1].start_ms === held[0].end_ms, 'with no holes between them');
ok(letters({ text: 'café', start_ms: 0, end_ms: 2000 })!.length === 4, 'an accent stays on its letter');
ok(letters({ text: 'नमस्ते', start_ms: 0, end_ms: 2000 }) === null, 'a joined script lights whole');
ok(letters({ text: 'a', start_ms: 0, end_ms: 2000 }) === null, 'one letter has nothing to split');
ok(letters({ text: 'supercalifragilistic', start_ms: 0, end_ms: 3000 }) === null, 'too many letters');

ok(scrollMs(0, false) === scrollMs(0, true), 'the line scrolled to has one time');
ok(scrollMs(-1, true) < scrollMs(0, false), 'a line ahead of the motion arrives first');
ok(scrollMs(1, false) > scrollMs(0, false), 'a trailing line arrives later');
ok(scrollMs(5, false) > scrollMs(2, false), 'and later the further back it is');
ok(scrollMs(-9, true) === scrollMs(-6, true), 'the lead stops growing');
console.log('ok');
