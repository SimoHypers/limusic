<script lang="ts">
	import { fade, fly } from 'svelte/transition';
	import { flip } from 'svelte/animate';
	import { cubicOut } from 'svelte/easing';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { InfinityIcon } from '@hugeicons/core-free-icons';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import * as api from '$lib/api';
	import { playback, openAddToPlaylist } from '$lib/player.svelte';
	import { lt } from '$lib/lt.svelte';

	let { onClose }: { onClose: () => void } = $props();

	// Guests are add-only in a session — no removing (theirs or anyone's). The playing row can't
	// be removed either (backend guards it too).
	const canRemove = $derived(lt.role !== 'guest');

	// Stable row keys (video_id + occurrence #) so animate:flip can slide rows up when one is
	// removed. Keying by index — the old scheme — rekeys every row after the removed one, which
	// recreates them and kills the animation. Duplicate tracks are why video_id alone won't do.
	const keyed = $derived.by(() => {
		const seen = new Map<string, number>();
		return playback.queue.items.map((item) => {
			const n = seen.get(item.video_id) ?? 0;
			seen.set(item.video_id, n + 1);
			return { item, key: `${item.video_id}:${n}` };
		});
	});
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
		{#each keyed as { item, key }, i (key)}
			<div animate:flip={{ duration: 200, easing: cubicOut }}>
				<!-- Subtle marker where the chosen queue ends and autoplay's continuation begins. -->
				{#if item.autoplay && !playback.queue.items[i - 1]?.autoplay}
					<div
						class="mt-2 flex items-center gap-2 border-t px-2 pt-2.5 pb-1.5 text-muted-foreground"
						title="Autoplay keeps the music going with similar songs. Turn it off in Settings ▸ Playback."
					>
						<HugeiconsIcon icon={InfinityIcon} class="h-3.5 w-3.5" />
						<span class="text-xs font-medium">Autoplay</span>
						<span class="truncate text-xs">· similar music</span>
					</div>
				{/if}
				<TrackRow
					song={item}
					index={i}
					active={i === playback.queue.currentIndex}
					onplay={() => api.playIndex(i)}
					onAdd={() => openAddToPlaylist(item)}
					onRemove={canRemove && i !== playback.queue.currentIndex
						? () => api.removeFromQueue(i)
						: undefined}
					removeLabel="Remove from queue"
				/>
			</div>
		{:else}
			<p class="p-4 text-sm text-muted-foreground">The queue is empty.</p>
		{/each}
	</div>
</aside>
