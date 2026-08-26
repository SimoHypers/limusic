import { en, type Translations } from './en';
import { tr } from './tr';

export type LocaleId = 'en' | 'tr';

export interface LocaleInfo {
	id: LocaleId;
	label: string;
	nativeLabel: string;
	flag?: string;
}

export const LOCALES: LocaleInfo[] = [
	{ id: 'en', label: 'English', nativeLabel: 'English' },
	{ id: 'tr', label: 'Turkish', nativeLabel: 'Türkçe' }
];

export const translations: Record<LocaleId, Translations> = {
	en,
	tr
};

export { en, tr };
export type { Translations };
