<script lang="ts">
	// Edit home: a panel beside the page rather than a modal over it. Every change lands on home as
	// it's made, so the page behind is the preview and Done only closes. That is also why there's no
	// Cancel any more: nothing is pending to throw away, and Reset puts home back the way it ships.
	//
	// It reads top to bottom the way it gets used: how home looks (three quick settings), then the
	// sections themselves, which is the long part.
	import { tick, untrack } from 'svelte';
	import { Dialog as DialogPrimitive, RadioGroup } from 'bits-ui';
	import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/svelte';
	import {
		ArrowTurnBackwardIcon,
		Cancel01Icon,
		Clock01Icon,
		DashboardSquare02Icon,
		DashboardSquareEditIcon,
		DragDropVerticalIcon,
		RefreshIcon,
		Tick02Icon,
		UserStar01Icon,
		YoutubeIcon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Switch } from '$lib/components/ui/switch';
	import * as Dialog from '$lib/components/ui/dialog';
	import { dragScroll, SECTION_ROW_MIME } from '$lib/dnd';
	import { personal, resetHome, saveHome } from '$lib/player.svelte';
	import { CARD_SIZES, hiddenSections, homeIsDefault, type CardSize } from '$lib/personal';
	import { t, type TranslationKey } from '$lib/i18n.svelte';

	let {
		open = $bindable(false),
		sections
	}: { open: boolean; sections: { key: string; title: string }[] } = $props();

	// `fresh`: YouTube started sending it after the last arrangement. Worked out once per opening, so
	// the badge stays on for the whole visit even though the first edit ranks every row.
	type Row = { key: string; title: string; shown: boolean; fresh: boolean };
	let rows = $state<Row[]>([]);
	// Index of the row being dragged. It follows the row as the list rearranges under it, so the
	// pointer keeps hold of the same section rather than of whatever slid into that slot.
	let dragging = $state<number | null>(null);
	let list = $state<HTMLElement | null>(null);
	/** Read out by screen readers after a keyboard move. */
	let announce = $state('');

	const shownCount = $derived(rows.filter((r) => r.shown).length);
	const customized = $derived(!homeIsDefault(personal));

	// The sections the app builds get their own glyph; everything else is a YouTube shelf.
	const OURS: Record<string, IconSvgElement> = {
		'@shortcuts': DashboardSquare02Icon,
		'@recent': ArrowTurnBackwardIcon,
		'@familiar': UserStar01Icon,
		'@forgotten': Clock01Icon
	};

	const SIZE_LABEL: Record<CardSize, TranslationKey> = {
		small: 'home.card_small',
		medium: 'home.card_medium',
		large: 'home.card_large'
	};
	// The picture on each size option: the same row width holding fewer, bigger cards.
	const PREVIEW: Record<CardSize, { n: number; px: number }> = {
		small: { n: 4, px: 11 },
		medium: { n: 3, px: 15 },
		large: { n: 2, px: 24 }
	};

	/**
	 * Snapshotted from `sections`, not derived: the feed keeps appending shelves as the page behind
	 * loads, and a list that grows mid-drag reshuffles under the cursor.
	 */
	function build() {
		const hidden = hiddenSections(personal);
		const ranked = new Set(personal.home.order);
		rows = sections.map((s) => ({
			key: s.key,
			title: s.title,
			shown: !hidden.has(s.key),
			fresh: ranked.size > 0 && !ranked.has(s.key)
		}));
		dragging = null;
	}

	// `open` is the only thing that may re-run this, hence `untrack`: a rebuild landing in the middle
	// of an edit would throw the edit away.
	$effect(() => {
		if (open) untrack(build);
	});

	/** The list as it stands. Every row goes into `order`, so nothing listed is new after this. */
	const arrangement = () => ({
		order: rows.map((r) => r.key),
		hidden: rows.filter((r) => !r.shown).map((r) => r.key)
	});
	const commit = () => saveHome(arrangement());

	/**
	 * Live reorder while dragging: the row moves as you pass over its neighbours, no drop marker.
	 * Written to home on dragend rather than per swap, so the page behind reflows once per drag.
	 *
	 * Only once the pointer is past the *middle* of the row it is over, not the moment it touches its
	 * edge. Swapping on contact leaves the pointer sitting on the boundary it just crossed, so a
	 * pixel of jitter (or the row heights differing by one) bounces the list between two orders for
	 * as long as you hold still. Half a row of hysteresis is what stops that.
	 */
	function moveTo(to: number) {
		if (dragging === null || dragging === to) return;
		const next = rows.slice();
		next.splice(to, 0, ...next.splice(dragging, 1));
		rows = next;
		dragging = to;
	}

	/** Arrow keys on a row's handle. Focus follows the row, so holding the key walks it along. */
	async function nudge(i: number, by: number) {
		const to = i + by;
		if (to < 0 || to >= rows.length) return;
		const next = rows.slice();
		next.splice(to, 0, ...next.splice(i, 1));
		rows = next;
		commit();
		const row = rows[to];
		announce = t('home.moved_section', { title: row.title, position: to + 1, total: rows.length });
		await tick();
		list?.querySelector<HTMLElement>(`[data-grip="${CSS.escape(row.key)}"]`)?.focus();
	}

	function show(row: Row, on: boolean) {
		row.shown = on;
		commit();
	}

	/** Clear the page in one go, then switch on only the few you want. Flips to "on" once all are off. */
	function showAll(on: boolean) {
		for (const r of rows) r.shown = on;
		commit();
	}

	/**
	 * Off ranks every section listed, so the ones on the page now stay put and only what YouTube
	 * sends after this is held back. On writes only the flag: whatever the policy was holding back is
	 * still unranked, so it shows again by itself, and the rebuild turns its switch back on.
	 */
	function setShowNew(on: boolean) {
		saveHome(on ? { hideNew: false } : { hideNew: true, ...arrangement() });
		if (on) build();
	}

	function reset() {
		resetHome();
		// `sections` re-derives from the reset arrangement on read, so this lays out YouTube's order.
		build();
	}
</script>

{#snippet toggleRow(title: string, desc: string, checked: boolean, set: (on: boolean) => void)}
	<div class="flex items-center justify-between gap-6 px-4 py-3.5">
		<div class="min-w-0">
			<div class="text-sm font-medium">{title}</div>
			<p class="mt-1 text-xs leading-relaxed text-muted-foreground">{desc}</p>
		</div>
		<Switch {checked} onCheckedChange={set} aria-label={title} />
	</div>
{/snippet}

<Dialog.Root bind:open>
	<Dialog.Portal>
		<!-- The page is the preview, so the overlay only shades the side the panel is on and leaves the
		     rest readable. No blur: a full-window backdrop-filter is what WebKitGTK chokes on. -->
		<Dialog.Overlay
			class="bg-transparent bg-gradient-to-l from-black/50 via-black/15 to-transparent supports-backdrop-filter:backdrop-blur-none"
		/>
		<!-- Floats clear of the titlebar and the window's rounded corner: flush to the edge, its square
		     corner would poke out past the window's round one. -->
		<DialogPrimitive.Content
			class="fixed bottom-3 right-3 top-12 z-50 flex w-[25rem] max-w-[calc(100%-1.5rem)] flex-col overflow-hidden rounded-2xl border bg-popover text-popover-foreground shadow-2xl outline-none duration-200 data-open:animate-in data-open:fade-in-0 data-open:slide-in-from-right-8 data-closed:animate-out data-closed:fade-out-0 data-closed:slide-out-to-right-8"
		>
			<div class="flex items-start gap-3 border-b px-5 py-4">
				<span
					class="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary"
				>
					<HugeiconsIcon icon={DashboardSquareEditIcon} class="h-5 w-5" />
				</span>
				<div class="min-w-0 flex-1">
					<Dialog.Title class="font-heading text-base font-semibold">{t('home.edit_home')}</Dialog.Title>
					<Dialog.Description class="text-xs leading-relaxed text-muted-foreground">
						{t('home.edit_home_desc')}
					</Dialog.Description>
				</div>
				<Button
					variant="ghost"
					size="icon-sm"
					class="-mr-1.5 shrink-0"
					aria-label={t('common.close')}
					onclick={() => (open = false)}
				>
					<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
				</Button>
			</div>

			<!-- dragScroll: the section list is taller than the panel once home has remembered a dozen
			     shelves, and without an edge pull you can only drop one somewhere already on screen. -->
			<div
				class="min-h-0 flex-1 overflow-y-auto px-5 py-5"
				{@attach (node) => dragScroll(node, SECTION_ROW_MIME)}
			>
				<h3 class="mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">
					{t('home.look')}
				</h3>
				<div class="mb-7 divide-y divide-border/60 overflow-hidden rounded-xl border bg-card">
					<div class="px-4 py-3.5">
						<div class="text-sm font-medium">{t('home.card_size')}</div>
						<p class="mt-1 text-xs leading-relaxed text-muted-foreground">{t('home.card_size_desc')}</p>
						<!-- A radio group, not three buttons: arrow keys move between the sizes. -->
						<RadioGroup.Root
							value={personal.home.cards}
							onValueChange={(v) => saveHome({ cards: v as CardSize })}
							aria-label={t('home.card_size')}
							class="mt-3 grid grid-cols-3 gap-2"
						>
							{#each CARD_SIZES as size (size)}
								<RadioGroup.Item
									value={size}
									class="group flex cursor-pointer flex-col items-center gap-2 rounded-xl border px-2 pb-2 pt-3 text-xs font-medium text-muted-foreground outline-none transition-colors hover:border-foreground/25 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-checked:border-primary data-checked:bg-primary/8 data-checked:text-foreground"
								>
									<span class="flex h-6 items-end gap-[3px]" aria-hidden="true">
										{#each Array(PREVIEW[size].n) as _, i (i)}
											<span
												class="rounded-[3px] bg-muted-foreground/30 transition-colors group-data-checked:bg-primary/70"
												style="width:{PREVIEW[size].px}px;height:{PREVIEW[size].px}px"
											></span>
										{/each}
									</span>
									{t(SIZE_LABEL[size])}
								</RadioGroup.Item>
							{/each}
						</RadioGroup.Root>
					</div>
					{@render toggleRow(t('home.mood_chips'), t('home.mood_chips_desc'), personal.home.chips, (on) =>
						saveHome({ chips: on })
					)}
					{@render toggleRow(
						t('home.artwork_backdrop'),
						t('home.artwork_backdrop_desc'),
						personal.home.backdrop,
						(on) => saveHome({ backdrop: on })
					)}
				</div>

				<div class="mb-2 flex items-baseline justify-between gap-3 px-1">
					<h3 class="text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">
						{t('home.sections')}
					</h3>
					<div class="flex items-baseline gap-3">
						<span class="text-xs tabular-nums text-muted-foreground">
							{t('home.sections_shown', { shown: shownCount, total: rows.length })}
						</span>
						<button
							onclick={() => showAll(shownCount === 0)}
							disabled={!rows.length}
							class="cursor-pointer text-xs font-medium text-primary transition-colors hover:text-primary/80 disabled:pointer-events-none disabled:opacity-50"
						>
							{shownCount === 0 ? t('home.turn_all_on') : t('home.turn_all_off')}
						</button>
					</div>
				</div>
				<div
					bind:this={list}
					role="list"
					class="divide-y divide-border/60 overflow-hidden rounded-xl border bg-card"
				>
					{#each rows as row, i (row.key)}
						<!-- The whole row is the drag source, not just the grip: aiming at a small handle inside
						     a scrolling list is a chore, and the grip still says which part to grab.

						     No `animate:flip`. A flip animates with a transform, and a transform moves the row's
						     hit box: for the 180ms after a swap, the element under the cursor is whichever row
						     is mid-flight over that point, and it reports the index it will have when it lands.
						     So the dragover that arrives during the glide asks to swap straight back, and the
						     list ping-pongs for as long as you keep moving. -->
						<div
							role="listitem"
							draggable="true"
							ondragstart={(e) => {
								dragging = i;
								if (!e.dataTransfer) return;
								// Our own type, not `text/plain`: it doubles as the payload some engines demand, it
								// keeps `blockForeignDrag` from treating our own drag as a stray file, and it is
								// what `dragScroll` above watches for.
								e.dataTransfer.setData(SECTION_ROW_MIME, row.key);
								e.dataTransfer.effectAllowed = 'move';
							}}
							ondragover={(e) => {
								// A file, or a card dragged in from the page behind the panel.
								if (dragging === null || !e.dataTransfer?.types.includes(SECTION_ROW_MIME)) return;
								e.preventDefault();
								e.dataTransfer.dropEffect = 'move';
								const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
								const past = e.clientY > r.top + r.height / 2;
								if (i > dragging ? past : !past) moveTo(i);
							}}
							ondragend={() => {
								dragging = null;
								commit();
							}}
							class="flex cursor-grab items-center gap-2.5 py-2 pl-1.5 pr-3 transition-colors {dragging ===
							i
								? 'bg-muted opacity-60'
								: 'hover:bg-muted/40'}"
						>
							<button
								data-grip={row.key}
								aria-label={t('home.move_section', { title: row.title })}
								onkeydown={(e) => {
									if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return;
									e.preventDefault();
									nudge(i, e.key === 'ArrowUp' ? -1 : 1);
								}}
								class="flex h-8 w-6 shrink-0 cursor-grab items-center justify-center rounded-md text-muted-foreground/50 outline-none transition-colors hover:text-foreground focus-visible:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
							>
								<HugeiconsIcon icon={DragDropVerticalIcon} class="h-4 w-4" />
							</button>
							<HugeiconsIcon
								icon={OURS[row.key] ?? YoutubeIcon}
								class="h-4 w-4 shrink-0 transition-colors {row.shown
									? 'text-primary/70'
									: 'text-muted-foreground/40'}"
							/>
							<span
								class="min-w-0 flex-1 truncate text-sm transition-colors {row.shown
									? ''
									: 'text-muted-foreground'}"
								title={row.title}
							>
								{row.title}
							</span>
							{#if row.fresh}
								<span
									class="shrink-0 rounded-full bg-primary/12 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-primary"
								>
									{t('home.new_section')}
								</span>
							{/if}
							<Switch
								size="sm"
								checked={row.shown}
								onCheckedChange={(on) => show(row, on)}
								aria-label={t('home.show_section', { title: row.title })}
							/>
						</div>
					{/each}
				</div>
				<p class="mt-2 px-1 text-xs leading-relaxed text-muted-foreground">{t('home.sections_hint')}</p>

				<div class="mt-4 overflow-hidden rounded-xl border bg-card">
					{@render toggleRow(t('home.show_new'), t('home.show_new_desc'), !personal.home.hideNew, setShowNew)}
				</div>
			</div>

			<div class="flex items-center gap-2 border-t px-5 py-3">
				<!-- Left, away from Done: it throws the whole arrangement out. -->
				<Button
					variant="ghost"
					size="sm"
					class="mr-auto text-muted-foreground"
					disabled={!customized}
					onclick={reset}
				>
					<HugeiconsIcon icon={RefreshIcon} class="h-4 w-4" />
					{t('common.reset')}
				</Button>
				<Button size="sm" onclick={() => (open = false)}>
					<HugeiconsIcon icon={Tick02Icon} class="h-4 w-4" />
					{t('common.done')}
				</Button>
			</div>

			<div class="sr-only" aria-live="polite">{announce}</div>
		</DialogPrimitive.Content>
	</Dialog.Portal>
</Dialog.Root>
