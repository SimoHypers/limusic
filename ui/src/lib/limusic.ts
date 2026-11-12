// What Limusic Lyrics' answers mean in the UI. Pure, so `limusic.check.ts` can run it. The state
// and the calls are in `limusic.svelte.ts`.
import type { LimusicLyricState, LimusicReportReason, LimusicSubmitted } from './api';
import type { TranslationKey } from './i18n.svelte';

/** Where a sign-in provider's name is spelled. The service's ids are lower case. */
export const PROVIDER_NAMES: Record<string, string> = { discord: 'Discord', github: 'GitHub' };
export const providerName = (id: string) => PROVIDER_NAMES[id] ?? id.charAt(0).toUpperCase() + id.slice(1);

export interface Message {
	/** Keys in `en.json`: a short title, and the sentence under it. */
	title: TranslationKey;
	key: TranslationKey;
	params?: Record<string, string>;
	kind: 'success' | 'info' | 'error';
	/** Done with the draft: it is on the service now. */
	sent?: boolean;
	/** The song changed under the draft: the user picks the new version or keeps theirs. */
	stale?: boolean;
}

/** Every answer the service documents for a submission, in words. */
export function submitMessage(o: LimusicSubmitted): Message {
	const E = 'lyrics_editor';
	if (o.kind === 'published') return { title: `${E}.sent_published_title`, key: `${E}.sent_published`, kind: 'success', sent: true };
	if (o.kind === 'pending') return { title: `${E}.sent_pending_title`, key: `${E}.sent_pending`, kind: 'success', sent: true };
	switch (o.code) {
		case 'unauthorized':
			return { title: `${E}.err_signed_out_title`, key: `${E}.err_signed_out`, kind: 'info' };
		case 'banned':
			return { title: `${E}.err_banned_title`, key: `${E}.err_banned`, kind: 'error' };
		case 'invalid':
			return o.detail
				? { title: `${E}.err_invalid_title`, key: `${E}.err_invalid_detail`, params: { detail: o.detail }, kind: 'error' }
				: { title: `${E}.err_invalid_title`, key: `${E}.err_invalid`, kind: 'error' };
		case 'stale':
			return { title: `${E}.err_stale_title`, key: `${E}.err_stale`, kind: 'error', stale: true };
		case 'unchanged':
			return { title: `${E}.err_unchanged_title`, key: `${E}.err_unchanged`, kind: 'info' };
		case 'rate_limited':
			return { title: `${E}.err_rate_limited_title`, key: `${E}.err_rate_limited`, kind: 'error' };
		default:
			return { title: `${E}.err_unavailable_title`, key: `${E}.err_unavailable`, kind: 'error' };
	}
}

/** The contributors of a community lyric as the locale lists names: "carla", "carla and moss". */
export const creditNames = (authors: string[], locale: string) =>
	new Intl.ListFormat(locale, { style: 'long', type: 'conjunction' }).format(authors);

/** What a click on a thumb asks for: the same thumb again takes the vote back. */
export const nextVote = (mine: -1 | 0 | 1, clicked: -1 | 1): -1 | 0 | 1 => (mine === clicked ? 0 : clicked);

/** The counts as they will read once the service has the vote, shown before it answers. */
export function withVote(s: LimusicLyricState, value: -1 | 0 | 1): LimusicLyricState {
	const up = s.up - (s.my_vote === 1 ? 1 : 0) + (value === 1 ? 1 : 0);
	const down = s.down - (s.my_vote === -1 ? 1 : 0) + (value === -1 ? 1 : 0);
	return { ...s, up: Math.max(0, up), down: Math.max(0, down), my_vote: value };
}

/** The reasons a lyric can be reported for, in the order the form lists them. */
export const REPORT_REASONS: { id: LimusicReportReason; label: TranslationKey }[] = [
	{ id: 'timing', label: 'lyrics.report_timing' },
	{ id: 'text', label: 'lyrics.report_text' },
	{ id: 'wrong_song', label: 'lyrics.report_wrong_song' },
	{ id: 'abuse', label: 'lyrics.report_abuse' }
];

/** Why a vote or a report was refused, as a key. */
export function feedbackErrorKey(code: string): TranslationKey {
	switch (code) {
		case 'own':
			return 'lyrics.feedback_own';
		case 'not_found':
			return 'lyrics.feedback_gone';
		case 'rate_limited':
			return 'lyrics.feedback_rate_limited';
		case 'banned':
			return 'lyrics.feedback_banned';
		default:
			return 'lyrics.feedback_failed';
	}
}

/** Why starting a link or a sign-in failed, as a key. */
export function signInErrorKey(code: string): TranslationKey {
	if (code === 'rate_limited') return 'limusic.signin_rate_limited';
	if (code === 'unavailable') return 'limusic.unreachable';
	return 'limusic.signin_failed';
}

/** A rename's or a sign-in's error code, as a key. */
export function accountErrorKey(code: string): TranslationKey {
	switch (code) {
		case 'taken':
			return 'limusic.name_taken';
		case 'invalid':
			return 'limusic.name_invalid';
		case 'rate_limited':
			return 'limusic.name_rate_limited';
		case 'expired':
			return 'limusic.signin_expired';
		case 'banned':
			return 'limusic.signin_banned';
		case 'session_ended':
			return 'limusic.session_ended';
		case 'failed':
			return 'limusic.signin_failed';
		default:
			return 'limusic.unreachable';
	}
}
