<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ShuffleIcon, Add01Icon, Tick02Icon } from '@hugeicons/core-free-icons';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import MediaCardSkeleton from '$lib/components/MediaCardSkeleton.svelte';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import TrackRowSkeleton from '$lib/components/TrackRowSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import * as api from '$lib/api';
	import type { ArtistPage } from '$lib/api';
	import { playback, openAddToPlaylist, toast } from '$lib/player.svelte';
	import { getCached, putCached } from '$lib/pagecache';

	let artist = $state<ArtistPage | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let expanded = $state(false);
	let subscribed = $state(false);
	let subBusy = $state(false);

	const id = $derived(page.params.id ?? '');
	const nowId = $derived(playback.now?.videoId);

	async function load(cid: string) {
		const key = `artist:${cid}`;
		const hit = getCached<ArtistPage>(key);
		if (hit) {
			artist = hit;
			subscribed = hit.subscribed;
			loading = false;
		} else {
			loading = true;
			artist = null;
		}
		error = null;
		expanded = false;
		try {
			const fresh = await api.getArtist(cid);
			if (cid !== id) return; // superseded by navigation — drop the stale response
			artist = fresh;
			subscribed = fresh.subscribed;
			putCached(key, fresh);
		} catch (e) {
			if (cid !== id) return;
			if (!hit) error = String(e);
		} finally {
			if (cid === id) loading = false;
		}
	}

	$effect(() => {
		if (id) load(id);
	});

	function shuffle() {
		if (!artist?.topSongs.length) return;
		// ponytail: shuffles the ~5 top songs, not the artist's full catalog radio. Deepen with the
		// header's shuffle playlistId if the shallow mix feels thin.
		const order = [...artist.topSongs].sort(() => Math.random() - 0.5);
		api.playPlaylist(order, 0);
	}

	async function toggleSub() {
		if (!artist || subBusy) return;
		const next = !subscribed;
		subBusy = true;
		subscribed = next; // optimistic
		try {
			await api.subscribe(artist.channelId, next);
			putCached(`artist:${id}`, { ...artist, subscribed: next }); // keep the cache truthful
			toast(next ? `Subscribed to ${artist.name ?? ''}` : `Unsubscribed`);
		} catch (e) {
			subscribed = !next; // revert
			toast(String(e));
		} finally {
			subBusy = false;
		}
	}

	function showMore(section: { title: string; moreBrowseId?: string; moreParams?: string }) {
		const q = new URLSearchParams({ id: section.moreBrowseId!, title: section.title });
		if (section.moreParams) q.set('params', section.moreParams);
		goto(`/list?${q.toString()}`);
	}
</script>

{#if loading}
	<div class="relative flex min-h-[45vh] flex-col justify-end overflow-hidden border-b">
		<Skeleton class="absolute inset-0 h-full w-full rounded-none" />
		<div class="relative space-y-4 p-8">
			<Skeleton class="h-12 w-1/2 rounded-lg" />
			<Skeleton class="h-4 w-40 rounded" />
			<div class="flex gap-3">
				<Skeleton class="h-11 w-28 rounded-full" />
				<Skeleton class="h-11 w-32 rounded-full" />
			</div>
		</div>
	</div>
	<div class="flex flex-col gap-8 p-6">
		<section>
			<Skeleton class="mb-3 h-6 w-32 rounded" />
			{#each Array(5) as _, i (i)}
				<TrackRowSkeleton />
			{/each}
		</section>
		<section>
			<Skeleton class="mb-3 h-6 w-40 rounded" />
			<div class="flex gap-2 overflow-hidden pb-2">
				{#each Array(6) as _, i (i)}
					<div class="w-40 shrink-0"><MediaCardSkeleton /></div>
				{/each}
			</div>
		</section>
	</div>
{:else if error}
	<div class="p-6"><ErrorState message={error} onRetry={() => load(id)} /></div>
{:else if artist}
	<!-- Hero -->
	<div class="relative flex min-h-[45vh] flex-col justify-end overflow-hidden">
		{#if artist.thumbnail}
			<img src={artist.thumbnail} alt="" class="absolute inset-0 h-full w-full object-cover" />
		{/if}
		<div
			class="absolute inset-0 bg-gradient-to-t from-background via-background/60 to-background/10"
		></div>
		<div class="relative max-w-3xl p-8">
			<h1 class="font-heading text-5xl font-bold tracking-tight drop-shadow-lg">{artist.name}</h1>
			{#if artist.subscribers}
				<p class="mt-2 text-sm text-muted-foreground">{artist.subscribers}</p>
			{/if}
			{#if artist.description}
				<p class="mt-3 max-w-2xl text-sm text-foreground/80 {expanded ? '' : 'line-clamp-2'}">
					{artist.description}
				</p>
				<button
					class="mt-1 text-xs font-semibold uppercase text-muted-foreground hover:text-foreground"
					onclick={() => (expanded = !expanded)}
				>
					{expanded ? 'Less' : 'More'}
				</button>
			{/if}
			<div class="mt-5 flex items-center gap-3">
				<button
					class="flex items-center gap-2 rounded-full bg-foreground px-5 py-2.5 text-sm font-semibold text-background transition hover:opacity-90 disabled:opacity-50"
					onclick={shuffle}
					disabled={!artist.topSongs.length}
				>
					<HugeiconsIcon icon={ShuffleIcon} class="h-4 w-4" /> Shuffle
				</button>
				<button
					class="flex items-center gap-2 rounded-full border px-5 py-2.5 text-sm font-semibold transition hover:bg-accent/10 disabled:opacity-60 {subscribed
						? 'border-primary text-primary'
						: ''}"
					onclick={toggleSub}
					disabled={subBusy}
				>
					<HugeiconsIcon icon={Add01Icon} altIcon={Tick02Icon} showAlt={subscribed} class="h-4 w-4" />
					{subscribed ? 'Subscribed' : 'Subscribe'}
				</button>
			</div>
		</div>
	</div>

	<div class="flex flex-col gap-8 p-6">
		{#if artist.topSongs.length}
			<section>
				<h2 class="mb-3 font-heading text-xl font-bold">Top songs</h2>
				{#each artist.topSongs as song, i (song.video_id + i)}
					<TrackRow
						{song}
						active={song.video_id === nowId}
						onplay={() => api.playPlaylist(artist!.topSongs, i)}
						onAdd={() => openAddToPlaylist(song.video_id)}
					/>
				{/each}
			</section>
		{/if}

		{#each artist.sections as section (section.title)}
			<section>
				<div class="mb-3 flex items-center justify-between">
					<h2 class="font-heading text-xl font-bold">{section.title}</h2>
					{#if section.moreBrowseId}
						<button
							class="text-xs font-semibold uppercase text-muted-foreground hover:text-foreground"
							onclick={() => showMore(section)}
						>
							More
						</button>
					{/if}
				</div>
				<div class="flex gap-2 overflow-x-auto pb-2">
					{#each section.items as item (item.id + item.title)}
						<div class="w-40 shrink-0">
							<MediaCard {item} />
						</div>
					{/each}
				</div>
			</section>
		{/each}
	</div>
{/if}
