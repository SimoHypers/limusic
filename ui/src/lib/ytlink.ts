// A pasted YouTube / YouTube Music link turned into something Limusic can open (#63). Some
// playlists are only ever reachable by URL: they don't show up in search and aren't in the
// library, so without this there is no way into them.
//
// Pure and route-free on purpose — the dialog decides what "open" means (see LinkDialog).
import type { BrowseItem } from './api';
import { parseYtStart } from './timecode.ts';

/** `start` is the `t=` / `start=` offset in seconds, and only ever set on a song. */
export type LinkTarget = { kind: BrowseItem['kind']; id: string; start?: number };

/** A song target, carrying the link's start time when it has a usable one. */
function song(id: string, u: URL): LinkTarget {
	const start = parseYtStart(u.searchParams.get('t') ?? u.searchParams.get('start'));
	return start ? { kind: 'song', id, start } : { kind: 'song', id };
}

/**
 * `null` for anything that isn't a YouTube link we know how to open. Accepts a bare host too
 * ("music.youtube.com/playlist?list=…"), because that is what a copy out of a chat message often
 * looks like.
 */
export function parseYtLink(input: string): LinkTarget | null {
	const text = input.trim();
	if (!text) return null;
	let u: URL;
	try {
		u = new URL(/^https?:\/\//i.test(text) ? text : `https://${text}`);
	} catch {
		return null;
	}
	// music., www., m., or none of them.
	if (!/(^|\.)(youtube\.com|youtu\.be)$/.test(u.hostname)) return null;

	const [seg, rest] = u.pathname.replace(/^\/+/, '').split('/');
	// youtu.be/<videoId>
	if (u.hostname.endsWith('youtu.be')) return seg ? song(seg, u) : null;

	// A watch link with both a video and a list is a track being played out of a playlist; the
	// track is what was shared.
	const v = u.searchParams.get('v');
	if (v) return song(v, u);
	const list = u.searchParams.get('list');
	// Playlist browse ids carry a `VL` prefix that the URL doesn't (the mirror of ShareDialog).
	// An album shared as its `OLAK5uy_…` audio playlist opens as a playlist of the same tracks;
	// resolving it back to the `MPRE…` album page would cost a lookup for a near-identical page.
	if (list) return { kind: 'playlist', id: `VL${list.replace(/^VL/, '')}` };

	if (seg === 'channel' && rest) return { kind: 'artist', id: rest };
	// YTM's own pages: albums, playlists and artists all live under /browse, told apart by the id.
	if (seg === 'browse' && rest) {
		if (rest.startsWith('MPRE')) return { kind: 'album', id: rest };
		if (rest.startsWith('VL')) return { kind: 'playlist', id: rest };
		if (rest.startsWith('UC')) return { kind: 'artist', id: rest };
	}
	return null;
}
