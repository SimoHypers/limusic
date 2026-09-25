<script lang="ts">
	// The "saved" mark on a track row: this song is already in one or more of your own playlists.
	// Pointing at it names them (three at most, "and N more" for the rest), and each one links
	// through to that playlist. The popup is `fixed`, anchored at the trigger and moved to <body>
	// (`toBody`), same as TrackMenu: a track list scrolls, and half of them are paint-contained.
	// Living at <body> is also why the links need no stopPropagation, despite the whole row being a
	// play target: nothing in there bubbles through the row any more.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { CheckmarkCircle03Icon } from '@hugeicons/core-free-icons';
	import type { BrowseItem } from '$lib/api';
	import { hrefFor } from '$lib/browse';
	import { anchorMenu, fitMenu, NO_ANCHOR, toBody } from '$lib/menu';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';

	let { playlists }: { playlists: BrowseItem[] } = $props();

	const SHOWN = 3;
	const shown = $derived(playlists.slice(0, SHOWN));
	const extra = $derived(playlists.length - shown.length);

	let open = $state(false);
	let anchor = $state(NO_ANCHOR);
	// Crossing the gap between the mark and the popup is a mouseleave with nothing under the
	// pointer, so closing waits long enough for the next mouseenter to cancel it.
	let closing: ReturnType<typeof setTimeout> | undefined;

	function show(e: Event) {
		clearTimeout(closing);
		anchor = anchorMenu(e, { align: 'right' });
		open = true;
	}
	function hide() {
		clearTimeout(closing);
		closing = setTimeout(() => (open = false), 140);
	}
	function keep() {
		clearTimeout(closing);
	}

	const label = $derived(
		playlists.length === 1
			? t('library.saved_in_one', { playlist: playlists[0].title })
			: t('library.saved_in_many', { count: playlists.length })
	);
</script>

<!-- Opens on hover and on focus, so it is reachable by tab as well as by pointer. The click does
     nothing but stay out of the row's way: toggling here would close the popup under a pointer
     that never left the mark. -->
<button
	class="cursor-pointer rounded-md p-1.5 text-primary transition hover:bg-accent/20"
	aria-label={label}
	title={label}
	onmouseenter={show}
	onmouseleave={hide}
	onfocus={show}
	onblur={hide}
	onclick={(e) => e.stopPropagation()}
>
	<HugeiconsIcon icon={CheckmarkCircle03Icon} class="h-4 w-4" />
</button>

{#if open}
	<div
		class="fixed z-50 min-w-52 max-w-72 animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style={anchor.style}
		{@attach fitMenu(anchor)}
		onmouseenter={keep}
		onmouseleave={hide}
		role="tooltip"
		{@attach toBody}
	>
		<p class="px-2 pb-1 pt-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground">
			{t('library.saved_in')}
		</p>
		{#each shown as pl (pl.id)}
			<a
				href={hrefFor(pl)}
				class="flex w-full items-center gap-2 rounded-md p-1.5 hover:bg-accent/10"
				onclick={() => (open = false)}
			>
				{#if pl.thumbnail}
					<!-- thumb(): a playlist on this machine can wear a local file's art, a path. -->
					<img src={thumb(pl.thumbnail, 96)} alt="" class="h-7 w-7 shrink-0 rounded object-cover" />
				{:else}
					<div class="h-7 w-7 shrink-0 rounded bg-muted"></div>
				{/if}
				<span class="min-w-0 truncate text-sm">{pl.title}</span>
			</a>
		{/each}
		{#if extra > 0}
			<p class="px-2 pb-1 pt-0.5 text-xs text-muted-foreground">
				{t('library.saved_in_more', { count: extra })}
			</p>
		{/if}
	</div>
{/if}
