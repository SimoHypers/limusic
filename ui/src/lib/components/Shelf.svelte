<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowLeft01Icon, ArrowRight01Icon } from '@hugeicons/core-free-icons';
	import MediaCard from './MediaCard.svelte';
	import type { BrowseItem } from '$lib/api';

	let {
		title,
		items,
		onMore,
		headingClass = 'font-heading text-lg font-semibold'
	}: {
		title?: string;
		items: BrowseItem[];
		/** Renders a "More" button in the header when provided. */
		onMore?: () => void;
		/** Artist pages use text-xl font-bold; home uses the default. */
		headingClass?: string;
	} = $props();

	let row = $state<HTMLDivElement | null>(null);
	let canLeft = $state(false);
	let canRight = $state(false);

	function update() {
		if (!row) return;
		canLeft = row.scrollLeft > 4;
		canRight = row.scrollLeft + row.clientWidth < row.scrollWidth - 4;
	}

	function page(dir: 1 | -1) {
		row?.scrollBy({ left: dir * Math.round(row.clientWidth * 0.9), behavior: 'smooth' });
	}

	$effect(() => {
		items; // re-measure when content changes
		update();
	});
</script>

<svelte:window onresize={update} />

<section>
	{#if title || onMore}
		<div class="mb-3 flex items-center justify-between">
			{#if title}<h2 class={headingClass}>{title}</h2>{/if}
			{#if onMore}
				<button
					class="cursor-pointer text-xs font-semibold uppercase text-muted-foreground hover:text-foreground"
					onclick={onMore}
				>
					More
				</button>
			{/if}
		</div>
	{/if}
	<div class="group/shelf relative">
		<div class="flex gap-2 overflow-x-auto pb-2" bind:this={row} onscroll={update}>
			{#each items as item, i (item.id + ':' + i)}
				<div class="w-40 shrink-0">
					<MediaCard {item} />
				</div>
			{/each}
		</div>
		{#if canLeft}
			<button
				aria-label="Scroll left"
				onclick={() => page(-1)}
				class="absolute left-0 top-1/2 flex h-8 w-8 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full bg-background/90 text-foreground opacity-0 shadow-md backdrop-blur-sm transition-opacity hover:bg-background focus-visible:opacity-100 group-hover/shelf:opacity-100"
			>
				<HugeiconsIcon icon={ArrowLeft01Icon} class="h-4 w-4" />
			</button>
		{/if}
		{#if canRight}
			<button
				aria-label="Scroll right"
				onclick={() => page(1)}
				class="absolute right-0 top-1/2 flex h-8 w-8 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full bg-background/90 text-foreground opacity-0 shadow-md backdrop-blur-sm transition-opacity hover:bg-background focus-visible:opacity-100 group-hover/shelf:opacity-100"
			>
				<HugeiconsIcon icon={ArrowRight01Icon} class="h-4 w-4" />
			</button>
		{/if}
	</div>
</section>
