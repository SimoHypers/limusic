import type { Reverb } from './api';

export type EffectSettings = { speed: number; semitones: number; reverb: Reverb };
export type EffectPreset = EffectSettings & { name: string };
export const EFFECT_PRESETS_KEY = 'limusic:effect-presets';
export const PRESET_NAME_MAX = 80;

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
export function presetSettings(preset: EffectPreset, inRoom: boolean): EffectSettings {
	return { speed: inRoom ? 1 : preset.speed, semitones: preset.semitones, reverb: { ...preset.reverb } };
}

export function sameEffects(a: EffectSettings, b: EffectSettings): boolean {
	return a.speed === b.speed && a.semitones === b.semitones
		&& (['enabled', 'preset', 'dry', 'wet', 'decay', 'preDelay', 'damping', 'reflections'] as const)
			.every((key) => a.reverb[key] === b.reverb[key]);
}
