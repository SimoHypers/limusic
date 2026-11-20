import * as api from './api';
import { playback } from './player.svelte';

/** "3:21" / "1:02:03" → seconds. */
export function durationSecs(d?: string): number | undefined {
	if (!d) return undefined;
	const parts = d.split(':').map(Number);
	if (!parts.length || parts.some(Number.isNaN)) return undefined;
	return parts.reduce((a, b) => a * 60 + b, 0);
}

/** The playing song's lyrics in this window, by videoId: what the line under the now-playing cover
 *  draws. Every LyricsView writes what it fetched and what its source picker changed (another
 *  source, a timing nudge), so the line never disagrees with the lyrics tab beside it. */
export const nowLyrics = $state({ id: '', lyrics: null as api.Lyrics | null });

/** A late answer for a song that has since moved on is dropped. */
export function setNowLyrics(id: string, lyrics: api.Lyrics | null) {
	if (playback.now?.videoId !== id) return;
	nowLyrics.id = id;
	nowLyrics.lyrics = lyrics;
}

let pending: { id: string; p: Promise<api.Lyrics | null> } | undefined;

/** The song's lyrics, one `get_lyrics` however many views ask at once: the backend shares no fetch
 *  in flight, so the line and the lyrics tab opening on a new song walked the provider list twice. */
export function lyricsFor(now: api.NowPlaying): Promise<api.Lyrics | null> {
	if (pending?.id === now.videoId) return pending.p;
	const p = api.getLyrics({
		videoId: now.videoId,
		title: now.title,
		artists: now.artists,
		// From now-playing, not the queue row: on a gapless advance the queue event lands after
		// this one, and the previous song's album makes Boidu and LRCLIB miss.
		album: now.album ?? undefined,
		// The track's own length, NOT playback.duration, which still holds the previous track's
		// value for a moment after a track change.
		duration: durationSecs(now.duration)
	});
	const done = () => {
		if (pending?.p === p) pending = undefined;
	};
	p.then(done, done);
	pending = { id: now.videoId, p };
	return p;
}
