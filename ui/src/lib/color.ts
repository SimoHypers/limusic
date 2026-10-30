// Colour maths for the theme picker. Pure, no DOM, no dependency: the picker only needs hex <->
// HSV and a "is this light?" decision, and that is all this file.
//
// HSV (not HSL) because the picker's square IS the HSV plane — x is saturation, y is value. Driving
// it from HSL needs a fudge factor at the edges that washes the top-left corner out.

export type Hsv = { h: number; s: number; v: number }; // h 0–360, s/v 0–1

const clamp01 = (n: number) => Math.min(1, Math.max(0, n));

function hexToRgb(hex: string): [number, number, number] | null {
	const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(hex.trim());
	if (!m) return null;
	const h = m[1].length === 3 ? [...m[1]].map((c) => c + c).join('') : m[1];
	return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255) as [number, number, number];
}

export function hexToHsv(hex: string): Hsv | null {
	const rgb = hexToRgb(hex);
	if (!rgb) return null;
	const [r, g, b] = rgb;
	const max = Math.max(r, g, b);
	const d = max - Math.min(r, g, b);
	let h = 0;
	if (d) {
		if (max === r) h = ((g - b) / d) % 6;
		else if (max === g) h = (b - r) / d + 2;
		else h = (r - g) / d + 4;
		h = (h * 60 + 360) % 360;
	}
	return { h, s: max ? d / max : 0, v: max };
}

export function hsvToHex({ h, s, v }: Hsv): string {
	s = clamp01(s);
	v = clamp01(v);
	// CSS Color 4's HSV->RGB, written as one channel function.
	const f = (n: number) => {
		const k = (n + h / 60) % 6;
		return Math.round(255 * v * (1 - s * Math.max(0, Math.min(k, 4 - k, 1))));
	};
	return '#' + [f(5), f(3), f(1)].map((c) => c.toString(16).padStart(2, '0')).join('');
}

/**
 * Perceived brightness (HSP), 0–1. Not WCAG relative luminance: luminance puts the light/dark
 * crossover so low that mid-blues get black text, which nobody ships. Unlike HSV's value it is
 * hue-aware, so a yellow and a blue that read as equally bright score the same.
 */
export function brightness(hex: string): number {
	const rgb = hexToRgb(hex);
	if (!rgb) return 0;
	const [r, g, b] = rgb;
	return Math.sqrt(0.299 * r * r + 0.587 * g * g + 0.114 * b * b);
}

/** Should text on this colour be dark? */
export function isLight(hex: string): boolean {
	return brightness(hex) > 0.6;
}

export function hexToOklch(hex: string): { c: number; h: number } | null {
	const rgb = hexToRgb(hex);
	if (!rgb) return null;
	const lin = (v: number) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
	const [r, g, b] = rgb.map(lin);
	const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
	const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
	const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
	const okA = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
	const okB = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
	const c = Math.hypot(okA, okB);
	const h = c < 0.004 ? 294 : ((Math.atan2(okB, okA) * 180) / Math.PI + 360) % 360;
	return { c, h };
}

export function hexToOklchHue(hex: string): number | null {
	const ok = hexToOklch(hex);
	return ok && ok.c >= 0.004 ? ok.h : null;
}
