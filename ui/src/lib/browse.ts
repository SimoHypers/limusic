// What a BrowseItem does when you click it. One implementation for every surface that renders a
// browse result — cards, the home recents rail, anything after them — so "open" and "play whole
// thing" can never drift apart between two components.
import { goto } from '$app/navigation';
import * as api from './api';
import type { ArtistPage, BrowseItem, SongItem } from './api';
import { getCached, putCached } from './pagecache';
import { t } from './i18n.svelte';
import { enqueue, openAddManyToPlaylist, playFrom, playSong, toast, touchPick } from './player.svelte';

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
	// Without it the card's ⋯ menu, and the player bar once it plays, have no "Go to album".
	album_id: i.albumId,
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

/** An artist's page through the browse cache, under the artist route's own key, so a visit there and
 *  a section showing their photo share one fetch. Null when the channel can't be had. */
export async function artistPage(id: string): Promise<ArtistPage | null> {
	const key = `artist:${id}`;
	const hit = getCached<ArtistPage>(key);
	if (hit) return hit;
	try {
		const page = await api.getArtist(id);
		putCached(key, page);
		return page;
	} catch {
		return null;
	}
}

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
 * first, so callers own the in-flight state (the pulsing button). `shuffle` starts it shuffled
 * (the card menu's shuffle button). Never throws — a failure toasts and the user stays where they
 * were.
 */
export async function playItem(item: BrowseItem, shuffle = false): Promise<void> {
	touchPick(item.id);
	if (item.kind === 'song') {
		playSong(asSong(item));
		return;
	}
	try {
		if (item.kind === 'album') {
			const album = await api.getAlbum(item.id);
			await playFrom(item, album.items, null, album.playlistId ?? undefined, shuffle);
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
				shuffle,
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
 * Save the whole thing to a playlist without opening it: fetch the tracks, then hand them to the
 * picker. Same contract as `enqueueItem` - callers own the in-flight state, and a failure toasts
 * rather than throwing.
 *
 * ponytail: the first page only. `enqueueItem` passes a playlist's `continuation` to the backend,
 * which walks the rest into the queue; the add path has no equivalent, so a very long playlist
 * copies the tracks that came back, and says so rather than quietly dropping the rest. Walk the
 * pages here if someone asks for the whole 500.
 */
export async function addItemToPlaylist(item: BrowseItem): Promise<void> {
	if (item.kind === 'song') {
		openAddManyToPlaylist([asSong(item)]);
		return;
	}
	try {
		const src =
			item.kind === 'album' ? await api.getAlbum(item.id) : await api.getPlaylist(item.id);
		if (src.continuation) toast.error(t('toasts.partial_playlist_added'));
		openAddManyToPlaylist(src.items);
	} catch {
		toast.error(t('toasts.could_not_add'));
	}
}

/**
 * The second line of a typeahead row. A song's subtitle is its artists alone, so it gets "Song"
 * in front. Every other row's already opens with YouTube's own "Album" or "Artist", and prefixing
 * our label as well read "Album • Album • Édith Piaf".
 */
export function rowMeta(i: BrowseItem): string {
	if (i.kind === 'song') return [t('common.song_singular'), i.subtitle].filter(Boolean).join(' • ');
	const kind: Record<string, string> = {
		album: t('common.album_singular'),
		artist: t('common.artist_singular'),
		playlist: t('common.playlist_singular')
	};
	return i.subtitle || kind[i.kind] || '';
}
