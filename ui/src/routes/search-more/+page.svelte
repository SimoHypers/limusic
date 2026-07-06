<script lang="ts">
	import { page } from '$app/state';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import * as api from '$lib/api';
	import type { BrowseItem, SongItem } from '$lib/api';
	import { openAddToPlaylist } from '$lib/player.svelte';

	let songs = $state<SongItem[]>([]);
	let cards = $state<BrowseItem[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	const q = $derived(page.url.searchParams.get('q') ?? '');
	const cat = $derived(page.url.searchParams.get('cat') ?? 'songs');
	const label = $derived({ songs: 'Songs', albums: 'Albums', artists: 'Artists', playlists: 'Playlists' }[cat] ?? 'Results');

	async function load(query: string, category: string) {
		loading = true;
		error = null;
		songs = [];
		cards = [];
		try {
			if (category === 'songs') {
				songs = await api.search(query);
			} else {
				cards = await api.searchCards(query, category as 'albums' | 'artists' | 'playlists');
			}
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
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
		<p class="text-sm text-muted-foreground">Loading…</p>
	{:else if error}
		<p class="text-sm text-destructive">{error}</p>
	{:else if cat === 'songs'}
		{#each songs as song (song.video_id)}
			<TrackRow {song} onplay={() => api.play(song)} onAdd={() => openAddToPlaylist(song.video_id)} />
		{:else}
			<p class="text-sm text-muted-foreground">Nothing found.</p>
		{/each}
	{:else if cards.length}
		<div class="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-2">
			{#each cards as item (item.id + item.title)}
				<MediaCard {item} />
			{/each}
		</div>
	{:else}
		<p class="text-sm text-muted-foreground">Nothing found.</p>
	{/if}
</div>
