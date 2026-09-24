// Self-check for the language picker's two computed bits (`langlist.ts`). No test runner in `ui/`,
// node 22 runs TypeScript directly:
//
//     node --experimental-strip-types ui/src/lib/langlist.check.ts
//
// Prints "ok" and exits 0, or throws. What this guards is the part a person stuck in a script they
// cannot read depends on: the search has to find a language by its English name, by its id, and by
// its native name typed without accents. Plus the coverage fraction, because a wrong denominator
// would quietly label a finished catalog as half done.
import { coverage, fold, matchLocales } from './langlist.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

const LOCALES = [
	{ id: 'en', nativeLabel: 'English', englishLabel: 'English' },
	{ id: 'fr', nativeLabel: 'Français', englishLabel: 'French' },
	{ id: 'ko', nativeLabel: '한국어', englishLabel: 'Korean' },
	{ id: 'ro', nativeLabel: 'Română', englishLabel: 'Romanian' },
	{ id: 'pt-BR', nativeLabel: 'Português (Brasil)', englishLabel: 'Portuguese (Brazil)' }
];
const ids = (list: typeof LOCALES) => list.map((l) => l.id).join(',');

// --- search ------------------------------------------------------------------------------------
ok(matchLocales(LOCALES, '') === LOCALES, 'an empty query hands back the same list');
ok(matchLocales(LOCALES, '   ').length === LOCALES.length, 'whitespace counts as empty');
ok(ids(matchLocales(LOCALES, 'francais')) === 'fr', 'accents are optional: francais finds Français');
ok(ids(matchLocales(LOCALES, 'Français')) === 'fr', 'and typing them still works');
ok(ids(matchLocales(LOCALES, 'romana')) === 'ro', 'romana finds Română');
ok(ids(matchLocales(LOCALES, 'korean')) === 'ko', 'a script you cannot read is findable in English');
ok(ids(matchLocales(LOCALES, '한국')) === 'ko', 'and in its own script');
ok(ids(matchLocales(LOCALES, 'pt-br')) === 'pt-BR', 'the id matches, case-insensitively');
ok(ids(matchLocales(LOCALES, 'BRAZIL')) === 'pt-BR', 'as does the English name, any case');
ok(matchLocales(LOCALES, 'klingon').length === 0, 'no match means no rows');
ok(fold('Português') === 'portugues', 'fold strips accents and lowercases');

// --- coverage ----------------------------------------------------------------------------------
const en = { a: 'one', b: { c: 'two', d: 'three' }, e: 'four' };
ok(coverage(en, 4) === 1, 'the complete catalog is 1');
// Weblate writes an untranslated string as "", so a blank is missing, not translated.
ok(coverage({ a: 'un', b: { c: '', d: 'trois' } }, 4) === 0.5, 'blanks do not count as translated');
ok(coverage({}, 4) === 0, 'an empty catalog is 0');
ok(coverage({ ...en, gone: 'stale key' }, 4) === 1, 'a key English dropped cannot push it past 1');
ok(coverage(en, 0) === 1, 'no reference keys is not a division by zero');

console.log('ok');
