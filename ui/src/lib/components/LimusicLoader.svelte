<script lang="ts">
	import { t } from '$lib/i18n.svelte';
	import { cn } from '$lib/utils';
	import { NOTE } from './LogoMark.svelte';

	// The Android app's loader, from the logo: the rind as an arc that turns, growing and shrinking,
	// round a note whose outline traces itself, fills, then fades and starts again. One colour,
	// `currentColor` (the accent unless the caller says otherwise).
	//
	// Paint only, no transform: the arc turns by its dash offset, so it never makes a layer, and a
	// scroller painted after it keeps its scrollbar on WebKitGTK (docs/UI-PERFORMANCE.md, #343).
	let { class: className = '' }: { class?: string } = $props();
</script>

<svg
	viewBox="0 0 200 200"
	role="status"
	aria-label={t('common.loading')}
	class={cn('size-12 shrink-0 text-primary', className)}
>
	<circle class="rind" cx="100" cy="100" r="80" />
	<!-- The note sits in the rind at two thirds of its size, centred on its own middle. -->
	<path class="note" d={NOTE} pathLength="1" transform="translate(100 100) scale(.66) translate(-106.5 -83)" />
</svg>

<style>
	/* r = 80, so the circumference is 502.65. Dash and gap always sum to it: with any other period
	   the pattern at the end of a turn differs from the start, and the arc blinks once a turn. */
	.rind {
		fill: none;
		stroke: currentColor;
		stroke-width: 18;
		stroke-linecap: round;
		stroke-dasharray: 60 442.65;
		animation:
			rind-turn 1.1s linear infinite,
			rind-swell 1.8s ease-in-out infinite;
	}
	@keyframes rind-turn {
		to {
			stroke-dashoffset: -502.65;
		}
	}
	@keyframes rind-swell {
		50% {
			stroke-dasharray: 300 202.65;
		}
	}

	/* Outline traces over the first 40%, fill comes in from 30% to 48%, both fade from 75%. */
	.note {
		fill: currentColor;
		stroke: currentColor;
		stroke-width: 9;
		stroke-linejoin: round;
		stroke-dasharray: 1 1;
		animation: note-trace 1.8s ease-in-out infinite;
	}
	@keyframes note-trace {
		0% {
			stroke-dashoffset: 1;
			fill-opacity: 0;
			stroke-opacity: 1;
		}
		30% {
			fill-opacity: 0;
		}
		40% {
			stroke-dashoffset: 0;
		}
		48%,
		75% {
			fill-opacity: 1;
			stroke-opacity: 1;
		}
		100% {
			stroke-dashoffset: 0;
			fill-opacity: 0;
			stroke-opacity: 0;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.rind {
			animation: none;
			stroke-dasharray: 300 202.65;
		}
		.note {
			animation: none;
			stroke-dashoffset: 0;
		}
	}
</style>
