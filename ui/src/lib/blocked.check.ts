// Self-check for blocked artists JSON parsing and deduplication in `blocked.ts`:
//
//     node --experimental-strip-types ui/src/lib/blocked.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant. Not imported by the app.
import { newBlockedArtists, parseBlockedArtists } from './blocked.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

const valid = parseBlockedArtists(
	JSON.stringify([
		{ id: ' UC1 ', name: ' Artist One ' },
		{ name: 'Artist Two' },
		{ id: '  ', name: 'No Id' },
		{ id: 'UC1', name: 'Duplicate Id' },
		{ name: 'artist two', id: 'UC2' }
	])
);
ok(
	same(valid, [
		{ id: 'UC1', name: 'Artist One' },
		{ name: 'Artist Two' },
		{ name: 'No Id' }
	]),
	'valid entries are trimmed, blank ids dropped, and duplicate ids/names deduplicated'
);

ok(same(parseBlockedArtists('[]'), []), 'empty array is valid');
ok(parseBlockedArtists('not json') === null, 'invalid json returns null');
ok(parseBlockedArtists('{}') === null, 'non-array json object returns null');
ok(parseBlockedArtists('"foo"') === null, 'non-array json string returns null');
ok(parseBlockedArtists('[{"foo":1}]') === null, 'entry without name returns null');
ok(parseBlockedArtists('[{"name":"   "}]') === null, 'blank name returns null');
ok(parseBlockedArtists('[{"name":"Valid","id":123}]') === null, 'non-string id returns null');

const merged = newBlockedArtists(
	[
		{ id: 'UC1', name: 'Renamed' },
		{ name: '  ARTIST TWO  ' },
		{ id: 'UC3', name: 'Artist Three' },
		{ name: 'Artist Four' }
	],
	[{ id: 'UC1', name: 'Artist One' }, { name: 'Artist Two' }]
);
ok(
	same(merged, [{ id: 'UC3', name: 'Artist Three' }, { name: 'Artist Four' }]),
	'newBlockedArtists filters out entries matching by id or case-insensitive name'
);

console.log('ok');
