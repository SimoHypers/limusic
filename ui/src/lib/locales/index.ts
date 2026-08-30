import { en, type Translations } from './en';
import { tr } from './tr';
import { ru } from './ru';
import { uk } from './uk';

export type LocaleId = 'en' | 'tr' | 'ru' | 'uk';

export interface LocaleInfo {
	id: LocaleId;
	label: string;
	nativeLabel: string;
	flag?: string;
}

export const LOCALES: LocaleInfo[] = [
	{ id: 'en', label: 'English', nativeLabel: 'English' },
	{ id: 'tr', label: 'Turkish', nativeLabel: 'Türkçe' },
	{ id: 'ru', label: 'Russian', nativeLabel: 'Русский' },
	{ id: 'uk', label: 'Ukrainian', nativeLabel: 'Українська' }
];

export const translations: Record<LocaleId, Translations> = {
	en,
	tr,
	ru,
	uk
};

export { en, tr, ru, uk };
export type { Translations };
