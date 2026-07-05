<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { PlayIcon, Delete02Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import * as api from '$lib/api';
	import type { PlaylistPage, SongItem } from '$lib/api';
	import { playback, openAddToPlaylist, toast } from '$lib/player.svelte';

	let pl = $state<PlaylistPage | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let loadingMore = $state(false);
	let confirmingDelete = $state(false);

	const id = $derived(page.params.id ?? '');
	const nowId = $derived(playback.now?.videoId);
	// The liked-songs auto-playlist isn't a user playlist — don't offer to delete it.
	const deletable = $derived(id !== 'VLLM');

	async function load(pid: string) {
		loading = true;
		error = null;
		pl = null;
		confirmingDelete = false;
		try {
			pl = await api.getPlaylist(pid);
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	// Reload whenever the route param changes (playlist → playlist navigation).
	$effect(() => {
		if (id) load(id);
	});

	async function loadMore() {
		if (!pl?.continuation || loadingMore) return;
		loadingMore = true;
		try {
			const more = await api.getPlaylistMore(pl.continuation);
			pl = { ...pl, items: [...pl.items, ...more.items], continuation: more.continuation };
		} catch {
			/* keep what we have */
		} finally {
			loadingMore = false;
		}
	}

	function playAll(start: number) {
		if (pl) api.playPlaylist(pl.items, start);
	}

	// The liked-music auto-playlist can't be edited like a normal one — removing = un-liking.
	const isLiked = $derived(id === 'VLLM');

	async function removeTrack(track: SongItem) {
		if (!pl) return;
		if (!isLiked && !track.set_video_id) return;
		const prev = pl.items;
		// Reassign `pl` (not mutate `pl.items`) so the list re-renders immediately. Match by the
		// per-instance setVideoId on normal playlists (duplicates), by videoId on liked music.
		const kept = pl.items.filter((t) =>
			isLiked ? t.video_id !== track.video_id : t.set_video_id !== track.set_video_id
		);
		pl = { ...pl, items: kept };
		try {
			if (isLiked) {
				await api.like(track.video_id, false);
				toast('Removed from Liked Music');
			} else {
				await api.removeFromPlaylist(id, track.video_id, track.set_video_id!);
				toast('Removed from playlist');
			}
		} catch (e) {
			pl = { ...pl, items: prev }; // revert
			toast(String(e));
		}
	}

	async function deleteThisPlaylist() {
		try {
			await api.deletePlaylist(id);
			toast('Playlist deleted');
			goto('/library');
		} catch (e) {
			toast(String(e));
			confirmingDelete = false;
		}
	}
</script>

<div class="flex h-full flex-col">
	{#if loading}
		<div class="p-6 text-sm text-muted-foreground">Loading…</div>
	{:else if error}
		<div class="p-6 text-sm text-destructive">{error}</div>
	{:else if pl}
		<div class="flex items-end gap-6 border-b bg-gradient-to-b from-accent/10 to-transparent p-6">
			{#if pl.thumbnail}
				<img src={pl.thumbnail} alt="" class="h-40 w-40 rounded-xl object-cover shadow-lg" />
			{:else}
				<div class="h-40 w-40 rounded-xl bg-muted"></div>
			{/if}
			<div class="min-w-0 flex-1">
				<div class="text-xs font-medium uppercase text-muted-foreground">Playlist</div>
				<h1 class="mt-1 font-heading text-3xl font-bold">{pl.title ?? 'Playlist'}</h1>
				{#if pl.subtitle}<p class="mt-2 text-sm text-muted-foreground">{pl.subtitle}</p>{/if}
				<div class="mt-4 flex items-center gap-2">
					<Button class="gap-2" onclick={() => playAll(0)} disabled={!pl.items.length}>
						<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" /> Play
					</Button>
					{#if deletable}
						{#if confirmingDelete}
							<div class="flex items-center gap-2 rounded-lg border border-destructive/40 px-2 py-1">
								<span class="text-xs text-muted-foreground">Delete this playlist?</span>
								<Button variant="destructive" size="sm" onclick={deleteThisPlaylist}>Delete</Button>
								<Button variant="ghost" size="sm" onclick={() => (confirmingDelete = false)}>
									Cancel
								</Button>
							</div>
						{:else}
							<Button
								variant="ghost"
								size="icon"
								aria-label="Delete playlist"
								onclick={() => (confirmingDelete = true)}
							>
								<HugeiconsIcon icon={Delete02Icon} class="h-5 w-5 text-muted-foreground" />
							</Button>
						{/if}
					{/if}
				</div>
			</div>
		</div>
		<div class="min-h-0 flex-1 overflow-y-auto p-4">
			{#each pl.items as item, i (item.video_id + i)}
				<TrackRow
					song={item}
					index={i}
					active={item.video_id === nowId}
					onplay={() => playAll(i)}
					onAdd={() => openAddToPlaylist(item.video_id)}
					onRemove={isLiked || item.set_video_id ? () => removeTrack(item) : undefined}
				/>
			{:else}
				<p class="p-4 text-sm text-muted-foreground">This playlist is empty.</p>
			{/each}
			{#if pl.continuation}
				<div class="p-3 text-center">
					<Button variant="outline" size="sm" onclick={loadMore} disabled={loadingMore}>
						{loadingMore ? 'Loading…' : 'Load more'}
					</Button>
				</div>
			{/if}
		</div>
	{/if}
</div>
