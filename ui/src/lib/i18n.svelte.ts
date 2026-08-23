import { translations, LOCALES, type LocaleId, type Translations } from './locales';
import * as api from './api';

export type { LocaleId };

// Helper type for nested dot notation paths
type NestedKeyOf<ObjectType extends object> = {
	[Key in keyof ObjectType & (string | number)]: ObjectType[Key] extends object
		? `${Key}` | `${Key}.${NestedKeyOf<ObjectType[Key]>}`
		: `${Key}`;
}[keyof ObjectType & (string | number)];

export type TranslationKey = NestedKeyOf<Translations>;

const LOCALE_STORAGE_KEY = 'limusic_locale';

function getInitialLocale(): LocaleId {
	if (typeof window !== 'undefined' && window.localStorage) {
		const saved = localStorage.getItem(LOCALE_STORAGE_KEY);
		if (saved && saved in translations) {
			return saved as LocaleId;
		}
	}
	if (typeof navigator !== 'undefined' && navigator.language) {
		const lang = navigator.language.toLowerCase().split('-')[0];
		if (lang === 'tr') {
			return 'tr';
		}
	}
	return 'en';
}

// Reactive state for the current locale initialized synchronously before first paint
let activeLocale = $state<LocaleId>(getInitialLocale());

/**
 * Initialize locale from saved setting or browser language.
 */
export async function initLocale(): Promise<LocaleId> {
	try {
		const settings = await api.getSettings();
		if (settings.locale && settings.locale in translations) {
			activeLocale = settings.locale as LocaleId;
			if (typeof window !== 'undefined' && window.localStorage) {
				localStorage.setItem(LOCALE_STORAGE_KEY, settings.locale);
			}
			return activeLocale;
		}
	} catch {}

	const initial = getInitialLocale();
	if (initial !== 'en') {
		try {
			await api.setSetting('locale', initial);
		} catch {}
	}

	return activeLocale;
}

/**
 * Change the active locale and persist to settings.
 */
export async function setLocale(locale: LocaleId): Promise<void> {
	if (locale in translations) {
		activeLocale = locale;
		if (typeof window !== 'undefined' && window.localStorage) {
			localStorage.setItem(LOCALE_STORAGE_KEY, locale);
		}
		try {
			await api.setSetting('locale', locale);
		} catch (e) {
			console.error('Failed to persist locale setting:', e);
		}
	}
}

/**
 * Get current active locale ID.
 */
export function getLocale(): LocaleId {
	return activeLocale;
}

/**
 * Reactive getter for the active locale
 */
export const currentLocale = {
	get id() {
		return activeLocale;
	}
};

/**
 * Helper to resolve nested path in object
 */
function getNestedValue(obj: any, path: string): any {
	return path.split('.').reduce((acc, part) => acc && acc[part], obj);
}

/**
 * Main translation function.
 * Supports dot notation paths and parameter replacement ({param}).
 *
 * Example:
 *   t('nav.home')
 *   t('settings.about.version', { version: '0.5.5' })
 */
export function t(key: TranslationKey | (string & {}), params?: Record<string, string | number>): string {
	// Try current locale
	let str = getNestedValue(translations[activeLocale], key);

	// Fallback to English if missing
	if (str === undefined || str === null) {
		str = getNestedValue(translations.en, key);
	}

	// Fallback to key if still missing
	if (typeof str !== 'string') {
		return key;
	}

	// Interpolate params
	if (params) {
		return str.replace(/\{(\w+)\}/g, (_, k) => (params[k] !== undefined ? String(params[k]) : `{${k}}`));
	}

	return str;
}

/**
 * Format subtitle strings (e.g., "20 songs" -> "20 şarkı") when active locale is Turkish.
 */
export function formatSubtitle(s?: string): string {
	if (!s) return '';
	if (activeLocale === 'tr') {
		return s
			.replace(/(\d+)\s+songs?/gi, '$1 şarkı')
			.replace(/(\d+)\s+tracks?/gi, '$1 parça')
			.replace(/Your most played/gi, 'En çok dinledikleriniz');
	}
	return s;
}

export { LOCALES };
