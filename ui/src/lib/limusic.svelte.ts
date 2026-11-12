// Contributing to Limusic Lyrics from the app: whether the service offers accounts, who is signed
// in, and the one function the lyric editor submits through. The Rust half is `limusic.rs`.
import * as api from '$lib/api';
import { toast } from '$lib/player.svelte';
import { t, type TranslationKey } from '$lib/i18n.svelte';
import { accountErrorKey, signInErrorKey } from './limusic';

export const limusic = $state({
	/** Whether "Sync lyrics" / "Improve these lyrics" and the account card are offered at all: the
	 *  service says it has anonymous accounts. False until it has said so, which is also what it
	 *  stays for an older service or one that can't be reached. */
	available: false,
	/** The identities that can be linked. */
	providers: [] as string[],
	/** There is a session. An anonymous one counts, and has no handle. */
	signedIn: false,
	handle: null as string | null,
	anonymous: false,
	account: null as api.LimusicAccount | null,
	/** Submissions waiting for review, for a reviewer: the first page only, so `more` says there are
	 *  others. Null until asked, and for everyone else. */
	waiting: null as { count: number; more: boolean } | null,
	/** The provider a sign-in is waiting on, and the code its confirm page should show. */
	signingIn: null as { provider: string; code: string } | null,
	/** A sign-in is being started: a second one now would cancel the first one's poll. */
	launching: false
});

function apply(s: api.LimusicAccountState) {
	limusic.signedIn = s.signed_in;
	limusic.handle = s.handle ?? null;
	limusic.anonymous = s.anonymous;
	if (!s.signed_in) {
		limusic.account = null;
		limusic.waiting = null;
	}
}

/** Once per start, from the root layout: asks the service whether it has accounts, in the
 *  background, and follows the session from then on. Returns the unlisten. */
export function watchLimusic(): () => void {
	api.limusicProviders()
		.then((p) => {
			limusic.providers = p.providers;
			limusic.available = p.anonymous;
		})
		.catch(() => {});
	api.limusicStatus().then(apply).catch(() => {});
	const sub = api.onLimusicAccount((s) => {
		const waiting = limusic.signingIn !== null;
		limusic.signingIn = null;
		apply(s);
		if (s.error) toast.error(t(accountErrorKey(s.error)));
		else if (s.signed_in && s.handle && waiting) toast.success(t('limusic.signed_in_toast', { handle: s.handle }));
		if (s.signed_in) refreshAccount();
	});
	return () => void sub.then((u) => u());
}

/** How many are waiting for review, from the first page of the queue. Not for non-reviewers. */
export async function refreshWaiting() {
	try {
		const q = await api.limusicReviewQueue();
		limusic.waiting = { count: q.items.length, more: q.next !== null };
	} catch {
		limusic.waiting = null;
	}
}

/** The account's score and role, fresh. A session the service has dropped signs out here. */
export async function refreshAccount() {
	try {
		const a = await api.limusicMe();
		if (a) {
			limusic.account = a;
			limusic.handle = a.handle;
			limusic.anonymous = a.anonymous;
			if (a.can_review) refreshWaiting();
			else limusic.waiting = null;
		}
	} catch {
		// Unreachable: the stored name is shown as it is.
	}
}

export async function signIn(provider: string) {
	if (limusic.launching) return;
	limusic.launching = true;
	try {
		limusic.signingIn = { provider, code: await api.limusicSignIn(provider) };
	} catch (e) {
		toast.error(t(signInErrorKey(String(e))));
	} finally {
		limusic.launching = false;
	}
}

export function cancelSignIn() {
	limusic.signingIn = null;
	api.limusicCancel().catch(() => {});
}

export function signOut() {
	api.limusicSignOut().catch((e) => toast.error(String(e)));
}

/** A rename; resolves with an error key for the card to show, or null when it took. */
export async function rename(handle: string): Promise<TranslationKey | null> {
	try {
		const a = await api.limusicRename(handle);
		limusic.account = a;
		limusic.handle = a.handle;
		limusic.anonymous = a.anonymous;
		return null;
	} catch (e) {
		return accountErrorKey(String(e));
	}
}

/** The song's live revision now, for keeping a draft on top of a newer version. Throws when the
 *  service can't be asked. */
export const liveRevision = (videoId: string) => api.limusicLiveRevision(videoId);

/** A finished lyric on its way to `POST /v1/lyrics/{video_id}/revisions`. */
export interface LyricsSubmission {
	videoId: string;
	/** `buildLines`' output: canonical, sorted, word-synced. */
	lines: api.LyricLine[];
	title: string;
	artist: string;
	album?: string;
	/** Seconds. */
	duration?: number;
	/** The live revision the draft was started from, `null` when the song had none. */
	baseRevision: number | null;
}

/** THE SUBMIT SEAM. The editor calls this and nothing else to send a lyric. A transport failure
 *  is an `unavailable` answer, never a throw. */
export async function submitLyrics(s: LyricsSubmission): Promise<api.LimusicSubmitted> {
	try {
		return await api.limusicSubmit({
			video_id: s.videoId,
			lines: s.lines,
			title: s.title,
			artist: s.artist,
			album: s.album,
			duration: s.duration,
			base_revision: s.baseRevision
		});
	} catch {
		return { kind: 'failed', code: 'unavailable', detail: null };
	}
}
