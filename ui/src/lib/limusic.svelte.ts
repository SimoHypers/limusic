// Contributing to Limusic Lyrics from the app: whether the service offers accounts, who is signed
// in, and the one function the lyric editor submits through. The Rust half is `limusic.rs`.
import * as api from '$lib/api';
import { toast } from '$lib/player.svelte';
import { t, type TranslationKey } from '$lib/i18n.svelte';
import { accountErrorKey } from './limusic';

export const limusic = $state({
	/** Whether "Sync lyrics" / "Improve these lyrics" and the account card are offered at all: the
	 *  service lists at least one sign-in provider. False until it has said so, which is also what
	 *  it stays while the service has no accounts yet or can't be reached. */
	available: false,
	providers: [] as string[],
	signedIn: false,
	handle: null as string | null,
	account: null as api.LimusicAccount | null,
	/** The provider a sign-in is waiting on, and the code its confirm page should show. */
	signingIn: null as { provider: string; code: string } | null
});

function apply(s: api.LimusicAccountState) {
	limusic.signedIn = s.signed_in;
	limusic.handle = s.handle ?? null;
	if (!s.signed_in) limusic.account = null;
}

/** Once per start, from the root layout: asks the service whether it has accounts, in the
 *  background, and follows the session from then on. Returns the unlisten. */
export function watchLimusic(): () => void {
	api.limusicProviders()
		.then((p) => {
			limusic.providers = p;
			limusic.available = p.length > 0;
		})
		.catch(() => {});
	api.limusicStatus().then(apply).catch(() => {});
	const sub = api.onLimusicAccount((s) => {
		const waiting = limusic.signingIn !== null;
		limusic.signingIn = null;
		apply(s);
		if (s.error) toast.error(t(accountErrorKey(s.error)));
		else if (s.signed_in && waiting) toast.success(t('limusic.signed_in_toast', { handle: s.handle ?? '' }));
		if (s.signed_in) refreshAccount();
	});
	return () => void sub.then((u) => u());
}

/** The account's score and role, fresh. A session the service has dropped signs out here. */
export async function refreshAccount() {
	try {
		const a = await api.limusicMe();
		if (a) {
			limusic.account = a;
			limusic.handle = a.handle;
		}
	} catch {
		// Unreachable: the stored name is shown as it is.
	}
}

export async function signIn(provider: string) {
	try {
		limusic.signingIn = { provider, code: await api.limusicSignIn(provider) };
	} catch (e) {
		toast.error(String(e));
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
		return null;
	} catch (e) {
		return accountErrorKey(String(e));
	}
}

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
