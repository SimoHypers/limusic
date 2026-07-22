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

	type Row = { item: import('$lib/api').SongItem; key: string; i: number; n: number };

	// Spotify-style sections over the one flat queue: the playing track, then upcoming manual
	// adds ("Next in queue"), then the source context ("Next from: …"), then autoplay's
	// continuation. Already-played tracks are hidden (Previous still works — they stay in the
	// backend queue). Keys are video_id + occurrence # (not index) so animate:flip slides rows
	// instead of recreating them; `n` renumbers the visible rows from 1.
	const sections = $derived.by(() => {
		const { items, currentIndex } = playback.queue;
		const seen = new Map<string, number>();
		let n = 0;
		const row = (i: number): Row => {
			const item = items[i];
			const occ = seen.get(item.video_id) ?? 0;
			seen.set(item.video_id, occ + 1);
			return { item, key: `${item.video_id}:${occ}`, i, n: ++n };
		};
		// Occurrence counting must walk the played prefix too, or a repeated track's key would
		// collide with its hidden earlier copy.
		for (let i = 0; i < currentIndex; i++) row(i);
		n = 0;
		const now = items[currentIndex] ? row(currentIndex) : null;
		const queued: Row[] = [];
		const upNext: Row[] = [];
		const auto: Row[] = [];
		for (let i = currentIndex + 1; i < items.length; i++) {
			const r = row(i);
			if (r.item.queued) queued.push(r);
			else if (r.item.autoplay) auto.push(r);
			else upNext.push(r);
		}
		return { now, queued, upNext, auto };
	});

	const nextFromLabel = $derived(
		playback.queue.sourceName ? `Next from: ${playback.queue.sourceName}` : 'Next up'
	);
</script>

{#snippet rows(list: Row[])}
	{#each list as { item, key, i, n } (key)}
		<div animate:flip={{ duration: 200, easing: cubicOut }}>
			<TrackRow
				song={item}
				index={n - 1}
				active={i === playback.queue.currentIndex}
				onplay={() => api.playIndex(i)}
				onAdd={() => openAddToPlaylist(item)}
				onRemove={canRemove && i !== playback.queue.currentIndex
					? () => api.removeFromQueue(i)
					: undefined}
				removeLabel="Remove from queue"
			/>
		</div>
	{/each}
{/snippet}

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
	<h2 class="border-b px-4 py-3 font-heading text-sm font-semibold">Queue</h2>
	<div class="min-h-0 flex-1 overflow-y-auto p-2">
		{#if sections.now}
			<h3 class="px-2 pt-2 pb-1.5 text-sm font-semibold">Now playing</h3>
			{@render rows([sections.now])}

			{#if sections.queued.length}
				<div class="mt-3 flex items-center justify-between px-2 pb-1.5">
					<h3 class="text-sm font-semibold">Next in queue</h3>
					{#if canRemove}
						<button
							class="cursor-pointer text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
							onclick={() => api.clearQueued()}
						>
							Clear queue
						</button>
					{/if}
				</div>
				{@render rows(sections.queued)}
			{/if}

			{#if sections.upNext.length}
				<h3 class="mt-3 px-2 pb-1.5 text-sm font-semibold">{nextFromLabel}</h3>
				{@render rows(sections.upNext)}
			{/if}

			{#if sections.auto.length}
				<div
					class="mt-3 flex items-center gap-2 border-t px-2 pt-2.5 pb-1.5 text-muted-foreground"
					title="Autoplay keeps the music going with similar songs. Turn it off in Settings ▸ Playback."
				>
					<HugeiconsIcon icon={InfinityIcon} class="h-3.5 w-3.5" />
					<span class="text-xs font-medium">Autoplay</span>
					<span class="truncate text-xs">· similar music</span>
				</div>
				{@render rows(sections.auto)}
			{/if}
		{:else}
			<p class="p-4 text-sm text-muted-foreground">The queue is empty.</p>
		{/if}
	</div>
</aside>
