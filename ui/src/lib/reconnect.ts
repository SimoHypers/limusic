// Retrying what failed while the network was down, once it is back (#483). A load that hit a dead
// connection used to sit on "No connection" until the user pressed Try again, which is what every
// launch at login looked like when Limusic beat the network up.
//
// Three triggers, because none is enough alone:
// - `online`: the webview's own signal, but at boot the adapter can be up before DNS answers, so
//   `navigator.onLine` is already true and the event never fires.
// - window focus: a launch to the tray is not looked at until the user opens it, and that is the
//   moment a retry matters.
// - a short ladder of timers, which covers the rest and then stops.
//
// Each trigger asks Rust whether YouTube Music answers before running anything. Retrying the page
// itself would swap "No connection" for skeletons and back on every rung while the network is
// still down.
//
// One ladder for everyone, kept at module level: a failed retry unmounts and remounts the error
// view that asked for it, so a ladder per caller would restart from the bottom on every failure.
import { invoke } from '@tauri-apps/api/core';

const LADDER_MS = [2_000, 4_000, 8_000, 15_000, 30_000, 30_000, 30_000, 30_000, 30_000, 30_000];
/** A quiet spell this long means the last outage is over and the next failure starts afresh. */
const FRESH_AFTER_MS = 5 * 60_000;

const waiting = new Set<() => void>();
let step = 0;
let armedAt = -Infinity;
let timer: ReturnType<typeof setTimeout> | undefined;
let probing = false;
let probe = () => invoke<boolean>('network_reachable');

/** Call `retry` once the network is back. Returns a cancel, for a caller that goes away. */
export function whenBackOnline(retry: () => void): () => void {
	waiting.add(retry);
	arm();
	return () => {
		waiting.delete(retry);
	};
}

function arm(now = Date.now()): void {
	if (timer !== undefined) return;
	if (now - armedAt > FRESH_AFTER_MS) step = 0;
	// Out of rungs: `online` and focus still check, the clock no longer does.
	if (step >= LADDER_MS.length) return;
	armedAt = now;
	timer = setTimeout(check, LADDER_MS[step++]);
}

/** Probe once; if the network answers, run everything waiting, otherwise wait for the next rung. */
async function check(): Promise<void> {
	clearTimeout(timer);
	timer = undefined;
	if (!waiting.size || probing) return;
	probing = true;
	const up = await probe().catch(() => false);
	probing = false;
	if (!up) return arm();
	const due = [...waiting];
	waiting.clear();
	// A retry that still fails registers again, which arms the next rung.
	for (const retry of due) retry();
}

if (typeof window !== 'undefined') {
	window.addEventListener('online', () => {
		step = 0;
		check();
	});
	window.addEventListener('focus', () => check());
}

/** For `reconnect.check.ts`. */
export const _test = {
	check,
	probeWith: (p: () => Promise<boolean>) => (probe = p),
	state: () => ({ step, pending: timer !== undefined, waiting: waiting.size })
};
