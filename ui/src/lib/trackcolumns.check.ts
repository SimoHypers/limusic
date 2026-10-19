// Self-check for the track list columns (`trackcolumns.ts`), run by `pnpm test`:
//
//     node --experimental-strip-types ui/src/lib/trackcolumns.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant.
import { addedLabel, availableColumns, DEFAULT_COLUMNS, parseColumns } from './trackcolumns.ts';
import type { SongItem } from './api.ts';

function eq<T>(got: T, want: T, what: string): void {
	const [g, w] = [JSON.stringify(got), JSON.stringify(want)];
	if (g !== w) throw new Error(`FAIL: ${what}: got ${g}, want ${w}`);
}

const song = (extra: Partial<SongItem> = {}): SongItem => ({
	video_id: 'v',
	title: 't',
	artists: 'a',
	...extra
});

// Only what some row can fill.
eq(availableColumns([song()]), ['mine', 'duration'], 'bare rows');
eq(
	availableColumns([song(), song({ album: 'X', added_at: 1, added_by_avatars: ['a.jpg'], play_count: '5M' })]),
	['album', 'added', 'by', 'plays', 'mine', 'duration'],
	'one full row is enough'
);
eq(availableColumns([song({ album: 'X', play_count: '5M' })], true), ['plays', 'mine', 'duration'], 'album page drops Album');
eq(availableColumns([]), ['mine', 'duration'], 'empty list');
eq(availableColumns([song({ added_by_avatars: [] })]), ['mine', 'duration'], 'no avatars is no Added by');

// Stored choice: unknown ids dropped, our order, broken values fall back.
eq(parseColumns('["duration","album","bogus"]'), ['album', 'duration'], 'reordered + unknown');
eq(parseColumns('[]'), [], 'everything off stays off');
eq(parseColumns(null), DEFAULT_COLUMNS, 'nothing stored');
eq(parseColumns('{oops'), DEFAULT_COLUMNS, 'broken JSON');
eq(parseColumns('"album"'), DEFAULT_COLUMNS, 'not an array');

// Date added: always the day, however recent.
const day = 1_755_417_934; // 2025-08-17 08:05 UTC
eq(addedLabel(day, 'en-US'), 'Aug 17, 2025', 'a date');
eq(addedLabel(Math.floor(Date.now() / 1000) - 30, 'en-US').includes('ago'), false, 'never a distance');

console.log('ok');
