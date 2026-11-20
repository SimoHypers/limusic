<script lang="ts">
	// One line of the lyric editor, drawn twice: large, as it is sung (which word the next tap lands
	// on, which are done, which are background vocals), and as a track under it with a ruler, a block
	// per timed word whose edges drag, and a mark where each tap of the pass in progress fell. A line
	// with background vocals gets a second lane under the lead's: they are timed on their own and
	// overlap it. Every rule is in `lyricsedit.ts`; this draws and reports.
	//
	// docs/UI-PERFORMANCE.md: the only thing that moves per frame is the playhead, a transform set
	// straight on the element. The blocks change `left`/`width` only while one is being dragged. The
	// one animation is a colour fade on the single word a tap just landed on.
	import { untrack, type Snippet } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowLeft02Icon, ArrowRight02Icon } from '@hugeicons/core-free-icons';
	import { t } from '$lib/i18n.svelte';
	import { LEAD_IN_MS, LEAD_OUT_MS, moveEdge, type EdWord, type Range } from '$lib/lyricsedit';

	let {
		words,
		readings,
		range,
		durationMs,
		running,
		position,
		nextTap,
		tapped,
		pass,
		marks,
		bgMode,
		compact = false,
		gap = false,
		editor,
		between,
		onwords,
		onbg,
		onseek
	}: {
		words: EdWord[];
		/** Each word's romanization, drawn under it (#676). */
		readings?: (string | null)[];
		/** The stretch of song the track shows: the line's loop. `null` before the line has a start. */
		range: Range | null;
		durationMs: number;
		/** The song is playing this line right now, so the playhead moves. */
		running: boolean;
		/** Where the song is, in ms: read once a frame while `running`. */
		position: () => number;
		/** The word the next tap lands on, or -1 outside a pass. */
		nextTap: number;
		/** Taps made in the pass in progress: the first `tapped` words of `pass` have landed. */
		tapped: number;
		/** The words the pass times, as indices: one voice of the line (`passOf`). */
		pass: number[];
		/** Where those taps fell, in song ms. */
		marks: number[];
		/** Clicking a word flags it as background vocals instead of seeking to it. */
		bgMode: boolean;
		/** A narrow page: the big line is set smaller. */
		compact?: boolean;
		/** An instrumental break: a start and no words, so the track never fills. */
		gap?: boolean;
		/** Drawn in place of the big line: the text field, while the line's text is edited. */
		editor?: Snippet;
		/** Drawn between the big line and the track: the line that follows. */
		between?: Snippet;
		onwords: (words: EdWord[]) => void;
		onbg: (k: number) => void;
		onseek: (ms: number) => void;
	} = $props();

	const timed = $derived(
		words.length > 0 && words.every((w) => w.start !== undefined && w.end !== undefined)
	);
	/** The lead and the background each get a lane. */
	const lanes = $derived(words.some((w) => w.bg) && words.some((w) => !w.bg));
	const bgCount = $derived(words.filter((w) => w.bg).length);
	/** The pass is the background's: its marks go in that lane. */
	const passBg = $derived(lanes && !!words[pass[0]]?.bg);
	/** Where a block sits: the whole track, or its voice's lane. */
	const lane = (bg: boolean) => (!lanes ? 'inset-y-[5px]' : bg ? 'bottom-[5px] h-[28px]' : 'top-[5px] h-[34px]');

	// A drag on the first or last word moves the line's own extent, and with it `range`. Rescaling
	// the track under the pointer would move the edge away from it, so the scale holds for the drag.
	let held = $state<Range | null>(null);
	/** How far the track is slid off the loop, in ms, to see song around it: a long line's word can
	 *  then be dragged past where the loop ends. Kept per loop: an edge drag or a tap that changes the
	 *  loop brings the track back over it. */
	let panned = $state<{ at: string; ms: number } | null>(null);
	const loopAt = $derived(range ? `${range.start}:${range.end}` : '');
	const pan = $derived(panned?.at === loopAt ? panned.ms : 0);
	const view = $derived(held ?? (range && pan ? { start: range.start + pan, end: range.end + pan } : range));
	const pct = (ms: number) => (view ? ((ms - view.start) / (view.end - view.start)) * 100 : 0);

	/** A long line gets a track wider than the page, scrolled sideways, so a word stays grabbable.
	 *  ponytail: a fixed width per word, not per second; scale by duration if short words crowd. */
	const minWidth = $derived(Math.max(words.length - bgCount, bgCount) * 56);

	let strip = $state<HTMLElement | null>(null);
	let scroller = $state<HTMLElement | null>(null);
	let head = $state<HTMLElement | null>(null);
	// Both from ResizeObserver, so following the playhead never asks for a layout.
	let stripW = $state(0);
	let scrollerW = $state(0);
	let scrollLeft = 0;
	let hovering = false;
	/** The last word of the lead, and of the background, the playhead has reached, for the big line.
	 *  Set only when they change. */
	let upTo = $state(-1);
	let upBg = $state(-1);

	function place(ms: number) {
		const raw = pct(ms);
		const x = Math.min(100, Math.max(0, raw));
		if (head) {
			head.style.transform = `translateX(${x}%)`;
			// Slid away from the loop, the song can be off the track: no playhead pinned to an end.
			head.style.visibility = raw === x ? '' : 'hidden';
		}
		// A track wider than its window follows the playhead, a windowful at a time, unless the
		// pointer is on it: it must not move under a hand reaching for a grip.
		if (scroller && stripW > scrollerW + 1 && !drag && !hovering) {
			const px = (x / 100) * stripW;
			if (px < scrollLeft || px > scrollLeft + scrollerW - 24) scroller.scrollLeft = Math.max(0, px - 48);
		}
		let k = -1;
		let b = -1;
		for (let i = 0; i < words.length; i++) {
			const s = words[i].start;
			if (s !== undefined && ms >= s) {
				if (words[i].bg) b = i;
				else k = i;
			}
		}
		// Untracked: the effect below writes these and must not also wait on them.
		if (k !== untrack(() => upTo)) upTo = k;
		if (b !== untrack(() => upBg)) upBg = b;
	}

	$effect(() => {
		if (!view) {
			upTo = upBg = -1;
			return;
		}
		void words;
		place(position());
		if (!running) return;
		let frame = requestAnimationFrame(function tick() {
			place(position());
			frame = requestAnimationFrame(tick);
		});
		return () => cancelAnimationFrame(frame);
	});

	let drag: { k: number; side: 'start' | 'end'; left: number; width: number } | null = null;
	/** The edge last focused or dragged: the line under the track says where it is. */
	let edge = $state<{ k: number; side: 'start' | 'end'; keys: boolean } | null>(null);
	$effect(() => {
		// Another line, or a different number of words: the edge is gone.
		if (edge && !words[edge.k]) edge = null;
	});

	function grab(e: PointerEvent, k: number, side: 'start' | 'end') {
		if (!strip || !view || e.button !== 0) return;
		// Read on demand, once per drag.
		const r = strip.getBoundingClientRect();
		drag = { k, side, left: r.left, width: r.width };
		held = view;
		edge = { k, side, keys: false };
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
		e.preventDefault();
	}

	function pull(e: PointerEvent) {
		if (!drag || !held || drag.width <= 0) return;
		const ms = held.start + ((e.clientX - drag.left) / drag.width) * (held.end - held.start);
		onwords(moveEdge(words, drag.k, drag.side, ms, durationMs));
	}

	function drop() {
		drag = null;
		held = null;
	}

	// Sliding the track: a drag anywhere on it but a grip, with any button (a grip takes the left
	// one first). Read off the drag's start, so the song under the pointer stays under it.
	let sliding: { x: number; ms: number } | null = null;
	let grabbing = $state(false);

	function slideFrom(e: PointerEvent) {
		if (!range || e.defaultPrevented || stripW <= 0) return;
		sliding = { x: e.clientX, ms: pan };
		grabbing = true;
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
		e.preventDefault();
	}

	function slide(e: PointerEvent) {
		if (!sliding || !range) return;
		const ms = sliding.ms - ((e.clientX - sliding.x) / stripW) * (range.end - range.start);
		// Not before the song starts, nor past its end.
		const lo = Math.min(0, -range.start);
		const hi = durationMs > 0 ? Math.max(0, durationMs - range.end) : Infinity;
		panned = { at: loopAt, ms: Math.min(hi, Math.max(lo, ms)) };
	}

	function slideEnd() {
		sliding = null;
		grabbing = false;
	}

	/** Arrow keys on a focused edge: 10 ms a press, 50 with Shift. */
	function nudge(e: KeyboardEvent, k: number, side: 'start' | 'end') {
		if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
		const by = (e.key === 'ArrowLeft' ? -1 : 1) * (e.shiftKey ? 50 : 10);
		const at = words[k][side];
		if (at === undefined) return;
		onwords(moveEdge(words, k, side, at + by, durationMs));
		e.preventDefault();
		e.stopPropagation();
	}

	const label = (w: EdWord) => w.text.trim();
	/** "0:16.63" */
	function clock(ms: number): string {
		const cs = Math.round(ms / 10);
		return `${Math.floor(cs / 6000)}:${String(Math.floor(cs / 100) % 60).padStart(2, '0')}.${String(cs % 100).padStart(2, '0')}`;
	}

	/** A tick every half second, a label on the whole ones. */
	const ticks = $derived.by(() => {
		if (!view) return [];
		const out: { x: number; label: string | null }[] = [];
		for (let ms = Math.ceil(view.start / 500) * 500; ms <= view.end; ms += 500) {
			const s = ms / 1000;
			out.push({
				x: pct(ms),
				label: ms % 1000 === 0 ? `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}` : null
			});
		}
		return out;
	});

	/** The loop's lead-in and lead-out: song that plays around the line, where no word can be. */
	const pads = $derived.by(() => {
		if (!view) return null;
		const a = timed ? Math.min(...words.map((w) => w.start!)) : view.start + LEAD_IN_MS;
		const b = timed ? Math.max(...words.map((w) => w.end!)) : view.end - LEAD_OUT_MS;
		return { left: Math.min(100, Math.max(0, pct(a))), right: Math.min(100, Math.max(0, 100 - pct(b))) };
	});

	const KEY =
		'inline-flex h-[22px] min-w-[24px] items-center justify-center rounded-[6px] border border-b-[3px] bg-muted px-1.5 text-[11px] font-semibold text-foreground';
</script>

{#if editor}
	{@render editor()}
{:else}
	<!-- The line. Big enough to read at a glance while the ears are on the song. -->
	<div
		class="-mx-1.5 flex flex-wrap items-baseline gap-y-1 font-heading leading-[1.22] font-bold tracking-[-0.015em] {compact
			? 'text-[1.75rem]'
			: 'text-[2.375rem]'}"
	>
		{#each words as w, k (k)}
			{@const p = tapped > 0 ? pass.indexOf(k) : -1}
			<button
				type="button"
				onclick={() => (bgMode ? onbg(k) : w.start !== undefined && onseek(w.start))}
				aria-pressed={bgMode ? !!w.bg : undefined}
				title={bgMode
					? t('lyrics_editor.bg_word_hint')
					: w.start !== undefined
						? t('lyrics_editor.word_seek_hint')
						: undefined}
				class="cursor-pointer rounded-[9px] px-1.5 py-px {w.bg ? 'text-[0.72em] italic' : ''} {bgMode
					? w.bg
						? 'bg-muted text-foreground outline-[1.5px] -outline-offset-2 outline-foreground outline-solid'
						: 'text-foreground outline-[1.5px] -outline-offset-2 outline-border outline-dashed hover:bg-muted'
					: k === nextTap
						? 'bg-primary text-primary-foreground'
						: p >= 0
							? p < tapped
								? `text-foreground ${p === tapped - 1 ? 'word-landed' : ''}`
								: 'text-muted-foreground'
							: w.start !== undefined && !(running && k > (w.bg ? upBg : upTo))
								? 'text-foreground'
								: 'text-muted-foreground'}"
			>
				{label(w)}
				{#if readings?.[k]}
					<!-- Under the word, inside the button: part of what gets tapped, in the word's own
					     colour, so it reads on the next tap's fill too. -->
					<span class="block text-center text-[0.42em] leading-tight font-medium tracking-normal opacity-75">{readings[k]}</span>
				{/if}
			</button>
		{/each}
	</div>
{/if}

{@render between?.()}

<div class="mt-4">
	{#if view}
		<div
			bind:this={scroller}
			bind:clientWidth={scrollerW}
			role="group"
			class="overflow-x-auto"
			onscroll={(e) => (scrollLeft = e.currentTarget.scrollLeft)}
			onpointerenter={() => (hovering = true)}
			onpointerleave={() => (hovering = false)}
		>
			<!-- `touch-none` so a drag on a touch screen is not taken for a scroll. -->
			<div
				bind:this={strip}
				bind:clientWidth={stripW}
				role="presentation"
				class="touch-none {grabbing ? 'cursor-grabbing' : 'cursor-grab'}"
				style="min-width:{minWidth}px"
				onpointerdown={slideFrom}
				onpointermove={slide}
				onpointerup={slideEnd}
				onpointercancel={slideEnd}
			>
				<div class="relative h-[18px] text-[10.5px] text-muted-foreground tabular-nums" aria-hidden="true">
					{#each ticks as tk (tk.x)}
						<i
							class="absolute bottom-0 w-px {tk.label ? 'h-[9px] bg-muted-foreground' : 'h-[5px] bg-border'}"
							style="left:{tk.x}%"
						></i>
						{#if tk.label}<span class="absolute -top-0.5 translate-x-1" style="left:{tk.x}%">{tk.label}</span>{/if}
					{/each}
				</div>
								<div class="relative {lanes ? 'h-[76px]' : 'h-[52px]'} overflow-hidden rounded-[10px] border bg-muted/45 select-none">
					{#if pads}
						<div class="lead-pad absolute inset-y-0 left-0" style="width:{pads.left}%"></div>
						<div class="lead-pad absolute inset-y-0 right-0" style="width:{pads.right}%"></div>
					{/if}
					<!-- Each timed word: one voice can be timed while the other waits for its pass. -->
					{#each words as w, k (k)}
						{#if w.start !== undefined && w.end !== undefined}
							{@const left = pct(w.start!)}
							{@const width = Math.max(0.2, pct(w.end!) - left)}
							<div
								class="absolute {lane(!!w.bg)} flex items-center justify-center overflow-hidden rounded-[7px] border text-xs font-semibold {w.bg
									? 'border-dashed border-foreground/40 bg-foreground/5 text-muted-foreground italic'
									: 'border-primary/55 bg-primary/20 text-foreground'} {edge?.k === k
									? 'outline-2 outline-offset-1 outline-foreground outline-solid'
									: ''}"
								style="left:{left}%;width:{width}%"
								title={label(w)}
							>
								<!-- Clipped, not ellipsed: in a narrow block "Leav" says more than "L...". -->
								<span class="min-w-0 overflow-hidden px-2 text-[11.5px] whitespace-nowrap">{label(w)}</span>
								{#each ['start', 'end'] as const as side (side)}
									<div
										role="slider"
										tabindex="0"
										aria-orientation="horizontal"
										aria-label={t(side === 'start' ? 'lyrics_editor.edge_start' : 'lyrics_editor.edge_end', {
											word: label(w)
										})}
										aria-valuenow={w[side]}
										aria-valuemin={0}
										aria-valuemax={Math.round(durationMs) || undefined}
										aria-valuetext={t('lyrics_editor.ms', { value: w[side] ?? 0 })}
										class="group/edge absolute inset-y-0 w-2.5 cursor-ew-resize outline-none {side === 'start'
											? 'left-0'
											: 'right-0'}"
										onpointerdown={(e) => grab(e, k, side)}
										onpointermove={pull}
										onpointerup={drop}
										onpointercancel={drop}
										onkeydown={(e) => nudge(e, k, side)}
										onfocus={() => (edge = { k, side, keys: true })}
										onblur={() => edge?.keys && (edge = { ...edge, keys: false })}
									>
										<!-- The grip: what says "this edge drags". -->
										<span
											class="absolute inset-y-[9px] w-[3px] rounded-full bg-foreground/50 group-hover/edge:bg-foreground group-focus-visible/edge:bg-foreground {side ===
											'start'
												? 'left-[3px]'
												: 'right-[3px]'}"
										></span>
									</div>
								{/each}
							</div>
						{/if}
					{/each}
					{#each marks as m, i (i)}
						<i class="absolute {lanes ? (passBg ? 'bottom-2 h-[24px]' : 'top-2 h-[30px]') : 'inset-y-2'} w-[3px] rounded-full bg-primary" style="left:{pct(m)}%"></i>
					{/each}
					<!-- The playhead: as wide as the track and moved by a percentage of itself, so placing
					     it never reads the track's width. Only its left border and its cap show. -->
					<div
						bind:this={head}
						aria-hidden="true"
						class="pointer-events-none absolute inset-0 border-l-2 border-foreground before:absolute before:top-0 before:-left-[6px] before:border-[5px] before:border-transparent before:border-t-foreground"
					></div>
				</div>
			</div>
		</div>
	{:else}
		<div class="mt-[18px] flex h-[52px] items-center justify-center rounded-[10px] border border-dashed px-3 text-center text-[13px] text-muted-foreground">
			{t(gap ? 'lyrics_editor.strip_gap_no_start' : 'lyrics_editor.strip_no_start')}
		</div>
	{/if}
	<div class="mt-2 flex min-h-[26px] flex-wrap items-center gap-x-3.5 gap-y-1.5 text-[13px] text-muted-foreground">
		{#if bgMode}
			<span>{t('lyrics_editor.bg_hint')}</span>
		{:else if edge && timed && words[edge.k]?.[edge.side] !== undefined}
			<span class="font-medium text-foreground tabular-nums">
				{t(edge.side === 'start' ? 'lyrics_editor.edge_at_start' : 'lyrics_editor.edge_at_end', {
					word: label(words[edge.k]),
					time: clock(words[edge.k][edge.side]!)
				})}
			</span>
			{#if edge.keys}
				<span class="flex items-center gap-1"><kbd class={KEY}><HugeiconsIcon icon={ArrowLeft02Icon} strokeWidth={2.25} class="size-3.5" /></kbd><kbd class={KEY}><HugeiconsIcon icon={ArrowRight02Icon} strokeWidth={2.25} class="size-3.5" /></kbd> {t('lyrics_editor.ms', { value: 10 })}</span>
				<span class="flex items-center gap-1"><kbd class={KEY}>Shift</kbd> {t('lyrics_editor.ms', { value: 50 })}</span>
			{/if}
		{:else if timed}
			<span>{t('lyrics_editor.drag_hint')}</span>
		{:else if view}
			<span>{t(gap ? 'lyrics_editor.strip_gap' : 'lyrics_editor.strip_empty')}</span>
		{/if}
	</div>
</div>

<style>
	/* The colour a tapped word arrives in, fading to the text colour: one element, colour only. */
	.word-landed {
		animation: word-landed 0.35s ease-out;
	}
	@keyframes word-landed {
		from {
			background: var(--primary);
			color: var(--primary-foreground);
		}
	}
	.lead-pad {
		background: repeating-linear-gradient(135deg, transparent 0 5px, var(--border) 5px 6px);
	}
</style>
