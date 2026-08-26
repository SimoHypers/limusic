<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { flip } from 'svelte/animate';
	import { cubicOut } from 'svelte/easing';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { HistoryIcon, InfinityIcon } from '@hugeicons/core-free-icons';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import * as api from '$lib/api';
	import { queueBlocks, moveTarget, type QueueRow } from '$lib/queue';
	import { blockWindows, fullWindow, type RowWindow } from '$lib/rows';
	import { rowScroller } from '$lib/rows.svelte';
	import { dragScroll, QUEUE_ROW_MIME } from '$lib/dnd';
	import { playback, openAddToPlaylist } from '$lib/player.svelte';
	import { lt } from '$lib/lt.svelte';
	import { t } from '$lib/i18n.svelte';

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
	let showPrev = $state(false);
	let el: HTMLElement;
	let nowEl: HTMLElement | undefined = $state();

	// Open on the playing track. Everything in front of it is drawn above, so a queue opened three
	// thousand tracks into Liked Songs would otherwise open on track 1.
	//
	// Measured off the heading rather than computed from row heights, because the run above reserves
	// `rows × rowPx` and `rowPx` starts at the assumed 56 before settling to the panel's real 72 a
	// frame later (`rows.svelte.ts`), which moves the heading down by a quarter of the run. So land,
	// then land again once it has settled.
	onMount(() => {
		const land = () => {
			if (!nowEl) return;
			el.scrollTop += nowEl.getBoundingClientRect().top - el.getBoundingClientRect().top;
		};
		land();
		let frame = requestAnimationFrame(() => (frame = requestAnimationFrame(land)));
		return () => cancelAnimationFrame(frame);
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
	// heading it doesn't draw, which shifts every window's *choice* of slice by 40px and none of
	// their heights: the overscan swallows it (see HEADING_PX).
	const counts = $derived([
		view.earlier.length,
		showPrev ? view.prev.length : 0,
		view.now ? 1 : 0,
		...view.blocks.map((b) => b.rows.length)
	]);
	const windowed = $derived(counts.reduce((a, c) => a + c, 0) > WINDOW_ABOVE);
	const wins = $derived(
		windowed
			? blockWindows(sc.scrollTop, sc.viewportPx, counts, sc.rowPx)
			: counts.map(fullWindow)
	);

	async function togglePrev() {
		const before = el.scrollHeight;
		showPrev = !showPrev;
		await tick();
		// Rows appear (or vanish) above the viewport, and WebKit implements no scroll anchoring, so
		// without this the panel jumps by the whole height of the history. Keeps Now playing still.
		el.scrollTop += el.scrollHeight - before;
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
				animate:flip={{ duration: windowed ? 0 : 200, easing: cubicOut }}
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
<div
	class="min-h-0 flex-1 overflow-y-auto p-2"
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
		{#if showPrev && view.prev.length}
			<h3 class="px-2 pt-2 pb-1.5 text-sm font-semibold text-muted-foreground">
				{t('player.history')}
			</h3>
			{@render rows(view.prev, wins[1])}
		{/if}
		<div bind:this={nowEl} class="flex items-center justify-between gap-2 px-2 pt-2 pb-1.5">
			<h3 class="truncate text-sm font-semibold">{t('player.now_playing')}</h3>
			{#if view.prev.length}
				<button
					class="flex shrink-0 cursor-pointer items-center gap-1.5 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
					onclick={togglePrev}
				>
					<HugeiconsIcon icon={HistoryIcon} class="h-3.5 w-3.5" />
					{showPrev ? t('player.hide_history') : t('player.show_history')}
				</button>
			{/if}
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
