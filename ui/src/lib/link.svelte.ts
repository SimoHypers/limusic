// Reactive device-link state, driven by the Rust `link-state` event. The Devices tab and the
// player bar's strip read it; changes go through `api` commands.
import type { LinkState } from './api';

export const link = $state<LinkState>({
	status: 'off',
	error: null,
	server: '',
	selfName: '',
	pairing: null,
	devices: [],
	player: null,
	track: null,
	isPlaying: false,
	remote: false
});

export function applyLinkState(s: LinkState) {
	Object.assign(link, s);
}
