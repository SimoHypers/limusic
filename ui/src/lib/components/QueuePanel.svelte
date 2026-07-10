<script lang="ts">
	import { fade, fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import * as api from '$lib/api';
	import { playback, openAddToPlaylist } from '$lib/player.svelte';

	let { onClose }: { onClose: () => void } = $props();
</script>

<!-- Below lg the panel floats over the content (see the `relative` wrapper in +layout); a scrim
     lets you dismiss it by clicking outside. At lg+ it's an in-flow column and the scrim is hidden. -->
<button
	class="absolute inset-0 z-20 cursor-default bg-black/40 lg:hidden"
	onclick={onClose}
	aria-label="Close queue"
	transition:fade={{ duration: 150 }}
></button>
<aside
	transition:fly={{ x: 32, duration: 220, easing: cubicOut }}
	class="absolute inset-y-0 right-0 z-30 flex h-full w-80 max-w-[80vw] shrink-0 flex-col border-l bg-card shadow-2xl lg:static lg:z-auto lg:max-w-none lg:bg-card/40 lg:shadow-none"
>
	<h2 class="border-b px-4 py-3 font-heading text-sm font-semibold">Up next</h2>
	<div class="min-h-0 flex-1 overflow-y-auto p-2">
		{#each playback.queue.items as item, i (item.video_id + i)}
			<TrackRow
				song={item}
				index={i}
				active={i === playback.queue.currentIndex}
				onplay={() => api.playIndex(i)}
				onAdd={() => openAddToPlaylist(item.video_id)}
			/>
		{:else}
			<p class="p-4 text-sm text-muted-foreground">The queue is empty.</p>
		{/each}
	</div>
</aside>
