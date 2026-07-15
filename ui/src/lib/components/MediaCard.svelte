<script lang="ts">
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { PlayIcon } from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import type { BrowseItem, SongItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { openAddToPlaylist } from '$lib/player.svelte';
	import TrackMenu from './TrackMenu.svelte';

	let { item }: { item: BrowseItem } = $props();

	const round = $derived(item.kind === 'artist');
	// Song cards get the same ⋯ menu as list rows (Add to queue / like) — the card shape it maps to.
	const asSong = (i: BrowseItem): SongItem => ({
		video_id: i.id,
		title: i.title,
		artists: i.subtitle ?? '',
		thumbnail: i.thumbnail
	});

	function activate() {
		if (item.kind === 'song') {
			api.play({
				video_id: item.id,
				title: item.title,
				artists: item.subtitle ?? '',
				thumbnail: item.thumbnail
			});
		} else if (item.kind === 'artist') {
			goto(`/artist/${encodeURIComponent(item.id)}`);
		} else if (item.kind === 'album') {
			goto(`/album/${encodeURIComponent(item.id)}`);
		} else {
			goto(`/playlist/${encodeURIComponent(item.id)}`);
		}
	}
</script>

<div class="group relative flex w-full flex-col gap-2">
	<button
		class="flex flex-col gap-2 rounded-xl p-2 text-left transition-colors hover:bg-accent/10"
		onclick={activate}
	>
		<div
			class="relative aspect-square w-full overflow-hidden bg-muted shadow-sm transition-shadow duration-300 group-hover:shadow-xl {round
				? 'rounded-full'
				: 'rounded-lg'}"
		>
			{#if item.thumbnail}
				<img
					src={thumb(item.thumbnail, 400)}
					alt=""
					class="h-full w-full object-cover transition-transform duration-300 ease-out group-hover:scale-105"
					loading="lazy"
				/>
			{/if}
			{#if item.kind !== 'artist'}
				<div
					class="absolute bottom-2 right-2 flex h-9 w-9 translate-y-1 items-center justify-center rounded-full bg-primary text-primary-foreground opacity-0 shadow-lg transition-all duration-200 ease-out group-hover:translate-y-0 group-hover:opacity-100"
				>
					<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" />
				</div>
			{/if}
		</div>
		<div class="min-w-0 {round ? 'text-center' : ''}">
			<div class="truncate text-sm font-medium">{item.title}</div>
			{#if item.subtitle}
				<div class="truncate text-xs text-muted-foreground">{item.subtitle}</div>
			{/if}
		</div>
	</button>
	{#if item.kind === 'song'}
		<TrackMenu
			song={asSong(item)}
			onAdd={() => openAddToPlaylist(item.id)}
			triggerClass="absolute right-3 top-3 flex h-8 w-8 items-center justify-center rounded-full bg-background/80 text-foreground opacity-0 shadow-md backdrop-blur-sm transition hover:bg-background focus-visible:opacity-100 group-hover:opacity-100"
		/>
	{/if}
</div>
