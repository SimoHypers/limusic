<script lang="ts">
	// The "saved" mark on a track row: this song is already in one or more of your own playlists.
	// Pointing at it opens a card with their covers fanned out like a hand of sleeves (three at
	// most, "and N more" for the rest) and each one's name, linking through to that playlist;
	// pointing at a name lifts its sleeve out of the hand. The popup is `fixed`, anchored at the
	// trigger and moved to <body> (`toBody`), same as TrackMenu: a track list scrolls, and half of
	// them are paint-contained. Living at <body> is also why the links need no stopPropagation,
	// despite the whole row being a play target: nothing in there bubbles through the row any more.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowRight01Icon, CheckmarkCircle03Icon } from '@hugeicons/core-free-icons';
	import type { BrowseItem } from '$lib/api';
	import { hrefFor } from '$lib/browse';
	import { anchorMenu, fitMenu, NO_ANCHOR, toBody } from '$lib/menu';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';

	let { playlists }: { playlists: BrowseItem[] } = $props();

	const SHOWN = 3;
	const shown = $derived(playlists.slice(0, SHOWN));
	const extra = $derived(playlists.length - shown.length);
	const single = $derived(playlists.length === 1);

	// Where each sleeve sits in the hand, front first: x offset (px) and tilt (deg).
	const FANS = [
		[[0, 0]],
		[[6, 5], [-8, -7]],
		[[0, 0], [-14, -10], [14, 10]]
	];
	const fan = $derived(FANS[shown.length - 1] ?? []);

	let open = $state(false);
	let anchor = $state(NO_ANCHOR);
	// The sleeve pulled out of the hand: whichever playlist the pointer (or focus) is on.
	let lifted = $state(-1);
	// Crossing the gap between the mark and the popup is a mouseleave with nothing under the
	// pointer, so closing waits long enough for the next mouseenter to cancel it.
	let closing: ReturnType<typeof setTimeout> | undefined;

	function show(e: Event) {
		clearTimeout(closing);
		anchor = anchorMenu(e, { align: 'right' });
		lifted = -1;
		open = true;
	}
	function hide() {
		clearTimeout(closing);
		closing = setTimeout(() => (open = false), 140);
	}
	function keep() {
		clearTimeout(closing);
	}

	const label = $derived(
		single
			? t('library.saved_in_one', { playlist: playlists[0].title })
			: t('library.saved_in_many', { count: playlists.length })
	);
</script>

<!-- Opens on hover and on focus, so it is reachable by tab as well as by pointer. The click does
     nothing but stay out of the row's way: toggling here would close the popup under a pointer
     that never left the mark. No `title`: the card is the tooltip, and a native one on top of it
     covered half of it. -->
<button
	class="cursor-pointer rounded-md p-1.5 text-primary transition hover:bg-primary/15"
	aria-label={label}
	onmouseenter={show}
	onmouseleave={hide}
	onfocus={show}
	onblur={hide}
	onclick={(e) => e.stopPropagation()}
>
	<HugeiconsIcon icon={CheckmarkCircle03Icon} class="h-4 w-4" />
</button>

{#snippet head()}
	<!-- The front cover, blurred into a wash behind the header. A 96px source, blurred once when the
	     card opens; image and fade share one clipped wrapper (docs/UI-PERFORMANCE.md). -->
	{#if playlists[0].thumbnail}
		<div class="pointer-events-none absolute inset-0 overflow-hidden">
			<img
				src={thumb(playlists[0].thumbnail, 96)}
				alt=""
				class="h-full w-full scale-150 object-cover opacity-45 blur-2xl"
			/>
			<div class="absolute inset-0 bg-gradient-to-b from-popover/10 to-popover"></div>
		</div>
	{/if}
	<!-- A lone cover has no hand to fan into, so it takes the whole slot. -->
	<div class="relative shrink-0 {single ? 'h-14 w-14' : 'h-12 w-[4.5rem]'}">
		{#each shown as pl, i (pl.id)}
			<div
				class="sleeve absolute overflow-hidden {single ? 'inset-0' : 'left-4 top-1 h-10 w-10'} rounded-md bg-muted shadow-md ring-2 ring-popover"
				class:lifted={lifted === i}
				style="--x:{fan[i][0]}px;--r:{fan[i][1]}deg;--i:{i};z-index:{lifted === i ? 9 : 3 - i}"
			>
				{#if pl.thumbnail}
					<!-- thumb(): a playlist on this machine can wear a local file's art, a path. -->
					<img src={thumb(pl.thumbnail, 96)} alt="" class="h-full w-full object-cover" />
				{/if}
			</div>
		{/each}
	</div>
	<div class="relative min-w-0 flex-1">
		<p class="text-[10px] font-semibold uppercase tracking-[0.14em] text-primary">
			{t('library.saved_in')}
		</p>
		<p class="truncate text-sm font-semibold">
			{single ? playlists[0].title : t('library.saved_in_count', { count: playlists.length })}
		</p>
		{#if single && playlists[0].subtitle}
			<p class="truncate text-xs text-muted-foreground">{playlists[0].subtitle}</p>
		{/if}
	</div>
{/snippet}

{#if open}
	<div
		class="fixed z-50 w-64 animate-in overflow-hidden rounded-xl border bg-popover text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style={anchor.style}
		{@attach fitMenu(anchor)}
		onmouseenter={keep}
		onmouseleave={hide}
		role="tooltip"
		{@attach toBody}
	>
		{#if single}
			<!-- One playlist: the header is the whole card, and the link. -->
			<a
				href={hrefFor(playlists[0])}
				class="relative flex items-center gap-3 p-3 outline-none"
				onclick={() => (open = false)}
				onmouseenter={() => (lifted = 0)}
				onmouseleave={() => (lifted = -1)}
				onfocus={() => (lifted = 0)}
				onblur={() => (lifted = -1)}
			>
				{@render head()}
			</a>
		{:else}
			<div class="relative flex items-center gap-3 p-3">
				{@render head()}
			</div>
			<div class="px-1.5 pb-1.5">
				{#each shown as pl, i (pl.id)}
					<a
						href={hrefFor(pl)}
						class="group/pl flex items-center gap-2 rounded-lg px-2 py-1.5 outline-none hover:bg-accent/10 focus-visible:bg-accent/10"
						onclick={() => (open = false)}
						onmouseenter={() => (lifted = i)}
						onmouseleave={() => (lifted = -1)}
						onfocus={() => (lifted = i)}
						onblur={() => (lifted = -1)}
					>
						<span class="min-w-0 flex-1">
							<span class="block truncate text-sm">{pl.title}</span>
							{#if pl.subtitle}
								<span class="block truncate text-xs text-muted-foreground">{pl.subtitle}</span>
							{/if}
						</span>
						<HugeiconsIcon
							icon={ArrowRight01Icon}
							class="h-3.5 w-3.5 shrink-0 text-muted-foreground opacity-0 group-hover/pl:opacity-100 group-focus-visible/pl:opacity-100"
						/>
					</a>
				{/each}
				{#if extra > 0}
					<p class="px-2 pb-0.5 pt-1 text-xs text-muted-foreground">
						{t('library.saved_in_more', { count: extra })}
					</p>
				{/if}
			</div>
		{/if}
	</div>
{/if}

<style>
	/* The hand deals itself out as the card opens, front sleeve first. Three small layers inside one
	   popup, once per open, so the transforms are cheap; the lift is the same three. */
	.sleeve {
		transform: translateX(var(--x)) rotate(var(--r));
		transition: transform 160ms ease-out;
		animation: deal 240ms cubic-bezier(0.2, 0.8, 0.2, 1) calc(var(--i) * 40ms) backwards;
	}
	.sleeve.lifted {
		transform: translateX(var(--x)) translateY(-5px) rotate(0deg) scale(1.08);
	}
	@keyframes deal {
		from {
			transform: translateY(6px) scale(0.9);
			opacity: 0;
		}
	}
</style>
