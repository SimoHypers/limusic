// Self-check for the helpers in `timecode.ts`. Same shape as `ytlink.check.ts`:
//
//     node --experimental-strip-types ui/src/lib/timecode.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant. Not imported by the app.
import { formatClock, parseClock, parseYtStart, splitTimecodes } from './timecode.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

// --- formatClock --------------------------------------------------------------------------------
ok(formatClock(0) === '0:00', 'zero');
ok(formatClock(5) === '0:05', 'seconds are padded');
ok(formatClock(83.9) === '1:23', 'fractions floor, never round up into the next second');
ok(formatClock(3600) === '1:00:00', 'an hour switches to h:mm:ss');
ok(formatClock(3723) === '1:02:03', 'h:mm:ss pads minutes too');
ok(formatClock(-4) === '0:00' && formatClock(NaN) === '0:00', 'junk reads as zero');

// --- parseClock ---------------------------------------------------------------------------------
ok(parseClock('83') === 83, 'bare seconds');
ok(parseClock('1:23') === 83, 'm:ss');
ok(parseClock('01:23') === 83, 'leading zero');
ok(parseClock('1:02:03') === 3723, 'h:mm:ss');
ok(parseClock(' 1:23 ') === 83, 'whitespace is trimmed');
ok(parseClock('120') === 120, 'bare seconds may exceed 59');
ok(parseClock('1:75') === null, 'seconds above 59 next to minutes are a typo');
ok(parseClock('1:02:75') === null && parseClock('1:75:00') === null, 'same for h:mm:ss');
ok(parseClock('') === null && parseClock('abc') === null, 'empty and text');
ok(parseClock('1:') === null && parseClock(':30') === null, 'half a clock');
ok(parseClock('-5') === null && parseClock('1.5') === null, 'signs and fractions');
ok(parseClock('1:2:3:4') === null, 'too many parts');

// --- parseYtStart -------------------------------------------------------------------------------
ok(parseYtStart('83') === 83, 't=83');
ok(parseYtStart('83s') === 83, 't=83s');
ok(parseYtStart('1m23s') === 83, 't=1m23s');
ok(parseYtStart('1h2m3s') === 3723, 't=1h2m3s');
ok(parseYtStart('2m') === 120, 'a lone unit');
ok(parseYtStart('1M23S') === 83, 'case-insensitive');
ok(parseYtStart('') === null && parseYtStart(null) === null && parseYtStart(undefined) === null, 'absent');
ok(parseYtStart('abc') === null && parseYtStart('1x') === null, 'garbage');
ok(parseYtStart('s') === null && parseYtStart('hms') === null, 'units with no numbers');
ok(parseYtStart('-5') === null, 'negative');

// --- splitTimecodes -----------------------------------------------------------------------------
const times = (text: string, max = 0) =>
	splitTimecodes(text, max)
		.filter((s) => s.secs !== undefined)
		.map((s) => `${s.text}=${s.secs}`)
		.join(',');
const joined = (text: string, max = 0) =>
	splitTimecodes(text, max)
		.map((s) => s.text)
		.join('');

ok(times('the drop at 1:23 is great') === '1:23=83', 'a timecode in a sentence');
ok(times('1:23') === '1:23=83', 'a comment that is only a timecode');
ok(times('at 1:02:03!') === '1:02:03=3723', 'h:mm:ss, punctuation after');
ok(times('0:30 intro, 1:45 verse, 3:10 outro') === '0:30=30,1:45=105,3:10=190', 'a tracklist');
ok(times('1:23-2:34') === '1:23=83,2:34=154', 'a range');
ok(times('(1:23)') === '1:23=83', 'in parentheses');
ok(times('1:23\n2:34') === '1:23=83,2:34=154', 'across lines');
ok(times('75:30') === '75:30=4530', 'minutes past 59 without an hour');
ok(times('no timecodes here') === '' && times('') === '', 'none');
ok(times('lunch at 12:30pm') === '', 'a clock time with a suffix');
ok(times('score 3:2') === '' && times('ratio 16:9') === '', 'not a time');
ok(times('v1:23abc') === '' && times('x1:23') === '', 'glued to a word');
ok(times('1.5:30') === '', 'a decimal before it');
ok(times('1:23:45:67') === '', 'too many parts');
ok(times('1:75') === '' && times('1:2:03') === '' && times('1:75:03') === '', 'impossible times');
ok(times('at 5:00 and 12:30', 200) === '', 'maxSecs filters every match past the end');
ok(times('at 2:00 and 12:30', 200) === '2:00=120', 'past the end stays text');
ok(times('at 3:20', 200) === '3:20=200', 'exactly the length is allowed');
ok(times('12:30', 0) === '12:30=750', 'unknown length filters nothing');

for (const text of ['a 1:23 b 2:34 c', '1:23', 'x', '', '1:75 and 0:05', '  1:23  \n 2:34\n', '12:30pm 1:23']) {
	ok(joined(text) === text, `spans join back to ${JSON.stringify(text)}`);
	ok(joined(text, 100) === text, `and with a length: ${JSON.stringify(text)}`);
}
ok(splitTimecodes('a 1:23 b').length === 3, 'text, timecode, text');
ok(splitTimecodes('plain').length === 1, 'plain text is one span');

console.log('ok');
