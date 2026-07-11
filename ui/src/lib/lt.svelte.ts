// Reactive Listen Together state, driven by the Rust `lt-state` event. The modal + home-header
// button read this; mutations go through `api` commands. context/19.
import type { LtState } from './api';

export const lt = $state<LtState>({
	status: 'disconnected',
	role: 'none',
	requesting: false,
	roomCode: null,
	myId: null,
	serverUrl: '',
	users: [],
	currentTrack: null,
	queue: [],
	pendingJoins: [],
	suggestions: []
});

/** True when we're in (or joining) a room. */
export function inRoom(): boolean {
	return lt.role !== 'none';
}

/** Replace the reactive state from a fresh `lt-state` snapshot. */
export function applyLtState(s: LtState) {
	Object.assign(lt, s);
}
