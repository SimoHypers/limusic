// What each lyrics provider is best at, for Settings and the source picker. The order and the on/off
// state come from Rust (`lyricsProviders`); this is only how the UI describes them.
import type { Lyrics } from './api';
import type { TranslationKey } from './i18n.svelte';

export type LyricsKind = 'words' | 'synced' | 'plain' | 'instrumental';

/** The best a provider can give, which Settings shows as its badge. */
export const SOURCE_KIND: Record<string, LyricsKind> = {
	boidu: 'words',
	unison: 'words',
	lyricsplus: 'words',
	lrclib: 'synced',
	youtube: 'synced',
	netease: 'synced',
	qq: 'synced',
	kugou: 'synced'
};

export const aboutKey = (id: string) => `lyrics.about.${id}` as TranslationKey;

export const KIND_LABEL: Record<LyricsKind, TranslationKey> = {
	words: 'lyrics.kind_words',
	synced: 'lyrics.kind_synced',
	plain: 'lyrics.kind_plain',
	instrumental: 'lyrics.instrumental'
};

/** What a set of lyrics actually turned out to be. */
export function lyricsKind(l: Lyrics): LyricsKind {
	if (l.instrumental) return 'instrumental';
	if (!l.synced) return 'plain';
	return l.lines.some((line) => line.words?.length) ? 'words' : 'synced';
}

/** One badge style per kind, so Settings and the picker read the same. */
export const KIND_CLASS: Record<LyricsKind, string> = {
	words: 'bg-primary/12 text-primary',
	synced: 'bg-foreground/8 text-foreground/80',
	plain: 'bg-muted text-muted-foreground',
	instrumental: 'bg-muted text-muted-foreground'
};
