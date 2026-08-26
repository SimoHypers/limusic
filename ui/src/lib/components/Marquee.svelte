<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		text,
		children,
		class: cls = ''
	}: {
		/** The line to scroll, and the value that restarts the scroll when it changes. */
		text: string;
		/** Renders in place of plain `text` (artist links); `text` still drives the restart. */
		children?: Snippet;
		class?: string;
	} = $props();

	/** Blank space between the text and the copy chasing it, in px. */
	const GAP = 40;

	let box = $state<HTMLElement>();
	/** Full text width in px once it is wider than the box; 0 when it fits and nothing scrolls. */
	let width = $state(0);

	$effect(() => {
		text; // remeasure whenever the line changes
		const el = box;
		if (!el) return;
		const measure = () => {
			const span = el.querySelector('[data-mq]');
			// A user who asked the OS to minimise motion gets the plain truncated line.
			const still = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
			const w = span ? span.scrollWidth : 0;
			width = !still && w > el.clientWidth + 1 ? w : 0;
		};
		measure();
		// The player bar's title column stretches with the window, so a resize can hide or
		// reveal the overflow on its own.
		const ro = new ResizeObserver(measure);
		ro.observe(el);
		return () => ro.disconnect();
	});
</script>

{#snippet body()}{#if children}{@render children()}{:else}{text}{/if}{/snippet}

<div bind:this={box} class="marquee-box overflow-hidden {cls}">
	{#key text}
		{#if width}
			<!-- The copy is what the loop scrolls into view: by the time the first one has moved
			     its own width plus the gap, the second sits exactly where it started, so the
			     restart is invisible and the line only ever travels one way. `inert` keeps any
			     links in the copy out of the tab order and unclickable. The gap is a margin, not
			     padding, so a hover underline drawn on this span stops at the text.
			     It is positioned, not laid out, on purpose. The title and the artist line share one
			     shrink-to-fit column, so a second in-flow copy would double this line's max-content
			     width, widen the column, and un-truncate the very text that asked to scroll: both
			     lines then flip between scrolling and still, resizing each other as they go. Out of
			     flow, scrolling and truncating measure exactly the same. -->
			<div
				class="marquee relative w-max"
				style="--marquee-dx:{width + GAP}px;animation-duration:{((width + GAP) / 30).toFixed(1)}s"
			>
				<span class="block whitespace-nowrap" data-mq>{@render body()}</span>
				<span
					class="absolute left-full top-0 whitespace-nowrap"
					style="margin-left:{GAP}px"
					aria-hidden="true"
					inert>{@render body()}</span
				>
			</div>
		{:else}
			<span class="block truncate" data-mq>{@render body()}</span>
		{/if}
	{/key}
</div>
