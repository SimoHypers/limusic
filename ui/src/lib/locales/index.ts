// Catalogs are plain JSON so Weblate can read and write them directly; see CONTRIBUTING.md.
// English is the source of truth and the only complete one: `t()` falls back to it per key, so a
// half-finished catalog renders English for what it is missing rather than a raw key.
import de from './de.json';
import en from './en.json';
import es from './es.json';
import fr from './fr.json';
import hu from './hu.json';
import id from './id.json';
import it from './it.json';
import ja from './ja.json';
import ko from './ko.json';
import pl from './pl.json';
import ptBR from './pt_BR.json';
import ro from './ro.json';
import ru from './ru.json';
import ta from './ta.json';
import tr from './tr.json';
import uk from './uk.json';
import vi from './vi.json';
import zhHans from './zh_Hans.json';
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
	| 'pl'
	| 'de'
	| 'it'
	| 'ja'
	| 'vi'
	| 'zh-Hans'
	| 'ta'
	| 'hu';

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
	{ id: 'de', nativeLabel: 'Deutsch', englishLabel: 'German' },
	{ id: 'es', nativeLabel: 'Español', englishLabel: 'Spanish' },
	{ id: 'fr', nativeLabel: 'Français', englishLabel: 'French' },
	{ id: 'hu', nativeLabel: 'Magyar', englishLabel: 'Hungarian' },
	{ id: 'id', nativeLabel: 'Bahasa Indonesia', englishLabel: 'Indonesian' },
	{ id: 'it', nativeLabel: 'Italiano', englishLabel: 'Italian' },
	{ id: 'ja', nativeLabel: '日本語', englishLabel: 'Japanese' },
	{ id: 'ko', nativeLabel: '한국어', englishLabel: 'Korean' },
	{ id: 'pl', nativeLabel: 'Polski', englishLabel: 'Polish' },
	{ id: 'pt-BR', nativeLabel: 'Português (Brasil)', englishLabel: 'Portuguese (Brazil)' },
	{ id: 'ro', nativeLabel: 'Română', englishLabel: 'Romanian' },
	{ id: 'ru', nativeLabel: 'Русский', englishLabel: 'Russian' },
	{ id: 'ta', nativeLabel: 'தமிழ்', englishLabel: 'Tamil' },
	{ id: 'tr', nativeLabel: 'Türkçe', englishLabel: 'Turkish' },
	{ id: 'uk', nativeLabel: 'Українська', englishLabel: 'Ukrainian' },
	{ id: 'vi', nativeLabel: 'Tiếng Việt', englishLabel: 'Vietnamese' },
	{ id: 'zh-Hans', nativeLabel: '简体中文', englishLabel: 'Chinese (Simplified)' },
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
	pl,
	de,
	it,
	ja,
	vi,
	'zh-Hans': zhHans,
	ta,
	hu
};
