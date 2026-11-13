<script lang="ts">
	// The whole song under the lyric editor's stage, on a page tall enough to hold it: wrapped into
	// rows like a page of text, every placed line drawn where it is sung, the current one filled,
	// one lane per singer. Clicking a line picks it.
	//
	// It is given a box and fills it: a taller box fits more rows, so each row covers less of the
	// song and its lines get room for their words. Nothing in here changes the box's size.
	//
	// docs/UI-PERFORMANCE.md: the playhead is the only thing that moves per frame, a transform on
	// one element, and only while the song plays. Sizes come from ResizeObserver bindings.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { MusicNote01Icon } from '@hugeicons/core-free-icons';
	import { t } from '$lib/i18n.svelte';
	import { lineSpan, lineText, wordsTimed, type EdLine } from '$lib/lyricsedit';

	let {
		lines,
		cur,
		durationMs,
		position,
		running,
		head,
		label,
		onselect
	}: {
		lines: EdLine[];
		cur: number;
		durationMs: number;
		/** Where the song is, in ms: read once a frame while `running`. */
		position: () => number;
		running: boolean;
		/** The song playing is this one, so the playhead means something. */
		head: boolean;
		/** A singer's name, from its `v1`, `v2`, ... */
		label: (agent: string) => string;
		onselect: (i: number) => void;
	} = $props();

	/** The shortest stretch a row may cover. */
	const MIN_ROW_MS = 10_000;

	/** One lane per singer the lines name. A song with none, or one, draws a single lane. */
	const lanes = $derived.by(() => {
		const keys = new Set<string>();
		for (const l of lines) if (!l.gap) keys.add(l.agent ?? '');
		const n = (a: string) => (a ? Number(a.slice(1)) : Infinity);
		return [...keys].sort((a, b) => n(a) - n(b));
	});
	const laneOf = (l: EdLine) => Math.max(0, lanes.indexOf(l.agent ?? ''));

	let boxH = $state(0);
	let boxW = $state(0);
	// Lanes are tighter on a duet: every pixel a lane takes is rows the song does not get.
	const minRow = $derived(lanes.length > 1 ? lanes.length * 17 + 10 : 32);
	/** As many rows as the box holds, each a whole number of 5 s so the times down the side read. */
	const rowMs = $derived.by(() => {
		const fit = Math.max(1, Math.floor(boxH / minRow));
		return Math.ceil(Math.max(MIN_ROW_MS, durationMs / fit) / 5000) * 5000;
	});
	const rows = $derived(Math.max(1, Math.ceil(durationMs / rowMs)));
	// ponytail: a short song on a very tall page leaves the rest of the box under its last row.
	const rowH = $derived(Math.min(boxH / rows, minRow * 1.6));
	const laneH = $derived((rowH - 10) / Math.max(1, lanes.length));

	/** The last row stops where the song does. */
	const lastW = $derived(((durationMs - (rows - 1) * rowMs) / rowMs) * 100);

	type Seg = { i: number; row: number; x: number; w: number; px: number; first: boolean; last: boolean };
	/** Every placed line cut at the row edges it crosses, and how many lines have no start yet. */
	const drawn = $derived.by(() => {
		const segs: Seg[] = [];
		let unplaced = 0;
		lines.forEach((l, i) => {
			let span = lineSpan(lines, i, durationMs);
			if (!span) return void unplaced++;
			if (l.gap) {
				// A break lasts until the next line starts, however long the solo.
				const s = span.start;
				const next = lines.slice(i + 1).find((n) => (n.time ?? n.words[0]?.start ?? -1) > s);
				span = { start: s, end: next?.time ?? next?.words[0]?.start ?? durationMs };
			}
			for (let r = Math.floor(span.start / rowMs); r < rows && r * rowMs < span.end; r++) {
				const a = Math.max(span.start, r * rowMs);
				const b = Math.min(span.end, (r + 1) * rowMs);
				segs.push({
					i,
					row: r,
					x: ((a - r * rowMs) / rowMs) * 100,
					w: ((b - a) / rowMs) * 100,
					px: ((b - a) / rowMs) * boxW,
					first: a === span.start,
					last: b === span.end
				});
			}
		});
		return { segs, unplaced };
	});

	const clock = (ms: number) => {
		const s = Math.round(ms / 1000);
		return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
	};

	let headEl = $state<HTMLElement | null>(null);
	function place(ms: number) {
		if (!headEl) return;
		const r = Math.min(rows - 1, Math.max(0, Math.floor(ms / rowMs)));
		const x = Math.min(100, Math.max(0, ((ms - r * rowMs) / rowMs) * 100));
		// Its own size is one row, so a percentage of itself is a row down.
		headEl.style.transform = `translate(${x}%, ${r * 100}%)`;
	}
	$effect(() => {
		if (!headEl) return;
		void rowMs;
		void rows;
		place(position());
		if (!running) return;
		let frame = requestAnimationFrame(function tick() {
			place(position());
			frame = requestAnimationFrame(tick);
		});
		return () => cancelAnimationFrame(frame);
	});
</script>

<div class="flex h-full flex-col">
	<div class="mb-1.5 flex flex-wrap items-center gap-x-4 gap-y-1 text-[12px] text-muted-foreground">
		<h3 class="font-heading text-sm font-semibold text-foreground">{t('lyrics_editor.map')}</h3>
		{#if lanes.length > 1}
			{#each lanes as a, k (a)}
				<span class="flex items-center gap-1.5">
					<!-- Which lane of a row, drawn: one bar per singer, this one's lit. -->
					<span class="flex h-3 w-2.5 flex-col justify-between" aria-hidden="true">
						{#each lanes as b, j (b)}<i class="h-[3px] rounded-full {j === k ? 'bg-foreground' : 'bg-foreground/25'}"></i>{/each}
					</span>
					{a ? label(a) : t('lyrics_editor.singer_none')}
				</span>
			{/each}
		{/if}
		{#if drawn.unplaced}
			<span class="ml-auto">
				{drawn.unplaced === 1 ? t('lyrics_editor.map_unplaced_one') : t('lyrics_editor.map_unplaced', { count: drawn.unplaced })}
			</span>
		{/if}
	</div>
	<div class="flex min-h-0 flex-1 gap-2.5">
		<div class="relative w-8 shrink-0 text-right text-[10.5px] text-muted-foreground tabular-nums" aria-hidden="true">
			{#each { length: rows }, r (r)}
				<span class="absolute right-0 -translate-y-1/2" style="top:{(r + 0.5) * rowH}px">{clock(r * rowMs)}</span>
			{/each}
		</div>
		<div class="relative min-w-0 flex-1 select-none" bind:clientHeight={boxH} bind:clientWidth={boxW}>
			{#each { length: rows }, r (r)}
				<div class="absolute left-0 rounded-md bg-muted/45" style="top:{r * rowH + 2}px;height:{rowH - 4}px;width:{r === rows - 1 ? lastW : 100}%"></div>
			{/each}
			{#each drawn.segs as s (`${s.i}:${s.row}`)}
				{@const l = lines[s.i]}
				{#if l.gap}
					<button
						type="button"
						tabindex="-1"
						class="lead-pad absolute flex cursor-pointer items-center justify-center rounded-[5px] {s.i === cur
							? 'text-foreground outline-2 outline-primary outline-solid'
							: 'text-muted-foreground hover:text-foreground'}"
						style="left:{s.x}%;width:{s.w}%;top:{s.row * rowH + 5}px;height:{rowH - 10}px"
						title="{s.i + 1}. {t('lyrics_editor.gap')}"
						onclick={() => onselect(s.i)}
					>
						{#if s.first && s.px >= 22}<HugeiconsIcon icon={MusicNote01Icon} class="size-3.5" />{/if}
					</button>
				{:else}
					<button
						type="button"
						tabindex="-1"
						class="absolute flex min-w-[3px] cursor-pointer items-center overflow-hidden rounded-[5px] border text-left {s.first
							? ''
							: 'rounded-l-none border-l-0'} {s.last ? '' : 'rounded-r-none border-r-0'} text-[11.5px] leading-none font-medium {s.i === cur
							? 'border-primary bg-primary text-primary-foreground'
							: wordsTimed(l)
								? 'border-primary/55 bg-primary/20 text-foreground hover:bg-primary/35'
								: 'border-dashed border-primary/55 text-muted-foreground hover:bg-primary/15'}"
						style="left:{s.x}%;width:{s.w}%;top:{s.row * rowH + 5 + laneOf(l) * laneH}px;height:{laneH - 2}px"
						title="{s.i + 1}. {lineText(l)}"
						onclick={() => onselect(s.i)}
					>
						<!-- Clipped, not ellipsed, as on the word track: a cut word says more than "...". -->
						{#if s.first && s.px >= 28}<span class="min-w-0 overflow-hidden px-1.5 whitespace-nowrap">{lineText(l)}</span>{/if}
					</button>
				{/if}
			{/each}
			{#if head}
				<div
					bind:this={headEl}
					aria-hidden="true"
					class="pointer-events-none absolute top-0 left-0 w-full border-l-2 border-foreground"
					style="height:{rowH}px"
				></div>
			{/if}
		</div>
	</div>
</div>

<style>
	.lead-pad {
		background: repeating-linear-gradient(135deg, transparent 0 5px, var(--border) 5px 6px);
	}
</style>
