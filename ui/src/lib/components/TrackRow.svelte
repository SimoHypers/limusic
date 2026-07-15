<script lang="ts">
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { PlayIcon, PlayListAddIcon } from '@hugeicons/core-free-icons';
	import type { SongItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { lt } from '$lib/lt.svelte';
	import TrackMenu from './TrackMenu.svelte';

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

	// In a session as guest, clicking a song adds it to the shared queue instead of playing it —
	// reflect that in the hover icon + label so the row doesn't lie.
	const guestAdd = $derived(lt.role === 'guest');

	// The whole row is a play target (role="button"), so mirror native button keyboard activation.
	// Only when the key lands on the row itself — keydowns bubble up from nested interactive
	// elements (⋯ menu, artist link), and hijacking those would play the row instead.
	function onKey(e: KeyboardEvent) {
		if (e.target !== e.currentTarget) return;
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
		<TrackMenu
			{song}
			{onAdd}
			{onRemove}
			{removeLabel}
			triggerClass="rounded-md p-1.5 text-muted-foreground opacity-0 transition hover:bg-accent/20 hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100"
		/>
	</div>
</div>
