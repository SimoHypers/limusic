// Casting (#399): the devices on the network, and the one playing. Rust owns it
// (src-tauri/src/castout.rs); this mirrors its events for the device picker and the bar.
import type { CastDevice, CastState } from './api';
import { t, type TranslationKey } from './i18n.svelte';

export const cast = $state<{
	devices: CastDevice[];
	/** The device playing, or null. */
	device: CastDevice | null;
	/** The device being connected to, for its row's spinner. */
	connecting: string | null;
}>({
	devices: [],
	device: null,
	connecting: null
});

export function applyCastDevices(devices: CastDevice[]) {
	cast.devices = devices;
}

export function applyCastState(state: CastState) {
	cast.device = state.device;
	cast.connecting = state.connecting;
}

/** A cast message from Rust, which names it by key: a notice, or why a cast couldn't start. */
export const castMessage = (key: string, name = '', title = '') =>
	t(`cast.${key}` as TranslationKey, { name, title });
