<script lang="ts">
	import { cubicOut, quintOut } from 'svelte/easing';
	import { MediaQuery } from 'svelte/reactivity';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { MusicNote01Icon } from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import { playback, ui } from '$lib/player.svelte';
	import { showVideo } from '$lib/video.svelte';
	import { lyricsFor, nowLyrics, setNowLyrics } from '$lib/nowlyrics.svelte';
	import { t } from '$lib/i18n.svelte';
	import { glowing, letters, sweep } from '$lib/lyricmotion';
	import { appearance } from '$lib/theme.svelte';

	// The line being sung, under the now-playing cover (the Android app's CoverLyricLine). Two lines
	// tall whatever line it holds, so the title under it never moves when a line ends. A one-line
	// lyric sits in the middle of it, so its gaps to the cover and to the title match. Word-synced
	// lines light as they are sung; line-synced ones show whole. Its own Romanized setting
	// (`coverLyricRomanized`) swaps the line for its reading: two lines hold one script, not both.
	// The sizes are in `em` off the font size set here, which NowPlaying's title budget (`ART_SIDE`)
	// counts. Clicking it opens the lyrics.
	let { onopen }: { onopen: () => void } = $props();

	// Asked only when nothing in this window has the song's lyrics yet. The lyrics tab asks too when
	// it is open, and `lyricsFor` makes the two one request.
	$effect(() => {
		const now = playback.now;
		if (!now || nowLyrics.id === now.videoId) return;
		lyricsFor(now).then((l) => setNowLyrics(now.videoId, l), () => {});
	});
	// The cache brought the song's lyrics up to date behind the ones on screen (`refresh_stale`).
	$effect(() => {
		const sub = api.onLyricsUpdated((id) => {
			const now = playback.now;
			if (now?.videoId === id) lyricsFor(now).then((l) => setNowLyrics(id, l), () => {});
		});
		return () => void sub.then((u) => u());
	});

	const lyrics = $derived(nowLyrics.id === playback.now?.videoId ? nowLyrics.lyrics : null);
	const lines = $derived(lyrics?.synced && !lyrics.instrumental ? lyrics.lines : null);
	// The frame clock is for the sweep and nothing else (docs/UI-PERFORMANCE.md), and theater mode
	// covers this view.
	const worded = $derived(!!lines?.some((l) => l.words?.length));

	// mpv's position arrives ~4x a second; run a local clock on from each tick so the sweep moves
	// every frame, at the song's speed. Same clock as LyricsView's.
	let pos = $state(playback.position);
	$effect(() => {
		const base = playback.position;
		pos = base;
		if (playback.paused || !worded || ui.theaterOpen) return;
		const at = performance.now();
		const speed = playback.speed;
		let frame = requestAnimationFrame(function tick() {
			pos = base + ((performance.now() - at) / 1000) * speed;
			frame = requestAnimationFrame(tick);
		});
		return () => cancelAnimationFrame(frame);
	});
	const ms = $derived(pos * 1000 - (lyrics?.offset_ms ?? 0));

	// Last line whose cue has passed (lines arrive sorted), held until the next one starts.
	const at = $derived.by(() => {
		let i = -1;
		for (const [j, l] of (lines ?? []).entries()) {
			if (l.time_ms === undefined) continue;
			if (l.time_ms > ms) break;
			i = j;
		}
		return i;
	});
	const line = $derived(lines?.[at]);
	// A song without synced lyrics closes the slot, leaving the cover and the title their old gap.
	// Only an answer moves it: while the next song's lyrics load it stays as it was, or every change
	// between two songs with lyrics would close and reopen it.
	let wasOpen = false;
	const open = $derived.by(() => {
		if (nowLyrics.id === playback.now?.videoId) wasOpen = !!lines;
		return wasOpen;
	});
	const text = $derived(line?.text.trim().replace(/^♪$/, ''));
	const roman = $derived(appearance.coverLyricRomanized ? line?.romanized?.trim() : undefined);
	// Background vocals stay in the lyrics tab: here they'd read as part of the lead line. A line
	// that is nothing but them (an "Oh, oh" answering the lead) sweeps them, or it would show flat.
	const lead = (all: api.LyricWord[]) => {
		const sung = all.filter((w) => !w.bg);
		return sung.length ? sung : all;
	};
	const words = $derived.by(() => {
		const sung = lead(line?.words ?? []);
		if (!roman) return sung;
		if (line?.romanized_words?.length) return lead(line.romanized_words);
		// Our own reading is timed to nothing (only Apple's is). Under a word-synced line it shares
		// the line's sung span out by length, so it still sweeps; under a line-synced one it shows whole.
		if (!sung.length) return [];
		const parts = roman.split(/\s+/);
		const from = sung[0].start_ms;
		const per = (sung[sung.length - 1].end_ms - from) / parts.reduce((n, p) => n + p.length, 0);
		let at = from;
		return parts.map((p, i) => ({
			text: i < parts.length - 1 ? `${p} ` : p,
			start_ms: at,
			end_ms: (at += p.length * per)
		}));
	});
	// A change of key is a new line rising in. Before the first line and in an instrumental break the
	// slot holds a pulsing note, so an intro doesn't read as a song without lyrics, or as lyrics
	// still loading.
	const key = $derived(!lines ? '' : text ? `${nowLyrics.id}:${at}` : 'wait');

	const still = new MediaQuery('prefers-reduced-motion: reduce');
	// Long enough to read as travel, short enough that fast lines don't queue.
	const dur = $derived(still.current ? 0 : 420);

	/** The handoff between two lines (Better Lyrics' one-line stage): the old line rises away and is
	 *  gone in 220 ms, out of the way, while the new one rises in from a third of the slot below,
	 *  a beat later, and sharpens out of a slight blur as it settles. The two share the cell, so the
	 *  quick fade out is what keeps them from reading as one smear. */
	function handoff(_: Element, { out }: { out: boolean }) {
		return {
			duration: dur,
			// `t` runs 0 to 1 coming in and 1 to 0 going out; `gone` is the time spent, either way.
			css: (t: number) => {
				const gone = (out ? 1 - t : t) * dur;
				const move = quintOut(gone / dur);
				const fade = out
					? Math.min(1, gone / 220)
					: cubicOut(Math.min(1, Math.max(0, (gone - 70) / 300)));
				const shown = out ? 1 - fade : fade;
				const y = out ? -35 * move : 35 * (1 - move);
				return `translate:0 ${y}%;opacity:${shown};filter:blur(${(3 * (1 - shown)).toFixed(2)}px)`;
			}
		};
	}
</script>

<!-- Open, the padding plus the two lines; closed, the padding alone, which with the title's own
     margin is the gap the two had before this line existed. It glides between them, except under a
     video: mpv's picture on Linux cannot follow a box that moves every frame (see `holeFor`). -->
<div
	class="overflow-hidden pt-3 font-heading leading-[1.35] font-semibold {showVideo() || still.current
		? ''
		: 'transition-[height] duration-300 ease-out'}"
	style="font-size:clamp(0.9375rem, var(--side) * 0.032, 1.125rem);height:{open
		? 'calc(2.7em + 0.75rem)'
		: '0.75rem'}"
>
	<button
		type="button"
		onclick={onopen}
		disabled={!lines}
		class="grid h-[2.7em] w-full cursor-pointer text-left disabled:cursor-default"
	>
		<!-- The old line and the new share the one cell while they cross: the new one rises in from a
		     third of the slot below as the old one rises away, the way the lyrics tab scrolls. Spans, not
		     divs: a button holds phrasing content only. -->
		{#key key}
			<span
				in:handoff={{ out: false }}
				out:handoff={{ out: true }}
				class="[grid-area:1/1] min-w-0 self-center"
				dir="auto"
			>
				{#if key === 'wait'}
					<!-- The note the lyrics tab draws for a break, pulsing while the song plays and
					     held while it's paused. -->
					<span class="flex h-[1.35em] items-center text-foreground">
						<HugeiconsIcon
							icon={MusicNote01Icon}
							class="size-[1.1em] motion-safe:animate-note-pulse {playback.paused
								? '[animation-play-state:paused]'
								: ''}"
						/>
						<span class="sr-only">{t('lyrics.instrumental')}</span>
					</span>
				{:else if key}
					<span class="line-clamp-2 text-start [overflow-wrap:anywhere]">
						{#if words.length}
							<!-- Each word lights the way it does in the lyrics tab (`lyricmotion.ts`), a
							     held note letter by letter, cresting and glowing. -->
							{#each words as w, i (i)}{@const held = letters(w)}{#if held}<span
										class="relative isolate inline-block whitespace-nowrap"
										><span class="lyric-halo" aria-hidden="true"
											>{#each held as l, k (k)}<span class={glowing(l, w, ms) ? 'lit' : ''}
													>{l.text}</span
												>{/each}</span
										>{#each held as l, k (k)}{@render glyph(l, 'lyric-held')}{/each}</span
									>{:else}{@render glyph(w, '')}{/if}{w.text.endsWith(' ') ? ' ' : ''}{/each}
						{:else}
							<span class="text-foreground">{roman || text}</span>
						{/if}
					</span>
				{/if}
			</span>
		{/key}
	</button>
</div>

<!-- Theme tokens both, opaque: a translucent colour composites glyph by glyph. -->
{#snippet glyph(w: api.LyricWord, extra: string)}<span
		class="inline-block bg-clip-text text-transparent [-webkit-text-fill-color:transparent] {extra} {extra &&
		ms >= w.start_ms
			? 'sung'
			: ''}"
		style={sweep(w, ms)}>{w.text.trimEnd()}</span
	>{/snippet}
