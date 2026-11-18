// UI language. localStorage is the source of truth, for the same reason the theme lives there: the
// first paint has to be in the right language, and an async read paints English and flips a frame
// later. Rust gets a copy too, because YouTube's own text is part of the language: see
// `pushLocaleToRust`.
import { browser } from '$app/environment';
import { invoke } from '@tauri-apps/api/core';
import { translations, LOCALES, type LocaleId, type Translations } from './locales';
import { coverage, pickLocale } from './langlist';

export type { LocaleId };

/** Every dot path through the catalog: `'nav.home' | 'player.play' | …`. */
type NestedKeyOf<ObjectType extends object> = {
	[Key in keyof ObjectType & (string | number)]: ObjectType[Key] extends object
		? `${Key}` | `${Key}.${NestedKeyOf<ObjectType[Key]>}`
		: `${Key}`;
}[keyof ObjectType & (string | number)];

export type TranslationKey = NestedKeyOf<Translations>;

const LOCALE_STORAGE_KEY = 'limusic_locale';

/**
 * The saved language, else the system one if we have a catalog for it, else English.
 *
 * Read synchronously at module load so the first paint is already in the right language: an async
 * read paints English and flips a frame later, on every launch.
 */
function getInitialLocale(): LocaleId {
	if (!browser) return 'en'; // prerender pass: no window, and nothing it renders is kept
	const saved = localStorage.getItem(LOCALE_STORAGE_KEY);
	// hasOwn, not `in`: localStorage is user-writable, and `in` would accept 'constructor'.
	if (saved && Object.hasOwn(translations, saved)) return saved as LocaleId;
	const hit = pickLocale(Object.keys(translations), navigator.language ?? '');
	return (hit as LocaleId) ?? 'en';
}

let activeLocale = $state<LocaleId>(getInitialLocale());

/**
 * Tell Rust which language to ask YouTube for (`hl`). Home shelf titles, the mood chips, playlist
 * subtitles and auto-playlist names are YouTube's strings, not ours, so a Korean UI on `hl=en` reads
 * half English (#274). Stored in SQLite there: the first home fetch of the next launch happens before
 * this module could push anything, so Rust has to already know.
 *
 * `initApp` reconciles the two at startup, for the launches where nobody touched this picker.
 */
export function pushLocaleToRust(locale: LocaleId): Promise<void> {
	return invoke<void>('set_setting', { key: 'locale', value: locale }).catch(() => {});
}

export function setLocale(locale: LocaleId): Promise<void> {
	if (!Object.hasOwn(translations, locale)) return Promise.resolve();
	activeLocale = locale;
	localStorage.setItem(LOCALE_STORAGE_KEY, locale);
	return pushLocaleToRust(locale);
}

/** Reactive: every `t()` in the markup re-runs when this changes. */
export const currentLocale = {
	get id() {
		return activeLocale;
	}
};

function getNestedValue(obj: unknown, path: string): unknown {
	return path.split('.').reduce<unknown>((acc, part) => (acc as any)?.[part], obj);
}

/**
 * Translate a key, falling back to English for anything the active catalog is missing or blank.
 *
 * `key` is typed against the English catalog, so a typo or a key that was never added is a build
 * error rather than a literal `home.remove_shortcut` rendered in the UI.
 *
 *   t('nav.home')
 *   t('settings.about.version', { version: '0.5.5' })
 */
export function t(key: TranslationKey, params?: Record<string, string | number>): string {
	let str = getNestedValue(translations[activeLocale], key);
	// Weblate writes an untranslated string as "", so empty counts as missing: a half-finished
	// catalog must render English, not a blank label.
	if (typeof str !== 'string' || str === '') str = getNestedValue(translations.en, key);
	if (typeof str !== 'string') return key;
	if (!params) return str;
	return str.replace(/\{(\w+)\}/g, (_, k) => (params[k] !== undefined ? String(params[k]) : `{${k}}`));
}

/** The English string whatever the active language: the settings search matches it too. */
export function tEn(key: TranslationKey): string {
	const str = getNestedValue(translations.en, key);
	return typeof str === 'string' ? str : key;
}

/**
 * How much of each catalog Weblate has actually landed, 0..1, so the picker can say a language is
 * half English before someone picks it and finds out the hard way.
 *
 * Counted once at module load: the catalogs are bundled and nothing here changes at runtime.
 */
export const COVERAGE: Record<LocaleId, number> = (() => {
	const ids = Object.keys(translations) as LocaleId[];
	return Object.fromEntries(
		ids.map((id) => [id, coverage(translations[id], translations.en)])
	) as Record<LocaleId, number>;
})();

export { LOCALES };
