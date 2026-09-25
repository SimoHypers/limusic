<script lang="ts">
	// One header for every section on home (and every shelf elsewhere), so the page reads as one
	// document instead of a stack of unrelated widgets.
	//
	// The rule that runs from the title out to the trailing action is the whole idea: it gives a
	// section a measurable width and an end, which a bare <h2> floating over a row of cards never
	// had. It fades out rather than reaching the edge — a hard line all the way across would read as
	// a divider between sections, and these sit above their content, not between them.
	import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/svelte';
	import { ArrowRight01Icon } from '@hugeicons/core-free-icons';
	import type { Snippet } from 'svelte';

	let {
		title,
		icon,
		onMore,
		moreLabel = 'See all',
		headingClass = 'font-heading text-lg font-semibold',
		children
	}: {
		title: string;
		/** Says what the section holds at a glance. Optional: not every section has a kind. */
		icon?: IconSvgElement;
		/** Renders the trailing "See all"; the title becomes a second way to click it. */
		onMore?: () => void;
		moreLabel?: string;
		/** Artist and album pages set their shelves a size larger. */
		headingClass?: string;
		/** Controls at the far end, before "See all". */
		children?: Snippet;
	} = $props();
</script>

<div class="mb-3 flex items-center gap-3">
	{#if icon}
		<!-- Keyed: HugeiconsIcon freezes `icon` at mount, and a shelf can settle on a different kind
		     once its items arrive. -->
		{#key icon}
			<HugeiconsIcon {icon} class="h-4 w-4 shrink-0 text-primary/60" />
		{/key}
	{/if}
	{#if onMore}
		<button class="min-w-0 cursor-pointer text-left" onclick={onMore} title="{moreLabel} {title}">
			<h2 class="{headingClass} truncate hover:underline">{title}</h2>
		</button>
	{:else}
		<h2 class="{headingClass} min-w-0 truncate">{title}</h2>
	{/if}
	<div class="h-px min-w-6 flex-1 bg-gradient-to-r from-border to-transparent"></div>
	{@render children?.()}
	{#if onMore}
		<button
			class="flex shrink-0 cursor-pointer items-center gap-0.5 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
			onclick={onMore}
		>
			{moreLabel}
			<HugeiconsIcon icon={ArrowRight01Icon} class="h-3.5 w-3.5" />
		</button>
	{/if}
</div>
