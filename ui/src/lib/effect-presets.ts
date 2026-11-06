import type { Reverb } from './api';

export type EffectSettings = { speed: number; semitones: number; reverb: Reverb };
export type EffectPreset = EffectSettings & { name: string };
export const EFFECT_PRESETS_KEY = 'limusic:effect-presets';
export const PRESET_NAME_MAX = 80;

export const DEFAULT_REVERB: Reverb = {
	enabled: false, preset: 3, dry: 100, wet: 25, decay: 1.1, preDelay: 0, damping: 8000, reflections: 100
};

/** The rooms in `player::REVERB_PRESETS` order, and each one's natural length (`REVERB_DECAYS`). */
export const ROOMS = ['small_room', 'medium_room', 'large_room', 'medium_hall', 'large_hall', 'plate'] as const;
export const ROOM_DECAYS = [0.4, 0.7, 1.1, 1.8, 3.2, 1.8] as const;

/** Reverb on, in `room` at its natural length: picking a room undoes a decay fine-tune. */
export function withRoom(reverb: Reverb, room: number): Reverb {
	return { ...reverb, enabled: true, preset: room, decay: ROOM_DECAYS[room - 1] };
}

/** One-tap sounds, on the 0.05 grid the tempo slider moves in. Slowed and nightcore shift pitch with
 *  speed the way a record does (12 * log2(0.85) is about -3), because that is how the edits people
 *  search for are made. Nightcore is set by ear, gentler than the genre's usual 25-35%: at 1.2x a
 *  record would rise 3.2 semitones. */
export const BUILTIN_PRESETS = [
	{ id: 'original', speed: 1, semitones: 0, reverb: DEFAULT_REVERB },
	{ id: 'slowed', speed: 0.85, semitones: -3, reverb: { ...withRoom(DEFAULT_REVERB, 4), wet: 40 } },
	{
		id: 'nightcore', speed: 1.2, semitones: 3,
		// Light, bright and short: a plate is the classic vocal reverb, and a long tail or no gap
		// before it smears fast, high vocals. Nightcore makers default to about 15% reverb.
		reverb: { ...withRoom(DEFAULT_REVERB, 6), wet: 15, decay: 1.2, preDelay: 20 }
	},
	{ id: 'practice', speed: 0.75, semitones: 0, reverb: DEFAULT_REVERB },
	{ id: 'hall', speed: 1, semitones: 0, reverb: withRoom(DEFAULT_REVERB, 5) }
] as const satisfies readonly (EffectSettings & { id: string })[];
export type BuiltinId = (typeof BUILTIN_PRESETS)[number]['id'];
export const ORIGINAL: EffectSettings = BUILTIN_PRESETS[0];

/** What you hear: a switched-off reverb's settings make no difference to it. */
export function sameSound(a: EffectSettings, b: EffectSettings): boolean {
	if (a.speed !== b.speed || a.semitones !== b.semitones || a.reverb.enabled !== b.reverb.enabled) return false;
	return !a.reverb.enabled || sameEffects(a, b);
}

function numberIn(value: unknown, min: number, max: number, integer = false): boolean {
	return typeof value === 'number' && Number.isFinite(value)
		&& value >= min && value <= max && (!integer || Number.isInteger(value));
}

function validPreset(value: unknown): value is EffectPreset {
	if (!value || typeof value !== 'object') return false;
	const p = value as Partial<EffectPreset>;
	const r = p.reverb;
	return typeof p.name === 'string' && p.name.trim() === p.name
		&& p.name.length > 0 && p.name.length <= PRESET_NAME_MAX
		&& numberIn(p.speed, 0.25, 2) && numberIn(p.semitones, -12, 12, true)
		&& !!r && typeof r.enabled === 'boolean'
		&& numberIn(r.preset, 1, 6, true)
		&& numberIn(r.dry, 0, 100, true) && numberIn(r.wet, 0, 100, true)
		&& numberIn(r.decay, 0.1, 10) && numberIn(r.preDelay, 0, 200, true)
		&& numberIn(r.damping, 500, 20000, true) && numberIn(r.reflections, 0, 100, true);
}

/** Reject unreadable data so saving cannot silently erase an existing collection. */
export function readEffectPresets(raw: string | null): EffectPreset[] {
	if (raw === null) return [];
	const presets: unknown = JSON.parse(raw);
	if (!Array.isArray(presets) || !presets.every(validPreset)
		|| new Set(presets.map((p) => p.name.toLowerCase())).size !== presets.length) {
		throw new Error('Invalid effect presets');
	}
	// Older app versions saved tempos below the current minimum.
	return presets.map((preset) => preset.speed < 0.5 ? { ...preset, speed: 0.5 } : preset);
}

/** Copy the saved sound; room playback must stay on its shared clock. */
export function presetSettings(preset: EffectSettings, inRoom: boolean): EffectSettings {
	return { speed: inRoom ? 1 : preset.speed, semitones: preset.semitones, reverb: { ...preset.reverb } };
}

export function sameEffects(a: EffectSettings, b: EffectSettings): boolean {
	return a.speed === b.speed && a.semitones === b.semitones
		&& (['enabled', 'preset', 'dry', 'wet', 'decay', 'preDelay', 'damping', 'reflections'] as const)
			.every((key) => a.reverb[key] === b.reverb[key]);
}
