// What Limusic Lyrics' answers mean in the UI. Pure, so `limusic.check.ts` can run it. The state
// and the calls are in `limusic.svelte.ts`.
import type { LimusicSubmitted } from './api';
import type { TranslationKey } from './i18n.svelte';

/** Where a sign-in provider's name is spelled. The service's ids are lower case. */
export const PROVIDER_NAMES: Record<string, string> = { discord: 'Discord', github: 'GitHub', google: 'Google' };
export const providerName = (id: string) => PROVIDER_NAMES[id] ?? id.charAt(0).toUpperCase() + id.slice(1);

export interface Message {
	/** A key in `en.json`. */
	key: TranslationKey;
	params?: Record<string, string>;
	kind: 'success' | 'info' | 'error';
	/** Done with the draft: it is on the service now. */
	sent?: boolean;
	/** The fix is in Settings ▸ Limusic account. */
	needsAccount?: boolean;
}

/** Every answer the service documents for a submission, in words. */
export function submitMessage(o: LimusicSubmitted): Message {
	if (o.kind === 'published') return { key: 'lyrics_editor.sent_published', kind: 'success', sent: true };
	if (o.kind === 'pending') return { key: 'lyrics_editor.sent_pending', kind: 'success', sent: true };
	switch (o.code) {
		case 'unauthorized':
			return { key: 'lyrics_editor.err_signed_out', kind: 'error', needsAccount: true };
		case 'banned':
			return { key: 'lyrics_editor.err_banned', kind: 'error' };
		case 'invalid':
			return o.detail
				? { key: 'lyrics_editor.err_invalid_detail', params: { detail: o.detail }, kind: 'error' }
				: { key: 'lyrics_editor.err_invalid', kind: 'error' };
		case 'stale':
			return { key: 'lyrics_editor.err_stale', kind: 'error' };
		case 'unchanged':
			return { key: 'lyrics_editor.err_unchanged', kind: 'info' };
		case 'rate_limited':
			return { key: 'lyrics_editor.err_rate_limited', kind: 'error' };
		default:
			return { key: 'lyrics_editor.err_unavailable', kind: 'error' };
	}
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
