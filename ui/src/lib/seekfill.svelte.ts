// Draws the seek bar's fill between position ticks. The maths is in `seekfill.ts`; this is the
// loop around it. Put it on the `.m3-seek` wrapper, with the same numbers `M3SeekWave` gets:
//
//     <div class="m3-seek" {@attach seekFill(() => ({ pct, paused, duration, dragging }))}>
//
// It writes `--pct` on the range input inside, which is what every theme but Material fills its
// track from. Material draws the bar with `M3SeekWave` and has its own loop, so this one idles
// there and takes its property back off.
//
// One writer: the input carries no inline `--pct` of its own, or the next tick would overwrite
// this one with the stale value. The input's `value` stays the tick's (`step="any"` lets it be
// fractional); the thumb is a tick behind the fill at most, which at 12 px is not visible.
//
// Cost, per docs/UI-PERFORMANCE.md: the property lands on a leaf (the input, not `<html>`), the
// loop runs only while the bar is playing and not being dragged, redraws at the rate of a pixel
// of travel and no faster than 30 a second (it is rAF-paced like `M3SeekWave`: a timer waits out
// the gap, then one animation frame draws), and the width comes from a ResizeObserver's entry, so
// nothing reads layout.
import type { Attachment } from 'svelte/attachments';
import { drawGap, frame, restart, sample, type Clock } from './seekfill';
import { theme } from './theme.svelte';

export type SeekFillParams = {
	/** 0..1 of the shown (tempo-adjusted) timeline. */
	pct: number;
	paused: boolean;
	/** Shown seconds: the track's length divided by the playback speed. */
	duration: number;
	dragging: boolean;
};

export function seekFill(get: () => SeekFillParams): Attachment<HTMLElement> {
	return (node) => {
		const input = node.querySelector<HTMLInputElement>('input[type="range"]');
		if (!input) return;

		let p: SeekFillParams = { pct: 0, paused: true, duration: 0, dragging: false };
		let running = false;
		let clock: Clock = restart(0, 0);
		let width = 0;
		let raf = 0;
		let timer: ReturnType<typeof setTimeout> | undefined;
		let lastDraw = 0;
		let written = '';

		const write = (v: number) => {
			const s = `${(Math.min(1, Math.max(0, v)) * 100).toFixed(3)}%`;
			if (s === written) return;
			written = s;
			input.style.setProperty('--pct', s);
		};

		const stop = () => {
			if (raf) cancelAnimationFrame(raf);
			clearTimeout(timer);
			raf = 0;
			timer = undefined;
		};

		const schedule = () => {
			// Half a display frame early: the animation frame it waits for lands up to one later.
			const wait = lastDraw + drawGap(p.duration, width) - 8 - performance.now();
			if (wait > 0) {
				timer = setTimeout(() => {
					timer = undefined;
					raf = requestAnimationFrame(tick);
				}, wait);
			} else {
				raf = requestAnimationFrame(tick);
			}
		};

		const tick = () => {
			raf = 0;
			const now = performance.now();
			clock = frame(clock, now, p.duration);
			write(clock.shown);
			lastDraw = now;
			schedule();
		};

		const ro = new ResizeObserver((entries) => {
			width = entries[0]?.contentRect.width ?? 0;
		});
		ro.observe(node);

		$effect(() => {
			const next = get();
			const prev = p;
			p = next;
			const now = performance.now();
			if (theme.id === 'material') {
				stop();
				running = false;
				written = '';
				input.style.removeProperty('--pct');
				return;
			}
			if (next.paused || next.dragging || !(next.duration > 0)) {
				// Paused, dragging or nothing loaded: the bar is exactly where it was told.
				stop();
				running = false;
				clock = restart(next.pct, now);
				write(next.pct);
				return;
			}
			if (!running) {
				// Mount, resume or the end of a drag. The last tick may be old (a pause leaves its
				// time behind), so start from now rather than coasting across the gap.
				running = true;
				clock = restart(next.pct, now);
				write(next.pct);
				stop();
				lastDraw = 0;
				schedule();
			} else if (next.pct !== prev.pct) {
				clock = sample(clock, next.pct, now, next.duration);
				// A jump (a seek, a new track) shows at once; a tick that only agrees waits for the loop.
				if (clock.shown === next.pct) write(clock.shown);
			}
		});

		return () => {
			stop();
			ro.disconnect();
			input.style.removeProperty('--pct');
		};
	};
}
