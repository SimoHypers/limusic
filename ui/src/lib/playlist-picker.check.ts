// Self-check for remembered playlist target helpers in `playlist-picker.ts`:
//
//     node --experimental-strip-types ui/src/lib/playlist-picker.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant. Not imported by the app.
import {
	orderWithRemembered,
	parseLastPlaylists,
	pickRemembered
} from './playlist-picker.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

ok(
	same(parseLastPlaylists('["PL1"," PL2 ","PL1","",123]'), ['PL1', 'PL2']),
	'parses valid strings, trims, deduplicates, and ignores bad entries'
);
ok(same(parseLastPlaylists(null), []), 'null returns empty list');
ok(same(parseLastPlaylists('not json'), []), 'invalid json returns empty list');
ok(same(parseLastPlaylists('{"id":"PL1"}'), []), 'non-array json returns empty list');

ok(
	same(pickRemembered(['PL_DEL', 'PL2', 'PL1'], ['PL1', 'PL2', 'PL3']), ['PL2', 'PL1']),
	'drops deleted or incompatible targets while keeping order'
);

const items = [{ id: 'a' }, { id: 'b' }, { id: 'c' }, { id: 'd' }];
ok(orderWithRemembered(items, []) === items, 'no remembered ids returns input unchanged');
ok(
	same(
		orderWithRemembered(items, ['c', 'a']).map((x) => x.id),
		['a', 'c', 'b', 'd']
	),
	'remembered items move to the top preserving relative order'
);
ok(
	orderWithRemembered(items, ['missing']) === items,
	'missing remembered ids return input unchanged'
);

console.log('ok');
