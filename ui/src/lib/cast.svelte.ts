import type { CastDevice } from './api';

export const cast = $state<{
	devices: CastDevice[];
	active: CastDevice | null;
}>({
	devices: [],
	active: null
});

export function applyCastDevices(devices: CastDevice[]) {
	cast.devices = devices;
}

export function applyCastState(active: CastDevice | null) {
	cast.active = active;
}
