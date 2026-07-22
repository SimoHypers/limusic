<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		PlayIcon,
		ShuffleIcon,
		PencilEdit02Icon,
		Delete02Icon,
		MoreVerticalIcon,
		Tick02Icon,
		Cancel01Icon,
		DashboardSquare02Icon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import TrackRowSkeleton from '$lib/components/TrackRowSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import * as api from '$lib/api';
	import type { BrowseItem, PlaylistPage, SongItem } from '$lib/api';
	import { getCached, putCached, invalidateCached } from '$lib/pagecache';
	import { addPick, playback, openAddToPlaylist, playFrom, toast } from '$lib/player.svelte';

	let pl = $state<PlaylistPage | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let loadingMore = $state(false);
	let confirmingDelete = $state(false);
	// A random song's cover, used as a blurred hero backdrop (like the artist/album pages).
	let bgImage = $state<string | null>(null);

	// ⋯ options menu, positioned `fixed` at the button so it isn't clipped (matches TrackRow).
	let menuOpen = $state(false);
	let mx = $state(0);
	let my = $state(0);

	// Inline rename state.
	let editingName = $state(false);
	let nameDraft = $state('');

	const id = $derived(page.params.id ?? '');
	const nowId = $derived(playback.now?.videoId);
	// The liked-music auto-playlist isn't a user playlist — no rename/delete, but shuffle is fine.
	const isLiked = $derived(id === 'VLLM');
	// Only offer rename/delete on playlists the signed-in user actually owns (backend `owned` flag).
	// Liked Music reports owned but can't be renamed/deleted, so exclude it explicitly.
	const editable = $derived((pl?.owned ?? false) && !isLiked);

	async function load(pid: string) {
		const key = `playlist:${pid}`;
		const hit = getCached<PlaylistPage>(key);
		confirmingDelete = false;
		editingName = false;
		if (hit) {
			pl = hit;
			bgImage = pickCover(hit.items);
			loading = false;
		} else {
			loading = true;
			pl = null;
			bgImage = null;
		}
		error = null;
		try {
			const fresh = await api.getPlaylist(pid);
			if (pid !== id) return; // superseded by navigation — drop the stale response
			pl = fresh;
			bgImage = pickCover(fresh.items);
			putCached(key, fresh);
		} catch (e) {
			if (pid !== id) return;
			if (!hit) error = String(e);
		} finally {
			if (pid === id) loading = false;
		}
	}

	// Reload whenever the route param changes (playlist → playlist navigation).
	$effect(() => {
		if (id) load(id);
	});

	// Keep the page cache in step with optimistic mutations so a revisit within the TTL never
	// resurrects pre-mutation data (the optimistic-UI contract). context: plans/007.
	function cacheCurrent() {
		if (pl) putCached(`playlist:${id}`, pl);
	}

	async function loadMore() {
		if (!pl?.continuation || loadingMore) return;
		loadingMore = true;
		try {
			const more = await api.getPlaylistMore(pl.continuation);
			pl = { ...pl, items: [...pl.items, ...more.items], continuation: more.continuation };
			cacheCurrent();
		} catch {
			/* keep what we have */
		} finally {
			loadingMore = false;
		}
	}

	// This playlist as a card, for the sidebar's last-played sort and the Quick Picks grid.
	const asItem = (): BrowseItem => ({
		kind: 'playlist',
		id,
		title: pl?.title ?? 'Playlist',
		subtitle: pl?.subtitle,
		thumbnail: pl?.thumbnail ?? bgImage ?? undefined
	});

	function playAll(start: number | null) {
		if (pl) playFrom(asItem(), pl.items, start, id);
	}

	// Random cover from the songs, picked once per load so it stays stable while browsing
	// (loadMore appends tracks without changing it).
	function pickCover(items: SongItem[]): string | null {
		const withThumb = items.filter((t) => t.thumbnail);
		if (!withThumb.length) return null;
		const url = withThumb[Math.floor(Math.random() * withThumb.length)].thumbnail!;
		return hiRes(url);
	}

	// List thumbnails come at a small size; YouTube/Google encode the size in the URL, so bump it
	// for a crisp full-width backdrop.
	function hiRes(url: string): string {
		return url.replace(/=w\d+-h\d+/, '=w1200-h1200').replace(/=s\d+/, '=s1200');
	}

	function shufflePlay() {
		if (!pl?.items.length) return;
		const a = [...pl.items];
		for (let i = a.length - 1; i > 0; i--) {
			const j = Math.floor(Math.random() * (i + 1));
			[a[i], a[j]] = [a[j], a[i]];
		}
		playFrom(asItem(), a, 0, id);
	}

	function openMenu(e: MouseEvent) {
		const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
		mx = r.left;
		my = r.bottom + 4;
		menuOpen = true;
	}
	function run(action: () => void) {
		menuOpen = false;
		action();
	}

	function startRename() {
		nameDraft = pl?.title ?? '';
		editingName = true;
	}

	async function saveRename() {
		const name = nameDraft.trim();
		if (!pl || !name || name === pl.title) {
			editingName = false;
			return;
		}
		const prev = pl.title;
		pl = { ...pl, title: name }; // optimistic
		editingName = false;
		try {
			await api.renamePlaylist(id, name);
			cacheCurrent();
			toast('Playlist renamed');
		} catch (e) {
			pl = { ...pl, title: prev }; // revert
			cacheCurrent();
			toast(String(e));
		}
	}

	// The liked-music auto-playlist can't be edited like a normal one — removing = un-liking.
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
			cacheCurrent();
		} catch (e) {
			pl = { ...pl, items: prev }; // revert
			cacheCurrent();
			toast(String(e));
		}
	}

	async function deleteThisPlaylist() {
		try {
			await api.deletePlaylist(id);
			invalidateCached(`playlist:${id}`);
			toast('Playlist deleted');
			goto('/library');
		} catch (e) {
			toast(String(e));
			confirmingDelete = false;
		}
	}

	function autofocus(node: HTMLInputElement) {
		node.focus();
		node.select();
	}
</script>

<div class="flex h-full flex-col">
	{#if loading}
		<div class="flex items-end gap-6 border-b p-6">
			<Skeleton class="h-40 w-40 shrink-0 rounded-xl" />
			<div class="flex-1 space-y-3">
				<Skeleton class="h-3 w-16 rounded" />
				<Skeleton class="h-10 w-2/3 rounded-lg" />
				<Skeleton class="h-4 w-40 rounded" />
				<Skeleton class="h-9 w-24 rounded-4xl" />
			</div>
		</div>
		<div class="p-4">
			{#each Array(8) as _, i (i)}
				<TrackRowSkeleton />
			{/each}
		</div>
	{:else if error}
		<div class="p-6"><ErrorState message={error} onRetry={() => load(id)} /></div>
	{:else if pl}
		<div class="content-in relative flex min-h-[38vh] items-end gap-6 overflow-hidden border-b p-6">
			{#if bgImage}
				<img
					src={bgImage}
					alt=""
					class="pointer-events-none absolute inset-0 h-full w-full object-cover object-center"
				/>
			{/if}
			<!-- Fade the cover into the page so the text stays readable: solid at the bottom and on the
			     left (behind the title), the image itself visible toward the top-right. -->
			<div
				class="absolute inset-0 bg-gradient-to-t from-background via-background/60 to-background/20"
			></div>
			<div class="absolute inset-0 bg-gradient-to-r from-background via-background/50 to-transparent"></div>
			{#if pl.thumbnail}
				<img
					src={pl.thumbnail}
					alt=""
					class="relative h-40 w-40 rounded-xl object-cover shadow-lg"
				/>
			{:else}
				<div class="relative h-40 w-40 rounded-xl bg-muted"></div>
			{/if}
			<div class="relative min-w-0 flex-1">
				<div class="text-xs font-medium uppercase text-muted-foreground">Playlist</div>
				{#if editingName}
					<div class="mt-1 flex items-center gap-2">
						<input
							use:autofocus
							bind:value={nameDraft}
							onkeydown={(e) => {
								if (e.key === 'Enter') saveRename();
								else if (e.key === 'Escape') (editingName = false);
							}}
							class="min-w-0 flex-1 rounded-md border bg-background px-2 py-1 font-heading text-3xl font-bold outline-none focus:border-accent"
							aria-label="Playlist name"
						/>
						<Button size="icon" aria-label="Save name" onclick={saveRename}>
							<HugeiconsIcon icon={Tick02Icon} class="h-5 w-5" />
						</Button>
						<Button
							variant="ghost"
							size="icon"
							aria-label="Cancel rename"
							onclick={() => (editingName = false)}
						>
							<HugeiconsIcon icon={Cancel01Icon} class="h-5 w-5 text-muted-foreground" />
						</Button>
					</div>
				{:else}
					<h1 class="mt-1 font-heading text-4xl font-bold tracking-tight drop-shadow-lg">
					{pl.title ?? 'Playlist'}
				</h1>
				{/if}
				{#if pl.subtitle}<p class="mt-2 text-sm text-muted-foreground">{pl.subtitle}</p>{/if}
				<div class="mt-4 flex items-center gap-2">
					<Button class="gap-2" onclick={() => playAll(null)} disabled={!pl.items.length}>
						<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" /> Play
					</Button>
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
							aria-label="Playlist options"
							onclick={openMenu}
						>
							<HugeiconsIcon icon={MoreVerticalIcon} class="h-5 w-5 text-muted-foreground" />
						</Button>
					{/if}
				</div>
			</div>
		</div>
		<div class="content-in min-h-0 flex-1 overflow-y-auto p-4">
			{#each pl.items as item, i (item.video_id + i)}
				<TrackRow
					song={item}
					index={i}
					active={item.video_id === nowId}
					onplay={() => playAll(i)}
					onAdd={() => openAddToPlaylist(item.video_id)}
					onRemove={isLiked || (editable && item.set_video_id) ? () => removeTrack(item) : undefined}
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

{#if menuOpen}
	<button
		class="fixed inset-0 z-40 cursor-default"
		onclick={() => (menuOpen = false)}
		aria-label="Close menu"
	></button>
	<div
		class="fixed z-50 min-w-52 origin-top-left animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style="left:{mx}px; top:{my}px;"
	>
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={() => run(shufflePlay)}
			disabled={!pl?.items.length}
		>
			<HugeiconsIcon icon={ShuffleIcon} class="h-4 w-4" /> Shuffle play
		</button>
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={() => run(() => addPick(asItem()))}
		>
			<HugeiconsIcon icon={DashboardSquare02Icon} class="h-4 w-4" /> Add to Quick Picks
		</button>
		{#if editable}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={() => run(startRename)}
			>
				<HugeiconsIcon icon={PencilEdit02Icon} class="h-4 w-4" /> Edit name
			</button>
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-destructive hover:bg-destructive/10"
				onclick={() => run(() => (confirmingDelete = true))}
			>
				<HugeiconsIcon icon={Delete02Icon} class="h-4 w-4" /> Delete playlist
			</button>
		{/if}
	</div>
{/if}
