// Lite mode's auto-arm poll (§4, §5).
//
// `autoArmed` is runtime-only on purpose: arming must never go through `setAppearance`, which
// persists. Someone's saved appearance is theirs — a machine that happens to be busy right now
// changes what renders, not what is stored. The poll reads the GPU busy counters through the
// `gpu_busy_percents` command once every LITE_POLL_MS (one `read_dir` and a few four-byte reads
// on the Rust side — the plan measured that as negligible), and a hysteresis does the deciding:
//
//   - ARM after LITE_ARM_SAMPLES consecutive samples with ANY card at or over LITE_ARM_AT;
//   - DISARM after LITE_DISARM_SAMPLES consecutive samples with ALL cards under LITE_IDLE_BELOW;
//   - and never sooner than LITE_DWELL_MS after the last flip, so a card parked on 84/85 cannot
//     throb between modes every 45 seconds.
//
// `liteAutoStep` holds that decision as a pure function of (state, sample, now), kept at module
// level so the hysteresis reads — and reasons about — on its own.

import { invoke } from '@tauri-apps/api/core';
import { apply } from './theme.svelte';

/** Poll cadence, ms (plan §4). */
export const LITE_POLL_MS = 15_000;
/** A card at or over this busy percent counts the sample busy. */
export const LITE_ARM_AT = 85;
/** Every card under this busy percent counts the sample idle. */
export const LITE_IDLE_BELOW = 40;
/** Consecutive busy samples before arming — 3 × 15 s lands the arm at ~45 s (§8). */
export const LITE_ARM_SAMPLES = 3;
/** Consecutive idle samples before disarming — 8 × 15 s is the §8 "within ~2 min". */
export const LITE_DISARM_SAMPLES = 8;
/** Minimum dwell on each side, ms. */
export const LITE_DWELL_MS = 60_000;

/** True while the gpu-busy poll has lite mode armed for this run. Never persisted. A `$state`
 *  object, not a bare bool, because Svelte refuses to export module state that gets reassigned
 *  (`state_invalid_export`) — mutating one property is the sanctioned shape; consumers read
 *  `autoArmed.value`. */
export const autoArmed = $state({ value: false });

/** The hysteresis' whole memory. */
export interface LiteAuto {
	armed: boolean;
	/** Consecutive samples with any card at or over `LITE_ARM_AT`. */
	busyStreak: number;
	/** Consecutive samples with every card under `LITE_IDLE_BELOW`. */
	idleStreak: number;
	/** When `armed` last flipped; 0 before the first flip — the first arm waits on no dwell. */
	flippedAt: number;
}

export const liteAutoInit = (): LiteAuto => ({
	armed: false,
	busyStreak: 0,
	idleStreak: 0,
	flippedAt: 0
});

/**
 * One hysteresis step: `percents` are the busy percents of the cards this sample saw. A sample
 * with no cards counts as idle — a machine without counters never arms, and counters vanishing
 * under a live poll is "nothing to watch", not a reason to stay armed.
 */
export function liteAutoStep(s: LiteAuto, percents: number[], now: number): LiteAuto {
	const busy = percents.some((p) => p >= LITE_ARM_AT);
	const idle = percents.every((p) => p < LITE_IDLE_BELOW);
	const busyStreak = busy ? s.busyStreak + 1 : 0;
	const idleStreak = idle ? s.idleStreak + 1 : 0;
	// The dwell counts from the last flip only; the first flip of a run waits for nothing.
	const sinceFlip = s.flippedAt === 0 ? LITE_DWELL_MS : now - s.flippedAt;
	if (!s.armed && busyStreak >= LITE_ARM_SAMPLES && sinceFlip >= LITE_DWELL_MS) {
		return { armed: true, busyStreak: 0, idleStreak: 0, flippedAt: now };
	}
	if (s.armed && idleStreak >= LITE_DISARM_SAMPLES && sinceFlip >= LITE_DWELL_MS) {
		return { armed: false, busyStreak: 0, idleStreak: 0, flippedAt: now };
	}
	return { ...s, busyStreak, idleStreak };
}

let state = liteAutoInit();
let timer: ReturnType<typeof setInterval> | undefined;

/** The wire shape of Rust's `GpuBusy` — §5 pins camelCase on the frontend side. */
interface GpuBusy {
	device: string;
	busyPercent: number;
}

async function poll(): Promise<void> {
	let cards: GpuBusy[];
	try {
		cards = await invoke<GpuBusy[]>('gpu_busy_percents');
	} catch {
		// A failed poll is not a sample: the streaks wait, the app never notices.
		return;
	}
	const next = liteAutoStep(state, cards.map((c) => c.busyPercent), Date.now());
	state = next;
	if (next.armed !== autoArmed.value) {
		autoArmed.value = next.armed;
		// The `.lite` class rides the same apply() as the palette classes. Reactive consumers
		// would re-derive through theme.svelte.ts on their own; this keeps the root class in
		// step even when nothing has subscribed to it yet. (theme imports this module for the
		// derivation and this module imports theme for apply() — a cycle neither side reads
		// from during evaluation, so by the first poll both are fully initialised.)
		apply();
	}
}

/**
 * Start the auto-arm poll. Called once, where the theme initialises. One poll per window; the
 * first sample lands immediately, so an arm takes 3 × 15 s from launch, not 4 × 15 s.
 */
export function startLiteAutoPolling(): void {
	if (timer !== undefined) return;
	void poll();
	timer = setInterval(() => void poll(), LITE_POLL_MS);
}
