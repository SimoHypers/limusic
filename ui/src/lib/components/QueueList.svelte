<script lang="ts">
	import { onMount, tick, untrack } from 'svelte';
	import { flip } from 'svelte/animate';
	import { cubicOut } from 'svelte/easing';
	import { MediaQuery } from 'svelte/reactivity';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { HistoryIcon, InfinityIcon } from '@hugeicons/core-free-icons';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import { Button } from '$lib/components/ui/button';
	import * as api from '$lib/api';
	import { queueBlocks, moveTarget, type QueueRow } from '$lib/queue';
	import { blockWindows, fullWindow, type RowWindow } from '$lib/rows';
	import { rowScroller } from '$lib/rows.svelte';
	import { dragScroll, QUEUE_ROW_MIME } from '$lib/dnd';
	import { playback, openAddToPlaylist, toast } from '$lib/player.svelte';
	import { appearance, setAppearance } from '$lib/theme.svelte';
	import { keepQueueAnchor, type QueueScrollMemory } from '$lib/queue-history';
	import { lt } from '$lib/lt.svelte';
	import { t } from '$lib/i18n.svelte';

	let { scrollMemory }: { scrollMemory?: QueueScrollMemory } = $props();
	const historyId = $props.id();
	const reducedMotion = new MediaQuery('(prefers-reduced-motion: reduce)');

	// Guests are add-only in a session — no removing (theirs or anyone's) and no reordering. The
	// playing row can't be removed either (backend guards it too).
	const canRemove = $derived(lt.role !== 'guest');

	// --- drag to reorder ---------------------------------------------------------------------
	// Upcoming rows only: the playing track and the history stay put (the backend clamps to the
	// same range). `dropAt` is the queue index the dragged row goes *in front of*.
	let dragFrom = $state<number | null>(null);
	let dropAt = $state<number | null>(null);
	const canDrag = (i: number) => canRemove && i > playback.queue.currentIndex;

	function onDragStart(e: DragEvent, i: number) {
		if (!e.dataTransfer) return;
		// Our own type, so a card dragged in from a page (`ITEM_MIME`) can't be read as a row index.
		e.dataTransfer.setData(QUEUE_ROW_MIME, String(i));
		e.dataTransfer.effectAllowed = 'move';
		dragFrom = i;
	}

	function onDragOver(e: DragEvent, i: number) {
		if (dragFrom === null || !e.dataTransfer?.types.includes(QUEUE_ROW_MIME)) return;
		e.preventDefault(); // without this the drop never fires
		e.dataTransfer.dropEffect = 'move';
		const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
		dropAt = e.clientY < r.top + r.height / 2 ? i : i + 1;
	}

	function onDrop() {
		const to = dragFrom !== null && dropAt !== null ? moveTarget(dragFrom, dropAt) : null;
		if (to !== null) api.moveInQueue(dragFrom!, to);
		dragFrom = null;
		dropAt = null;
	}

	// Blocks in play order, cut wherever the upcoming tracks change origin (`queue.ts`).
	const view = $derived(queueBlocks(playback.queue));
	// The tail of the queue, for the one drop position no row can mark from its own top edge.
	const lastIndex = $derived(view.blocks.at(-1)?.rows.at(-1)?.i ?? -1);

	// The tracks already heard, hidden until asked for: a queue played deep into has hundreds of
	// them, and they sit above everything anyone opened the panel to look at. The untouched prefix
	// above them (`view.earlier`) is not hidden: it is bounded by the playlist and shrinks every
	// time you press previous, where history only grows.
	const showPrev = $derived(appearance.queueHistoryVisible);
	// Keep outgoing rows mounted until the collapse finishes.
	let renderHistory = $state(untrack(() => showPrev));
	let historyEl: HTMLDivElement | undefined = $state();
	let historyButton: HTMLButtonElement | null = $state(null);
	let historyAnimating = $state(false);
	let el: HTMLElement;
	let nowEl: HTMLElement | undefined = $state();
	// Only the view whose Show button was activated should move to reveal the history.
	let revealRequested = false;
	function revealedHeadingTop(scroller: HTMLElement, heading: HTMLElement, historyHeight: number) {
		const padding = parseFloat(getComputedStyle(scroller).paddingTop) || 0;
		const playingHeight = heading.nextElementSibling?.getBoundingClientRect().height ?? 0;
		const available = Math.max(0, scroller.clientHeight - heading.offsetHeight - playingHeight - padding);
		// Fit short histories in full; reserve space for Now Playing when long.
		return Math.min(historyHeight + padding, scroller.clientHeight * 2 / 3, available);
	}
	function rememberScroll() {
		if (scrollMemory && el?.isConnected) {
			// A half-collapsed layout is not a viewport another mount can restore.
			scrollMemory.position = historyAnimating ? undefined : {
				scrollTop: el.scrollTop,
				queue: playback.queue,
				currentIndex: playback.queue.currentIndex,
				historyVisible: showPrev
			};
		}
	}

	// A tab switch recreates this list. The owner can retain its viewport for the same queue;
	// otherwise a fresh queue reveals saved history alongside the current track after row heights settle.
	onMount(() => {
		const scroller = el;
		const queue = playback.queue;
		const currentIndex = queue.currentIndex;
		const historyVisible = showPrev;
		const saved = scrollMemory?.position;
		const restore = saved?.queue === queue && saved.currentIndex === currentIndex
			&& saved.historyVisible === historyVisible;
		const land = () => {
			if (!nowEl || playback.queue !== queue || queue.currentIndex !== currentIndex
				|| showPrev !== historyVisible) return;
			if (restore) scroller.scrollTop = saved.scrollTop;
			else {
				const offset = historyVisible && historyEl && view.prev.length
					? revealedHeadingTop(scroller, nowEl, historyEl.getBoundingClientRect().height) : 0;
				scroller.scrollTop += nowEl.getBoundingClientRect().top - scroller.getBoundingClientRect().top - offset;
			}
			rememberScroll();
		};
		scroller.addEventListener('scroll', rememberScroll, { passive: true });
		land();
		let frame = requestAnimationFrame(() => (frame = requestAnimationFrame(land)));
		return () => {
			cancelAnimationFrame(frame);
			rememberScroll();
			scroller.removeEventListener('scroll', rememberScroll);
		};
	});

	// Playing a playlist queues the whole playlist, so this panel can be handed five figures of
	// rows the moment it opens, at roughly 165 KB of web-process memory each (`rows.ts`). Past a
	// couple of hundred it renders only what is near the viewport.
	//
	// Below that it is exactly what it always was, flip animation included: windowing costs the
	// reorder animation (flip measures against the viewport, so it would fight the scroll), and
	// that is a bad trade for a queue you can see the end of.
	const WINDOW_ABOVE = 200;
	const sc = rowScroller();
	// One entry per block, in render order. A collapsed history is 0 rows but still charged a
	// heading it doesn't draw. The usual history label is also outside the scroller. These shift
	// the window's *choice* of slice, not its row heights: overscan absorbs it (see HEADING_PX).
	const counts = $derived([
		view.earlier.length,
		renderHistory ? view.prev.length : 0,
		view.now ? 1 : 0,
		...view.blocks.map((b) => b.rows.length)
	]);
	const windowed = $derived(counts.reduce((a, c) => a + c, 0) > WINDOW_ABOVE);
	const wins = $derived(
		windowed
			? blockWindows(sc.scrollTop, sc.viewportPx, counts, sc.rowPx)
			: counts.map(fullWindow)
	);

	// Each mounted queue anchors its own viewport, even when another view changes the preference.
	// Initial hydration is not a toggle: onMount above owns the first positioning.
	let previousVisibility: boolean | undefined;
	$effect.pre(() => {
		const visible = showPrev;
		const reduce = reducedMotion.current;
		const queue = playback.queue;
		const currentIndex = queue.currentIndex;
		const changed = previousVisibility !== undefined && previousVisibility !== visible;
		previousVisibility = visible;
		const reveal = changed && visible && revealRequested;
		revealRequested = false;
		const heading = untrack(() => nowEl);
		const history = untrack(() => historyEl);
		// Another queue view can hide this one while a history row has keyboard focus.
		if (!visible && history?.contains(document.activeElement)) {
			untrack(() => historyButton)?.focus({ preventScroll: true });
		}
		const clearHeight = () => {
			history?.style.removeProperty('height');
			history?.style.removeProperty('overflow');
		};
		if (!changed || !el || !heading || !history || !view.prev.length) {
			// Queue/track or motion-setting changes cancel the previous animation. Settle the
			// new layout without losing the heading's intermediate viewport position.
			const scroller = el;
			const cancel = scroller && heading && history && untrack(() => historyAnimating)
				? keepQueueAnchor(scroller, heading, tick(), () => nowEl === heading
					&& playback.queue === queue && queue.currentIndex === currentIndex
					&& scroller.isConnected)
				: undefined;
			renderHistory = visible;
			historyAnimating = false;
			clearHeight();
			return cancel;
		}
		const scroller = el;
		const current = () => nowEl === heading && playback.queue === queue
			&& queue.currentIndex === currentIndex && scroller.isConnected;
		// Windowed queues already omit row motion: animating their reserved thousands of pixels
		// would invalidate the window offsets. Reduced motion uses the same instant anchoring.
		const animate = queue.items.length <= WINDOW_ABOVE && !reduce;
		if (!animate) {
			const cancel = keepQueueAnchor(scroller, heading, tick(), current, reveal
				? () => revealedHeadingTop(scroller, heading, history.getBoundingClientRect().height)
				: undefined);
			renderHistory = visible;
			historyAnimating = false;
			clearHeight();
			return cancel;
		}
		// Retain the actual intermediate height on reversal instead of restarting from an end.
		const from = history.getBoundingClientRect().height;
		const headingTop = heading.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
		history.style.height = `${from}px`;
		history.style.overflow = 'hidden';
		renderHistory = true;
		historyAnimating = true;
		let cancelled = false;
		let frame = 0;
		void tick().then(() => {
			if (cancelled || !current()) return;
			const to = visible ? history.scrollHeight : 0;
			const start = performance.now();
			const revealDistance = reveal ? revealedHeadingTop(scroller, heading, to) - headingTop : 0;
			let previousProgress = 0;
			let remainder = 0;
			const step = (time: number) => {
				if (cancelled || !current()) return;
				const progress = Math.min(1, (time - start) / 200);
				const eased = cubicOut(progress);
				const before = heading.getBoundingClientRect().top;
				history.style.height = `${from + (to - from) * eased}px`;
				// Reveal the requested history as it unfolds, instead of scrolling it offscreen.
				// Other views and collapsing history retain their own anchor. Per-frame deltas
				// also preserve any wheel movement the user makes during the transition.
				const revealStep = revealDistance * (eased - previousProgress);
				previousProgress = eased;
				const wanted = scroller.scrollTop + heading.getBoundingClientRect().top - before - revealStep + remainder;
				scroller.scrollTop = wanted;
				// Chromium can round scrollTop to pixels. Carry the fraction, not a clamped
				// distance, so several frames do not accumulate a visible drift.
				remainder = wanted > 0 && wanted < scroller.scrollHeight - scroller.clientHeight
					? wanted - scroller.scrollTop : 0;
				if (progress < 1) frame = requestAnimationFrame(step);
				else {
					renderHistory = visible;
					historyAnimating = false;
					clearHeight();
					void tick().then(() => {
						if (!cancelled && current()) rememberScroll();
					});
				}
			};
			frame = requestAnimationFrame(step);
		});
		return () => {
			cancelled = true;
			cancelAnimationFrame(frame);
		};
	});

	function togglePrev() {
		revealRequested = !appearance.queueHistoryVisible;
		try {
			setAppearance({ queueHistoryVisible: !appearance.queueHistoryVisible });
		} catch {
			// setAppearance updates the shared state before writing. Keep that in-session choice;
			// a later toggle retries persistence, and the anchoring effect still runs on failure.
			toast.error(t('player.history_not_saved'));
		}
	}
</script>

{#snippet rows(list: QueueRow[], w: RowWindow)}
	<!-- The padding stands in for the rows outside the window, so this block is exactly as tall as
	     all of its rows and every heading below it stays where it was. -->
	<div role="list" style="padding-top:{w.padTop}px;padding-bottom:{w.padBottom}px">
		{#each list.slice(w.start, w.end) as { item, key, i } (key)}
			<!-- data-row: what the scroller measures a row's real height from. -->
			<div
				data-row
				role="listitem"
				class="relative"
				animate:flip={{ duration: windowed || historyAnimating || reducedMotion.current ? 0 : 200, easing: cubicOut }}
				draggable={canDrag(i)}
				ondragstart={(e) => onDragStart(e, i)}
				ondragover={(e) => onDragOver(e, i)}
				ondrop={onDrop}
			>
				<!-- Where the drop lands: a bar across the edge of the row it goes in front of. The
				     last row also draws one below itself — nothing else can show a drop at the end. -->
				{#if dropAt === i}
					<div
						class="pointer-events-none absolute inset-x-2 top-0 z-10 h-0.5 rounded-full bg-primary"
					></div>
				{:else if dropAt === i + 1 && i === lastIndex}
					<div
						class="pointer-events-none absolute inset-x-2 bottom-0 z-10 h-0.5 rounded-full bg-primary"
					></div>
				{/if}
				<TrackRow
					song={item}
					index={i}
					active={i === playback.queue.currentIndex}
					hideRating
					onplay={() => api.playIndex(i)}
					onAdd={() => openAddToPlaylist(item)}
					onRemove={canRemove && i !== playback.queue.currentIndex
						? () => api.removeFromQueue(i)
						: undefined}
					removeLabel={t('player.remove_from_queue')}
				/>
			</div>
		{/each}
	</div>
{/snippet}

<!-- A drag cancelled with Esc, or dropped outside the list, never reaches `drop` — without this the
     bar stays painted and the next dragover thinks a drag is still in flight. -->
<svelte:window
	ondragend={() => {
		dragFrom = null;
		dropAt = null;
	}}
/>

<!-- The list on its own, so the side panel and the now-playing view's Queue tab render the same
     one instead of drifting apart. dragScroll: reordering across a queue taller than the panel
     needs the edges to pull. -->
<!-- Keep the disclosure outside the scroller so its hit target stays in place. -->
{#if view.now && view.prev.length}
	<div class="flex shrink-0 items-center justify-between gap-2 px-2 py-1">
		<h3 id="{historyId}-label" class="min-w-0 flex-1 truncate px-2 text-sm font-semibold text-muted-foreground">
			{t('player.history')}
		</h3>
		<Button
			bind:ref={historyButton}
			variant="ghost"
			size="xs"
			class="h-7 shrink-0 cursor-pointer gap-1.5 rounded-md px-2 text-muted-foreground transition-colors duration-150 hover:bg-transparent dark:hover:bg-transparent aria-expanded:bg-transparent focus-visible:ring-2 active:not-aria-[haspopup]:translate-y-0 motion-reduce:transition-none"
			aria-expanded={showPrev}
			aria-controls={historyId}
			onkeydown={(event) => {
				// Keep native Space activation on keyup; the window shortcut must not pause music.
				if (event.key === ' ') event.stopPropagation();
			}}
			onclick={togglePrev}
		>
			<HugeiconsIcon icon={HistoryIcon} class="size-3.5" />
			<!-- Reserve both labels' width so changing state never moves the icon or hit area. -->
			<span class="grid">
				<span class="col-start-1 row-start-1" class:invisible={showPrev} aria-hidden={showPrev}>
					{t('player.show_history')}
				</span>
				<span class="col-start-1 row-start-1" class:invisible={!showPrev} aria-hidden={!showPrev}>
					{t('player.hide_history')}
				</span>
			</span>
		</Button>
	</div>
{/if}
<div
	class="min-h-0 flex-1 overflow-y-auto px-2 pt-1 pb-2"
	bind:this={el}
	{@attach sc.attach}
	{@attach (node) => dragScroll(node, QUEUE_ROW_MIME)}
>
	{#if view.now}
		<!-- The queue in front of the playing track that was never reached: start an album at track
		     4 and the backend still queues 1-3. Always drawn: they are not history. -->
		{#if view.earlier.length}
			<h3 class="truncate px-2 pt-2 pb-1.5 text-sm font-semibold text-muted-foreground">
				{view.earlierHeading}
			</h3>
			{@render rows(view.earlier, wins[0])}
		{/if}
		<div
			id={historyId}
			role="group"
			aria-labelledby="{historyId}-label"
			bind:this={historyEl}
			class:small-history={playback.queue.items.length <= WINDOW_ABOVE}
			hidden={!renderHistory || !view.prev.length}
			inert={!showPrev}
			aria-hidden={!showPrev}
			data-history-transitioning={historyAnimating ? '' : undefined}
		>
			{#if renderHistory && view.prev.length}
				<!-- Keep an in-list boundary when unplayed tracks precede the played history. -->
				{#if view.earlier.length}
					<h3 class="px-2 pt-2 pb-1.5 text-sm font-semibold text-muted-foreground">
						{t('player.history')}
					</h3>
				{/if}
				{@render rows(view.prev, wins[1])}
			{/if}
		</div>
		<div bind:this={nowEl} class="flex items-center justify-between gap-2 px-2 pt-2 pb-1.5">
			<h3 class="truncate text-sm font-semibold">{t('player.now_playing')}</h3>
		</div>
		{@render rows([view.now], wins[2])}

		{#each view.blocks as block, b (block.key)}
			{#if block.autoplay}
				<div
					class="mt-3 flex items-center gap-2 border-t px-2 pt-2.5 pb-1.5 text-muted-foreground"
					title={t('player.autoplay_notice')}
				>
					<HugeiconsIcon icon={InfinityIcon} class="h-3.5 w-3.5" />
					<span class="text-xs font-medium">{t('settings.playback.autoplay')}</span>
				</div>
			{:else}
				<div class="mt-3 flex items-center justify-between gap-2 px-2 pb-1.5">
					<h3 class="truncate text-sm font-semibold">{block.heading}</h3>
					{#if block.clearable && canRemove}
						<button
							class="shrink-0 cursor-pointer text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
							onclick={() => api.clearQueued()}
						>
							{t('player.clear_queue')}
						</button>
					{/if}
				</div>
			{/if}
			{@render rows(block.rows, wins[b + 3])}
		{/each}
	{:else}
		<p class="p-4 text-sm text-muted-foreground">{t('player.empty_queue')}</p>
	{/if}
</div>

<style>
	/* This history has at most 200 rows. Measure their real layout: content-visibility's
	   estimated row heights otherwise change as the disclosure opens, leaving a jump at the end. */
	.small-history :global([data-row] > [data-ctx]) {
		content-visibility: visible;
	}
</style>
