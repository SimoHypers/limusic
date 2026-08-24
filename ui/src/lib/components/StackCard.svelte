<script lang="ts">
	// A playlist is a pile of things, so it's drawn as one: the cover with two sheet edges showing
	// above it, fanning further out under the pointer. It costs two divs and no extra requests, and
	// it's the one glance that separates "a playlist" from "an album" in a mixed shelf, which square
	// artwork alone never does.
	//
	// The sheets are full-size siblings behind an opaque cover, scaled narrower and lifted, so only
	// their top strips are ever visible. Transform-only, so the fan composites.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { PlayIcon, MusicNote01Icon, ListRestartIcon } from '@hugeicons/core-free-icons';
	import { ON_REPEAT_ID } from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { setDragItem } from '$lib/dnd';
	import { openItem, playItem } from '$lib/browse';
	import PlaylistMenu from './PlaylistMenu.svelte';
	import { t } from '$lib/i18n.svelte';

	let { item }: { item: BrowseItem } = $props();

	const onRepeat = $derived(item.id === ON_REPEAT_ID);

	let attempt = $state(0);
	$effect(() => {
		item.thumbnail;
		attempt = 0;
	});
	const sized = $derived(thumb(item.thumbnail, 400));
	const src = $derived(attempt === 0 ? sized : item.thumbnail);
	const imgFailed = () => (attempt = attempt === 0 && sized !== item.thumbnail ? 1 : 2);
	const hasArt = $derived(!!item.thumbnail && attempt < 2 && !onRepeat);

	let busy = $state(false);
	async function play() {
		if (busy) return;
		busy = true;
		try {
			await playItem(item);
		} finally {
			busy = false;
		}
	}
</script>

<!-- pt-3 is headroom for the lifted sheets: the shelf scrolls horizontally, which makes it clip
     vertically too, so anything reaching above the cover has to be inside the card's own box. -->
<div class="group relative w-full pt-4" data-ctx>
	<div
		class="cursor-pointer"
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
		<div class="relative aspect-square w-full">
			<div
				class="absolute inset-0 origin-bottom -translate-y-[7px] scale-x-[0.84] rounded-xl bg-muted-foreground/15 transition-transform duration-300 ease-out group-hover:-translate-y-[13px]"
			></div>
			<div
				class="absolute inset-0 origin-bottom -translate-y-[3px] scale-x-[0.92] rounded-xl bg-muted-foreground/25 transition-transform duration-300 ease-out group-hover:-translate-y-[7px]"
			></div>
			<!-- No resting shadow: see MediaCard. The sheet edges above are what gives the card
			     its depth, and they cost a transform instead of a gaussian blur per card. -->
			<div class="relative h-full w-full overflow-hidden rounded-xl bg-muted">
				{#if hasArt}
					<img
						{src}
						alt=""
						class="h-full w-full object-cover transition-transform duration-500 ease-out group-hover:scale-105"
						loading="lazy"
						draggable="false"
						onerror={imgFailed}
					/>
				{:else}
					<div
						class="flex h-full w-full items-center justify-center {onRepeat
							? 'bg-primary/10 text-primary'
							: 'text-muted-foreground/50'}"
					>
						<!-- altIcon/showAlt, not a ternary: `icon` is read once at mount. -->
						<HugeiconsIcon
							icon={MusicNote01Icon}
							altIcon={ListRestartIcon}
							showAlt={onRepeat}
							class={onRepeat ? 'h-10 w-10' : 'h-7 w-7'}
						/>
					</div>
				{/if}
				<button
					class="absolute bottom-2 right-2 flex h-9 w-9 translate-y-1 cursor-pointer items-center justify-center rounded-full bg-primary text-primary-foreground opacity-0 shadow-lg transition-[opacity,transform] duration-200 ease-out focus-visible:opacity-100 group-hover:translate-y-0 group-hover:opacity-100"
					class:animate-pulse={busy}
					disabled={busy}
					aria-label={t('a11y.play_item', { title: item.title })}
					onclick={(e) => {
						e.stopPropagation();
						play();
					}}
				>
					<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" />
				</button>
			</div>
		</div>
		<div class="mt-2.5 min-w-0">
			<div class="truncate text-sm font-medium">{item.title}</div>
			{#if item.subtitle}
				<div class="truncate text-xs text-muted-foreground">{item.subtitle}</div>
			{/if}
		</div>
	</div>
	<PlaylistMenu
		{item}
		triggerClass="absolute right-2 top-6 flex h-8 w-8 items-center justify-center rounded-full bg-background/90 text-foreground opacity-0 shadow-md transition hover:bg-background focus-visible:opacity-100 group-hover:opacity-100 cursor-pointer z-10"
	/>
</div>
