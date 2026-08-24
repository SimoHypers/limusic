<script lang="ts">
	// The home grid the user curates (was "Quick Picks" — renamed: YouTube Music has a shelf by that
	// name and it isn't this one). It holds what was put in it, in the order it was dragged into, plus
	// On Repeat once that has enough songs (the only tile the app suggests, and removing it is
	// permanent). Unlike before it renders even when empty — a section that hides itself is a section
	// nobody discovers. Logic in $lib/personal.ts.
	//
	// Not square cards: these were 5.5rem tiles and every label came out as "アプソリュ…" over a
	// subtitle that was "Simo Hypers •…" fifteen times. A shortcut is a thing you already know, so
	// what it owes you is its *name* at a size you can read, not another piece of cover art competing
	// with the shelves below. Hence wide tiles with the art flush to the leading edge — four to a row
	// instead of seven, and the title gets four times the width.
	//
	// ponytail: drag is the only way to reorder (no keyboard equivalent). Add/remove/open all work
	// from the keyboard; wire arrow-key moves onto the tiles if anyone actually needs it.
	import { flip } from 'svelte/animate';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Cancel01Icon,
		Add01Icon,
		DashboardSquare02Icon,
		Edit01Icon,
		PlayIcon,
		MusicNote01Icon,
		UserIcon,
		ListRestartIcon
	} from '@hugeicons/core-free-icons';
	import SectionHeading from './SectionHeading.svelte';
	import ShortcutPicker from './ShortcutPicker.svelte';
	import { ON_REPEAT_ID } from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { openItem, playItem } from '$lib/browse';
	import { library, personal, placePick, removePick } from '$lib/player.svelte';
	import { freshen, MAX_PICKS } from '$lib/personal';
	import { getDragItem, isDragItem, setDragItem } from '$lib/dnd';
	import { formatSubtitle, t } from '$lib/i18n.svelte';
	import ItemMenu from './ItemMenu.svelte';

	// The page owns the Edit-home modal; this section only lends it a place to be opened from. Its
	// header is the first thing on home and the one row that's always there, so the button that
	// rearranges the rest of the page lives here rather than following a section that can be hidden.
	let { onEdit }: { onEdit?: () => void } = $props();

	// Tiles are stored as a snapshot of the card, so a playlist that has gained tracks since it was
	// pinned would keep showing the old count; `freshen` overlays the live library row (#67).
	const picks = $derived(personal.picks.map((p) => freshen(p, library.items)));
	let picking = $state(false);
	// Where a drop would land: the id of the tile it goes in front of, `null` for the end of the grid,
	// `undefined` when no drag of ours is over the section at all.
	let before = $state<string | null | undefined>(undefined);
	let busy = $state<string | null>(null); // id of the tile whose fetch-then-play is in flight
	// Google's CDN 404s some rewritten sizes; a dead thumb degrades to a placeholder icon rather than
	// the browser's broken-image glyph.
	let failed = $state<Record<string, boolean>>({});

	// One handler for the whole section: the tile under the cursor carries its id in `data-pick`, so
	// hovering anywhere else (the header, the empty dropzone, past the last row) means "append".
	// The gaps *between* tiles are the exception — they belong to the grid, and reading them as
	// "append" made the marker teleport to the end of the row every time the cursor crossed one, so
	// there they hold whatever the last tile decided.
	function targetId(e: DragEvent): string | null {
		const el = e.target as HTMLElement | null;
		const tile = el?.closest('[data-pick]');
		if (tile) return tile.getAttribute('data-pick');
		return el?.closest('[data-grid]') ? (before ?? null) : null;
	}

	function over(e: DragEvent) {
		if (!isDragItem(e)) return; // a file or a link — leave it to the page
		e.preventDefault(); // required, or the drop is refused
		if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
		before = targetId(e);
	}

	function drop(e: DragEvent) {
		const beforeId = before ?? targetId(e);
		before = undefined;
		const item = getDragItem(e);
		if (!item) return;
		e.preventDefault();
		placePick(item, beforeId);
	}

	async function play(item: BrowseItem) {
		if (busy) return;
		busy = item.id;
		try {
			await playItem(item);
		} finally {
			busy = null;
		}
	}
</script>

<!-- A drag cancelled with Esc never reaches `drop`, and only fires `dragleave` if the pointer happens
     to be moving — without this the marker stays painted until the next drag. -->
<svelte:window ondragend={() => (before = undefined)} />

<section>
	<SectionHeading title={t('home.shortcuts')} icon={DashboardSquare02Icon}>
		{#snippet lead()}
			{#if onEdit}
				<!-- Labelled and outlined, not a bare glyph: this is the only entry point to rearranging
				     home, and as an icon on its own nobody found it. The tooltip carries the scope the
				     one-word label can't, sitting as it does under the Shortcuts heading. -->
				<button
					onclick={onEdit}
					title={t('home.edit_home')}
					class="flex shrink-0 cursor-pointer items-center gap-1.5 rounded-lg border px-2.5 py-1 text-sm font-medium text-muted-foreground transition-colors hover:border-foreground/20 hover:bg-muted hover:text-foreground"
				>
					<HugeiconsIcon icon={Edit01Icon} class="h-4 w-4" />
					{t('home.edit_home')}
				</button>
			{/if}
		{/snippet}
		{#if picks.length && picks.length < MAX_PICKS}
			<!-- Adding is an action, not a tile: as a trailing "+" square it left a dashed hole at the
			     end of the grid forever, and a ragged one whenever the last row was short. -->
			<button
				onclick={() => (picking = true)}
				class="flex shrink-0 cursor-pointer items-center gap-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
			>
				<HugeiconsIcon icon={Add01Icon} class="h-3.5 w-3.5" />
				{t('common.add')}
			</button>
		{/if}
	</SectionHeading>

	<div
		role="group"
		aria-label={t('home.shortcuts')}
		ondragover={over}
		ondrop={drop}
		ondragleave={(e) => {
			// Only when the pointer leaves the section itself, not on every hop between its children —
			// and measured against the box, not `relatedTarget`, which WebKit leaves null on drag
			// events. That made every hop between a tile's cover, its title and the next tile read as a
			// real exit, so the marker blinked out from under the cursor while it was still inside.
			const r = e.currentTarget.getBoundingClientRect();
			if (e.clientX < r.left || e.clientX >= r.right || e.clientY < r.top || e.clientY >= r.bottom)
				before = undefined;
		}}
	>
		{#if !picks.length}
			<!-- Empty: one wide drop target, not a heading plus a paragraph plus a row of ghost tiles.
			     This sits at the top of home for anyone who never uses the feature, so it earns its
			     space by being a single line and an obvious place to drop something. -->
			<button
				onclick={() => (picking = true)}
				class="flex w-full cursor-pointer items-center gap-3 rounded-xl border border-dashed p-4 text-left transition-colors hover:border-foreground/30 hover:bg-accent/5 {before ===
				null
					? 'border-primary bg-accent/5'
					: ''}"
			>
				<span
					class="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-dashed text-muted-foreground"
				>
					<HugeiconsIcon icon={Add01Icon} class="h-5 w-5" />
				</span>
				<span class="min-w-0">
					<span class="block text-sm font-medium">{t('home.add_shortcut')}</span>
					<span class="block text-xs text-muted-foreground">
						{t('home.shortcuts_desc')}
					</span>
				</span>
			</button>
		{:else}
			<div data-grid class="grid grid-cols-[repeat(auto-fill,minmax(15rem,1fr))] gap-2">
				{#each picks as item (item.id)}
					{@const round = item.kind === 'artist'}
					{@const onRepeat = item.id === ON_REPEAT_ID}
					<!-- group/pick, not `group`: nested unnamed groups would fire each other's hovers. -->
					<div class="group/pick relative" data-ctx data-pick={item.id} animate:flip={{ duration: 200 }}>
						<!-- Where the drop lands: a bar down the leading edge of the tile it goes in front of. -->
						{#if before === item.id}
							<div class="absolute -left-1 bottom-0 top-0 z-20 w-0.5 rounded-full bg-primary"></div>
						{/if}
						<div
							class="flex h-16 cursor-pointer items-center gap-3 overflow-hidden rounded-xl border bg-card/40 text-left transition-colors hover:border-foreground/20 hover:bg-card"
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
							<!-- Art bleeds into the tile's leading edge (a circle can't, so an artist's is
							     inset). Play lives *on* the cover rather than as a third control on the right:
							     no reserved gutter, and it puts the action where the eye already is. -->
							<div
								class="relative shrink-0 overflow-hidden bg-muted {round
									? 'my-2 ml-2 h-12 w-12 rounded-full'
									: 'h-16 w-16'}"
							>
								{#if item.thumbnail && !failed[item.id] && !onRepeat}
									<img
										src={thumb(item.thumbnail, 400)}
										alt=""
										class="h-full w-full object-cover"
										loading="lazy"
										draggable="false"
										onerror={() => (failed = { ...failed, [item.id]: true })}
									/>
								{:else}
									<div
										class="flex h-full w-full items-center justify-center {onRepeat
											? 'bg-primary/10 text-primary'
											: 'text-muted-foreground/50'}"
									>
										<!-- altIcon/showAlt, not a third ternary: `icon` is read once at mount. -->
										<HugeiconsIcon
											icon={round ? UserIcon : MusicNote01Icon}
											altIcon={ListRestartIcon}
											showAlt={onRepeat}
											class={onRepeat ? 'h-7 w-7' : 'h-5 w-5'}
										/>
									</div>
								{/if}
								{#if !round}
									<button
										class="absolute inset-0 flex cursor-pointer items-center justify-center bg-black/50 text-white opacity-0 transition-opacity hover:bg-black/60 focus-visible:opacity-100 group-hover/pick:opacity-100"
										class:animate-pulse={busy === item.id}
										disabled={busy === item.id}
										aria-label="Play {item.title}"
										onclick={(e) => {
											e.stopPropagation();
											play(item);
										}}
									>
										<HugeiconsIcon icon={PlayIcon} class="h-5 w-5" />
									</button>
								{/if}
							</div>
							<!-- pr-14 keeps the title clear of the ⋯ and remove buttons in the corner. -->
							<div class="min-w-0 flex-1 pr-14">
								<div class="truncate text-sm font-medium">{item.title}</div>
								{#if item.subtitle}
									<div class="truncate text-xs text-muted-foreground">{formatSubtitle(item.subtitle)}</div>
								{/if}
							</div>
						</div>
						<ItemMenu
							{item}
							triggerClass="absolute right-7 top-1 z-10 flex h-6 w-6 cursor-pointer items-center justify-center rounded-full text-muted-foreground opacity-0 transition hover:bg-muted hover:text-foreground focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring group-hover/pick:opacity-100"
						/>
						<button
							onclick={() => removePick(item.id)}
							title="Remove from shortcuts"
							aria-label="Remove from shortcuts"
							class="absolute right-1 top-1 z-10 flex h-6 w-6 cursor-pointer items-center justify-center rounded-full text-muted-foreground opacity-0 transition hover:bg-muted hover:text-foreground focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring group-hover/pick:opacity-100"
						>
							<HugeiconsIcon icon={Cancel01Icon} class="h-3 w-3" />
						</button>
					</div>
				{/each}

				<!-- The append slot only exists while a drag is actually heading for the end of the grid,
				     so nothing dangles off the last row the rest of the time. -->
				{#if before === null && picks.length < MAX_PICKS}
					<div
						class="flex h-16 items-center justify-center rounded-xl border border-dashed border-primary text-xs font-medium text-primary"
					>
						Add to the end
					</div>
				{/if}
			</div>
		{/if}
	</div>
</section>

{#if picking}
	<ShortcutPicker onclose={() => (picking = false)} />
{/if}
