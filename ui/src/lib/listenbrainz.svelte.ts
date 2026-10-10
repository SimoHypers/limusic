// ListenBrainz connection state. Mirrors `lastfm.svelte.ts`: the Scrobbling settings tab is
// the only surface that reads and writes it, but one owner keeps the card from going stale.
import * as api from '$lib/api';
import { toast } from '$lib/player.svelte';
import { t } from '$lib/i18n.svelte';

export const listenbrainz = $state({
	connected: false,
	username: null as string | null,
	/** UI-local: set while the token validates, cleared by the `listenbrainz-state` event. */
	connecting: false,
	/** ListenBrainz refusing listens, from the scrobbler. Cleared by the next accepted listen
	 *  or a connect/disconnect. */
	problem: null as api.ListenBrainzProblem | null
});

/** Where a MetaBrainz account verifies its email. */
export const METABRAINZ_PROFILE = 'https://metabrainz.org/profile';

export const problemText = (p: api.ListenBrainzProblem) =>
	p === 'email'
		? t('settings.scrobbling.lb_problem_email')
		: t('settings.scrobbling.lb_problem_rejected');

/** Load the stored token and follow the backend's answers. Call once; returns the unlisten. */
export function watchListenBrainz(): () => void {
	// Subscribe before snapshotting: a state event arriving while the status request is in
	// flight is newer than the snapshot, so the snapshot must not overwrite it.
	let eventSeen = false;
	const sub = api.onListenBrainzState((s) => {
		eventSeen = true;
		const wasConnecting = listenbrainz.connecting;
		listenbrainz.connecting = false;
		listenbrainz.problem = null;
		listenbrainz.connected = s.connected;
		listenbrainz.username = s.username ?? null;
		if (s.error) toast.error(s.error);
		else if (s.connected)
			toast.success(t('integrations.listenbrainz_connected_as', { user: s.username ?? '' }));
		else if (!wasConnecting) toast.success(t('integrations.listenbrainz_disconnected'));
	});
	api.listenbrainzStatus()
		.then((s) => {
			if (eventSeen) return;
			listenbrainz.connected = s.connected;
			listenbrainz.username = s.username ?? null;
		})
		.catch(() => {});
	// One toast when refusing starts (the card keeps saying it), none per track or on recovery.
	const problem = api.onListenBrainzProblem((p) => {
		listenbrainz.problem = p;
		if (p) toast.error(problemText(p));
	});
	return () => {
		void sub.then((u) => u());
		void problem.then((u) => u());
	};
}

/** Validate and store a pasted user token. */
export async function connectListenBrainz(token: string) {
	listenbrainz.connecting = true;
	try {
		await api.listenbrainzConnect(token);
	} catch (err) {
		listenbrainz.connecting = false;
		toast.error(String(err));
	}
}

export function disconnectListenBrainz() {
	api.listenbrainzDisconnect().catch((e) => toast.error(String(e)));
}
