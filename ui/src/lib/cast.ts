// The cast picker's own logic, kept out of the component so it can be checked (see
// `cast.check.ts`). The device list itself comes from the backend, which owns the sorting.
import type { CastDevice } from './api.ts';

/** What a device is called in the picker: its model when it reports one, and its kind when it
 *  does not ("Google Cast" for a speaker that only gives its name, "DLNA renderer" for a
 *  television on the network). */
export function deviceSubtitle(
	device: CastDevice,
	chromecastLabel: string,
	dlnaLabel: string
): string {
	if (device.model) return device.model;
	return device.kind === 'dlna' ? dlnaLabel : chromecastLabel;
}

export function isPlaying(device: CastDevice, playingId: string | null): boolean {
	return playingId !== null && device.id === playingId;
}
