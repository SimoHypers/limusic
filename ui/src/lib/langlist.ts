// The two computed bits behind the language picker, kept pure so `langlist.check.ts` can run them
// without a DOM or a Svelte runtime: which languages a query matches, and how much of a catalog
// Weblate has actually landed.

/** The fields the search reads. `LocaleInfo` satisfies it; the check file makes its own. */
export interface Searchable {
	id: string;
	nativeLabel: string;
	englishLabel: string;
}

/**
 * Lowercased and stripped of accents, so `francais` finds Français and `romana` finds Română. The
 * keyboard someone is typing on is rarely the one their own language would want.
 */
export function fold(s: string): string {
	return s
		.normalize('NFD')
		.replace(/\p{Diacritic}/gu, '')
		.toLowerCase();
}

/**
 * Languages matching `query`, in the order they were given. An empty query matches everything,
 * because the picker opens showing the whole list.
 *
 * The id is searchable alongside both names, which is the way out of a script you cannot read: a
 * German speaker who landed in Korean can type `de`.
 */
export function matchLocales<T extends Searchable>(locales: T[], query: string): T[] {
	const q = fold(query.trim());
	if (!q) return locales;
	return locales.filter((l) => fold(`${l.nativeLabel} ${l.englishLabel} ${l.id}`).includes(q));
}

/** Non-blank strings anywhere in a catalog. Weblate writes an untranslated string as "". */
export function filledStrings(node: unknown): number {
	if (typeof node === 'string') return node.trim() ? 1 : 0;
	if (node && typeof node === 'object')
		return Object.values(node).reduce<number>((n, v) => n + filledStrings(v), 0);
	return 0;
}

/**
 * How much of `catalog` is translated, 0..1, against English as the complete one.
 *
 * Clamped, for a catalog that still carries keys English has since dropped: Weblate keeps its own
 * copy and lags a little behind the source.
 */
export function coverage(catalog: unknown, englishKeys: number): number {
	if (englishKeys <= 0) return 1;
	return Math.min(1, filledStrings(catalog) / englishKeys);
}
