<script lang="ts">
	import { fade, fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import * as api from '$lib/api';
	import { playback } from '$lib/player.svelte';

	let { onClose }: { onClose: () => void } = $props();

	/** "3:21" / "1:02:03" → seconds. */
	function durationSecs(d?: string): number | undefined {
		if (!d) return undefined;
		const parts = d.split(':').map(Number);
		if (!parts.length || parts.some(Number.isNaN)) return undefined;
		return parts.reduce((a, b) => a * 60 + b, 0);
	}

	let lyrics = $state<api.Lyrics | null>(null);
	let loading = $state(true);
	let scroller: HTMLElement | undefined = $state();

	// videoId of the fetch whose result is (or will be) shown — guards stale responses.
	let requested = '';

	$effect(() => {
		const now = playback.now;
		if (!now) {
			requested = '';
			lyrics = null;
			loading = false;
			return;
		}
		if (now.videoId === requested) return;
		const id = (requested = now.videoId);
		loading = true;
		lyrics = null;
		// Album isn't in now-playing, but the queue item usually has it — better LRCLIB matching.
		const album = playback.queue.items[playback.queue.currentIndex]?.album;
		api.getLyrics({
			videoId: id,
			title: now.title,
			artists: now.artists,
			album: album ?? undefined,
			// The track's own length — NOT playback.duration, which still holds the previous
			// track's value for a moment after a track change.
			duration: durationSecs(now.duration)
		})
			.then((l) => {
				if (requested !== id) return;
				lyrics = l;
				loading = false;
				hasScrolled = false; // first positioning on a new track is an instant jump
			})
			.catch(() => {
				if (requested !== id) return;
				loading = false;
			});
	});

	// Last synced line whose cue has passed (lines arrive sorted by time).
	const activeIndex = $derived.by(() => {
		if (!lyrics?.synced) return -1;
		const posMs = playback.position * 1000;
		let i = -1;
		for (let j = 0; j < lyrics.lines.length; j++) {
			const t = lyrics.lines[j].time_ms;
			if (t === undefined) continue;
			if (t > posMs) break;
			i = j;
		}
		return i;
	});

	// Auto-scroll pauses while the user is scrolling (wheel/touch/scrollbar), resumes after 3s.
	// Tracked via input events, not `scroll`, so our own smooth scrolls don't trip it.
	let userScrollUntil = 0;
	let hasScrolled = false;
	function onUserScroll() {
		userScrollUntil = Date.now() + 3000;
	}

	$effect(() => {
		const i = activeIndex;
		if (i < 0 || !scroller || Date.now() < userScrollUntil) return;
		scroller.querySelector(`[data-line="${i}"]`)?.scrollIntoView({
			// Opening mid-song jumps straight to the line; after that, glide.
			behavior: hasScrolled ? 'smooth' : 'instant',
			block: 'center'
		});
		hasScrolled = true;
	});

	function seekTo(line: api.LyricLine) {
		if (line.time_ms === undefined) return;
		const secs = line.time_ms / 1000;
		playback.position = secs; // optimistic — the mpv tick confirms
		userScrollUntil = 0; // jump the view along with the seek
		api.seek(secs);
	}
</script>

<!-- Same overlay pattern as QueuePanel: floating over content below lg (with a dismiss scrim),
     in-flow column at lg+. -->
<button
	class="absolute inset-0 z-20 cursor-default bg-black/40 lg:hidden"
	onclick={onClose}
	aria-label="Close lyrics"
	transition:fade={{ duration: 150 }}
></button>
<aside
	transition:fly={{ x: 32, duration: 220, easing: cubicOut }}
	class="absolute inset-y-0 right-0 z-30 flex h-full w-80 max-w-[80vw] shrink-0 flex-col border-l bg-card shadow-2xl lg:static lg:z-auto lg:max-w-none lg:bg-card/40 lg:shadow-none"
>
	<h2 class="border-b px-4 py-3 font-heading text-sm font-semibold">Lyrics</h2>
	<!-- svelte-ignore a11y_no_static_element_interactions -- handlers only detect scroll intent -->
	<div
		bind:this={scroller}
		onwheel={onUserScroll}
		ontouchmove={onUserScroll}
		onpointerdown={onUserScroll}
		class="min-h-0 flex-1 overflow-y-auto px-5 py-6"
	>
		{#if loading}
			<div class="space-y-3">
				{#each { length: 8 } as _, i (i)}
					<div class="h-5 animate-pulse rounded bg-muted" style="width:{55 + ((i * 17) % 40)}%"></div>
				{/each}
			</div>
		{:else if lyrics?.instrumental}
			<p class="py-8 text-center text-lg text-muted-foreground">Instrumental ♪</p>
		{:else if lyrics && lyrics.synced}
			<!-- Padding lets the first/last lines center-scroll. -->
			<div class="py-[35vh]">
				{#each lyrics.lines as line, i (i)}
					<button
						data-line={i}
						onclick={() => seekTo(line)}
						class="block w-full cursor-pointer py-1.5 text-left font-heading text-lg font-semibold leading-snug transition-colors duration-200 hover:text-foreground
							{i === activeIndex
							? 'text-foreground'
							: i < activeIndex
								? 'text-muted-foreground/40'
								: 'text-muted-foreground'}"
					>
						{line.text || '♪'}
					</button>
				{/each}
			</div>
		{:else if lyrics}
			<div class="space-y-1 text-[15px] leading-relaxed text-foreground/90">
				{#each lyrics.lines as line, i (i)}
					{#if line.text}<p>{line.text}</p>{:else}<div class="h-4"></div>{/if}
				{/each}
			</div>
		{:else}
			<p class="py-8 text-center text-sm text-muted-foreground">No lyrics found for this track.</p>
		{/if}
	</div>
	{#if lyrics && !loading}
		<p class="border-t px-4 py-2 text-xs text-muted-foreground">
			{lyrics.source.startsWith('Source:') ? lyrics.source : `Lyrics from ${lyrics.source}`}
		</p>
	{/if}
</aside>
