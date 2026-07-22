<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Search01Icon } from '@hugeicons/core-free-icons';
	import { Input } from '$lib/components/ui/input';
	import { Button } from '$lib/components/ui/button';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import MediaCardSkeleton from '$lib/components/MediaCardSkeleton.svelte';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import TrackRowSkeleton from '$lib/components/TrackRowSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import * as api from '$lib/api';
	import type { SearchResults } from '$lib/api';
	import { getCached, putCached } from '$lib/pagecache';
	import { openAddToPlaylist } from '$lib/player.svelte';

	let query = $state('');
	let res = $state<SearchResults | null>(null);
	let searched = $state('');
	let searching = $state(false);
	let error = $state<string | null>(null);

	async function runSearch() {
		if (!query.trim()) return;
		const q = query;
		const key = `search:${q}`;
		const hit = getCached<SearchResults>(key);
		if (hit) {
			res = hit;
			searched = q;
			searching = false;
		} else {
			searching = true;
		}
		error = null;
		try {
			const fresh = await api.searchAll(q);
			if (urlQuery && urlQuery !== q) return; // a newer URL-driven search superseded this one
			res = fresh;
			searched = q;
			putCached(key, fresh);
		} catch (e) {
			if (urlQuery && urlQuery !== q) return;
			if (!hit) error = String(e);
		} finally {
			if (!urlQuery || urlQuery === q) searching = false;
		}
	}

	function showMore(cat: 'songs' | 'albums' | 'artists' | 'playlists') {
		goto(`/search-more?${new URLSearchParams({ q: searched, cat }).toString()}`);
	}

	// Run the search when arriving with a ?q= (e.g. from the Home search box).
	const urlQuery = $derived(page.url.searchParams.get('q') ?? '');
	$effect(() => {
		if (urlQuery && urlQuery !== searched) {
			query = urlQuery;
			runSearch();
		}
	});

	// Sections are horizontal card rows, except Songs which is a vertical list. `top` has no "show more".
	const sections = $derived(
		res
			? [
					{ key: 'top', label: 'Top results', items: res.top, max: 4, more: false, list: false },
					{ key: 'songs', label: 'Songs', items: res.songs, max: 6, more: true, list: true },
					{ key: 'albums', label: 'Albums', items: res.albums, max: 5, more: true, list: false },
					{ key: 'artists', label: 'Artists', items: res.artists, max: 3, more: true, list: false },
					{ key: 'playlists', label: 'Playlists', items: res.playlists, max: 5, more: true, list: false }
				].filter((s) => s.items.length)
			: []
	);

	// A song search result is a flat BrowseItem (kind=song); map it to the SongItem shape TrackRow
	// wants. `artists` = the full "Song • Artist • plays" subtitle YouTube gives the row.
	const asSong = (item: SearchResults['songs'][number]) => ({
		video_id: item.id,
		title: item.title,
		artists: item.subtitle ?? '',
		thumbnail: item.thumbnail
	});
</script>

<div class="flex h-full flex-col">
	<div class="border-b p-6">
		<h1 class="mb-4 font-heading text-2xl font-bold">Search</h1>
		<form
			class="flex max-w-xl gap-2"
			onsubmit={(e) => {
				e.preventDefault();
				runSearch();
			}}
		>
			<Input bind:value={query} placeholder="Search songs, albums, artists, playlists…" />
			<Button type="submit" class="gap-2" disabled={searching}>
				<HugeiconsIcon icon={Search01Icon} class="h-4 w-4" />
				{searching ? 'Searching…' : 'Search'}
			</Button>
		</form>
		{#if error}<div class="mt-2"><ErrorState message={error} onRetry={runSearch} /></div>{/if}
	</div>

	<div class="min-h-0 flex-1 overflow-y-auto p-6">
		{#if searching}
			<div class="flex flex-col gap-10">
				<section>
					<Skeleton class="mb-3 h-6 w-40 rounded" />
					{#each Array(5) as _, i (i)}
						<TrackRowSkeleton />
					{/each}
				</section>
				<section>
					<Skeleton class="mb-3 h-6 w-32 rounded" />
					<div class="flex gap-2 overflow-hidden pb-2">
						{#each Array(5) as _, i (i)}
							<div class="w-40 shrink-0"><MediaCardSkeleton /></div>
						{/each}
					</div>
				</section>
			</div>
		{:else if !res}
			<p class="text-sm text-muted-foreground">Search for a song, album, artist, or playlist.</p>
		{:else if !sections.length}
			<p class="text-sm text-muted-foreground">No results for “{searched}”.</p>
		{:else}
			<div class="content-in flex flex-col gap-10">
				{#each sections as sec (sec.key)}
					<section>
						<div class="mb-3 flex items-center justify-between">
							<h2 class="font-heading text-xl font-bold">{sec.label}</h2>
							{#if sec.more}
								<button
									class="cursor-pointer text-xs font-semibold uppercase text-muted-foreground hover:text-foreground"
									onclick={() => showMore(sec.key as 'songs' | 'albums' | 'artists' | 'playlists')}
								>
									Show more
								</button>
							{/if}
						</div>
						{#if sec.list}
							{#each sec.items.slice(0, sec.max) as item (item.id)}
								{@const song = asSong(item)}
								<TrackRow {song} onplay={() => api.play(song)} onAdd={() => openAddToPlaylist(song)} />
							{/each}
						{:else}
							<div class="flex gap-2 overflow-x-auto pb-2">
								{#each sec.items.slice(0, sec.max) as item (item.id + item.title)}
									<div class="w-40 shrink-0"><MediaCard {item} /></div>
								{/each}
							</div>
						{/if}
					</section>
				{/each}
			</div>
		{/if}
	</div>
</div>
