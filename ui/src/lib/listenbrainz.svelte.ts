// ListenBrainz connection state. Mirrors `lastfm.svelte.ts`: the Scrobbling settings tab is
// the only surface that reads and writes it, but one owner keeps the card from going stale.
import * as api from '$lib/api';
import { toast } from '$lib/player.svelte';
import { t } from '$lib/i18n.svelte';

export const listenbrainz = $state({
	connected: false,
	username: null as string | null,
	/** UI-local: set while the token validates, cleared by the `listenbrainz-state` event. */
	connecting: false
});

/** Load the stored token and follow the backend's answers. Call once; returns the unlisten. */
export function watchListenBrainz(): () => void {
	// Subscribe before snapshotting: a state event arriving while the status request is in
	// flight is newer than the snapshot, so the snapshot must not overwrite it.
	let eventSeen = false;
	const sub = api.onListenBrainzState((s) => {
		eventSeen = true;
		const wasConnecting = listenbrainz.connecting;
		listenbrainz.connecting = false;
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
	return () => void sub.then((u) => u());
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
