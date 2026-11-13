<script lang="ts" module>
	const rails = new WeakMap<Element, () => void>();
	const railSizes = new ResizeObserver((entries) => {
		for (const e of entries) rails.get(e.target)?.();
	});
</script>

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
		PlayIcon,
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
	import type { CardSize } from '$lib/personal';
	import { asSong } from '$lib/browse';
	import { openAddToPlaylist, openPlayer, playSong, playback } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import { untrack } from 'svelte';

	let {
		title,
		items,
		onMore,
		community = false,
		rich = true,
		queueAll = true,
		size = 'medium'
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
		/**
		 * Whether clicking a song row queues the rest of the shelf behind it. True for a shelf that
		 * is a set (an album's songs, an artist's top tracks). False on home, where a shelf is a pile
		 * of unrelated suggestions: clicking one there plays that one and lets autoplay take over.
		 */
		queueAll?: boolean;
		/** Card width, set in Edit home. Rows of songs and community cards keep their own. */
		size?: CardSize;
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
	// Unless the shelf isn't a set (`queueAll={false}`), where only the clicked song plays.
	const play = (start: number) => {
		if (!queueAll) return playSong(songs[start]);
		openPlayer();
		return api.playPlaylist(songs, start, undefined, title);
	};
	// The header's Play button queues the whole shelf regardless of `queueAll` (#236): on home a row
	// click is "play this one", and this is the explicit way to ask for all of them.
	const playAll = () => {
		openPlayer();
		return api.playPlaylist(songs, 0, undefined, title);
	};

	// Slot width per form, and the height the rail reserves before it has been laid out. Songs are
	// columns of rows, so they take a share of the rail; everything else is a card of `size` width,
	// a playlist's a rem wider for the stack showing behind it. Inline widths, not w-* classes: a
	// stale dev stylesheet that hasn't generated one collapses the card to its artwork's size.
	const SONG_SLOT = 'basis-full sm:basis-1/2 xl:basis-1/3';
	const WIDTH: Record<CardSize, number> = { small: 8, medium: 10, large: 13 };
	const cardWidth = (playlist = false) => `width:${WIDTH[size] + (playlist ? 1 : 0)}rem`;
	// A card is about its width plus 7.5rem of heading, caption and padding.
	const height = $derived(mode === 'song' ? '17rem' : `${WIDTH[size] + 7.5}rem`);

	let row = $state<HTMLDivElement | null>(null);
	let canLeft = $state(false);
	let canRight = $state(false);

	// A shelf mounts the cards the rail can show, not the twenty it holds. Every slot keeps its box,
	// so the rail's width, its snap points and the arrows don't change; a slot past the right edge
	// gets its card once the rail has scrolled to within a page of it, and keeps it. Every pass the
	// engine makes over the page (style, layout, layers) is priced per mounted element, and a home
	// feed was 160 cards with about fifteen of them on screen. perf/framecost.py, Home, WebKitGTK
	// 2.52: a wheel scroll held the main thread at 65% with 40% of frames over 20 ms and hitches to
	// 220 ms; with seven cards a shelf, 40%, 18% and 74 ms.
	//
	// The first count is an estimate, because reading the rail's width at mount is the forced
	// layout that made Home take 870 ms to come back (see `measured`). The window is never
	// narrower than the rail, so this over-counts by about the sidebar's worth of cards.
	const REM = 16;
	function firstScreen() {
		if (mode === 'song') return others.length + 4; // at most three columns show, plus one
		if (community) return 5; // at most four show
		const pitch = (WIDTH[size] + (mode === 'playlist' ? 1 : 0) + 0.5) * REM;
		return Math.ceil(window.innerWidth / pitch) + 1;
	}
	// ponytail: grow-only. A rail someone scrolled to the end keeps all its cards until the page is
	// left; window both edges if a shelf ever holds hundreds.
	let shown = $state(untrack(firstScreen));

	function update() {
		if (!row) return;
		canLeft = row.scrollLeft > 4;
		canRight = row.scrollLeft + row.clientWidth < row.scrollWidth - 4;
		// What the rail shows plus a little past its right edge (two cards, or one column of songs),
		// so a card is there before it scrolls in. The arrow's glide and a wheel both come through
		// here a frame at a time, which spreads the mounting out.
		const last = row.lastElementChild as HTMLElement | null;
		const prev = last?.previousElementSibling as HTMLElement | null;
		const pitch = last && prev ? last.offsetLeft - prev.offsetLeft : 0;
		if (pitch > 0) {
			const need = Math.ceil((row.scrollLeft + row.clientWidth + Math.max(pitch, 320)) / pitch);
			if (need > shown) shown = need;
		}
	}

	function page(dir: 1 | -1) {
		row?.scrollBy({ left: dir * Math.round(row.clientWidth * 0.9), behavior: 'smooth' });
	}

	// The rail is measured when its size changes, which is also when it is first laid out and when
	// content-visibility brings a skipped shelf back, and never at mount: a mount-time read of
	// scrollWidth runs before content-visibility has skipped anything and makes WebKitGTK lay out
	// every shelf on the page synchronously (going back to Home spent ~640 of its ~870 ms there with
	// 8 shelves, perf/navprobe.py). A ResizeObserver reports after layout, so its reads are free, and
	// one observer serves every shelf on the page.
	let laidOut = false;
	const measured = (el: HTMLElement) => {
		rails.set(el, () => {
			laidOut = true;
			update();
		});
		railSizes.observe(el);
		return () => {
			railSizes.unobserve(el);
			rails.delete(el);
		};
	};
	// Content or card size changed under a rail that is already laid out: its own size did not, so
	// nothing above fires.
	$effect(() => {
		items;
		size;
		if (laidOut) update();
	});
</script>

<!-- content-visibility: a home feed grows to hundreds of cards and WebKit keeps every one of them in
     style, layout and paint. Because it rasterizes in tiles, one card's hover repaint re-rasterizes
     the images around it, so hovering gets slower the further you scroll. Skipping off-screen
     shelves caps that at a screenful. The intrinsic size is a shelf of this form (heading + row);
     the `auto` keyword swaps in the real size once measured, so the scrollbar stays put. -->
<section
	class="[content-visibility:auto]"
	style="contain-intrinsic-size: auto {height};"
>
	{#if title || onMore}
		<SectionHeading title={title ?? ''} icon={ICONS[mode]} {onMore}>
			{#if songs.length}
				<button
					onclick={playAll}
					class="flex shrink-0 cursor-pointer items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary transition-colors hover:bg-primary/20"
				>
					<HugeiconsIcon icon={PlayIcon} class="h-3.5 w-3.5" />
					{t('common.play_all')}
				</button>
			{/if}
		</SectionHeading>
	{/if}
	<div class="group/shelf relative">
		<div
			class="rail flex snap-x overflow-x-auto pb-2 {mode === 'song'
				? 'gap-0'
				: community
					? 'gap-3'
					: 'gap-2'}"
			bind:this={row}
			onscroll={update}
			{@attach measured}
		>
			{#if mode === 'song'}
				{#each others as item, i (item.id)}
					<div class="min-w-0 shrink-0 snap-start pr-4" style={cardWidth()}>
						{#if i < shown}<MediaCard {item} />{/if}
					</div>
				{/each}
				<!-- A gutter between columns, not a rule down each one: the rows' own hover fill already
				     shows where a column ends, and the rules were four more lines per shelf (#319). -->
				{#each columns as col, c (c)}
					<div class="min-w-0 shrink-0 snap-start {SONG_SLOT} pr-6">
						{#each others.length + c < shown ? col : [] as song, r (song.video_id + ':' + r)}
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
						class="min-w-0 shrink-0 snap-start {own && community
							? 'basis-full sm:basis-[calc((100%-0.75rem)/2)] lg:basis-[calc((100%-2.25rem)/4)]'
							: ''}"
						style={own && community ? undefined : cardWidth(own && mode === 'playlist')}
					>
						{#if i >= shown}
							<!-- an empty slot: see `shown` -->
						{:else if !own}
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
		<!-- Always mounted and shown by `visibility`, not inserted when the rail can scroll: an
		     insert dirties layout from the document root, and it used to happen under the pointer,
		     the first time each shelf was hovered or scrolled past. -->
		<div
			class="pointer-events-none absolute inset-y-0 left-0 w-16 bg-gradient-to-r from-background to-transparent {canLeft
				? ''
				: 'invisible'}"
		></div>
		<button
			aria-label={t('a11y.scroll_left')}
			onclick={() => page(-1)}
			class="absolute left-1 top-1/2 flex h-9 w-9 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full border bg-background text-foreground opacity-0 shadow-lg transition hover:scale-105 focus-visible:shown group-hover/shelf:shown {canLeft
				? ''
				: 'invisible'}"
		>
			<HugeiconsIcon icon={ArrowLeft01Icon} class="h-4 w-4" />
		</button>
		<div
			class="pointer-events-none absolute inset-y-0 right-0 w-16 bg-gradient-to-l from-background to-transparent {canRight
				? ''
				: 'invisible'}"
		></div>
		<button
			aria-label={t('a11y.scroll_right')}
			onclick={() => page(1)}
			class="absolute right-1 top-1/2 flex h-9 w-9 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full border bg-background text-foreground opacity-0 shadow-lg transition hover:scale-105 focus-visible:shown group-hover/shelf:shown {canRight
				? ''
				: 'invisible'}"
		>
			<HugeiconsIcon icon={ArrowRight01Icon} class="h-4 w-4" />
		</button>
	</div>
</section>
