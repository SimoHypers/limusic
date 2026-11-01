// Self-check for the helpers in `timecode.ts`. Same shape as `ytlink.check.ts`:
//
//     node --experimental-strip-types ui/src/lib/timecode.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant. Not imported by the app.
import { formatClock, parseClock, parseYtStart } from './timecode.ts';

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

console.log('ok');
