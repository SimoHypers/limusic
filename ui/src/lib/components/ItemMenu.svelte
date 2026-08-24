<script lang="ts">
	// The ⋯ menu for a browse item, whichever kind it is: songs get TrackMenu, everything else
	// PlaylistMenu. Cards and rows all want the same split, so it lives here instead of in each one.
	import type { BrowseItem } from '$lib/api';
	import { asSong } from '$lib/browse';
	import { openAddToPlaylist } from '$lib/player.svelte';
	import TrackMenu from './TrackMenu.svelte';
	import PlaylistMenu from './PlaylistMenu.svelte';

	let {
		item,
		triggerClass
	}: { item: BrowseItem; triggerClass: string } = $props();
</script>

{#if item.kind === 'song'}
	<TrackMenu song={asSong(item)} onAdd={() => openAddToPlaylist(asSong(item))} {triggerClass} />
{:else}
	<PlaylistMenu {item} showPin={item.kind === 'playlist'} {triggerClass} />
{/if}
