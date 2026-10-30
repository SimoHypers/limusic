<script lang="ts">
	import { theme } from '$lib/theme.svelte';

	let {
		pct,
		paused,
		duration = 0,
		dragging = false,
		strokeWidth = 3.5,
		thumbWidth = 4,
		thumbHeight = 18,
		maxAmp = 2.8,
		wavelength = 20
	}: {
		pct: number;
		paused: boolean;
		duration?: number;
		dragging?: boolean;
		strokeWidth?: number;
		thumbWidth?: number;
		thumbHeight?: number;
		maxAmp?: number;
		wavelength?: number;
	} = $props();

	let svgEl: SVGSVGElement | undefined = $state();
	let pathEl: SVGPathElement | undefined = $state();
	let unplayedEl: SVGLineElement | undefined = $state();
	let thumbEl: SVGLineElement | undefined = $state();
	let seekW = 0;
	let seekH = 16;
	let wavePhase = 0;
	let waveAmp = 0;
	let curPct = 0;
	let displayPct = 0;
	let curPlaying = false;
	let curDuration = 0;
	let curDragging = false;
	let kickWave: () => void = () => {};

	function updateSvg(width: number, height: number, progress: number, phase: number, amp: number) {
		if (!pathEl || !unplayedEl || !thumbEl) return;
		const cy = (height || 16) / 2;
		const r = strokeWidth / 2;
		const x0 = Math.ceil(r);
		const clamped = Math.min(1, Math.max(0, progress));
		const thumbX = x0 + clamped * Math.max(0, width - x0 * 2);
		const gap = strokeWidth + thumbWidth / 2 + 2;

		const thHalf = thumbHeight / 2;
		const tx = thumbX.toFixed(2);
		thumbEl.setAttribute('x1', tx);
		thumbEl.setAttribute('x2', tx);
		thumbEl.setAttribute('y1', (cy - thHalf).toFixed(2));
		thumbEl.setAttribute('y2', (cy + thHalf).toFixed(2));

		const ux1 = thumbX + gap;
		const ux2 = width - x0;
		if (ux2 > ux1 + 0.5) {
			unplayedEl.setAttribute('x1', ux1.toFixed(2));
			unplayedEl.setAttribute('x2', ux2.toFixed(2));
			unplayedEl.setAttribute('y1', cy.toFixed(2));
			unplayedEl.setAttribute('y2', cy.toFixed(2));
			unplayedEl.setAttribute('visibility', 'visible');
		} else {
			unplayedEl.setAttribute('visibility', 'hidden');
		}

		const x1 = thumbX - gap;
		if (x1 <= x0 + 0.5) {
			pathEl.setAttribute('d', '');
			return;
		}
		const a = amp * amp * (3 - 2 * amp);
		if (a < 0.002) {
			pathEl.setAttribute('d', `M ${x0} ${cy} L ${x1.toFixed(2)} ${cy}`);
			return;
		}
		const len = x1 - x0;
		const peak = maxAmp * a;
		const taper = Math.min(10, len * 0.4);
		const step = 2;
		let d = `M ${x0} ${cy}`;
		for (let x = x0 + step; x < x1; x += step) {
			const edge = Math.min(x - x0, x1 - x);
			const env = taper > 0 && edge < taper ? 0.5 * (1 - Math.cos(Math.PI * (edge / taper))) : 1;
			const y = cy + env * peak * Math.sin(((x - x0) / wavelength) * Math.PI * 2 - phase);
			d += ` L ${x.toFixed(1)} ${y.toFixed(2)}`;
		}
		d += ` L ${x1.toFixed(2)} ${cy}`;
		pathEl.setAttribute('d', d);
	}

	$effect(() => {
		if (theme.id !== 'material') return;
		const nextPct = pct;
		curPlaying = !paused;
		curDuration = duration;
		curDragging = dragging;
		if (dragging || paused || Math.abs(nextPct - displayPct) > 0.025) {
			displayPct = nextPct;
		}
		curPct = nextPct;
		kickWave();
	});

	$effect(() => {
		if (theme.id !== 'material' || !svgEl || !pathEl) return;
		const svg = svgEl;
		let raf = 0;
		let lastT = 0;

		const render = () => {
			updateSvg(seekW, seekH, displayPct, wavePhase, waveAmp);
		};

		const tick = (now: number) => {
			raf = 0;
			const dt = lastT ? Math.min(64, now - lastT) : 16;
			lastT = now;
			const target = curPlaying ? 1 : 0;
			if (waveAmp < target) {
				waveAmp = Math.min(1, waveAmp + dt / 280);
			} else if (waveAmp > target) {
				waveAmp = Math.max(0, waveAmp - dt / 280);
			}
			if (curPlaying && !curDragging) {
				if (curDuration > 0) {
					displayPct = Math.min(1, displayPct + dt / (curDuration * 1000));
				}
				displayPct += (curPct - displayPct) * Math.min(1, dt / 180);
			} else {
				displayPct = curPct;
			}
			if (waveAmp > 0) {
				wavePhase = (wavePhase + (dt / 1100) * Math.PI * 2) % (Math.PI * 2);
			}
			render();
			if (curPlaying || waveAmp > 0) {
				raf = requestAnimationFrame(tick);
			} else {
				lastT = 0;
			}
		};

		kickWave = () => {
			render();
			if ((curPlaying || waveAmp > 0) && !raf) {
				lastT = performance.now();
				raf = requestAnimationFrame(tick);
			}
		};

		const ro = new ResizeObserver((entries) => {
			const rect = entries[0]?.contentRect;
			const w = rect?.width ?? 0;
			const h = rect?.height || 16;
			if (w !== seekW || h !== seekH) {
				seekW = w;
				seekH = h;
				kickWave();
			}
		});
		ro.observe(svg);
		kickWave();

		return () => {
			ro.disconnect();
			if (raf) cancelAnimationFrame(raf);
			kickWave = () => {};
		};
	});
</script>

{#if theme.id === 'material'}
	<svg
		bind:this={svgEl}
		class="m3-seek-svg pointer-events-none absolute inset-0 h-full w-full overflow-visible"
		aria-hidden="true"
	>
		<line
			bind:this={unplayedEl}
			class="m3-seek-unplayed"
			stroke="var(--secondary)"
			stroke-width={strokeWidth}
			stroke-linecap="round"
			visibility="hidden"
		/>
		<path
			bind:this={pathEl}
			fill="none"
			stroke="var(--primary)"
			stroke-width={strokeWidth}
			stroke-linecap="round"
			stroke-linejoin="round"
		/>
		<line
			bind:this={thumbEl}
			class="m3-seek-thumb"
			stroke="var(--primary)"
			stroke-width={thumbWidth}
			stroke-linecap="round"
		/>
	</svg>
{/if}
