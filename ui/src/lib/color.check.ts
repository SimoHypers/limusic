// Self-check for the pure colour maths in `color.ts`. No test runner in `ui/` (see
// personal.check.ts) — node 22 runs TypeScript directly:
//
//     node --experimental-strip-types ui/src/lib/color.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant.
import { hexToHsv, hexToOklchHue, hsvToHex, isLight } from './color.ts';

const eq = (a: unknown, b: unknown, msg: string) => {
	if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`${msg}: ${a} !== ${b}`);
};

// Round-trip: every hex the picker can emit must survive hex -> HSV -> hex unchanged, or dragging
// the square would drift the colour on each pass.
for (const hex of ['#000000', '#ffffff', '#6366f1', '#a3e635', '#ff0000', '#00ff80', '#123456']) {
	eq(hsvToHex(hexToHsv(hex)!), hex, `round-trip ${hex}`);
}

// Shorthand and a missing '#' are both accepted (users paste both).
eq(hsvToHex(hexToHsv('fff')!), '#ffffff', 'shorthand');
eq(hsvToHex(hexToHsv('#F00')!), '#ff0000', 'shorthand + case');

// Garbage is rejected rather than silently becoming black.
eq(hexToHsv('nope'), null, 'invalid');
eq(hexToHsv('#12345'), null, 'wrong length');

// Hue is preserved on greys only as a formality — saturation 0 means it can't matter.
eq(hexToHsv('#808080')!.s, 0, 'grey has no saturation');

// Out-of-range HSV clamps instead of wrapping to a wrong colour.
eq(hsvToHex({ h: 0, s: 2, v: 2 }), '#ff0000', 'clamps');

// Foreground picks: the thresholds the existing preset themes already use by hand.
eq(isLight('#a3e635'), true, 'lime wants dark text');
eq(isLight('#5eead4'), true, 'teal wants dark text');
eq(isLight('#6366f1'), false, 'indigo wants light text');
eq(isLight('#000000'), false, 'black wants light text');
eq(isLight('#ffffff'), true, 'white wants dark text');
eq(isLight('garbage'), false, 'invalid falls back to light text');

eq(Math.round(hexToOklchHue('#6750A4')!), 294, 'Material baseline purple OKLCH hue');
eq(hexToOklchHue('#808080'), null, 'achromatic grey has no OKLCH hue');

console.log('ok');
