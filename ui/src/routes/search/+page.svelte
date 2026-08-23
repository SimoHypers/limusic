<script module lang="ts">
	// Survives remounts (module scope), so coming back to /search — from a result you clicked, or
	// from the sidebar — shows the last search instead of a blank page. The results themselves come
	// back from the page cache, so the rerun paints instantly and just revalidates.
	let lastQuery = '';
</script>

<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Search01Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import MediaCardSkeleton from '$lib/components/MediaCardSkeleton.svelte';
	import SearchSuggest from '$lib/components/SearchSuggest.svelte';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import TrackRowSkeleton from '$lib/components/TrackRowSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import Shelf from '$lib/components/Shelf.svelte';
	import * as api from '$lib/api';
	import type { SearchResults, SongItem } from '$lib/api';
	import { getCached, putCached } from '$lib/pagecache';
	import { openAddToPlaylist, playSong } from '$lib/player.svelte';
	import { asSong } from '$lib/browse';
	import { t } from '$lib/i18n.svelte';

	type Cached = { res: SearchResults; songs: SongItem[] };

	let query = $state(lastQuery);
	let res = $state<SearchResults | null>(null);
	// The Songs shelf comes from the songs-filtered search, not from `res.songs`: an unfiltered
	// response gives a song row either its artist or its length, never both, so those rows land
	// duration-less. The filtered endpoint returns "Artist • Album • 3:58" on every row.
	let songs = $state<SongItem[]>([]);
	let searched = $state('');
	let searching = $state(false);
	let error = $state<string | null>(null);

	// The query of the most recent runSearch call, so an older in-flight one can't clobber it.
	let latest = '';

	async function runSearch() {
		if (!query.trim()) return;
		const q = query;
		latest = q;
		lastQuery = q;
		const key = `search:${q}`;
		const hit = getCached<Cached>(key);
		if (hit) {
			res = hit.res;
			songs = hit.songs;
			searched = q;
			searching = false;
		} else {
			searching = true;
		}
		error = null;
		try {
			// In parallel, and the filtered one may fail on its own: the shelf falls back to the
			// unfiltered rows rather than the whole search erroring out.
			const [fresh, freshSongs] = await Promise.all([
				api.searchAll(q),
				api.search(q).catch(() => [] as SongItem[])
			]);
			if (latest !== q) return; // a newer search superseded this one
			res = fresh;
			songs = freshSongs;
			searched = q;
			putCached(key, { res: fresh, songs: freshSongs });
		} catch (e) {
			if (latest !== q) return;
			if (!hit) error = String(e);
		} finally {
			if (latest === q) searching = false;
		}
	}

	function showMore(cat: 'songs' | 'albums' | 'artists' | 'playlists') {
		goto(`/search-more?${new URLSearchParams({ q: searched, cat }).toString()}`);
	}

	// Run the search when arriving with a ?q= (e.g. from the Home search box). Keyed on the URL
	// alone: typing a new query in the field must not look like a URL change and bounce us back.
	const urlQuery = $derived(page.url.searchParams.get('q') ?? '');
	let lastUrlQuery = '';
	$effect(() => {
		if (urlQuery && urlQuery !== lastUrlQuery) {
			lastUrlQuery = urlQuery;
			query = urlQuery;
			runSearch();
		}
	});

	// Arriving without a ?q= (back from a result, or the sidebar link): rerun whatever was last
	// searched. onMount, not the effect above, so a ?q= arrival still wins.
	onMount(() => {
		if (!urlQuery && query) runSearch();
	});

	const songRows = $derived(songs.length ? songs : (res?.songs ?? []).map(asSong));

	// Sections are horizontal card rows, except Songs which is a vertical list. `top` has no "show more".
	const sections = $derived(
		res
			? [
					{ key: 'top', label: t('common.top_results'), items: res.top, max: 4, more: false, list: false },
					{ key: 'songs', label: t('common.songs'), items: res.songs, max: 6, more: true, list: true },
					{ key: 'albums', label: t('common.albums'), items: res.albums, max: 5, more: true, list: false },
					{ key: 'artists', label: t('common.artists'), items: res.artists, max: 3, more: true, list: false },
					{ key: 'playlists', label: t('common.playlists'), items: res.playlists, max: 5, more: true, list: false }
				].filter((s) => (s.list ? songRows.length : s.items.length))
			: []
	);

</script>

<div class="flex h-full flex-col">
	<div class="border-b p-6">
		<h1 class="mb-4 font-heading text-2xl font-bold">{t('common.search')}</h1>
		<form
			class="flex max-w-xl gap-2"
			onsubmit={(e) => {
				e.preventDefault();
				runSearch();
			}}
		>
			<SearchSuggest
				bind:value={query}
				placeholder={t('common.search_placeholder')}
				onpick={() => (lastQuery = query)}
			/>
			<Button type="submit" class="gap-2" disabled={searching}>
				<HugeiconsIcon icon={Search01Icon} class="h-4 w-4" />
				{searching ? t('common.searching') : t('common.search')}
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
							{#each songRows.slice(0, sec.max) as song (song.video_id)}
								<TrackRow
									{song}
									showPlayCount
									onplay={() => playSong(song)}
									onAdd={() => openAddToPlaylist(song)}
								/>
							{/each}
						{:else}
							<Shelf items={sec.items.slice(0, sec.max)} />
						{/if}
					</section>
				{/each}
			</div>
		{/if}
	</div>
</div>
