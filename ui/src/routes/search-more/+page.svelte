<script lang="ts">
	import { page } from '$app/state';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import TrackRowSkeleton from '$lib/components/TrackRowSkeleton.svelte';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import MediaCardSkeleton from '$lib/components/MediaCardSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import * as api from '$lib/api';
	import type { BrowseItem, SongItem } from '$lib/api';
	import { openAddToPlaylist } from '$lib/player.svelte';
	import { getCached, putCached } from '$lib/pagecache';

	type MoreResult = { songs: SongItem[]; cards: BrowseItem[] };

	let songs = $state<SongItem[]>([]);
	let cards = $state<BrowseItem[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	const q = $derived(page.url.searchParams.get('q') ?? '');
	const cat = $derived(page.url.searchParams.get('cat') ?? 'songs');
	const label = $derived({ songs: 'Songs', albums: 'Albums', artists: 'Artists', playlists: 'Playlists' }[cat] ?? 'Results');

	async function load(query: string, category: string) {
		const key = `searchmore:${category}:${query}`;
		const hit = getCached<MoreResult>(key);
		if (hit) {
			songs = hit.songs;
			cards = hit.cards;
			loading = false;
		} else {
			loading = true;
			songs = [];
			cards = [];
		}
		error = null;
		try {
			let fresh: MoreResult;
			if (category === 'songs') {
				fresh = { songs: await api.search(query), cards: [] };
			} else {
				fresh = {
					songs: [],
					cards: await api.searchCards(query, category as 'albums' | 'artists' | 'playlists')
				};
			}
			if (query !== q || category !== cat) return; // superseded by navigation
			songs = fresh.songs;
			cards = fresh.cards;
			putCached(key, fresh);
		} catch (e) {
			if (query !== q || category !== cat) return;
			if (!hit) error = String(e);
		} finally {
			if (query === q && category === cat) loading = false;
		}
	}

	$effect(() => {
		if (q) load(q, cat);
	});
</script>

<div class="p-6">
	<h1 class="mb-1 font-heading text-2xl font-bold">{label}</h1>
	<p class="mb-6 text-sm text-muted-foreground">Results for “{q}”</p>

	{#if loading}
		{#if cat === 'songs'}
			{#each Array(10) as _, i (i)}
				<TrackRowSkeleton />
			{/each}
		{:else}
			<div class="grid grid-cols-[repeat(auto-fill,10rem)] gap-4">
				{#each Array(12) as _, i (i)}
					<MediaCardSkeleton />
				{/each}
			</div>
		{/if}
	{:else if error}
		<ErrorState message={error} onRetry={() => load(q, cat)} />
	{:else if cat === 'songs'}
		<div class="content-in">
			{#each songs as song (song.video_id)}
				<TrackRow {song} onplay={() => api.play(song)} onAdd={() => openAddToPlaylist(song)} />
			{:else}
				<p class="text-sm text-muted-foreground">Nothing found.</p>
			{/each}
		</div>
	{:else if cards.length}
		<div class="content-in grid grid-cols-[repeat(auto-fill,10rem)] gap-4">
			{#each cards as item (item.id + item.title)}
				<MediaCard {item} />
			{/each}
		</div>
	{:else}
		<p class="text-sm text-muted-foreground">Nothing found.</p>
	{/if}
</div>
