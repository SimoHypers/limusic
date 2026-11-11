<script lang="ts">
	// One line of the lyric editor: its words as chips (which one is next to tap, which is being
	// sung, which are background vocals) and, once they are timed, a strip with a block per word
	// whose edges drag. Every rule is in `lyricsedit.ts`; this draws and reports.
	//
	// docs/UI-PERFORMANCE.md: the only thing that moves per frame is the playhead, a transform set
	// straight on the element. The blocks change `left`/`width` only while one is being dragged,
	// and nothing here transitions.
	import { t } from '$lib/i18n.svelte';
	import { moveEdge, type EdWord, type Range } from '$lib/lyricsedit';

	let {
		words,
		range,
		durationMs,
		running,
		position,
		nextTap,
		bgMode,
		onwords,
		onbg,
		onseek
	}: {
		words: EdWord[];
		/** The stretch of song the strip shows: the line's loop. */
		range: Range | null;
		durationMs: number;
		/** The song is playing this line right now, so the playhead moves. */
		running: boolean;
		/** Where the song is, in ms: read once a frame while `running`. */
		position: () => number;
		/** The word the next tap lands on, or -1 outside a pass. */
		nextTap: number;
		/** Clicking a word flags it as background vocals instead of seeking to it. */
		bgMode: boolean;
		onwords: (words: EdWord[]) => void;
		onbg: (k: number) => void;
		onseek: (ms: number) => void;
	} = $props();

	const timed = $derived(
		words.length > 0 && words.every((w) => w.start !== undefined && w.end !== undefined)
	);

	// A drag on the first or last word moves the line's own extent, and with it `range`. Rescaling
	// the strip under the pointer would move the edge away from it, so the scale holds for the drag.
	let held = $state<Range | null>(null);
	const view = $derived(held ?? range);
	const pct = (ms: number) => (view ? ((ms - view.start) / (view.end - view.start)) * 100 : 0);

	let strip = $state<HTMLElement | null>(null);
	let head = $state<HTMLElement | null>(null);
	/** The word under the playhead, for the chips. Set only when it changes, not every frame. */
	let sung = $state(-1);

	function place(ms: number) {
		if (head) head.style.transform = `translateX(${Math.min(100, Math.max(0, pct(ms)))}%)`;
		let k = -1;
		for (let i = 0; i < words.length; i++) {
			const w = words[i];
			if (w.start !== undefined && w.end !== undefined && ms >= w.start && ms < w.end) k = i;
		}
		if (k !== sung) sung = k;
	}

	$effect(() => {
		if (!timed || !view) {
			sung = -1;
			return;
		}
		place(position());
		if (!running) return;
		let frame = requestAnimationFrame(function tick() {
			place(position());
			frame = requestAnimationFrame(tick);
		});
		return () => cancelAnimationFrame(frame);
	});

	let drag: { k: number; side: 'start' | 'end'; left: number; width: number } | null = null;

	function grab(e: PointerEvent, k: number, side: 'start' | 'end') {
		if (!strip || !view || e.button !== 0) return;
		// Read on demand, once per drag.
		const r = strip.getBoundingClientRect();
		drag = { k, side, left: r.left, width: r.width };
		held = view;
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
</script>

<!-- The words. Big enough to read at a glance while the eyes are on the song. -->
<div class="flex flex-wrap items-baseline gap-y-2 font-heading text-2xl font-bold leading-snug">
	{#each words as w, k (k)}
		<button
			type="button"
			onclick={() => (bgMode ? onbg(k) : w.start !== undefined && onseek(w.start))}
			aria-pressed={bgMode ? !!w.bg : undefined}
			title={bgMode
				? t('lyrics_editor.bg_word_hint')
				: w.start !== undefined
					? t('lyrics_editor.word_seek_hint')
					: undefined}
			class="cursor-pointer rounded-md px-1 {/\s$/.test(w.text) ? 'mr-1.5' : ''} {w.bg
				? 'italic underline decoration-dashed decoration-2 underline-offset-4'
				: ''} {k === nextTap
				? 'bg-primary text-primary-foreground'
				: k === sung
					? 'text-primary'
					: w.start === undefined || (nextTap >= 0 && k > nextTap)
						? 'text-muted-foreground'
						: 'text-foreground'} {bgMode ? 'hover:bg-foreground/10' : ''}"
		>
			{label(w)}
		</button>
	{/each}
</div>

{#if timed && view}
	<!-- The strip. `touch-none` so a drag on a touch screen is not taken for a scroll. -->
	<div
		bind:this={strip}
		class="relative mt-4 h-11 touch-none overflow-hidden rounded-lg border bg-muted/40 select-none"
	>
		{#each words as w, k (k)}
			{@const left = pct(w.start!)}
			{@const width = Math.max(0.2, pct(w.end!) - left)}
			<div
				class="absolute inset-y-1 flex items-center overflow-hidden rounded-md border text-xs font-medium {w.bg
					? 'border-dashed border-foreground/40 bg-foreground/5 italic text-muted-foreground'
					: 'border-primary/40 bg-primary/15 text-foreground'}"
				style="left:{left}%;width:{width}%"
			>
				<span class="min-w-0 flex-1 truncate px-2.5 text-center">{label(w)}</span>
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
						class="absolute inset-y-0 w-2 cursor-ew-resize bg-primary/0 outline-none hover:bg-primary/60 focus-visible:bg-primary {side ===
						'start'
							? 'left-0'
							: 'right-0'}"
						onpointerdown={(e) => grab(e, k, side)}
						onpointermove={pull}
						onpointerup={drop}
						onpointercancel={drop}
						onkeydown={(e) => nudge(e, k, side)}
					></div>
				{/each}
			</div>
		{/each}
		<!-- The playhead: as wide as the strip and moved by a percentage of itself, so placing it
		     never reads the strip's width. Only its left border shows. -->
		<div
			bind:this={head}
			aria-hidden="true"
			class="pointer-events-none absolute inset-0 border-l-2 border-foreground"
		></div>
	</div>
	<p class="mt-2 text-xs text-muted-foreground">
		{t('lyrics_editor.drag_hint')}
	</p>
{/if}
