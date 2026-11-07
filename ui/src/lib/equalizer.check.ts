// Run: node --experimental-strip-types ui/src/lib/equalizer.check.ts
import { EQ_BANDS, EQ_MAX_DB, EQ_PRESETS, FLAT, clampDb, eqAudible, matchPreset, normalizeEq } from './equalizer.ts';

function ok(condition: boolean, message: string) {
	if (!condition) throw new Error(message);
}

ok(EQ_BANDS.length === 10, 'Ten bands');
const ids = new Set<string>();
for (const p of EQ_PRESETS) {
	ok(!ids.has(p.id), `Duplicate preset ${p.id}`);
	ids.add(p.id);
	ok(p.bands.length === 10, `${p.id} has ten bands`);
	for (const g of [p.preamp, ...p.bands]) ok(Math.abs(g) <= EQ_MAX_DB, `${p.id} stays in range`);
	// Presets change tone, not loudness: the loudest band lands within 4 dB of unity.
	ok(p.preamp + Math.max(0, ...p.bands) <= 4, `${p.id} is level-matched`);
	ok(matchPreset({ enabled: true, ...p }) === p.id, `${p.id} is recognized`);
}
for (const id of ['bass_enhancer', 'clear_audio', 'low_latency']) ok(ids.has(id), `${id} exists`);
// Low latency must never need the limiter, which is the only part of the chain with lookahead.
const ll = EQ_PRESETS.find((p) => p.id === 'low_latency')!;
ok(ll.preamp + Math.max(...ll.bands) <= 0, 'Low latency never boosts');

ok(!eqAudible(FLAT), 'Flat is silent');
ok(!eqAudible({ ...EQ_PRESETS[1], enabled: false }), 'Off is silent');
ok(eqAudible({ ...EQ_PRESETS[1], enabled: true }), 'Bass is audible');
ok(clampDb(99) === EQ_MAX_DB && clampDb(-99) === -EQ_MAX_DB, 'Clamped');
ok(clampDb(1.26) === 1.5, 'Snapped to half dB');
const junk = normalizeEq({ enabled: 'yes', preamp: NaN, bands: [1, 2] });
ok(!junk.enabled && junk.preamp === 0 && junk.bands.length === 10, 'Junk normalizes to flat');
ok(normalizeEq(null).bands.every((g) => g === 0), 'Null normalizes to flat');
console.log('ok');
