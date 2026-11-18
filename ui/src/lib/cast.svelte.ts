// Reactive cast state, driven by the Rust `cast-devices` / `cast-state` events. The player
// bar's button and the device picker read it; the commands go through `api`.
import type { CastDevice, CastState } from './api';

export const castState = $state({
	/** Whether something is playing on a device. */
	active: false,
	/** The device that is playing, if any. */
	device: null as CastDevice | null,
	/** Everything that was found on the network. */
	devices: [] as CastDevice[],
	/** Whether the device has answered yet. */
	synced: false
});

export function applyCastState(s: CastState) {
	castState.active = s.active;
	castState.device = s.device;
	castState.synced = s.synced;
}

export function applyCastDevices(devices: CastDevice[]) {
	castState.devices = devices;
}
