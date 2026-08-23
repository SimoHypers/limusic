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

// Reactive state for the current locale
let activeLocale = $state<LocaleId>('en');

/**
 * Initialize locale from saved setting or browser language.
 */
export async function initLocale(): Promise<LocaleId> {
	try {
		const settings = await api.getSettings();
		if (settings.locale && settings.locale in translations) {
			activeLocale = settings.locale as LocaleId;
			return activeLocale;
		}
	} catch {}

	// Fallback to system / browser language
	if (typeof navigator !== 'undefined' && navigator.language) {
		const lang = navigator.language.toLowerCase().split('-')[0];
		if (lang === 'tr') {
			activeLocale = 'tr';
			try {
				await api.setSetting('locale', 'tr');
			} catch {}
		}
	}

	return activeLocale;
}

/**
 * Change the active locale and persist to settings.
 */
export async function setLocale(locale: LocaleId): Promise<void> {
	if (locale in translations) {
		activeLocale = locale;
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
export function t(key: string, params?: Record<string, string | number>): string {
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

export { LOCALES };
