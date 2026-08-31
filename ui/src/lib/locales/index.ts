// Catalogs are plain JSON so Weblate can read and write them directly; see CONTRIBUTING.md.
// English is the source of truth and the only complete one: `t()` falls back to it per key, so a
// half-finished catalog renders English for what it is missing rather than a raw key.
import en from './en.json';
import ptBR from './pt_BR.json';
import tr from './tr.json';

export type Translations = typeof en;

/** A catalog that has not been fully translated yet: every key optional, all the way down. */
type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K] };

export type LocaleId = 'en' | 'tr' | 'pt-BR';

export interface LocaleInfo {
	id: LocaleId;
	/** Shown in the language picker, in the language itself. */
	nativeLabel: string;
}

export const LOCALES: LocaleInfo[] = [
	{ id: 'en', nativeLabel: 'English' },
	{ id: 'pt-BR', nativeLabel: 'Português (Brasil)' },
	{ id: 'tr', nativeLabel: 'Türkçe' }
];

// Filenames are Weblate's language codes (pt_BR), the ids here are BCP-47 (pt-BR) because that is
// what `navigator.language` reports. They differ on purpose; do not rename the files to match.
// Partial: only English is guaranteed complete, the rest are whatever Weblate has landed so far.
export const translations: Record<LocaleId, DeepPartial<Translations>> = { en, tr, 'pt-BR': ptBR };
