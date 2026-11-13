<script lang="ts">
	import { fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { MediaQuery } from 'svelte/reactivity';
	import * as api from '$lib/api';
	import { playback, ui } from '$lib/player.svelte';
	import { lyricsFor, nowLyrics, setNowLyrics } from '$lib/nowlyrics.svelte';

	// The line being sung, under the now-playing cover (the Android app's CoverLyricLine). Always two
	// lines tall, sung or not, so the title under it never moves when a line ends or a song has no
	// synced lyrics. A one-line lyric sits in the middle of it, so its gaps to the cover and to the
	// title match. Word-synced lines light as they are sung; line-synced ones show whole. The sizes
	// are in `em` off the font size set here, which NowPlaying's title budget (`ART_SIDE`) counts.

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
	const text = $derived(line?.text.trim().replace(/^♪$/, ''));
	// Background vocals stay in the lyrics tab: here they'd read as part of the lead line.
	const words = $derived(line?.words?.filter((w) => !w.bg) ?? []);
	// A change of key is a new line rising in. Before the first line and in an instrumental break the
	// slot holds three dots, so an intro doesn't read as a song without lyrics.
	const key = $derived(!lines ? '' : text ? `${nowLyrics.id}:${at}` : 'wait');

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

<div
	class="mt-3 grid h-[2.7em] font-heading leading-[1.35] font-semibold"
	style="font-size:clamp(0.9375rem, var(--side) * 0.032, 1.125rem)"
>
	<!-- The old line and the new share the one cell while they cross: the new one rises in from a
	     third of the slot below as the old one rises away, the way the lyrics tab scrolls. -->
	{#key key}
		<div
			in:fly={{ y: '35%', duration: dur, easing: cubicOut }}
			out:fly={{ y: '-35%', duration: dur, easing: cubicOut }}
			class="[grid-area:1/1] min-w-0 self-center"
			dir="auto"
		>
			{#if key === 'wait'}
				<span class="flex h-[1.35em] items-center gap-[0.3em]" aria-hidden="true">
					{#each [0, 1, 2] as i (i)}
						<span
							class="size-[0.32em] rounded-full bg-muted-foreground motion-safe:animate-pulse {playback.paused
								? '[animation-play-state:paused]'
								: ''}"
							style="animation-delay:{i * 0.25}s"
						></span>
					{/each}
				</span>
			{:else if key}
				<p class="line-clamp-2 text-start [overflow-wrap:anywhere]">
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
				</p>
			{/if}
		</div>
	{/key}
</div>
