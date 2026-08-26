// What a BrowseItem does when you click it. One implementation for every surface that renders a
// browse result — cards, the home recents rail, anything after them — so "open" and "play whole
// thing" can never drift apart between two components.
import { goto } from '$app/navigation';
import * as api from './api';
import type { BrowseItem, SearchResults, SongItem } from './api';
import { getCached, putCached } from './pagecache';
import { t } from './i18n.svelte';
import { enqueue, playFrom, playSong, toast, touchPick } from './player.svelte';

/**
 * A song card carries everything a queue entry needs; the ⋯ menus take this shape. The one mapping
 * for every card surface (search rows, home shelves, carousels): a card's `subtitle` is already the
 * artist alone for songs, and that string is what the player bar shows and what gets scrobbled, so
 * a second copy of this that drifts is a wrong scrobble.
 */
export const asSong = (i: BrowseItem): SongItem => ({
	video_id: i.id,
	title: i.title,
	artists: i.subtitle ?? '',
	// The links belong to the same line as `artists`; without them a card played from search or a
	// home shelf reaches the player bar (and the row menu) with an artist you can't click.
	artist_runs: i.artistRuns,
	artist_id: i.artistRuns?.find((r) => r.id)?.id,
	duration: i.duration,
	play_count: i.playCount,
	thumbnail: i.thumbnail,
	explicit: i.explicit,
	// Without this a card played from a shelf reaches the orchestrator as an ordinary track and
	// gets the anonymous fallback chain, which can never stream an upload.
	is_upload: i.isUpload
});

/** Where a non-song item lives. Songs have no page — they play. */
export const hrefFor = (i: BrowseItem): string =>
	// A local artist is drawn as an artist (circle, no play button) but opens the album route:
	// there is no channel behind files on disk, so the real artist page has nothing to show.
	i.id.startsWith(api.LOCAL_ARTIST_PREFIX)
		? `/album/${encodeURIComponent(i.id)}`
		: i.kind === 'artist'
			? `/artist/${encodeURIComponent(i.id)}`
			: i.kind === 'album'
				? `/album/${encodeURIComponent(i.id)}`
				: `/playlist/${encodeURIComponent(i.id)}`;

/** Primary click: a song plays, everything else opens its page. */
export function openItem(item: BrowseItem): void {
	// A click counts as "used" for Shortcuts eviction wherever the item was rendered. No-op unless
	// this item is actually on the grid.
	touchPick(item.id);
	if (item.kind === 'song') playSong(asSong(item));
	else goto(hrefFor(item));
}

/**
 * Play the whole thing without opening it. A song is immediate; a playlist/album has to be fetched
 * first, so callers own the in-flight state (the pulsing button). Never throws — a failure toasts
 * and the user stays where they were.
 */
export async function playItem(item: BrowseItem): Promise<void> {
	touchPick(item.id);
	if (item.kind === 'song') {
		playSong(asSong(item));
		return;
	}
	try {
		if (item.kind === 'album') {
			const album = await api.getAlbum(item.id);
			await playFrom(item, album.items, null, album.playlistId ?? undefined);
		} else {
			const pl = await api.getPlaylist(item.id);
			// `sourceId` seeds autoplay off that playlist's radio. On Repeat is local, so there is
			// no radio for it. Pass none and let autoplay seed off the last video instead. The
			// continuation hands the rest of the playlist (past this first page) to the backend.
			await playFrom(
				item,
				pl.items,
				null,
				item.id === api.ON_REPEAT_ID ? undefined : item.id,
				undefined,
				pl.continuation
			);
		}
	} catch {
		toast.error(t('toasts.could_not_play'));
	}
}

/**
 * Queue the whole thing without opening it: "Play next" (`next`) or "Add to queue". A song is one
 * track; an album/playlist is fetched first, so callers own the in-flight state (same contract as
 * `playItem`). Never throws — a failure toasts.
 *
 * The playlist's next-page token rides along: the backend walks the rest of a long playlist into
 * the queue in the background rather than the UI chaining pages before anything is queued.
 */
export async function enqueueItem(item: BrowseItem, next: boolean): Promise<void> {
	if (item.kind === 'song') {
		await enqueue([asSong(item)], next);
		return;
	}
	try {
		if (item.kind === 'album') {
			const album = await api.getAlbum(item.id);
			await enqueue(album.items, next, album.title ?? item.title, album.continuation);
		} else {
			const pl = await api.getPlaylist(item.id);
			await enqueue(pl.items, next, pl.title ?? item.title, pl.continuation);
		}
	} catch {
		toast.error(t('toasts.could_not_queue'));
	}
}

/**
 * The handful of rows a typeahead shows for a query: one top hit, then a spread across the
 * categories rather than six songs. Cache-first, and it writes the same `search:<q>` key the search
 * page reads, so previewing a query and then running it doesn't search twice. Shared by the search
 * field (SearchSuggest) and the Ctrl+K palette, which is what keeps the two showing the same rows.
 */
export async function searchPreview(q: string): Promise<BrowseItem[]> {
	const key = `search:${q}`;
	let res = getCached<SearchResults>(key);
	if (!res) {
		res = await api.searchAll(q);
		putCached(key, res);
	}
	const out: BrowseItem[] = [];
	const seen = new Set<string>();
	const take = (from: BrowseItem[], n: number) => {
		for (const i of from) {
			if (n <= 0) break;
			if (seen.has(i.id)) continue;
			seen.add(i.id);
			out.push(i);
			n--;
		}
	};
	take(res.top, 1);
	take(res.songs, 3);
	take(res.artists, 1);
	take(res.albums, 1);
	take(res.playlists, 1);
	return out;
}
