// Self-check for the queue panel's block split (`queue.ts`). Same deal as `dnd.check.ts` — no test
// runner in `ui/`, node 22 runs TypeScript directly:
//
//     node --experimental-strip-types ui/src/lib/queue.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant. The bug this guards: an album
// added with "Add to queue" sits at the tail of a playlist queue, and grouping the panel by kind
// instead of by play order drew it under the *playing* playlist's "Next from" heading.
import type { QueueState, SongItem } from './api.ts';
import { moveTarget, queueBlocks } from './queue.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

const song = (id: string, extra: Partial<SongItem> = {}): SongItem => ({
	video_id: id,
	title: id,
	artists: '',
	...extra
});
const q = (
	items: SongItem[],
	currentIndex: number,
	sourceName?: string,
	shuffle = false,
	playedFrom?: number
): QueueState => ({ items, currentIndex, sourceName, shuffle, playedFrom });

// Playing "Afro", an "Add to queue" of "Nightcore Bangers" behind it, autoplay filler behind that.
const added = (id: string) => song(id, { queued_end: true, queued_from: 'Nightcore Bangers' });
let v = queueBlocks(
	q(
		[
			song('a1'),
			song('a2'),
			added('n1'),
			added('n2'),
			song('r1', { autoplay: true })
		],
		0,
		'Afro'
	)
);
ok(v.now?.item.video_id === 'a1', 'the playing track is its own row');
ok(v.blocks.length === 3, 'context, added block, autoplay');
ok(v.blocks[0].heading === 'Next from: Afro', 'the playing playlist keeps its name');
ok(v.blocks[0].rows.length === 1, 'only the rest of Afro is under it');
ok(v.blocks[1].heading === 'Next from: Nightcore Bangers', 'the added block is named after itself');
ok(v.blocks[1].rows.map((r) => r.item.video_id).join() === 'n1,n2', 'the whole added block');
ok(v.blocks[2].autoplay, 'autoplay filler is last and marked');
ok(v.blocks[1].clearable && !v.blocks[0].clearable, 'Clear queue sits on the manual block');
// The row number is the queue index (drawn as i + 1), which is also what play/remove act on.
ok(v.now?.i === 0 && v.blocks[0].rows[0].i === 1 && v.blocks[2].rows[0].i === 4, 'numbered in order');

// "Play next" (queued) stays right behind the current track, ahead of the context.
v = queueBlocks(q([song('a1'), song('p1', { queued: true }), song('a2')], 0, 'Afro'));
ok(v.blocks[0].heading === 'Next in queue', 'a single-song add has no source to name');
ok(v.blocks[1].heading === 'Next from: Afro', 'the context follows it');

// Both kinds of manual add fill one block (#26: they used to split, drawing "Next in queue" twice
// in a row with one track under each).
v = queueBlocks(
	q([song('a1'), song('p1', { queued: true }), song('p2', { queued_end: true })], 0, 'Afro')
);
ok(v.blocks.length === 1, '"Play next" and "Add to queue" are one block');
ok(v.blocks[0].heading === 'Next in queue', 'headed once');

// Two albums added back to back are two blocks, in the order they were added.
v = queueBlocks(
	q(
		[
			song('a1'),
			song('x1', { queued_end: true, queued_from: 'X' }),
			song('y1', { queued_end: true, queued_from: 'Y' })
		],
		0
	)
);
ok(v.blocks.map((b) => b.heading).join() === 'Next from: X,Next from: Y', 'one block each');
ok(v.blocks[0].key !== v.blocks[1].key, 'block keys are distinct (keyed rendering)');

// The one that leaked: advancing a track must not change a block's key. It used to be the first
// row's key, so every track change re-keyed the block and Svelte rebuilt every row under it.
const long = [song('a'), song('b'), song('c'), song('d')];
ok(
	queueBlocks(q(long, 0)).blocks[0].key === queueBlocks(q(long, 1)).blocks[0].key,
	'a block keeps its key when the queue advances'
);

// No source name and nothing manual: the plain "Next up" case still works, as one block.
v = queueBlocks(q([song('a'), song('b'), song('c')], 0));
ok(v.blocks.length === 1 && v.blocks[0].heading === 'Next up', 'unnamed context is one block');

// Played tracks are out of the upcoming blocks, and a repeated track doesn't collide with its
// earlier copy's key.
v = queueBlocks(q([song('dup'), song('now'), song('dup')], 1));
ok(v.blocks[0].rows.length === 1 && v.now?.item.video_id === 'now', 'history is not drawn');
ok(v.blocks[0].rows[0].key === 'dup:1', 'the second copy of a track gets its own key');

// --- previously played, and the prefix that was never played ------------------------------------
// The prefix is not the history: opening a playlist at track 3 leaves two untouched tracks in front
// of the playing one, and the backend says so with `playedFrom`. They are their own run, named
// after where they came from, and drawn without a toggle.
v = queueBlocks(q([song('a'), song('b'), song('now'), song('d')], 2, 'Afro', false, 2));
ok(v.prev.length === 0, 'starting mid-playlist has played nothing');
ok(v.earlier.map((r) => r.item.video_id).join() === 'a,b', 'the untouched prefix is its own run');
ok(v.earlier.map((r) => r.i).join() === '0,1', 'its indices point at the backend queue');
ok(v.earlierHeading === 'Earlier from: Afro', 'named after the playlist it was started inside');

// Two origins in front of the playing track and no one name is the truth (same rule as the
// upcoming blocks, which is why both go through `sharedOrigin`).
v = queueBlocks(q([song('a'), added('n1'), song('now')], 2, 'Afro', false, 2));
ok(v.earlierHeading === 'Earlier', 'mixed origins in the prefix go unnamed');

// Jumping forward counts everything passed over, oldest first, so the last thing heard sits
// directly above the playing track. It moves the border, not the rows: what playback still never
// reached stays in the prefix.
v = queueBlocks(q([song('a'), song('b'), song('c'), song('now')], 3, 'Afro', false, 1));
ok(v.prev.map((r) => r.item.video_id).join() === 'b,c', 'skipped-over tracks count as played');
ok(v.prev.map((r) => r.i).join() === '1,2', 'indices still point at the backend queue');
ok(v.earlier.map((r) => r.item.video_id).join() === 'a', 'only what playback never reached');
// #25: the played rows and the playing row share one run of numbers, so the third track of an
// album reads 3 rather than restarting at 1 under the two above it.
ok(v.now?.i === 3, 'the playing row follows the history it was played after');

// A blob saved before this existed (no `playedFrom`) restores with nothing played, rather than
// claiming the whole prefix was heard.
v = queueBlocks(q([song('a'), song('b'), song('now')], 2));
ok(v.prev.length === 0, 'no playedFrom ⇒ empty history');
ok(v.earlier.length === 2, '...and the whole prefix reads as never played');

// Keys stay unique when the same track shows up in the history and again ahead of the playing one.
v = queueBlocks(q([song('dup'), song('now'), song('dup')], 1, undefined, false, 0));
ok(v.prev[0].key === 'dup:0' && v.blocks[0].rows[0].key === 'dup:1', 'each copy keeps its own key');

// An empty queue draws nothing rather than throwing.
v = queueBlocks(q([], 0));
ok(v.now === null && v.blocks.length === 0, 'empty queue');

// --- shuffle: the interleaved run is one block ------------------------------------------------
// Shuffle mixes the playing playlist with everything added to it, so origins alternate row by row.
// Split per origin, that's a heading above every single track (what the owner reported).
const mixed = [
	song('now'),
	song('a1'),
	added('n1'),
	song('a2'),
	added('n2'),
	song('r1', { autoplay: true })
];
v = queueBlocks(q(mixed, 0, 'Afro', true));
ok(v.blocks.length === 2, 'the shuffled run is one block, autoplay still its own');
ok(v.blocks[0].heading === 'Next up', 'two origins in one block ⇒ neither name is the truth');
ok(v.blocks[0].rows.length === 4, 'every shuffled track is in it');
ok(v.blocks[0].clearable, 'Clear queue is offered even though the block opens on a playlist track');
ok(v.blocks[1].autoplay, 'filler is not merged into the shuffle');

// Same queue unshuffled: the blocks come back (this is what turning shuffle off restores).
v = queueBlocks(q([song('now'), song('a1'), song('a2'), added('n1'), added('n2')], 0, 'Afro'));
ok(
	v.blocks.map((b) => b.heading).join() === 'Next from: Afro,Next from: Nightcore Bangers',
	'unshuffled ⇒ back to one block per origin'
);

// Shuffling a plain playlist (nothing added) still names it — no regression on the common case.
v = queueBlocks(q([song('now'), song('a1'), song('a2')], 0, 'Afro', true));
ok(v.blocks.length === 1 && v.blocks[0].heading === 'Next from: Afro', 'one origin keeps its name');

// The "Play next" block is pinned ahead of the shuffle, so it keeps its own heading.
v = queueBlocks(
	q([song('now'), song('p1', { queued: true }), song('a1'), added('n1')], 0, 'Afro', true)
);
ok(v.blocks[0].heading === 'Next in queue' && v.blocks[0].rows.length === 1, 'pinned block stands');
ok(v.blocks[1].heading === 'Next up' && v.blocks[1].rows.length === 2, 'the rest is the shuffle');
ok(v.blocks[0].clearable && !v.blocks[1].clearable, 'Clear queue on the first manual block only');

// Only added playlists left in the shuffled run (the context ran out): still the queue, unnamed.
v = queueBlocks(
	q([song('now'), added('n1'), song('x1', { queued_end: true, queued_from: 'X' })], 0, 'Afro', true)
);
ok(v.blocks[0].heading === 'Next in queue', 'all manual, two origins ⇒ "Next in queue"');

// --- drag to reorder ---------------------------------------------------------------------------
// Dropping in front of a row *below* you lands one slot earlier once you're out of the way.
ok(moveTarget(2, 6) === 5, 'dragging down: the target shifts up by the hole left behind');
ok(moveTarget(6, 2) === 2, 'dragging up: the target is where the bar is');
ok(moveTarget(2, 9) === 8, 'dropping past the last row appends');
ok(moveTarget(2, 2) === null, 'in front of yourself is a no-op');
ok(moveTarget(2, 3) === null, 'and so is just behind yourself');

console.log('ok');
