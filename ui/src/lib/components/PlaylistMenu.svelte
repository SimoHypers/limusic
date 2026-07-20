<script lang="ts">
	// The ⋯ menu on a sidebar library row. Positioned `fixed` like TrackMenu — the playlist list is a
	// scroll container, so an absolute popup would be clipped by it.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		MoreHorizontalIcon,
		PinIcon,
		PinOffIcon,
		DashboardSquare02Icon
	} from '@hugeicons/core-free-icons';
	import type { BrowseItem } from '$lib/api';
	import { addPick, personal, togglePin } from '$lib/player.svelte';

	let { item }: { item: BrowseItem } = $props();

	const pinned = $derived(personal.pins.includes(item.id));

	let menuOpen = $state(false);
	let mx = $state(0);
	let my = $state(0);

	function openMenu(e: MouseEvent) {
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

<button
	class="absolute right-1 top-1/2 flex h-7 w-7 -translate-y-1/2 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition hover:bg-sidebar-accent hover:text-foreground focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring group-hover/row:opacity-100 {menuOpen
		? 'opacity-100'
		: 'opacity-0'}"
	onclick={openMenu}
	aria-label="Playlist options"
>
	<HugeiconsIcon icon={MoreHorizontalIcon} class="h-4 w-4" />
</button>

{#if menuOpen}
	<button
		class="fixed inset-0 z-40 cursor-default"
		onclick={() => (menuOpen = false)}
		aria-label="Close menu"
	></button>
	<div
		class="fixed z-50 min-w-48 origin-top-right animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style="right:{mx}px; top:{my}px;"
	>
		<button
			class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={() => run(() => togglePin(item.id))}
		>
			<HugeiconsIcon icon={pinned ? PinOffIcon : PinIcon} class="h-4 w-4" />
			{pinned ? 'Unpin' : 'Pin to top'}
		</button>
		<button
			class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={() => run(() => addPick(item))}
		>
			<HugeiconsIcon icon={DashboardSquare02Icon} class="h-4 w-4" /> Add to Quick Picks
		</button>
	</div>
{/if}
