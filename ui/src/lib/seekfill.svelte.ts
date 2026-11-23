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
// The thumb (the circle that shows on hover) is positioned by the input's `value`, which Svelte
// writes once a tick. While the bar is hovered or focused, and so the thumb is on screen, this
// writes `value` on every draw as well, right after Svelte's own write in the same task, so the
// two never show. Otherwise `value` is left to Svelte: nothing sees the thumb, and moving it
// asks for layout.
//
// The fill's edge is a 1 px soft ramp in layout.css, not a hard gradient stop. A hard stop paints
// on whole pixels whatever `--pct` says (WebKitGTK 2.52.6: steps of 0.24 px repainted the same
// pixels three or four writes running, then jumped one), so no draw rate could look smooth.
//
// Cost, per docs/UI-PERFORMANCE.md: the property lands on a leaf (the input, not `<html>`), the
// loop runs only while the bar is playing and not being dragged, and it draws once per quarter
// pixel of travel, on an animation frame: a timer waits out the gap (like `M3SeekWave`) and one
// frame draws, so a long track in a narrow bar draws a few times a second and a short one in a
// wide bar every frame. The width comes from a ResizeObserver's entry, so nothing reads layout.
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
		// Looked up when needed, not once: the input is static in every bar today, but a bar that
		// remounts it (a keyed block, a future refactor) must not leave this writing to a dead node.
		let input: HTMLInputElement | null = null;
		const find = (): HTMLInputElement | null => {
			if (!input?.isConnected) {
				input = node.querySelector<HTMLInputElement>('input[type="range"]');
				written = '';
			}
			return input;
		};

		let p: SeekFillParams = { pct: 0, paused: true, duration: 0, dragging: false };
		let running = false;
		let clock: Clock = restart(0, 0);
		let width = 0;
		let raf = 0;
		let timer: ReturnType<typeof setTimeout> | undefined;
		let lastDraw = 0;
		let written = '';

		const write = (v: number) => {
			const el = find();
			if (!el) return;
			const s = `${(Math.min(1, Math.max(0, v)) * 100).toFixed(3)}%`;
			if (s === written) return;
			written = s;
			el.style.setProperty('--pct', s);
		};

		// Thumb on screen: the pointer is over the bar or something in it has focus.
		let hover = false;
		let focus = false;
		let live = false; // this theme draws the bar with `--pct`, so `clock` is meaningful
		const hot = () => hover || focus;
		const writeThumb = () => {
			const el = find();
			if (!el || !live || !hot() || p.dragging || !(p.duration > 0)) return;
			el.value = String(clock.shown * p.duration);
		};
		const enter = () => ((hover = true), writeThumb());
		const leave = () => (hover = false);
		const focusIn = () => ((focus = true), writeThumb());
		const focusOut = () => (focus = false);
		node.addEventListener('pointerenter', enter);
		node.addEventListener('pointerleave', leave);
		node.addEventListener('focusin', focusIn);
		node.addEventListener('focusout', focusOut);

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
			writeThumb();
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
			live = theme.id !== 'material';
			if (!live) {
				stop();
				running = false;
				written = '';
				find()?.style.removeProperty('--pct');
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
				// A seek, a new track or a tick ahead of the fill shows at once; one that only agrees
				// with it waits for the loop's next frame.
				if (clock.shown === next.pct) write(clock.shown);
			}
			// Svelte has just set `value` to the tick's: put the thumb back with the fill.
			writeThumb();
		});

		return () => {
			stop();
			ro.disconnect();
			node.removeEventListener('pointerenter', enter);
			node.removeEventListener('pointerleave', leave);
			node.removeEventListener('focusin', focusIn);
			node.removeEventListener('focusout', focusOut);
			input?.style.removeProperty('--pct');
		};
	};
}
