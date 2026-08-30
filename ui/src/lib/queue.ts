// How the queue panel cuts one flat queue into blocks. Kept out of the component so it can be
// checked without a DOM (`queue.check.ts`) — the ordering bug it exists to prevent (an added
// playlist drawn under the playing playlist's name) is invisible until you look at real data.
import type { QueueState, SongItem } from './api';

export interface QueueRow {
	item: SongItem;
	/** video_id + occurrence, so `animate:flip` slides rows instead of recreating them. */
	key: string;
	/** Index in the backend queue — what play/remove act on, and what the row is numbered by. */
	i: number;
}

export interface QueueBlock {
	/** Stable id for keyed rendering: the block's kind plus its ordinal among blocks of that kind.
	 *  Deliberately not the first row's key. That changes on every track advance (the row it names
	 *  is the one that just started playing), which changed the block's key, which made Svelte tear
	 *  down and rebuild the entire block, every track, for as long as the panel stayed open. WebKit
	 *  does not give all of that back: it cost ~40 KB per rendered row per track, which is how a
	 *  two-hour listen reached 3.5 GB. */
	key: string;
	/** What groups a run: same kind ⇒ same block. */
	kind: string;
	heading: string;
	autoplay: boolean;
	/** The "Clear queue" button goes on the first manual block only. */
	clearable: boolean;
	rows: QueueRow[];
}

/**
 * Everything in front of the playing track, then the playing track, then the upcoming queue in play
 * order, split wherever the tracks change origin: a manual block ("Play next" / "Add to queue",
 * headed by what it was added from), the playing context ("Next from: …"), autoplay's continuation.
 *
 * What sits in front of the playing track is two different things, and `playedFrom` is the border
 * (`state.rs`). `prev` is the history: what was actually heard or skipped past, oldest first, so the
 * last thing heard sits directly above the playing track. `earlier` is the rest of the queue in
 * front of it, which playback never reached: start an album at track 4 and the backend still queues
 * tracks 1-3, untouched. The panel draws `earlier` and hides `prev` behind a toggle, because the
 * prefix is bounded by the playlist and shrinks every time you press previous, while history is
 * unbounded and grows all session.
 *
 * Play order, not kind order: grouping by kind would draw an "Add to queue" block that sits at the
 * tail under the playing playlist's heading, naming a playlist those tracks never came from.
 *
 * Shuffle collapses that split. The backend deliberately interleaves everything it shuffles, so
 * origins alternate track by track and a per-origin split degenerates into one heading per row —
 * the shuffled run becomes a single block instead. What shuffle leaves alone keeps its own: the
 * pinned "Play next" block ahead of it, autoplay's filler behind it. Turning shuffle off restores
 * the real order, and with it the blocks.
 */
export function queueBlocks(q: QueueState): {
	earlier: QueueRow[];
	earlierHeading: string;
	prev: QueueRow[];
	now: QueueRow | null;
	blocks: QueueBlock[];
} {
	const { items, currentIndex, sourceName } = q;
	const playedFrom = Math.min(q.playedFrom ?? currentIndex, currentIndex);
	const seen = new Map<string, number>();
	const row = (i: number): QueueRow => {
		const item = items[i];
		const occ = seen.get(item.video_id) ?? 0;
		seen.set(item.video_id, occ + 1);
		return { item, key: `${item.video_id}:${occ}`, i };
	};
	// Occurrence counting must walk the whole prefix, not just the played part of it, or a repeated
	// track's key would collide with an earlier copy that isn't on screen.
	const earlier: QueueRow[] = [];
	const prev: QueueRow[] = [];
	for (let i = 0; i < currentIndex; i++) {
		const r = row(i);
		(i >= playedFrom ? prev : earlier).push(r);
	}
	// Rows are drawn in queue order throughout, so the number is the queue index and nothing else:
	// counting per run restarted the playing track at 1 under the two tracks already heard (#25).
	const now = items[currentIndex] ? row(currentIndex) : null;

	// Where the shuffled run starts: shuffle pins the leading "Play next" block in place and
	// shuffles everything after it.
	let shuffledFrom = items.length;
	if (q.shuffle) {
		shuffledFrom = currentIndex + 1;
		while (items[shuffledFrom]?.queued) shuffledFrom++;
	}

	const blocks: QueueBlock[] = [];
	const kindSeen = new Map<string, number>();
	let cleared = false;
	for (let i = currentIndex + 1; i < items.length; i++) {
		const r = row(i);
		const it = r.item;
		const manual = !!(it.queued || it.queued_end);
		// `queued_from` splits two albums added back to back, and keeps a continuation walked in
		// later with the block it belongs to. "Play next" and "Add to queue" do *not* split: they
		// fill the two ends of one block, and splitting drew "Next in queue" twice in a row.
		const kind = it.autoplay
			? 'auto'
			: i >= shuffledFrom
				? 'shuffled'
				: manual
					? `manual:${it.queued_from ?? ''}`
					: 'context';
		const last = blocks.at(-1);
		if (last?.kind === kind) {
			last.rows.push(r);
			continue;
		}
		const ord = kindSeen.get(kind) ?? 0;
		kindSeen.set(kind, ord + 1);
		blocks.push({
			key: `${kind}#${ord}`,
			kind,
			heading: '',
			autoplay: !!it.autoplay,
			clearable: false,
			rows: [r]
		});
	}
	// Both need the whole block, not its first row: a shuffled run only has one origin to name if
	// every track in it agrees, and "Clear queue" belongs on the first block holding anything the
	// user queued — under shuffle that's a mixed block whose first row may well be a playlist track.
	for (const block of blocks) {
		block.heading = headingFor(block, sourceName);
		const manual = block.rows.some((r) => r.item.queued || r.item.queued_end);
		block.clearable = manual && !cleared;
		if (block.clearable) cleared = true;
	}
	const earlierName = sharedOrigin(earlier, sourceName);
	return {
		earlier,
		earlierHeading: earlierName ? `Earlier from: ${earlierName}` : 'Earlier',
		prev,
		now,
		blocks
	};
}

/**
 * Drag-to-reorder: the backend index a row dragged from `from` must end up at, given it was dropped
 * *in front of* the row at `dropAt`. Dropping in front of a row below yourself lands one slot
 * earlier once you're out of the way, which is the off-by-one every hand-rolled DnD ships with.
 * `null` when the drop is a no-op (onto itself, or immediately after itself).
 */
export function moveTarget(from: number, dropAt: number): number | null {
	const to = dropAt > from ? dropAt - 1 : dropAt;
	return to === from ? null : to;
}

/**
 * The one origin a run of rows shares, or null when they disagree (or have none to give).
 *
 * What a single row offers: what it was added from, the queue's own source for a plain context
 * track, nothing for a single-song add.
 */
function sharedOrigin(rows: QueueRow[], sourceName?: string | null): string | null {
	const names = new Set(
		rows.map(
			(r) => r.item.queued_from ?? (r.item.queued || r.item.queued_end ? '' : (sourceName ?? ''))
		)
	);
	const [name] = names;
	return names.size === 1 && name ? name : null;
}

/** The name a block goes under: its one origin if its tracks share one, else a neutral label. */
function headingFor(block: QueueBlock, sourceName?: string | null): string {
	if (block.autoplay) return 'Autoplay';
	const name = sharedOrigin(block.rows, sourceName);
	if (name) return `Next from: ${name}`;
	return block.rows.every((r) => r.item.queued || r.item.queued_end) ? 'Next in queue' : 'Next up';
}
