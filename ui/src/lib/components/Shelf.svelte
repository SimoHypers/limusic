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
	import { HugeiconsIcon, type IconSvgElement } from "@hugeicons/svelte";
	import {
		ArrowLeft01Icon,
		ArrowRight01Icon,
		CdIcon,
		MusicNote01Icon,
		PlayIcon,
		PlayListIcon,
		UserMultiple02Icon,
	} from "@hugeicons/core-free-icons";
	import MediaCard from "./MediaCard.svelte";
	import CommunityCard from "./CommunityCard.svelte";
	import PortraitCard from "./PortraitCard.svelte";
	import StackCard from "./StackCard.svelte";
	import SectionHeading from "./SectionHeading.svelte";
	import TrackRow from "./TrackRow.svelte";
	import * as api from "$lib/api";
	import type { BrowseItem } from "$lib/api";
	import type { CardSize } from "$lib/personal";
	import { asSong } from "$lib/browse";
	import {
		openAddToPlaylist,
		openPlayer,
		playSong,
		playback,
	} from "$lib/player.svelte";
	import { Button } from "$lib/components/ui/button";
	import { t } from "$lib/i18n.svelte";

	let {
		title,
		items,
		onMore,
		community = false,
		rich = true,
		queueAll = true,
		size = "medium",
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
	type Mode = "song" | "album" | "artist" | "playlist" | "card";
	const mode = $derived.by<Mode>(() => {
		if (community || !rich || !items.length) return "card";
		const counts = new Map<string, number>();
		for (const i of items)
			counts.set(i.kind, (counts.get(i.kind) ?? 0) + 1);
		const [kind, n] = [...counts].sort((a, b) => b[1] - a[1])[0];
		return n / items.length >= MOSTLY ? (kind as Mode) : "card";
	});

	const ICONS: Record<Mode, IconSvgElement | undefined> = {
		song: MusicNote01Icon,
		album: CdIcon,
		artist: UserMultiple02Icon,
		playlist: PlayListIcon,
		card: undefined,
	};

	// Song mode: four rows to a column, paged sideways. Twelve legible tracks per screenful against
	// the six anonymous squares that fitted before. A non-song can't be a row that queues with the
	// rest, so it leads the shelf as a plain card instead, the same thing the other modes do with an
	// item that doesn't fit their form. Search's "Top results" is exactly this shape (the artist you
	// searched for plus three of their songs), and dropping it hid the match entirely.
	const ROWS = 4;
	const songs = $derived(
		mode === "song"
			? items.filter((i) => i.kind === "song").map(asSong)
			: [],
	);
	const others = $derived(
		mode === "song" ? items.filter((i) => i.kind !== "song") : [],
	);
	const otherColumns = $derived(
		Array.from({ length: Math.ceil(others.length / 2) }, (_, c) =>
			others.slice(c * 2, c * 2 + 2),
		),
	);
	const columns = $derived(
		Array.from({ length: Math.ceil(songs.length / ROWS) }, (_, c) =>
			songs.slice(c * ROWS, c * ROWS + ROWS),
		),
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
	const SONG_SLOT =
		"basis-full sm:basis-[calc((100%-1rem)/2)] xl:basis-[calc((100%-2rem)/3)]";
	const WIDTH: Record<CardSize, number> = { small: 8, medium: 10, large: 13 };
	const cardWidth = (playlist = false) =>
		`width:${WIDTH[size] + (playlist ? 1 : 0)}rem`;
	// A card is about its width plus 7.5rem of heading, caption and padding.
	const height = $derived(
		mode === "song" ? "17rem" : `${WIDTH[size] + 7.5}rem`,
	);

	let row = $state<HTMLDivElement | null>(null);
	let canLeft = $state(false);
	let canRight = $state(false);
	let intersecting = $state(false);

	function update() {
		if (!row) return;
		canLeft = row.scrollLeft > 4;
		canRight = row.scrollLeft + row.clientWidth < row.scrollWidth - 4;
	}

	let hovered = false;
	const measureOnEnter = (el: HTMLElement) => {
		const enter = () => {
			hovered = true;
			update();
		};
		const leave = () => (hovered = false);
		el.addEventListener("pointerenter", enter);
		el.addEventListener("pointerleave", leave);
		return () => {
			el.removeEventListener("pointerenter", enter);
			el.removeEventListener("pointerleave", leave);
		};
	};

	function page(dir: 1 | -1) {
		row?.scrollBy({
			left: dir * Math.round(row.clientWidth * 0.9),
			behavior: "smooth",
		});
	}

	// Re-measure when the content changes under the pointer, and only then. Measuring at mount reads
	// scrollWidth before content-visibility has skipped anything, which makes WebKitGTK lay out every
	// shelf on the page synchronously: going back to Home spent ~640 of its ~870 ms there with 8
	// shelves (perf/navprobe.py). The arrows only show on hover, and pointerenter measures then.
	$effect(() => {
		items;
		if (hovered || intersecting) update();
	});

	$effect(() => {
		if (!row) return;
		const observer = new IntersectionObserver(([entry]) => {
			intersecting = entry.isIntersecting;
			if (entry.isIntersecting) update();
		});
		observer.observe(row);
		return () => observer.disconnect();
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
	style="contain-intrinsic-size: auto {height};"
>
	{#if title || onMore || canLeft || canRight}
		<SectionHeading title={title ?? ""} icon={ICONS[mode]} {onMore}>
			{#if songs.length}
				<button
					onclick={playAll}
					class="flex shrink-0 cursor-pointer items-center gap-1 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary transition-colors hover:bg-primary/20"
				>
					<HugeiconsIcon icon={PlayIcon} class="h-3.5 w-3.5" />
					{t("common.play_all")}
				</button>
			{/if}
			{#snippet trailing()}
				{#if canLeft || canRight}
					<div class="flex items-center gap-0.5">
						<Button
							variant="ghost"
							size="icon-sm"
							onclick={() => page(-1)}
							disabled={!canLeft}
							aria-label={t("a11y.scroll_left")}
						>
							<HugeiconsIcon
								icon={ArrowLeft01Icon}
								class="h-4 w-4"
							/>
						</Button>
						<Button
							variant="ghost"
							size="icon-sm"
							onclick={() => page(1)}
							disabled={!canRight}
							aria-label={t("a11y.scroll_right")}
						>
							<HugeiconsIcon
								icon={ArrowRight01Icon}
								class="h-4 w-4"
							/>
						</Button>
					</div>
				{/if}
			{/snippet}
		</SectionHeading>
	{/if}
	<!-- Measure on pointer enter, because a shelf skipped by content-visibility has no layout at
	     mount: scrollWidth reads 0 and the arrows never appear. They only show on hover, so measuring
	     as the pointer arrives is both correct and later than the mount-time forced layout.
	     An attachment rather than onpointerenter: the handler doesn't make this div interactive. -->
	<div class="group/shelf relative" {@attach measureOnEnter}>
		<div
			class="rail flex snap-x overflow-x-auto pb-2 {mode === 'song'
				? 'gap-4'
				: community
					? 'gap-3'
					: 'gap-2'}"
			style={mode === "song" && others.length > 0
				? `--card-space: ${others.length * (WIDTH[size] + 1)}rem`
				: undefined}
			bind:this={row}
			onscroll={update}
		>
			{#if mode === "song"}
				{#each otherColumns as col, c (c)}
					<div
						class="min-w-0 shrink-0 snap-start {SONG_SLOT} flex gap-4"
					>
						{#each col as item (item.id)}
							<div class="min-w-0 flex-1">
								<MediaCard {item} />
							</div>
						{/each}
					</div>
				{/each}
				<!-- A gutter between columns, not a rule down each one: the rows' own hover fill already
				     shows where a column ends, and the rules were four more lines per shelf (#319). -->
				{#each columns as col, c (c)}
					<div class="min-w-0 shrink-0 snap-start {SONG_SLOT}">
						{#each col as song, r (song.video_id + ":" + r)}
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
				{#each items as item, i (item.id + ":" + i)}
					<!-- A shelf keeps its form even where one item doesn't fit it: a stray song in an
					     artist shelf gets the plain card and its width, not a poster it isn't. -->
					{@const own = community
						? item.kind === "playlist"
						: item.kind === mode}
					<!-- min-w-0: a flex item's automatic minimum size is its min-content, which overrides
					     the basis, so without this a card with a long title grows past its slot. -->
					<div
						class="min-w-0 shrink-0 snap-start {own && community
							? 'basis-full sm:basis-[calc((100%-0.75rem)/2)] lg:basis-[calc((100%-2.25rem)/4)]'
							: ''}"
						style={own && community
							? undefined
							: cardWidth(own && mode === "playlist")}
					>
						{#if !own}
							<MediaCard {item} />
						{:else if community}
							<CommunityCard {item} />
						{:else if mode === "artist"}
							<PortraitCard {item} />
						{:else if mode === "playlist"}
							<StackCard {item} />
						{:else}
							<MediaCard {item} />
						{/if}
					</div>
				{/each}
			{/if}
		</div>
	</div>
</section>
