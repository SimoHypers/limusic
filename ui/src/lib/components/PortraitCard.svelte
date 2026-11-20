<script lang="ts">
	// An artist as a poster, not a circle. A circular avatar crops the photo to a face and then sets
	// the name in 12px underneath it, which is how every music app draws an artist and why none of
	// them are memorable. A tall frame keeps the photograph, and the name goes on it, big, where the
	// eye already is.
	//
	// object-position sits above centre: press photos are shot with headroom, and a square-ish crop
	// of the middle lands on a chest.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { UserIcon } from '@hugeicons/core-free-icons';
	import type { BrowseItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { setDragItem } from '$lib/dnd';
	import { openItem } from '$lib/browse';
	import PlaylistMenu from './PlaylistMenu.svelte';

	let { item }: { item: BrowseItem } = $props();

	let attempt = $state(0);
	$effect(() => {
		item.thumbnail;
		attempt = 0;
	});
	const sized = $derived(thumb(item.thumbnail, 400));
	const src = $derived(attempt === 0 ? sized : item.thumbnail);
	const imgFailed = () => (attempt = attempt === 0 && sized !== item.thumbnail ? 1 : 2);
	const hasArt = $derived(!!item.thumbnail && attempt < 2);
</script>

<div class="group relative w-full" data-ctx>
	<div
		class="relative aspect-[3/4] w-full cursor-pointer overflow-hidden rounded-2xl bg-muted"
		role="button"
		tabindex="0"
		draggable="true"
		ondragstart={(e) => setDragItem(e, item)}
		onclick={() => openItem(item)}
		onkeydown={(e) => {
			if (e.target !== e.currentTarget) return;
			if (e.key === 'Enter' || e.key === ' ') {
				e.preventDefault();
				openItem(item);
			}
		}}
		title={item.subtitle ? `${item.title} — ${item.subtitle}` : item.title}
	>
		{#if hasArt}
			<img
				{src}
				alt=""
				class="h-full w-full object-cover object-[center_22%]"
				loading="lazy"
				draggable="false"
				onerror={imgFailed}
			/>
		{:else}
			<div class="flex h-full w-full items-center justify-center text-muted-foreground/40">
				<HugeiconsIcon icon={UserIcon} class="h-10 w-10" />
			</div>
		{/if}
		<!-- Two stops, weighted to the bottom third: enough to hold small white text without
		     graying out the face above it. -->
		<div
			class="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/85 via-black/20 to-transparent"
		></div>
		<div class="pointer-events-none absolute inset-x-0 bottom-0 p-3">
			<div class="line-clamp-2 font-heading text-sm font-semibold leading-tight text-white">
				{item.title}
			</div>
			{#if item.subtitle}
				<div class="mt-0.5 truncate text-[0.6875rem] text-white/65">{item.subtitle}</div>
			{/if}
		</div>
	</div>
	<PlaylistMenu
		{item}
		showPin={false}
		triggerClass="absolute right-2 top-2 flex h-8 w-8 items-center justify-center rounded-full bg-black/50 text-white opacity-0 transition hover:bg-black/70 focus-visible:shown group-hover:shown cursor-pointer"
	/>
</div>
