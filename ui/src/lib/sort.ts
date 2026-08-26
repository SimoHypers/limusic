// Playlist sorting. Pure so it can be checked without a DOM (`sort.check.ts`).
//
// YouTube sorts a playlist itself, and remembers the choice, so wherever it offers a sort menu the
// page asks it for an ordered list instead of ordering one here (see `PlaylistPage.sortMenu`). The
// list then reads the same way in Limusic, in YouTube Music, and in any other client on the
// account. `sortSongs` below is the fallback for the two things YouTube cannot do — rank by our own
// play counts, and reverse a playlist's stored order — plus lists it offers no menu for at all.
import type { ServerSort, SongItem } from './api';

export type SortKey = ServerSort | 'plays';

/** `top` is YouTube's "Top voted": a list can already be in it, so the key has to round-trip, but
 *  it is deliberately not offered here — there is no way to compute it for a list we sort locally. */
export const SORTS: SortKey[] = ['default', 'newest', 'oldest', 'title', 'artist', 'album', 'plays'];

/**
 * The nearest order YouTube can store for this choice, or `null` when it has none at all.
 *
 * Direction only exists on the date sorts, where reversing one simply *is* the other. There is no
 * descending Title/Artist/Album (`playlistDynamicSortPreference` takes 1..3 and nothing else) and
 * no reversed manual order, so a reversed sort stores the ascending one and keeps the reverse to
 * itself: YouTube Music then shows the same sort the right way up, which beats leaving it on an
 * unrelated order. Only "Most played" maps to nothing.
 */
export function persistedSort(sort: SortKey, desc: boolean): ServerSort | null {
	if (sort === 'plays') return null;
	if (sort === 'newest' || sort === 'oldest')
		return (sort === 'newest') !== desc ? 'newest' : 'oldest';
	return sort;
}

/**
 * Whether YouTube's stored order reproduces this choice exactly, direction included — i.e. whether
 * there is anything left for this machine to remember. Reversing a date sort just *is* the other
 * date sort, so those count; every other reverse, and "Most played", do not.
 */
export const storedExactly = (sort: SortKey, desc: boolean): boolean =>
	sort !== 'plays' && (!desc || sort === 'newest' || sort === 'oldest');

/**
 * What to ask YouTube for when the page wants `sort`. "Most played" has no server equivalent, so
 * it ranks the playlist's own order rather than whatever order the account happens to be on.
 */
export const fetchSort = (sort: SortKey): ServerSort => (sort === 'plays' ? 'default' : sort);

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

/**
 * Sort a playlist's tracks. Returns the input array untouched when there is nothing to do, a new
 * array otherwise — the page compares identity to skip re-rendering.
 *
 * `baseNewestFirst` says which end of the incoming order is the newest addition. Playlist rows
 * carry no timestamp, so newest/oldest is add order, i.e. the position YouTube hands them back in:
 * Liked Music arrives most-recently-liked first, every other playlist arrives oldest-added first.
 *
 * `desc` reverses whatever the key ordered, and nothing else: ties still fall back to playlist
 * order, and a track with no album still sorts last rather than jumping to the top.
 *
 * `plays` is the local listening history (videoId → count, see `api.getPlayCounts`); a track it
 * doesn't mention counts as zero. It orders most-played first, so descending means least-played
 * first, the same way "Newest first" reads.
 */
export function sortSongs(
	items: SongItem[],
	sort: SortKey,
	baseNewestFirst: boolean,
	desc = false,
	plays: Record<string, number> = {}
): SongItem[] {
	if (items.length < 2) return items;
	// "Top voted" is YouTube's ranking and there is nothing local to recompute it from, so off the
	// server it behaves like Default rather than quietly ordering by something else.
	if (sort === 'default' || sort === 'top') return desc ? items.slice().reverse() : items;
	if (sort === 'newest' || sort === 'oldest')
		return ((sort === 'newest') !== desc) === baseNewestFirst ? items : items.slice().reverse();
	const dir = desc ? -1 : 1;
	// Ties (everything unplayed, most of a big playlist) keep playlist order.
	if (sort === 'plays')
		return items
			.slice()
			.sort((a, b) => dir * ((plays[b.video_id] ?? 0) - (plays[a.video_id] ?? 0)));
	// Album is a grouping, so inside one album the playlist's own order stands (Array#sort is
	// stable): an album added in one go keeps its track order, which alphabetising would destroy.
	// Descending reverses the albums, not the tracks within one.
	if (sort === 'album')
		return items
			.slice()
			.sort(
				(a, b) =>
					Number(!a.album) - Number(!b.album) ||
					dir * collator.compare(a.album ?? '', b.album ?? '')
			);
	// Title orders an artist's block; on a title sort the tiebreak is a no-op.
	const key = (t: SongItem) => (sort === 'title' ? t.title : t.artists);
	return items
		.slice()
		.sort(
			(a, b) => dir * (collator.compare(key(a), key(b)) || collator.compare(a.title, b.title))
		);
}
