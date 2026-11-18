// Self-check for the settings search index (`settings-search.ts`), run by `pnpm test`:
//
//     node --experimental-strip-types ui/src/lib/settings-search.check.ts
//
// What it guards: every settings row is searchable (a row added to a settings component without an
// index entry fails here, naming the key), and a query matches the English text as well as the
// active language's.
//
// @ts-nocheck: it reads the sources with node:fs, and ui/ has no Node types for svelte-check.
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SETTINGS, indexSettings } from './settings-search.ts';
import { match } from './localsearch.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

const here = dirname(fileURLToPath(import.meta.url));
const en = JSON.parse(readFileSync(join(here, 'locales/en.json'), 'utf8'));
const lookup = (cat: unknown, key: string): string => {
	const v = key.split('.').reduce<any>((o, p) => o?.[p], cat);
	return typeof v === 'string' ? v : key;
};

// --- coverage: every `row({ title: t('…') })` / `row_({ … })` has an entry ----------------------
const indexed = new Set(SETTINGS.map((e) => e.title));
const dir = join(here, 'components');
const files = readdirSync(dir).filter((f) => /Settings(Dialog)?\.svelte$/.test(f));
ok(files.includes('SettingsDialog.svelte'), 'found the settings components');
const missing: string[] = [];
for (const f of files) {
	const src = readFileSync(join(dir, f), 'utf8');
	for (const m of src.matchAll(/render row_?\(\{\s*title: t\('([^']+)'\)/g)) {
		if (!indexed.has(m[1] as never)) missing.push(`${f}: ${m[1]}`);
	}
}
ok(missing.length === 0, `settings rows missing from settings-search.ts:\n  ${missing.join('\n  ')}`);

// --- every key resolves -------------------------------------------------------------------------
for (const e of SETTINGS) {
	for (const k of [e.title, e.desc, e.section, e.under]) {
		if (k) ok(lookup(en, k) !== k, `${k} is in en.json`);
	}
	if (e.under) ok(indexed.has(e.under), `${e.title}'s parent ${e.under} is itself an entry`);
}

// --- matching -----------------------------------------------------------------------------------
const tr = (k: string) => lookup(en, k);
const ix = indexSettings(SETTINGS, tr as never, tr as never);
const titles = (q: string) => match(ix, q).map((e) => e.title);

ok(titles('').length === 0, 'an empty query lists nothing');
ok(titles('crossfade')[0] === 'settings.playback.crossfade', 'the exact title ranks first');
ok(titles('crossfade').includes('settings.playback.crossfade_duration'), 'a longer title matches too');
ok(titles('TRAY').includes('settings.general.close_to_tray'), 'case-insensitive');
ok(titles('ambient').includes('settings.playback.ambient_light'), 'a row behind a switch is listed');
ok(titles('bandwidth').includes('settings.playback.audio_quality'), 'the description matches');
ok(titles('video').includes('settings.playback.hide_videos'), 'the section matches');
ok(titles('phone').includes('settings.tabs.devices'), 'a tab is found by its hint');

// A translated catalog: the local word and the English one both find the row, the local one ranks.
const fr = { settings: { general: { close_to_tray: 'Réduire dans la zone de notification' } } };
const trFr = (k: string) => (lookup(fr, k) !== k ? lookup(fr, k) : tr(k));
const ixFr = indexSettings(SETTINGS, trFr as never, tr as never);
ok(match(ixFr, 'zone').some((e) => e.title === 'settings.general.close_to_tray'), 'the active language matches');
ok(match(ixFr, 'tray').some((e) => e.title === 'settings.general.close_to_tray'), 'English still matches');

console.log('ok');
