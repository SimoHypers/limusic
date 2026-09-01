// One job: turn the playing track's cover into an accent colour the rest of the theme can use.
// Used only when "Adapt colors to artwork" is on (theme.svelte.ts).
//
// The picking is deliberately crude — 32x32, buckets, one winner. A cover has one or two colours
// that read as "its" colour, and a 3-bit-per-channel histogram finds them; anything smarter
// (k-means, palette libraries) is a dependency and a frame budget for a result nobody can tell
// apart at accent size.

import { brightness, hexToHsv, hsvToHex } from './color.ts';

const SIZE = 32;

/** Cover URL -> accent (or `null` for "this cover has no colour"). */
const cache = new Map<string, string | null>();

// How bright an accent has to be, per theme mode, as perceived brightness (`color.ts`). One band
// cannot serve both: what reads on a light page is mud on a dark one, which is what #137 reported.
// Brightness, not HSV value, because value is hue-blind: v=0.9 yellow and v=0.9 blue are nowhere
// near each other on screen, and the accent is text (`text-primary`) as often as it is a fill.
const TARGET_DARK = 0.65;
const TARGET_LIGHT = 0.39;
// Saturation still has to read as a colour without turning neon. Only a floor and a ceiling: what
// the colour actually needs is decided against the brightness target below.
const S_MIN = 0.45;
const S_MAX = 0.85;

/**
 * The cover's colour pulled to the brightness the current theme needs. Value alone gets there for
 * most hues; when it can't (a deep blue is dark even at full value, a yellow is bright even at low
 * one) saturation gives up exactly as much as it takes and no more.
 *
 * Also decides the text that sits ON the accent, for free: `isLight` uses the same brightness, so a
 * dark-theme accent lands above its threshold and a light-theme one below it.
 */
export function toAccent(hex: string, dark: boolean): string {
	const hsv = hexToHsv(hex);
	if (!hsv) return hex;
	const target = dark ? TARGET_DARK : TARGET_LIGHT;
	const at = (s: number, v: number) => brightness(hsvToHex({ h: hsv.h, s, v }));
	let s = Math.min(S_MAX, Math.max(S_MIN, hsv.s));
	if (at(s, 1) < target) {
		// Too dark to reach even at full value. s=0 is white, which is always bright enough, so the
		// answer is somewhere below the current saturation: bisect for the most colour we can keep.
		let lo = 0;
		let hi = s;
		for (let i = 0; i < 12; i++) {
			const mid = (lo + hi) / 2;
			if (at(mid, 1) < target) hi = mid;
			else lo = mid;
		}
		s = lo;
	}
	// Brightness is linear in value at a fixed hue and saturation, so the rest is one division.
	const full = at(s, 1);
	return hsvToHex({ h: hsv.h, s, v: full ? Math.min(1, target / full) : 1 });
}

/**
 * Winning colour of an RGBA buffer, or `null` when the artwork has no colour worth taking (a
 * greyscale cover would otherwise have a hue invented for it out of JPEG noise). Returned raw:
 * `toAccent` does the theme-dependent part, so a cover decoded in dark mode is still right after
 * the user flips to light.
 */
export function pickAccent(data: Uint8ClampedArray): string | null {
	// key = 3 bits per channel. score favours saturated, mid-value pixels: the near-black and
	// near-white that dominate most covers must not win just by area.
	const buckets = new Map<number, { n: number; r: number; g: number; b: number; score: number }>();
	for (let i = 0; i < data.length; i += 4) {
		if (data[i + 3] < 128) continue; // transparent
		const [r, g, b] = [data[i], data[i + 1], data[i + 2]];
		const max = Math.max(r, g, b) / 255;
		const min = Math.min(r, g, b) / 255;
		const sat = max ? (max - min) / max : 0;
		const score = sat * (1 - Math.abs(max - 0.65));
		const key = ((r >> 5) << 6) | ((g >> 5) << 3) | (b >> 5);
		const cur = buckets.get(key) ?? { n: 0, r: 0, g: 0, b: 0, score: 0 };
		buckets.set(key, {
			n: cur.n + 1,
			r: cur.r + r,
			g: cur.g + g,
			b: cur.b + b,
			score: cur.score + score
		});
	}
	let best: { n: number; r: number; g: number; b: number; score: number } | null = null;
	for (const bucket of buckets.values()) if (!best || bucket.score > best.score) best = bucket;
	if (!best) return null;

	const hex =
		'#' +
		[best.r, best.g, best.b]
			.map((c) => Math.round(c / best!.n).toString(16).padStart(2, '0'))
			.join('');
	const hsv = hexToHsv(hex);
	if (!hsv || hsv.s < 0.15) return null; // greyscale cover: leave the user's theme alone
	return hex;
}

/**
 * Accent for a cover URL, or `null` if it can't be read. Memoized: the same cover comes back around
 * constantly (repeat, a queue walked backwards, the setting toggled), and a hit is what lets the
 * colour start moving on the same frame the artwork does instead of after a decode.
 *
 * The image is fetched with CORS so the canvas stays untainted (googleusercontent and ytimg both
 * send `access-control-allow-origin: *`); a host that doesn't throws on `getImageData` and lands in
 * the same `null`.
 */
export async function artworkAccent(url: string): Promise<string | null> {
	const hit = cache.get(url);
	if (hit !== undefined) return hit;
	const accent = await read(url);
	// ponytail: a whole session's covers, one short string each. Dumped wholesale rather than kept
	// in LRU order; swap in a real LRU if a cover ever costs more than a hex string to remember.
	if (cache.size > 500) cache.clear();
	cache.set(url, accent);
	return accent;
}

/**
 * Decode a cover before anything needs its colour, so the track change itself pays for nothing.
 * Idle-time work: at the moment this is called the app is mid-playback, and a fetch plus a decode
 * on the main thread is exactly the sort of thing that shows up as a dropped frame.
 */
export function warmAccent(url: string): void {
	if (cache.has(url)) return;
	const run = () => artworkAccent(url);
	if ('requestIdleCallback' in window) requestIdleCallback(run, { timeout: 2000 });
	else setTimeout(run, 500);
}

// One scratch canvas, reused: a fresh SIZExSIZE backing store per cover is a native allocation the
// JS heap measurement cannot see. Everything after the `await` in `read` is synchronous, so two
// overlapping decodes cannot interleave on it.
let scratch: CanvasRenderingContext2D | null | undefined;
function scratchCtx(): CanvasRenderingContext2D | null {
	if (scratch === undefined) {
		const c = document.createElement('canvas');
		c.width = c.height = SIZE;
		scratch = c.getContext('2d', { willReadFrequently: true });
	}
	return scratch;
}

async function read(url: string): Promise<string | null> {
	try {
		const img = new Image();
		img.crossOrigin = 'anonymous';
		img.src = url;
		await img.decode();
		const ctx = scratchCtx();
		if (!ctx) return null;
		ctx.drawImage(img, 0, 0, SIZE, SIZE);
		return pickAccent(ctx.getImageData(0, 0, SIZE, SIZE).data);
	} catch {
		return null; // offline, 404, throttled, tainted — the current accent just stays
	}
}
