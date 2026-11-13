<script lang="ts">
	import { fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { MediaQuery } from 'svelte/reactivity';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { MusicNote01Icon } from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import { playback, ui } from '$lib/player.svelte';
	import { showVideo } from '$lib/video.svelte';
	import { lyricsFor, nowLyrics, setNowLyrics } from '$lib/nowlyrics.svelte';
	import { t } from '$lib/i18n.svelte';

	// The line being sung, under the now-playing cover (the Android app's CoverLyricLine). Two lines
	// tall whatever line it holds, so the title under it never moves when a line ends. A one-line
	// lyric sits in the middle of it, so its gaps to the cover and to the title match. Word-synced
	// lines light as they are sung; line-synced ones show whole. The sizes are in `em` off the font
	// size set here, which NowPlaying's title budget (`ART_SIDE`) counts. Clicking it opens the lyrics.
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
	// Background vocals stay in the lyrics tab: here they'd read as part of the lead line.
	const words = $derived(line?.words?.filter((w) => !w.bg) ?? []);
	// A change of key is a new line rising in. Before the first line and in an instrumental break the
	// slot holds a note and how much of the break is left, so an intro doesn't read as a song without
	// lyrics, or as lyrics still loading.
	const key = $derived(!lines ? '' : text ? `${nowLyrics.id}:${at}` : 'wait');
	/** The break being waited through: from its cue (the song's start, for an intro) to the next
	 *  sung line's. None after the last line. */
	const gap = $derived.by(() => {
		if (key !== 'wait' || !lines) return null;
		const from = (at < 0 ? 0 : lines[at].time_ms) ?? 0;
		const to = lines.slice(at + 1).find((l) => l.time_ms !== undefined && l.text.trim())?.time_ms;
		return to !== undefined && to > from ? { from, to } : null;
	});
	const fill = $derived(gap ? Math.min(1, Math.max(0, (ms - gap.from) / (gap.to - gap.from))) : 0);

	const still = new MediaQuery('prefers-reduced-motion: reduce');
	// Long enough to read as travel, short enough that fast lines don't queue (Android's 340 ms).
	const dur = $derived(still.current ? 0 : 340);

	/** How far the light has got across a word, as the gradient's stop. Feathered: the stops run
	 *  from -12% to 112%, so a word not yet reached is fully dim and a sung one fully lit. */
	function stop(w: api.LyricWord): number {
		const span = w.end_ms - w.start_ms;
		const p = span > 0 ? (ms - w.start_ms) / span : ms >= w.start_ms ? 1 : 0;
		return Math.min(1, Math.max(0, p)) * 124 - 12;
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
				in:fly={{ y: '35%', duration: dur, easing: cubicOut }}
				out:fly={{ y: '-35%', duration: dur, easing: cubicOut }}
				class="[grid-area:1/1] min-w-0 self-center"
				dir="auto"
			>
				{#if key === 'wait'}
					<!-- The note the lyrics tab draws for a break, and a bar that fills until the
					     singing comes back. Without the frame clock (line-synced lyrics) the bar gets a
					     position four times a second, and a linear glide of the same length joins them
					     up; with it, every frame, and no glide. Transform only. -->
					<span class="flex h-[1.35em] items-center gap-[0.5em] text-muted-foreground">
						<HugeiconsIcon icon={MusicNote01Icon} class="size-[1.05em] shrink-0" />
						<span class="sr-only">{t('lyrics.instrumental')}</span>
						{#if gap}
							<span class="h-[0.2em] w-[4.5em] overflow-hidden rounded-full bg-muted-foreground/30">
								<span
									class="block h-full origin-left rounded-full bg-foreground {worded
										? ''
										: 'transition-transform duration-[250ms] ease-linear'}"
									style="transform:scaleX({fill})"
								></span>
							</span>
						{/if}
					</span>
				{:else if key}
					<span class="line-clamp-2 text-start [overflow-wrap:anywhere]">
						{#if words.length}
							<!-- Each word is its own gradient, lit to where the singer has got to. Theme
							     tokens both, opaque: a translucent colour composites glyph by glyph. -->
							{#each words as w, i (i)}<span
									class="bg-clip-text text-transparent [-webkit-text-fill-color:transparent]"
									style="background-image:linear-gradient(90deg,var(--foreground) {stop(
										w
									)}%,var(--muted-foreground) {stop(w) + 12}%)">{w.text.trimEnd()}</span
								>{w.text.endsWith(' ') ? ' ' : ''}{/each}
						{:else}
							<span class="text-foreground">{text}</span>
						{/if}
					</span>
				{/if}
			</span>
		{/key}
	</button>
</div>
