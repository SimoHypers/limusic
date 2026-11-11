// Contributing to Limusic Lyrics from the app: the one flag that shows the lyric editor's entry
// points, and the one function the editor submits through.
//
// Both are placeholders for the account work that comes next. The editor itself
// (`routes/lyrics-editor`) is complete without them: it builds and keeps a draft, and hands the
// finished lyric to `submitLyrics`.
import type { LyricLine } from './api';

export const limusic = $state({
	/** Whether "Sync lyrics" / "Improve these lyrics" is offered at all. Off in every release
	 *  build until the service's account endpoints exist and answer: it is to become "`GET
	 *  /v1/auth/providers` listed at least one provider", checked once per start in the background.
	 *  On in a dev build so the editor can be opened from `cargo tauri dev`. */
	available: import.meta.env.DEV
});

/** A finished lyric on its way to `POST /v1/lyrics/{video_id}/revisions`. */
export interface LyricsSubmission {
	videoId: string;
	/** `buildLines`' output: canonical, sorted, word-synced. */
	lines: LyricLine[];
	title: string;
	artist: string;
	album?: string;
	/** Seconds. */
	duration?: number;
	/** The live revision the draft was started from, `null` when the song had none. The editor
	 *  only carries it: nothing fills it in yet (`lyrics_editor_start` is where it should come from). */
	baseRevision: number | null;
}

/** What became of a submission. Only `unavailable` exists so far; the service's own answers
 *  (published, pending, stale, unchanged, invalid, signed out, rate limited) join it here. */
export type SubmitOutcome = { kind: 'unavailable' };

/** THE SUBMIT SEAM. The editor calls this and nothing else to send a lyric. */
export async function submitLyrics(_submission: LyricsSubmission): Promise<SubmitOutcome> {
	return { kind: 'unavailable' };
}
