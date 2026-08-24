<script lang="ts">
	// A horizontal shelf, drawn according to what it actually holds.
	//
	// Every shelf used to be the same row of square cards, which is exactly what YouTube Music does
	// and the reason a home feed reads as one undifferentiated wall. A square of artwork is the right
	// shape for an album and the wrong shape for everything else: a song needs its title and its
	// artist, an artist needs a face at a size you can see, a playlist needs to look like more than
	// one thing. So the shelf picks a form from the items:
	//
	//   songs     -> columns of readable, numbered rows you page through: no artwork worth showing,
	//                all information
	//   artists   -> tall poster frames with the name set on the photograph
	//   playlists -> a cover with the stack behind it showing
	//   anything else, or a mixed shelf -> the plain card, unchanged
	//
	// The rail, its arrows, the edge fades and the content-visibility budget are shared by all of
	// them; only the slot changes.
	import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/svelte';
	import {
		ArrowLeft01Icon,
		ArrowRight01Icon,
		CdIcon,
		MusicNote01Icon,
		PlayListIcon,
		UserMultiple02Icon
	} from '@hugeicons/core-free-icons';
	import MediaCard from './MediaCard.svelte';
	import CommunityCard from './CommunityCard.svelte';
	import PortraitCard from './PortraitCard.svelte';
	import StackCard from './StackCard.svelte';
	import SectionHeading from './SectionHeading.svelte';
	import TrackRow from './TrackRow.svelte';
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { asSong } from '$lib/browse';
	import { openAddToPlaylist, openPlayer, playback } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	let {
		title,
		items,
		onMore,
		community = false,
		rich = true,
		headingClass = 'font-heading text-lg font-semibold'
	}: {
		title?: string;
		items: BrowseItem[];
		/** Renders a "See all" button in the header when provided. */
		onMore?: () => void;
		/**
		 * Community playlist cards: four per row at most, stretching to fill the width instead of
		 * fitting more cards as the window grows. The arrows page through the rest.
		 */
		community?: boolean;
		/** Opt out of the per-kind forms and render plain cards. */
		rich?: boolean;
		/** Artist and album pages use text-xl font-bold; home uses the default. */
		headingClass?: string;
	} = $props();

	// A shelf is only worth a form of its own when it's overwhelmingly one kind of thing. Below the
	// threshold it's a mixed bag ("Listen again"), and the plain card is the honest way to draw it.
	const MOSTLY = 0.75;
	type Mode = 'song' | 'album' | 'artist' | 'playlist' | 'card';
	const mode = $derived.by<Mode>(() => {
		if (community || !rich || !items.length) return 'card';
		const counts = new Map<string, number>();
		for (const i of items) counts.set(i.kind, (counts.get(i.kind) ?? 0) + 1);
		const [kind, n] = [...counts].sort((a, b) => b[1] - a[1])[0];
		return n / items.length >= MOSTLY ? (kind as Mode) : 'card';
	});

	const ICONS: Record<Mode, IconSvgElement | undefined> = {
		song: MusicNote01Icon,
		album: CdIcon,
		artist: UserMultiple02Icon,
		playlist: PlayListIcon,
		card: undefined
	};

	// Song mode: four rows to a column, paged sideways. Twelve legible tracks per screenful against
	// the six anonymous squares that fitted before. A non-song can't be a row that queues with the
	// rest, so it leads the shelf as a plain card instead, the same thing the other modes do with an
	// item that doesn't fit their form. Search's "Top results" is exactly this shape (the artist you
	// searched for plus three of their songs), and dropping it hid the match entirely.
	const ROWS = 4;
	const songs = $derived(mode === 'song' ? items.filter((i) => i.kind === 'song').map(asSong) : []);
	const others = $derived(mode === 'song' ? items.filter((i) => i.kind !== 'song') : []);
	const columns = $derived(
		Array.from({ length: Math.ceil(songs.length / ROWS) }, (_, c) =>
			songs.slice(c * ROWS, c * ROWS + ROWS)
		)
	);
	// Clicking any row starts there and queues the whole shelf, so a shelf plays as the set it is.
	const play = (start: number) => {
		openPlayer();
		return api.playPlaylist(songs, start, undefined, title);
	};

	// Slot width per form, and the height the rail reserves before it has been laid out.
	const SLOT: Record<Mode, string> = {
		song: 'basis-full sm:basis-1/2 xl:basis-1/3',
		album: 'w-40',
		artist: 'w-40',
		playlist: 'w-44',
		card: 'w-40'
	};
	const HEIGHT: Record<Mode, string> = {
		song: '17rem',
		album: '17.5rem',
		artist: '17.5rem',
		playlist: '17.5rem',
		card: '17.5rem'
	};

	let row = $state<HTMLDivElement | null>(null);
	let canLeft = $state(false);
	let canRight = $state(false);

	function update() {
		if (!row) return;
		canLeft = row.scrollLeft > 4;
		canRight = row.scrollLeft + row.clientWidth < row.scrollWidth - 4;
	}

	const measureOnEnter = (el: HTMLElement) => {
		el.addEventListener('pointerenter', update);
		return () => el.removeEventListener('pointerenter', update);
	};

	function page(dir: 1 | -1) {
		row?.scrollBy({ left: dir * Math.round(row.clientWidth * 0.9), behavior: 'smooth' });
	}

	$effect(() => {
		items; // re-measure when content changes
		update();
	});
</script>

<svelte:window onresize={update} />

<!-- content-visibility: a home feed grows to hundreds of cards and WebKit keeps every one of them in
     style, layout and paint. Because it rasterizes in tiles, one card's hover repaint re-rasterizes
     the images around it, so hovering gets slower the further you scroll. Skipping off-screen
     shelves caps that at a screenful. The intrinsic size is a shelf of this form (heading + row);
     the `auto` keyword swaps in the real size once measured, so the scrollbar stays put. -->
<section
	class="[content-visibility:auto]"
	style="contain-intrinsic-size: auto {HEIGHT[mode]};"
>
	{#if title || onMore}
		<SectionHeading title={title ?? ''} icon={ICONS[mode]} {onMore} {headingClass} />
	{/if}
	<!-- Measure on pointer enter, because a shelf skipped by content-visibility has no layout at
	     mount: scrollWidth reads 0 and the arrows never appear. They only show on hover, so measuring
	     as the pointer arrives is both correct and later than the mount-time forced layout.
	     An attachment rather than onpointerenter: the handler doesn't make this div interactive. -->
	<div class="group/shelf relative" {@attach measureOnEnter}>
		<div
			class="flex snap-x overflow-x-auto pb-2 {mode === 'song'
				? 'gap-0'
				: community
					? 'gap-3'
					: 'gap-2'}"
			bind:this={row}
			onscroll={update}
		>
			{#if mode === 'song'}
				{#each others as item (item.id)}
					<div class="min-w-0 w-40 shrink-0 snap-start pr-4"><MediaCard {item} /></div>
				{/each}
				<!-- A rule down each column but the first: the same editorial device as the heading, and
				     what makes a paged block of rows read as columns rather than one long list. -->
				{#each columns as col, c (c)}
					<div
						class="min-w-0 shrink-0 snap-start {SLOT.song} {c || others.length
							? 'border-l pl-4'
							: ''} pr-4"
					>
						{#each col as song, r (song.video_id + ':' + r)}
							<TrackRow
								{song}
								compact
								index={c * ROWS + r}
								active={playback.now?.videoId === song.video_id}
								onplay={() => play(c * ROWS + r)}
								onAdd={() => openAddToPlaylist(song)}
							/>
						{/each}
					</div>
				{/each}
			{:else}
				{#each items as item, i (item.id + ':' + i)}
					<!-- A shelf keeps its form even where one item doesn't fit it: a stray song in an
					     artist shelf gets the plain card and its width, not a poster it isn't. -->
					{@const own = community ? item.kind === 'playlist' : item.kind === mode}
					<!-- min-w-0: a flex item's automatic minimum size is its min-content, which overrides
					     the basis, so without this a card with a long title grows past its slot. -->
					<div
						class="min-w-0 shrink-0 snap-start {own
							? community
								? 'basis-full sm:basis-[calc((100%-0.75rem)/2)] lg:basis-[calc((100%-2.25rem)/4)]'
								: SLOT[mode]
							: 'w-40'}"
					>
						{#if !own}
							<MediaCard {item} />
						{:else if community}
							<CommunityCard {item} />
						{:else if mode === 'artist'}
							<PortraitCard {item} />
						{:else if mode === 'playlist'}
							<StackCard {item} />
						{:else}
							<MediaCard {item} />
						{/if}
					</div>
				{/each}
			{/if}
		</div>
		<!-- Fades, not just arrows: a card sliced by the edge should read as "the row continues", which
		     is also what makes the arrow legible sitting on top of artwork. Both are pointer-transparent
		     so they never eat a click meant for the card underneath. -->
		{#if canLeft}
			<div
				class="pointer-events-none absolute inset-y-0 left-0 w-16 bg-gradient-to-r from-background to-transparent"
			></div>
			<button
				aria-label={t('a11y.scroll_left')}
				onclick={() => page(-1)}
				class="absolute left-1 top-1/2 flex h-9 w-9 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full border bg-background text-foreground opacity-0 shadow-lg transition hover:scale-105 focus-visible:opacity-100 group-hover/shelf:opacity-100"
			>
				<HugeiconsIcon icon={ArrowLeft01Icon} class="h-4 w-4" />
			</button>
		{/if}
		{#if canRight}
			<div
				class="pointer-events-none absolute inset-y-0 right-0 w-16 bg-gradient-to-l from-background to-transparent"
			></div>
			<button
				aria-label={t('a11y.scroll_right')}
				onclick={() => page(1)}
				class="absolute right-1 top-1/2 flex h-9 w-9 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full border bg-background text-foreground opacity-0 shadow-lg transition hover:scale-105 focus-visible:opacity-100 group-hover/shelf:opacity-100"
			>
				<HugeiconsIcon icon={ArrowRight01Icon} class="h-4 w-4" />
			</button>
		{/if}
	</div>
</section>
