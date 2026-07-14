<script lang="ts">
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		PlayIcon,
		MoreHorizontalIcon,
		PlayListAddIcon,
		PlayListRemoveIcon,
		AddToListIcon,
		FavouriteIcon,
		UserListIcon,
		Vynil02Icon
	} from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import type { SongItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { lt } from '$lib/lt.svelte';
	import { playback, toast } from '$lib/player.svelte';

	let {
		song,
		index,
		active = false,
		hideThumb = false,
		onplay,
		onAdd,
		onRemove,
		removeLabel = 'Remove from playlist'
	}: {
		song: SongItem;
		/** Position badge when set (playlist/queue); omitted for flat search results. */
		index?: number;
		active?: boolean;
		/** Hide the leading thumbnail (album track lists show a number, not a cover). */
		hideThumb?: boolean;
		onplay: () => void;
		/** Adds an "Add to playlist" menu item. */
		onAdd?: () => void;
		/** Adds a remove menu item (label via `removeLabel`). */
		onRemove?: () => void;
		removeLabel?: string;
	} = $props();

	const hasMenu = $derived(!!onAdd || !!onRemove);
	// In a session as guest, clicking a song adds it to the shared queue instead of playing it —
	// reflect that in the hover icon + label so the row doesn't lie.
	const guestAdd = $derived(lt.role === 'guest');

	// A ⋯ menu, positioned `fixed` at the button so it isn't clipped by the scroll container.
	// Anchored by its right edge (distance from the viewport's right) so the zoom-in origin stays put.
	let menuOpen = $state(false);
	let mx = $state(0);
	let my = $state(0);

	function openMenu(e: MouseEvent) {
		e.stopPropagation();
		const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
		mx = window.innerWidth - r.right;
		my = r.bottom + 4;
		menuOpen = true;
	}
	function run(action?: () => void) {
		menuOpen = false;
		action?.();
	}

	// "Add to queue" is universal — every row with a menu gets it. Guests get their toast from the
	// session flow ("Added to the session queue."), so only toast locally for host/solo.
	function addToQueue() {
		api.addToQueue(song);
		if (lt.role !== 'guest') toast('Added to queue');
	}

	// Like state: the player bar owns it for the current track, so mirror that; otherwise track it
	// locally, seeded from the row's own likeStatus.
	let rowLiked = $state<boolean | undefined>(undefined); // set once the user toggles this row
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

	// The whole row is a play target (role="button"), so mirror native button keyboard activation.
	function onKey(e: KeyboardEvent) {
		if (e.key === 'Enter' || e.key === ' ') {
			e.preventDefault();
			onplay();
		}
	}
</script>

<div
	role="button"
	tabindex="0"
	onclick={onplay}
	onkeydown={onKey}
	aria-label={guestAdd ? `Add ${song.title} to the session queue` : `Play ${song.title}`}
	class="group flex w-full cursor-pointer items-center gap-3 rounded-lg p-2 transition-colors hover:bg-accent/10 {active
		? 'bg-accent/10'
		: ''}"
>
	<div class="flex min-w-0 flex-1 items-center gap-3">
		<div class="flex min-w-0 shrink-0 items-center gap-3">
			{#if index !== undefined}
				<span
					class="relative w-5 shrink-0 text-center text-xs {active
						? 'text-primary'
						: 'text-muted-foreground'}"
				>
					<span class="group-hover:opacity-0">{index + 1}</span>
					<HugeiconsIcon
						icon={guestAdd ? PlayListAddIcon : PlayIcon}
						class="absolute inset-0 m-auto h-3.5 w-3.5 opacity-0 group-hover:opacity-100"
					/>
				</span>
			{/if}
			{#if song.thumbnail && !hideThumb}
				<img src={thumb(song.thumbnail, 96)} alt="" class="h-10 w-10 shrink-0 rounded-md object-cover" loading="lazy" />
			{/if}
		</div>
		<div class="min-w-0 flex-1">
			<div class="flex min-w-0 items-center gap-2">
				<span class="min-w-0 truncate text-sm font-medium {active ? 'text-primary' : ''}">
					{song.title}
				</span>
				{#if song.queued_by}
					<span
						class="shrink-0 rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium text-primary"
					>
						{song.queued_by}
					</span>
				{/if}
			</div>
			{#if song.artist_id}
				<button
					class="block max-w-full cursor-pointer truncate text-left text-xs text-muted-foreground hover:text-foreground hover:underline"
					onclick={(e) => {
						e.stopPropagation();
						goto(`/artist/${encodeURIComponent(song.artist_id!)}`);
					}}
				>
					{song.artists}
				</button>
			{:else}
				<div class="truncate text-xs text-muted-foreground">{song.artists}</div>
			{/if}
		</div>
	</div>

	<div class="flex shrink-0 items-center gap-2">
		{#if song.duration}
			<span class="text-xs text-muted-foreground">{song.duration}</span>
		{/if}
		{#if hasMenu}
			<button
				class="rounded-md p-1.5 text-muted-foreground opacity-0 transition hover:bg-accent/20 hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100 {menuOpen
					? 'opacity-100'
					: ''}"
				onclick={openMenu}
				aria-label="Track options"
			>
				<HugeiconsIcon icon={MoreHorizontalIcon} class="h-4 w-4" />
			</button>
		{/if}
	</div>
</div>

{#if menuOpen}
	<button class="fixed inset-0 z-40 cursor-default" onclick={() => (menuOpen = false)} aria-label="Close menu"
	></button>
	<div
		class="fixed z-50 min-w-44 origin-top-right animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style="right:{mx}px; top:{my}px;"
	>
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={() => run(addToQueue)}
		>
			<HugeiconsIcon icon={AddToListIcon} class="h-4 w-4" /> Add to queue
		</button>
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={() => run(toggleLike)}
		>
			<HugeiconsIcon icon={FavouriteIcon} class="h-4 w-4 {liked ? 'fill-current text-primary' : ''}" />
			{liked ? 'Remove from Liked Songs' : 'Save to Liked Songs'}
		</button>
		{#if song.artist_id}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={() => run(() => goto(`/artist/${encodeURIComponent(song.artist_id!)}`))}
			>
				<HugeiconsIcon icon={UserListIcon} class="h-4 w-4" /> Go to artist
			</button>
		{/if}
		{#if song.album_id}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={() => run(() => goto(`/album/${encodeURIComponent(song.album_id!)}`))}
			>
				<HugeiconsIcon icon={Vynil02Icon} class="h-4 w-4" /> Go to album
			</button>
		{/if}
		{#if onAdd}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={() => run(onAdd)}
			>
				<HugeiconsIcon icon={PlayListAddIcon} class="h-4 w-4" /> Add to playlist
			</button>
		{/if}
		{#if onRemove}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-destructive hover:bg-destructive/10"
				onclick={() => run(onRemove)}
			>
				<HugeiconsIcon icon={PlayListRemoveIcon} class="h-4 w-4" /> {removeLabel}
			</button>
		{/if}
	</div>
{/if}
