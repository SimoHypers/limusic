<script lang="ts">
	// One header for every section on home (and every shelf elsewhere), so the page reads as one
	// document instead of a stack of unrelated widgets.
	//
	// Type does the work a rule used to. There was a line from the title out to the trailing action,
	// on every section of every page, and with a dozen shelves that is a dozen lines boxing the feed
	// in (#319). A title set larger and bolder than anything in the row under it separates the
	// sections on its own, with the page's spacing between them.
	import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/svelte';
	import { ArrowRight01Icon } from '@hugeicons/core-free-icons';
	import type { Snippet } from 'svelte';
	import { t } from '$lib/i18n.svelte';

	let {
		title,
		icon,
		onMore,
		moreLabel,
		children,
		trailing
	}: {
		title: string;
		/** Says what the section holds at a glance. Optional: not every section has a kind. */
		icon?: IconSvgElement;
		/** Renders the trailing "See all"; the title becomes a second way to click it. */
		onMore?: () => void;
		moreLabel?: string;
		/** Controls at the far end, before "See all". */
		children?: Snippet;
		/** Trailing controls after "See all" (e.g. pagination arrows). */
		trailing?: Snippet;
	} = $props();
	const more = $derived(moreLabel ?? t('common.see_all'));
</script>

<div class="mb-4 flex items-center gap-2.5">
	{#if icon}
		<!-- Keyed: HugeiconsIcon freezes `icon` at mount, and a shelf can settle on a different kind
		     once its items arrive. -->
		{#key icon}
			<HugeiconsIcon {icon} class="h-5 w-5 shrink-0 text-primary/70" />
		{/key}
	{/if}
	{#if onMore}
		<button class="min-w-0 cursor-pointer text-left" onclick={onMore} title="{more} {title}">
			<h2 class="truncate font-heading text-xl font-bold tracking-tight hover:underline">{title}</h2>
		</button>
	{:else}
		<h2 class="min-w-0 truncate font-heading text-xl font-bold tracking-tight">{title}</h2>
	{/if}
	<div class="min-w-6 flex-1"></div>
	{@render children?.()}
	{#if onMore}
		<button
			class="flex shrink-0 cursor-pointer items-center gap-0.5 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
			onclick={onMore}
		>
			{more}
		</button>
	{/if}
	{@render trailing?.()}
</div>
