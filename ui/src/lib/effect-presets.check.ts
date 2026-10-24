// Run: node --experimental-strip-types ui/src/lib/effect-presets.check.ts
import { readEffectPresets, presetSettings, sameEffects, type EffectPreset } from './effect-presets.ts';

function ok(condition: boolean, message: string) {
	if (!condition) throw new Error(message);
}
function rejects(raw: string) {
	let rejected = false;
	try { readEffectPresets(raw); } catch { rejected = true; }
	ok(rejected, `Invalid storage accepted: ${raw}`);
}

const preset: EffectPreset = {
	name: 'Slow room', speed: 0.8, semitones: -2,
	reverb: { enabled: true, preset: 3, dry: 100, wet: 40, decay: 2.5, preDelay: 30, damping: 6000, reflections: 70 }
};
ok(readEffectPresets(null).length === 0, 'No storage starts empty');
ok(JSON.stringify(readEffectPresets(JSON.stringify([preset]))) === JSON.stringify([preset]), 'Full settings survive restart');
ok(readEffectPresets(JSON.stringify([{ ...preset, speed: 0.3 }]))[0].speed === 0.5, 'Legacy tempos use the new minimum');
const solo = presetSettings(preset, false);
ok(sameEffects(preset, solo), 'Loading matches the saved sound');
solo.reverb.wet = 50;
ok(preset.reverb.wet === 40, 'Editing cannot mutate the saved sound');
ok(!sameEffects(preset, solo), 'Editing marks the sound modified');
const room = presetSettings(preset, true);
ok(room.speed === 1 && preset.speed === 0.8, 'Room clock leaves saved tempo intact');
ok(JSON.stringify(room.reverb) === JSON.stringify(preset.reverb) && room.semitones === -2, 'Rooms apply pitch and reverb');
for (const key of ['enabled', 'preset', 'dry', 'wet', 'decay', 'preDelay', 'damping', 'reflections'] as const) {
	const changed = presetSettings(preset, false);
	Object.assign(changed.reverb, { [key]: key === 'enabled' ? false : (changed.reverb[key] as number) + 1 });
	ok(!sameEffects(preset, changed), `${key} changes mark a preset modified`);
}
for (const value of [null, {}, [null], [{ ...preset, name: ' ' }], [{ ...preset, speed: 3 }],
	[{ ...preset, semitones: 1.5 }], [{ ...preset, reverb: { ...preset.reverb, preset: 7 } }],
	[{ ...preset, reverb: { ...preset.reverb, decay: -1 } }],
	[{ ...preset, reverb: { ...preset.reverb, wet: '40' } }],
	[preset, { ...preset, name: 'slow room' }]]) {
	rejects(JSON.stringify(value));
}
rejects('{broken');
console.log('ok — effect presets');
