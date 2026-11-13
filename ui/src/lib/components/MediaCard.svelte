<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { PlayIcon, ListRestartIcon } from '@hugeicons/core-free-icons';
	import LogoMark from './LogoMark.svelte';
	import LimusicLoader from './LimusicLoader.svelte';
	import { ON_REPEAT_ID } from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { setDragItem } from '$lib/dnd';
	import { openItem, playItem } from '$lib/browse';
	import ItemMenu from './ItemMenu.svelte';
	import ExplicitIcon from './ExplicitIcon.svelte';
	import LiveBadge from './LiveBadge.svelte';
	import { t } from '$lib/i18n.svelte';

	let { item, compact = false }: { item: BrowseItem; compact?: boolean } = $props();

	const round = $derived(item.kind === 'artist');
	// On Repeat has no artwork by nature, so its cover is the icon rather than the neutral
	// placeholder every failed thumbnail lands on.
	const onRepeat = $derived(item.id === ON_REPEAT_ID);

	// Google's CDN doesn't serve every rewritten size — asking for one it doesn't have 404s, and the
	// browser then paints its broken-image glyph. So: try the sized URL, retry the original once, and
	// only then fall back to a neutral icon tile.
	let attempt = $state(0);
	$effect(() => {
		item.thumbnail; // re-arm when the card is reused for a different item
		attempt = 0;
	});
	const sized = $derived(thumb(item.thumbnail, 400));
	// The cover slot is 144px (`.card-grid` is 10rem columns), so 400px is roughly seven times the
	// pixels a 1x display can show, and WebKit holds the decoded bitmap at the size it was given.
	// srcset hands the choice to the engine instead of guessing: 200 where that is all the screen
	// has, 400 where the pixels are real. Both sizes verified live against yt3 covers, and the
	// retry chain below still catches a size the CDN turns out not to serve.
	const small = $derived(thumb(item.thumbnail, 200));
	// Undefined when `thumb` left the URL alone (not a Google CDN URL, or a local file), where two
	// candidates would be the same image twice, and on the retry, which is deliberately unsized.
	const srcset = $derived(
		attempt === 0 && small && sized && small !== sized ? `${small} 1x, ${sized} 2x` : undefined
	);
	const src = $derived(attempt === 0 ? sized : item.thumbnail);
	// Skip the retry when `thumb` left the URL untouched — it would refetch the same dead URL.
	const imgFailed = () => (attempt = attempt === 0 && sized !== item.thumbnail ? 1 : 2);

	let playing = $state(false); // in-flight guard for the fetch-then-play path

	async function playNow() {
		if (playing) return;
		playing = true;
		try {
			await playItem(item);
		} finally {
			playing = false;
		}
	}
</script>

<!-- data-ctx: right-clicking anywhere on the card opens the ⋯ menu below at the pointer. -->
<div class="group relative flex w-full flex-col gap-2" data-ctx>
	<!-- draggable: every card is a drag source for home's Shortcuts grid (the only drop target). -->
	<div
		class="flex flex-col text-left transition-colors hover:bg-accent/10 {compact
			? 'gap-1.5 rounded-lg p-1.5'
			: 'gap-2 rounded-xl p-2'}"
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
		<!-- The hover lift fades in a shadow that is already rasterized instead of transitioning
		     box-shadow. Interpolating shadow-sm to shadow-xl makes WebKit compute a *different*
		     gaussian blur on every frame for 300ms, over a region half again wider than the cover, and
		     dragging the pointer across a grid has several cards doing it at once. As an opacity fade
		     the blur is rasterized once and reused at every step, and opacity is composited.
		     A wrapper, because the shadow has to paint outside a box that the cover below clips. -->
		<div class="relative">
			<div
				class="pointer-events-none absolute inset-0 opacity-0 shadow-xl transition-opacity duration-300 group-hover:shown {round
					? 'rounded-full'
					: 'rounded-lg'}"
			></div>
			<!-- No resting shadow. Measured (perf/hover.mjs, 300 cards, scrolled): it was three quarters
			     of the hover cost, and not because of the hovered card. WebKit rasterizes in tiles, so
			     repainting one card re-rasterizes its whole tile, and that means re-blurring the shadow
			     of every card in the tile. One blurred shadow per cover, nine covers a tile, on every
			     card the pointer crosses. Dropping it took p90 frame time from ~105ms to ~33ms; the
			     hover lift below is nearly free by comparison because only one card ever has it. -->
			<div
				class="relative aspect-square w-full overflow-hidden bg-muted {round
					? 'rounded-full'
					: 'rounded-lg'}"
			>
				{#if onRepeat}
					<div class="flex h-full w-full items-center justify-center bg-primary/10 text-primary">
						<HugeiconsIcon icon={ListRestartIcon} class={compact ? 'h-7 w-7' : 'h-10 w-10'} />
					</div>
				{:else}
					<!-- The logo sits under the cover: it shows while the cover loads, and stays when
					     there is none. ponytail: a cover with transparency shows it through. -->
					<LogoMark class="absolute inset-0 m-auto size-3/4 text-muted-foreground/20" />
					{#if item.thumbnail && attempt < 2}
						<!-- No hover zoom (#341). On Chromium (WebView2) a transform transition is a
						     composited animation, and each one starting or ending re-layerizes the whole
						     page, so sweeping the pointer across a shelf did it on every card. Home top,
						     perf/scroll.mjs at 4x CPU, hovering: 46-52% of frames over 20 ms in 1.0.0, 7-9%
						     without this, StackCard's lift and the art-wash promotion. Fades are cheap
						     enough to keep. -->
						<img
							{src}
							{srcset}
							alt=""
							class="relative h-full w-full object-cover"
							loading="lazy"
							draggable="false"
							onerror={imgFailed}
						/>
					{/if}
				{/if}
				{#if item.live}
					<div class="absolute left-2 top-2"><LiveBadge onArt /></div>
				{/if}
				{#if item.kind !== 'artist'}
					<!-- A fade only: the slide it used to have is a transform transition (see above). -->
					<button
						data-card-play
						class="absolute flex cursor-pointer items-center justify-center rounded-full bg-primary text-primary-foreground opacity-0 shadow-lg transition-opacity duration-200 ease-out group-hover:shown focus-visible:shown {compact
							? 'bottom-1.5 right-1.5 h-7 w-7'
							: 'bottom-2 right-2 h-9 w-9'} {playing ? 'shown!' : ''}"
						disabled={playing}
						aria-label={t('player.play')}
						onclick={(e) => {
							e.stopPropagation();
							playNow();
						}}
					>
						{#if playing}
							<LimusicLoader class="{compact ? 'size-5' : 'size-6'} text-primary-foreground" />
						{:else}
							<HugeiconsIcon icon={PlayIcon} class={compact ? 'h-3 w-3' : 'h-4 w-4'} />
						{/if}
					</button>
				{/if}
			</div>
		</div>
		<div class="min-w-0 {round ? 'text-center' : ''}">
			<div class="truncate font-medium {compact ? 'text-xs' : 'text-sm'}">{item.title}</div>
			{#if item.subtitle || item.explicit}
				<div
					class="flex items-center gap-1 text-muted-foreground {round
						? 'justify-center'
						: ''} {compact ? 'text-[0.6875rem]' : 'text-xs'}"
				>
					{#if item.explicit}
						<ExplicitIcon class="h-3 w-3 shrink-0" />
					{/if}
					<span class="truncate">{item.subtitle}</span>
				</div>
			{/if}
		</div>
	</div>
	<!-- bg-background/90, not /80 with a backdrop-blur: a backdrop-filter makes WebKit snapshot and
	     blur everything under the button, on every card in the grid, whether or not the card is
	     hovered. At 90% over artwork there is nothing left to blur that you can see. -->
	<ItemMenu
		{item}
		triggerClass="absolute right-3 top-3 flex h-8 w-8 items-center justify-center rounded-full bg-background/90 text-foreground opacity-0 shadow-md transition hover:bg-background focus-visible:shown group-hover:shown cursor-pointer"
	/>
</div>
