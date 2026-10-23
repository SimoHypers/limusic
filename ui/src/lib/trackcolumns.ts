// The optional columns of a playlist or album track list, and which of them a list can fill. Pure,
// so it can be checked without a DOM (`trackcolumns.check.ts`). The chosen set and the header that
// picks it live in `TrackColumns.svelte`.
//
// Only what the rows really carry is offered: a column every row would leave blank is worse than
// no column. Date added comes off YouTube's playlist rows (`voteSortValue`, see `added_at` in
// metadata.rs) and off the rows of a playlist kept on this machine. Nothing offers a release date:
// no row carries one.
import type { SongItem } from './api';

export type TrackColumn = 'album' | 'added' | 'by' | 'plays' | 'mine' | 'duration';

/** Picker order, which is also the cells' order left to right. Duration stays last, by the ⋯. */
export const COLUMNS: TrackColumn[] = ['album', 'added', 'by', 'plays', 'mine', 'duration'];

/** A fresh install. Added by and Plays keep what the rows always showed (a collaborative
 *  playlist's avatars, an album's play counts); Your plays is opt-in. */
export const DEFAULT_COLUMNS: TrackColumn[] = ['album', 'added', 'by', 'plays', 'duration'];

/*
 * The row, left to right, laid out the same way by the header and by every row, so the labels sit
 * over their cells without a grid (the rows are separate elements, and windowed):
 *
 *   # | art + title / artist  (TITLE) | album (shares the rest 3:4 with the title) | fixed cells | END
 *
 * Title and album split what is left in proportion, so the album starts at the same place on every
 * row however long the names run. Everything that varies row to row (the saved mark, the thumbs, a
 * local file having neither) lives in END, which is one fixed width with its contents pushed right,
 * so it can never shift a column. Duration and the ⋯ end every row there, as before.
 *
 * Which columns fit goes by the list's own width (`fitColumns`), so it follows the sidebar as well
 * as the window.
 */
export const TITLE = 'flex-[4_1_0%]';
export const END = 'w-44 justify-end';
export const CELL: Record<Exclude<TrackColumn, 'duration'>, string> = {
	album: 'min-w-0 flex-[3_1_0%]',
	added: 'w-28 shrink-0',
	by: 'w-16 shrink-0',
	plays: 'w-14 shrink-0 text-right',
	mine: 'w-16 shrink-0 text-right'
};

/** The narrowest list, in px, each column still shows in (40, 44 and 52rem). They keep the title
 *  readable with every column on, and give way in the order the columns matter: Date added first,
 *  then Your plays, then Album. */
const MIN_PX: Partial<Record<TrackColumn, number>> = { album: 640, mine: 704, added: 832 };

/**
 * The columns that fit a list `px` wide.
 *
 * Measured, not a `@container` query: a size container around the playlist's windowed rows made
 * WebKitGTK lose the scroll position near the bottom. Each window shift got a first layout pass with
 * the rows' old padding and the new rows not counted, so the list came up short for that pass and
 * the scroller clamped to it: the bottom bounced under the wheel and a jump to the end landed rows
 * short. `ui/perf/endprobe.py` measures it.
 */
export const fitColumns = (cols: TrackColumn[], px: number): TrackColumn[] =>
	cols.filter((c) => px >= (MIN_PX[c] ?? 0));

/**
 * The columns this list can fill. An album page leaves Album out: every row would repeat the title
 * above it.
 */
export function availableColumns(items: SongItem[], albumPage = false): TrackColumn[] {
	return COLUMNS.filter((c) => {
		if (c === 'album') return !albumPage && items.some((i) => i.album);
		if (c === 'added') return items.some((i) => i.added_at);
		if (c === 'by') return items.some((i) => i.added_by_avatars?.length);
		if (c === 'plays') return items.some((i) => i.play_count);
		return true; // Your plays is our own history, and every row has a length
	});
}

/** A stored choice, read back defensively: an unknown id is dropped, the order is ours. */
export function parseColumns(raw: string | null): TrackColumn[] {
	try {
		const v: unknown = JSON.parse(raw ?? 'null');
		if (Array.isArray(v)) return COLUMNS.filter((c) => v.includes(c));
	} catch {
		/* a broken value falls back to the default */
	}
	return DEFAULT_COLUMNS;
}

const formats = new Map<string, Intl.DateTimeFormat>();
const format = (key: string, opts: Intl.DateTimeFormatOptions, tag: string): Intl.DateTimeFormat => {
	let f = formats.get(key + tag);
	if (!f) formats.set(key + tag, (f = new Intl.DateTimeFormat(tag, opts)));
	return f;
};

/** The day it was added ("Aug 17, 2025"), always: a distance like "3 weeks ago" reads differently
 *  from one row to the next and from one day to the next. `at` is unix seconds. */
export const addedLabel = (at: number, tag: string): string =>
	format('d', { dateStyle: 'medium' }, tag).format(at * 1000);

/** The full date and time, for the cell's tooltip. */
export const addedTitle = (at: number, tag: string): string =>
	format('f', { dateStyle: 'long', timeStyle: 'short' }, tag).format(at * 1000);
