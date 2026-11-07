// Ten-band equalizer: the curve's shape, the built-in presets, and the maths the dialog draws with.
// No Tauri imports, so a Node self-check can load it (equalizer.check.ts).

/** Mirrors `player::Equalizer`. Bands are dB, in `EQ_BANDS` order. */
export interface Equalizer {
	enabled: boolean;
	preamp: number;
	bands: number[];
}

/** Centre frequencies, Hz. Must match `EQ_BANDS` in crates/player. */
export const EQ_BANDS = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000] as const;
export const EQ_MAX_DB = 12;
export const EQ_STEP = 0.5;

export const FLAT: Equalizer = { enabled: false, preamp: 0, bands: Array(10).fill(0) };

export type EqPresetId =
	| 'flat'
	| 'bass_enhancer'
	| 'clear_audio'
	| 'low_latency'
	| 'bass_clear'
	| 'vocal'
	| 'rock'
	| 'electronic';

/**
 * Built-in curves. Each one is tuned hard enough to hear at once, and each carries a preamp that
 * keeps its loudest band near 0 dB, so switching presets changes tone, not volume. Anything that
 * still reaches above unity goes through the backend's limiter.
 *
 * Low Latency only ever cuts: with every gain at or below 0 dB the backend leaves the limiter (and
 * its 5 ms lookahead) out, so the chain is plain biquads and adds no delay at all.
 */
export const EQ_PRESETS: { id: EqPresetId; preamp: number; bands: number[] }[] = [
	{ id: 'flat', preamp: 0, bands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
	// Deep sub and punch, a hair of mid scoop so the low end doesn't muddy the vocal.
	{ id: 'bass_enhancer', preamp: -6, bands: [10, 9, 7, 3, 0, -1, 0, 1, 2, 2] },
	// Mud out at 250–500 Hz, presence at 2–4 kHz, air on top: detail without harshness.
	{ id: 'clear_audio', preamp: -4, bands: [-1, -1, -2, -3, -1, 1, 3, 5, 5, 4] },
	// A V-shape built from cuts alone: zero added latency, can never clip.
	{ id: 'low_latency', preamp: 0, bands: [0, 0, -1, -3, -4, -3, -1, 0, 0, 0] },
	// Bass Enhancer and Clear Audio together.
	{ id: 'bass_clear', preamp: -6, bands: [8, 7, 5, 1, -2, -1, 2, 4, 5, 4] },
	{ id: 'vocal', preamp: -4, bands: [-3, -2, -1, 1, 3, 4, 4, 3, 1, 0] },
	{ id: 'rock', preamp: -5, bands: [6, 5, 3, -1, -2, -1, 2, 4, 5, 5] },
	{ id: 'electronic', preamp: -6, bands: [9, 8, 4, 0, -2, 1, 0, 2, 5, 6] }
];

export const clampDb = (v: number) =>
	Math.max(-EQ_MAX_DB, Math.min(EQ_MAX_DB, Math.round(v / EQ_STEP) * EQ_STEP));

/** Same curve, ignoring the on/off switch. */
export function sameCurve(a: Pick<Equalizer, 'preamp' | 'bands'>, b: Pick<Equalizer, 'preamp' | 'bands'>) {
	return a.preamp === b.preamp && a.bands.length === b.bands.length && a.bands.every((g, i) => g === b.bands[i]);
}

export const matchPreset = (eq: Equalizer) => EQ_PRESETS.find((p) => sameCurve(p, eq))?.id ?? null;

/** Whether the curve changes the sound at all. */
export const eqAudible = (eq: Equalizer) => eq.enabled && (eq.preamp !== 0 || eq.bands.some((g) => g !== 0));

export const bandLabel = (hz: number) => (hz >= 1000 ? `${hz / 1000}k` : `${hz}`);

export const signedDb = (v: number) => (v > 0 ? `+${v}` : v < 0 ? `−${-v}` : '0');

/** Anything the webview or an old row hands back, made into a curve the backend will accept. */
export function normalizeEq(raw: unknown): Equalizer {
	const r = (raw ?? {}) as Partial<Equalizer>;
	const bands = Array.isArray(r.bands) && r.bands.length === 10 ? r.bands : FLAT.bands;
	const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? clampDb(v) : 0);
	return { enabled: r.enabled === true, preamp: num(r.preamp), bands: bands.map(num) };
}
