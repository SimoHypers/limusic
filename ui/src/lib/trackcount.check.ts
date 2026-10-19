// Self-check for the playlist header's track count (`trackcount.ts`), run by `pnpm test`:
//
//     node --experimental-strip-types ui/src/lib/trackcount.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant.
import { subtitleCount, withSubtitleCount } from './trackcount.ts';

function eq<T>(got: T, want: T, what: string): void {
	if (got !== want) throw new Error(`FAIL: ${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
}

eq(subtitleCount('190 tracks • 9+ hours'), 190, 'private playlist');
// #490: the views came first and were offered as the count, "214K" read as 214.
eq(subtitleCount('601 views • 305 tracks • 18+ hours'), 305, 'public playlist');
eq(subtitleCount('214K views • 1,682 tracks • 102+ hours'), 1682, 'abbreviated views');
eq(subtitleCount('18 songs • 1 hour, 8 minutes'), 18, 'album');
eq(subtitleCount('1 song'), 1, 'no duration');
eq(subtitleCount('12 views • 트랙 305개 • 18시간 이상'), 305, 'number after the word');
eq(subtitleCount('1 682 titres • 102+ heures'), 1682, 'space grouping');
eq(subtitleCount('Playlist • Someone • 2024'), undefined, 'no count');
eq(subtitleCount(undefined), undefined, 'no subtitle');

eq(withSubtitleCount('601 views • 305 tracks • 18+ hours', 300), '601 views • 300 tracks • 18+ hours', 'views left alone');
eq(withSubtitleCount('2 tracks • 6 minutes', 1), '1 track • 6 minutes', 'singular');
eq(withSubtitleCount('1 song • 3 minutes', 2), '2 songs • 3 minutes', 'plural');
eq(withSubtitleCount('12 views • 트랙 305개 • 18시간 이상', 300), '12 views • 트랙 300개 • 18시간 이상', 'words left alone');

console.log('ok');
