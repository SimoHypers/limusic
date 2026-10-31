// Self-check for playlist row reordering (`reorder.ts`).
// Run with:
//     node --experimental-strip-types ui/src/lib/reorder.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant.
import type { SongItem } from './api.ts';
import { canDropAt, reorder, successorId } from './reorder.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

const song = (id: string, set_video_id?: string): SongItem => ({
	video_id: id,
	title: id,
	artists: '',
	set_video_id
});

const items = [song('a', 'sv_a'), song('b', 'sv_b'), song('c', 'sv_c'), song('d', 'sv_d')];

// --- reorder -----------------------------------------------------------------------------------
const down = reorder(items, 0, 3);
ok(down.map((s) => s.video_id).join(',') === 'b,c,a,d', 'move down');
ok(down !== items, 'returns a new array on change');

const up = reorder(items, 2, 0);
ok(up.map((s) => s.video_id).join(',') === 'c,a,b,d', 'move up');

const noop1 = reorder(items, 1, 1);
ok(noop1 === items, 'no-op when dropAt === from returns same array');

const noop2 = reorder(items, 1, 2);
ok(noop2 === items, 'no-op when dropAt === from + 1 returns same array');

const end = reorder(items, 0, 4);
ok(end.map((s) => s.video_id).join(',') === 'b,c,d,a', 'move to the very end');

// --- successorId -------------------------------------------------------------------------------
ok(successorId(items, 0, 2) === 'sv_c', 'successorId for middle row');
ok(successorId(items, 0, 4) === null, 'successorId at or past end is null');

// --- canDropAt ---------------------------------------------------------------------------------
ok(canDropAt(2, items, true) === true, 'can drop in middle when hasMore is true');
ok(canDropAt(4, items, true) === false, 'cannot drop at end when hasMore is true');
ok(canDropAt(4, items, false) === true, 'can drop at end when hasMore is false');
const itemsMissing = [song('a', 'sv_a'), song('b'), song('c', 'sv_c')];
ok(canDropAt(1, itemsMissing, false) === false, 'cannot drop in front of row without set_video_id');
ok(canDropAt(2, itemsMissing, false) === true, 'can drop after row without set_video_id');

console.log('ok');
