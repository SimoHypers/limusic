<script lang="ts">
	// The ⋯ options menu shared by TrackRow (inline trigger) and MediaCard (overlay trigger).
	// "Add to queue" + like are universal; go-to-artist/album/playlist show when the song carries
	// them. The popup is `fixed`, anchored at the trigger, so it isn't clipped by a scroll container.
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		MoreHorizontalIcon,
		PlayListAddIcon,
		PlayListRemoveIcon,
		AddToListIcon,
		FavouriteIcon,
		UserListIcon,
		Vynil02Icon,
		DashboardSquare02Icon
	} from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import type { SongItem } from '$lib/api';
	import { lt } from '$lib/lt.svelte';
	import { anchorMenu } from '$lib/menu';
	import { addPick, playback, toast } from '$lib/player.svelte';

	let {
		song,
		triggerClass = '',
		onAdd,
		onRemove,
		removeLabel = 'Remove from playlist'
	}: {
		song: SongItem;
		/** Classes for the ⋯ trigger button (positioning differs per host: inline vs overlay). */
		triggerClass?: string;
		/** Adds an "Add to playlist" menu item. */
		onAdd?: () => void;
		/** Adds a remove menu item (label via `removeLabel`). */
		onRemove?: () => void;
		removeLabel?: string;
	} = $props();

	let menuOpen = $state(false);
	let mx = $state(0);
	let my = $state(0);
	let openUp = $state(false);

	function openMenu(e: MouseEvent) {
		e.stopPropagation();
		({ right: mx, y: my, openUp } = anchorMenu(e.currentTarget as HTMLElement));
		menuOpen = true;
	}
	// stopPropagation everywhere: the menu can live inside a clickable row (TrackRow's whole row is
	// a play target) — the popup is `fixed` visually but still a DOM child, so any click that
	// bubbles out would ALSO trigger the row's onplay (e.g. replacing the queue with the playlist).
	function run(e: MouseEvent, action?: () => void) {
		e.stopPropagation();
		menuOpen = false;
		action?.();
	}
	function close(e: MouseEvent) {
		e.stopPropagation();
		menuOpen = false;
	}

	// "Add to queue" is universal. Guests get their toast from the session flow ("Added to the
	// session queue."), so only toast locally for host/solo.
	function addToQueue() {
		api.addToQueue(song);
		if (lt.role !== 'guest') toast('Added to queue');
	}

	// Like state: the player bar owns it for the current track, so mirror that; otherwise track it
	// locally, seeded from the row's own likeStatus.
	let rowLiked = $state<boolean | undefined>(undefined); // set once the user toggles this song
	const isNow = $derived(playback.now?.videoId === song.video_id);
	const liked = $derived(isNow ? playback.liked : (rowLiked ?? song.liked ?? false));

	async function toggleLike() {
		const next = !liked;
		rowLiked = next; // optimistic
		if (isNow) playback.liked = next;
		try {
			await api.like(song.video_id, next);
			toast(next ? 'Added to liked songs' : 'Removed from liked songs');
		} catch (e) {
			rowLiked = !next; // revert on failure
			if (isNow) playback.liked = !next;
			toast(String(e));
		}
	}
</script>

<button class="{triggerClass} {menuOpen ? 'opacity-100' : ''}" onclick={openMenu} aria-label="Track options">
	<HugeiconsIcon icon={MoreHorizontalIcon} class="h-4 w-4" />
</button>

{#if menuOpen}
	<button class="fixed inset-0 z-40 cursor-default" onclick={close} aria-label="Close menu"
	></button>
	<div
		class="fixed z-50 min-w-44 animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95 {openUp
			? 'origin-bottom-right'
			: 'origin-top-right'}"
		style="right:{mx}px; {openUp ? 'bottom' : 'top'}:{my}px;"
	>
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={(e) => run(e, addToQueue)}
		>
			<HugeiconsIcon icon={AddToListIcon} class="h-4 w-4" /> Add to queue
		</button>
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={(e) => run(e, toggleLike)}
		>
			<HugeiconsIcon icon={FavouriteIcon} class="h-4 w-4 {liked ? 'fill-current text-primary' : ''}" />
			{liked ? 'Remove from Liked Songs' : 'Save to Liked Songs'}
		</button>
		{#if song.artist_id}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => goto(`/artist/${encodeURIComponent(song.artist_id!)}`))}
			>
				<HugeiconsIcon icon={UserListIcon} class="h-4 w-4" /> Go to artist
			</button>
		{/if}
		{#if song.album_id}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => goto(`/album/${encodeURIComponent(song.album_id!)}`))}
			>
				<HugeiconsIcon icon={Vynil02Icon} class="h-4 w-4" /> Go to album
			</button>
		{/if}
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={(e) =>
				run(e, () =>
					addPick({
						kind: 'song',
						id: song.video_id,
						title: song.title,
						subtitle: song.artists,
						thumbnail: song.thumbnail
					})
				)}
		>
			<HugeiconsIcon icon={DashboardSquare02Icon} class="h-4 w-4" /> Add to Quick Picks
		</button>
		{#if onAdd}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, onAdd)}
			>
				<HugeiconsIcon icon={PlayListAddIcon} class="h-4 w-4" /> Add to playlist
			</button>
		{/if}
		{#if onRemove}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-destructive hover:bg-destructive/10"
				onclick={(e) => run(e, onRemove)}
			>
				<HugeiconsIcon icon={PlayListRemoveIcon} class="h-4 w-4" /> {removeLabel}
			</button>
		{/if}
	</div>
{/if}
