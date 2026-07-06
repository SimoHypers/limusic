<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ShuffleIcon, Add01Icon, Tick02Icon } from '@hugeicons/core-free-icons';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import * as api from '$lib/api';
	import type { ArtistPage } from '$lib/api';
	import { playback, openAddToPlaylist, toast } from '$lib/player.svelte';

	let artist = $state<ArtistPage | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let expanded = $state(false);
	let subscribed = $state(false);
	let subBusy = $state(false);

	const id = $derived(page.params.id ?? '');
	const nowId = $derived(playback.now?.videoId);

	async function load(cid: string) {
		loading = true;
		error = null;
		artist = null;
		expanded = false;
		try {
			artist = await api.getArtist(cid);
			subscribed = artist.subscribed;
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
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
	<div class="p-6 text-sm text-muted-foreground">Loading…</div>
{:else if error}
	<div class="p-6 text-sm text-destructive">{error}</div>
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
					<HugeiconsIcon icon={subscribed ? Tick02Icon : Add01Icon} class="h-4 w-4" />
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
