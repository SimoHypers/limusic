// The track count in a playlist header's subtitle. Pure so it can be checked without a DOM
// (`trackcount.check.ts`).
//
// YouTube writes "190 tracks • 9+ hours", but a public playlist leads with its views: "601 views •
// 305 tracks • 18+ hours" (#490). The text is in the UI's language ("트랙 305개", "1 682 titres"),
// so the count is found by position, not by word: it is the part just before the duration, or the
// only part there is.

const SEP = ' • ';
// A number with its grouping ("1,682", "1.682", "1 682"), wherever it sits in its part.
const NUM = /\d(?:[\d,.\s]*\d)?/;

function countPart(parts: string[]): number {
	return parts.length > 1 ? parts.length - 2 : 0;
}

/** The count YouTube put in the subtitle, or undefined when that part holds no number. */
export function subtitleCount(subtitle: string | undefined): number | undefined {
	if (!subtitle) return undefined;
	const parts = subtitle.split(SEP);
	const digits = parts[countPart(parts)].match(NUM)?.[0].replace(/\D/g, '');
	return digits ? Number(digits) : undefined;
}

/** The subtitle with its count swapped for `count`, the words around it left as YouTube wrote them. */
export function withSubtitleCount(subtitle: string, count: number): string {
	const parts = subtitle.split(SEP);
	const i = countPart(parts);
	parts[i] = parts[i]
		.replace(NUM, count.toLocaleString())
		// English is the one plural we can fix up.
		.replace(/^([\d,]+) (track|song)s?$/, (_, n, word) => `${n} ${word}${n === '1' ? '' : 's'}`);
	return parts.join(SEP);
}
