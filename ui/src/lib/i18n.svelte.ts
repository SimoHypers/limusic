// UI language. localStorage rather than SQLite, for the same reason the theme lives there: nothing
// outside the webview reads it. YouTube is deliberately not part of this — see `getInitialLocale`.
import { browser } from '$app/environment';
import { translations, LOCALES, type LocaleId, type Translations } from './locales';

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
	if (saved && Object.hasOwn(translations, saved)) return saved as LocaleId;
	const raw = navigator.language?.toLowerCase();
	if (!raw) return 'en';
	const exact = Object.keys(translations).find((k) => k.toLowerCase() === raw);
	if (exact) return exact as LocaleId;
	const base = raw.split('-')[0];
	const baseMatch = Object.keys(translations).find((k) => k.toLowerCase() === base);
	if (baseMatch) return baseMatch as LocaleId;
	return 'en';
}

let activeLocale = $state<LocaleId>(getInitialLocale());

export function setLocale(locale: LocaleId): void {
	if (!Object.hasOwn(translations, locale)) return;
	activeLocale = locale;
	localStorage.setItem(LOCALE_STORAGE_KEY, locale);
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
 * Translate a key, falling back to English for anything the active catalog is missing.
 *
 * `key` is typed against the English catalog, so a typo or a key that was never added is a build
 * error rather than a literal `home.remove_shortcut` rendered in the UI.
 *
 *   t('nav.home')
 *   t('settings.about.version', { version: '0.5.5' })
 */
export function t(key: TranslationKey, params?: Record<string, string | number>): string {
	let str = getNestedValue(translations[activeLocale], key);
	if (typeof str !== 'string') str = getNestedValue(translations.en, key);
	if (typeof str !== 'string') return key;
	if (!params) return str;
	return str.replace(/\{(\w+)\}/g, (_, k) => (params[k] !== undefined ? String(params[k]) : `{${k}}`));
}

export { LOCALES };
