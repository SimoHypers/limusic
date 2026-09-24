// Catalogs are plain JSON so Weblate can read and write them directly; see CONTRIBUTING.md.
// English is the source of truth and the only complete one: `t()` falls back to it per key, so a
// half-finished catalog renders English for what it is missing rather than a raw key.
import en from './en.json';
import es from './es.json';
import fr from './fr.json';
import id from './id.json';
import ko from './ko.json';
import pl from './pl.json';
import ptBR from './pt_BR.json';
import ro from './ro.json';
import ru from './ru.json';
import tr from './tr.json';
import uk from './uk.json';
import zhHant from './zh_Hant.json';

export type Translations = typeof en;

/** A catalog that has not been fully translated yet: every key optional, all the way down. */
type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K] };

export type LocaleId =
	| 'en'
	| 'es'
	| 'fr'
	| 'tr'
	| 'pt-BR'
	| 'id'
	| 'ro'
	| 'ko'
	| 'ru'
	| 'uk'
	| 'zh-Hant'
	| 'pl';

export interface LocaleInfo {
	id: LocaleId;
	/** Shown in the language picker, in the language itself. */
	nativeLabel: string;
	/**
	 * The same language named in English, shown under the native name and searchable.
	 *
	 * Deliberately not a translatable key: somebody who landed in a script they cannot read needs a
	 * second name in Latin letters to find their way out, and translating it would take that away.
	 */
	englishLabel: string;
}

export const LOCALES: LocaleInfo[] = [
	{ id: 'en', nativeLabel: 'English', englishLabel: 'English' },
	{ id: 'es', nativeLabel: 'Español', englishLabel: 'Spanish' },
	{ id: 'fr', nativeLabel: 'Français', englishLabel: 'French' },
	{ id: 'id', nativeLabel: 'Bahasa Indonesia', englishLabel: 'Indonesian' },
	{ id: 'ko', nativeLabel: '한국어', englishLabel: 'Korean' },
	{ id: 'pl', nativeLabel: 'Polski', englishLabel: 'Polish' },
	{ id: 'pt-BR', nativeLabel: 'Português (Brasil)', englishLabel: 'Portuguese (Brazil)' },
	{ id: 'ro', nativeLabel: 'Română', englishLabel: 'Romanian' },
	{ id: 'ru', nativeLabel: 'Русский', englishLabel: 'Russian' },
	{ id: 'tr', nativeLabel: 'Türkçe', englishLabel: 'Turkish' },
	{ id: 'uk', nativeLabel: 'Українська', englishLabel: 'Ukrainian' },
	{ id: 'zh-Hant', nativeLabel: '繁體中文', englishLabel: 'Chinese (Traditional)' }
];

// Filenames are Weblate's language codes (pt_BR), the ids here are BCP-47 (pt-BR) because that is
// what `navigator.language` reports. They differ on purpose; do not rename the files to match.
// The id is also what goes to YouTube as `hl`, so half the app's text depends on it (#274): a tag
// YouTube does not know answers 400 to every browse, not English. Adding a locale means checking
// its id against music.youtube.com, not just landing the catalog.
// Partial: only English is guaranteed complete, the rest are whatever Weblate has landed so far.
export const translations: Record<LocaleId, DeepPartial<Translations>> = {
	en,
	es,
	fr,
	tr,
	'pt-BR': ptBR,
	id,
	ro,
	ko,
	ru,
	uk,
	'zh-Hant': zhHant,
	pl
};
