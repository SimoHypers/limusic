<script lang="ts">
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		PlayIcon,
		MoreHorizontalIcon,
		PlayListAddIcon,
		PlayListRemoveIcon
	} from '@hugeicons/core-free-icons';
	import type { SongItem } from '$lib/api';

	let {
		song,
		index,
		active = false,
		hideThumb = false,
		onplay,
		onAdd,
		onRemove
	}: {
		song: SongItem;
		/** Position badge when set (playlist/queue); omitted for flat search results. */
		index?: number;
		active?: boolean;
		/** Hide the leading thumbnail (album track lists show a number, not a cover). */
		hideThumb?: boolean;
		onplay: () => void;
		/** Adds an "Add to playlist" menu item. */
		onAdd?: () => void;
		/** Adds a "Remove from this playlist" menu item. */
		onRemove?: () => void;
	} = $props();

	const hasMenu = $derived(!!onAdd || !!onRemove);

	// A ⋯ menu, positioned `fixed` at the button so it isn't clipped by the scroll container.
	// Anchored by its right edge (distance from the viewport's right) so the zoom-in origin stays put.
	let menuOpen = $state(false);
	let mx = $state(0);
	let my = $state(0);

	function openMenu(e: MouseEvent) {
		e.stopPropagation();
		const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
		mx = window.innerWidth - r.right;
		my = r.bottom + 4;
		menuOpen = true;
	}
	function run(action?: () => void) {
		menuOpen = false;
		action?.();
	}
</script>

<div
	class="group flex w-full items-center gap-3 rounded-lg p-2 transition-colors hover:bg-accent/10 {active
		? 'bg-accent/10'
		: ''}"
>
	<div class="flex min-w-0 flex-1 items-center gap-3">
		<button
			class="flex min-w-0 shrink-0 items-center gap-3 text-left"
			onclick={onplay}
			aria-label="Play {song.title}"
		>
			{#if index !== undefined}
				<span
					class="relative w-5 shrink-0 text-center text-xs {active
						? 'text-primary'
						: 'text-muted-foreground'}"
				>
					<span class="group-hover:opacity-0">{index + 1}</span>
					<HugeiconsIcon
						icon={PlayIcon}
						class="absolute inset-0 m-auto h-3.5 w-3.5 opacity-0 group-hover:opacity-100"
					/>
				</span>
			{/if}
			{#if song.thumbnail && !hideThumb}
				<img src={song.thumbnail} alt="" class="h-10 w-10 shrink-0 rounded-md object-cover" loading="lazy" />
			{/if}
		</button>
		<div class="min-w-0 flex-1">
			<button class="block max-w-full truncate text-left text-sm font-medium {active ? 'text-primary' : ''}" onclick={onplay}>
				{song.title}
			</button>
			{#if song.artist_id}
				<button
					class="block max-w-full cursor-pointer truncate text-left text-xs text-muted-foreground hover:text-foreground hover:underline"
					onclick={() => goto(`/artist/${encodeURIComponent(song.artist_id!)}`)}
				>
					{song.artists}
				</button>
			{:else}
				<div class="truncate text-xs text-muted-foreground">{song.artists}</div>
			{/if}
		</div>
	</div>

	<div class="flex shrink-0 items-center gap-2">
		{#if song.duration}
			<span class="text-xs text-muted-foreground">{song.duration}</span>
		{/if}
		{#if hasMenu}
			<button
				class="rounded-md p-1.5 text-muted-foreground opacity-0 transition hover:bg-accent/20 hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 {menuOpen
					? 'opacity-100'
					: ''}"
				onclick={openMenu}
				aria-label="Track options"
			>
				<HugeiconsIcon icon={MoreHorizontalIcon} class="h-4 w-4" />
			</button>
		{/if}
	</div>
</div>

{#if menuOpen}
	<button class="fixed inset-0 z-40 cursor-default" onclick={() => (menuOpen = false)} aria-label="Close menu"
	></button>
	<div
		class="fixed z-50 min-w-44 origin-top-right animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style="right:{mx}px; top:{my}px;"
	>
		{#if onAdd}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={() => run(onAdd)}
			>
				<HugeiconsIcon icon={PlayListAddIcon} class="h-4 w-4" /> Add to playlist
			</button>
		{/if}
		{#if onRemove}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-destructive hover:bg-destructive/10"
				onclick={() => run(onRemove)}
			>
				<HugeiconsIcon icon={PlayListRemoveIcon} class="h-4 w-4" /> Remove from playlist
			</button>
		{/if}
	</div>
{/if}
