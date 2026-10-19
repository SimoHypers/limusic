// Pixel work for the cover wash `artroom.svelte.ts` bakes. Plain TS, so `wash.check.ts` can run it.

// Blurred by hand, not with `ctx.filter`: WebKit has none (WebKitGTK 2.52 reports it undefined,
// and WKWebView is the same engine), so Linux and macOS used to get a 20px mosaic that kept the
// cover's hottest blocks bright right behind the lyrics. Three box blurs by running sums make a
// gaussian with sigma about `r`, edges held so nothing darkens toward the frame. About 15 ms at
// 160px in WebKitGTK 2.52, once per track.
export function blur(px: Uint8ClampedArray, n: number, r: number) {
	const line = new Float32Array(n * 4);
	const at = (i: number) => (i < 0 ? 0 : i >= n ? n - 1 : i) * 4;
	const pass = (start: number, step: number) => {
		for (let i = 0; i < n; i++)
			for (let c = 0; c < 4; c++) line[i * 4 + c] = px[start + i * step + c];
		for (let c = 0; c < 4; c++) {
			let sum = 0;
			for (let k = -r; k <= r; k++) sum += line[at(k) + c];
			for (let i = 0; i < n; i++) {
				px[start + i * step + c] = sum / (2 * r + 1);
				sum += line[at(i + r + 1) + c] - line[at(i - r) + c];
			}
		}
	};
	for (let k = 0; k < 3; k++) {
		for (let y = 0; y < n; y++) pass(y * n * 4, 4);
		for (let x = 0; x < n; x++) pass(x * 4, n * 4);
	}
}

/** Pushes colour away from grey in place, and returns the darkest and brightest pixels' perceived
 *  brightness (HSP, 0-1, the measure `color.ts` uses) for `washVars`. */
export function saturate(px: Uint8ClampedArray, amount: number): { lo: number; hi: number } {
	let lo = 1;
	let hi = 0;
	for (let i = 0; i < px.length; i += 4) {
		const grey = 0.299 * px[i] + 0.587 * px[i + 1] + 0.114 * px[i + 2];
		for (let c = 0; c < 3; c++) px[i + c] = grey + (px[i + c] - grey) * amount;
		const hsp = 0.299 * px[i] ** 2 + 0.587 * px[i + 1] ** 2 + 0.114 * px[i + 2] ** 2;
		lo = Math.min(lo, Math.sqrt(hsp) / 255);
		hi = Math.max(hi, Math.sqrt(hsp) / 255);
	}
	return { lo, hi };
}
