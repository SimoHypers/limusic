<script lang="ts">
	import TrackRow from '$lib/components/TrackRow.svelte';
	import * as api from '$lib/api';
	import { playback, openAddToPlaylist } from '$lib/player.svelte';
</script>

<aside class="flex h-full w-80 shrink-0 flex-col border-l bg-card/40">
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
