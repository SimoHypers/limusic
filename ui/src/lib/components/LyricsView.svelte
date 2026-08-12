<script lang="ts">
	import * as api from '$lib/api';
	import { playback } from '$lib/player.svelte';

	// `expanded` sizes the type and centres the column.
	let { expanded = false }: { expanded?: boolean } = $props();

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
		const album = playback.queue.items[playback.queue.currentIndex]?.album;
		api.getLyrics({
			videoId: id,
			title: now.title,
			artists: now.artists,
			album: album ?? undefined,
			duration: durationSecs(now.duration)
		})
			.then((l) => {
				if (requested !== id) return;
				lyrics = l;
				loading = false;
				hasScrolled = false;
			})
			.catch(() => {
				if (requested !== id) return;
				loading = false;
			});
	});

	// Last synced line whose cue has passed.
	const activeIndex = $derived.by(() => {
		if (!lyrics?.synced) return -1;
		const currentMs = posMs;
		let i = -1;
		for (let j = 0; j < lyrics.lines.length; j++) {
			const t = lyrics.lines[j].time_ms;
			if (t === undefined) continue;
			if (t > currentMs) break;
			i = j;
		}
		return i;
	});

	let userScrollUntil = 0;
	let hasScrolled = false;
	function onUserScroll() {
		userScrollUntil = Date.now() + 3000;
	}

	let wasExpanded: boolean | undefined;

	$effect(() => {
		const i = activeIndex;
		if (expanded !== wasExpanded) {
			wasExpanded = expanded;
			hasScrolled = false;
			userScrollUntil = 0;
		}
		if (i < 0 || !scroller || Date.now() < userScrollUntil) return;
		scroller.querySelector(`[data-line="${i}"]`)?.scrollIntoView({
			behavior: hasScrolled ? 'smooth' : 'instant',
			block: 'center'
		});
		hasScrolled = true;
	});

	function seekTo(line: api.LyricLine) {
		if (line.time_ms === undefined) return;
		const secs = line.time_ms / 1000;
		playback.position = secs;
		userScrollUntil = 0;
		api.seek(secs);
	}

	// 60 FPS smooth high-precision position interpolation for fluid word-by-word karaoke sweep
	let interpolatedPosSecs = $state(playback.position);
	let lastBackendPos = playback.position;
	let lastBackendTime = 0;

	$effect(() => {
		const pos = playback.position;
		const paused = playback.paused;
		const now = performance.now();

		if (paused) {
			interpolatedPosSecs = pos;
			lastBackendPos = pos;
			lastBackendTime = now;
			return;
		}

		// When backend emits a new position tick (or jump/seek), resync base
		if (Math.abs(pos - lastBackendPos) > 0.05 || lastBackendTime === 0) {
			lastBackendPos = pos;
			lastBackendTime = now;
		}

		let frameId: number;
		function updateFrame() {
			if (!playback.paused) {
				const elapsedSecs = (performance.now() - lastBackendTime) / 1000;
				interpolatedPosSecs = lastBackendPos + elapsedSecs;
				frameId = requestAnimationFrame(updateFrame);
			}
		}
		frameId = requestAnimationFrame(updateFrame);

		return () => {
			if (frameId) cancelAnimationFrame(frameId);
		};
	});

	const posMs = $derived(interpolatedPosSecs * 1000);

	function getWordProgress(word: api.LyricWord, currentMs: number): number {
		if (currentMs <= word.start_ms) return 0;
		if (currentMs >= word.end_ms) return 1;
		const dur = word.end_ms - word.start_ms;
		if (dur <= 0) return 1;
		return (currentMs - word.start_ms) / dur;
	}
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
	bind:this={scroller}
	onwheel={onUserScroll}
	ontouchmove={onUserScroll}
	onpointerdown={onUserScroll}
	class="min-h-0 flex-1 overflow-y-auto py-6 {expanded ? 'px-10' : 'px-5'}"
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
		<div class="py-[35vh] {expanded ? 'mx-auto max-w-3xl' : ''}">
			{#each lyrics.lines as line, i (i)}
				{@const isActive = i === activeIndex}
				{@const isPast = i < activeIndex}
				<button
					data-line={i}
					onclick={() => seekTo(line)}
					class="group block w-full origin-left cursor-pointer text-left font-heading font-bold leading-snug transition-all duration-300 ease-out hover:text-foreground
						{expanded ? 'py-3 text-3xl' : 'py-2 text-xl'}
						{isActive
						? 'scale-[1.04] text-foreground filter drop-shadow-[0_0_12px_rgba(255,255,255,0.4)]'
						: isPast
							? 'text-muted-foreground/40'
							: 'text-muted-foreground/70'}"
				>
					{#if line.words && line.words.length > 0}
						<!-- Word-by-Word Karaoke Sweep Animation (Better-Lyrics style, highly optimized) -->
						<span class="inline-flex flex-wrap items-baseline">
							{#each line.words as word, wIdx (wIdx)}
								{@const isWordEnd = word.text.endsWith(' ')}
								{@const cleanText = word.text.trimEnd()}
								{#if isActive}
									{@const progress = getWordProgress(word, posMs)}
									{@const pct = Math.round(Math.min(1, Math.max(0, progress)) * 100)}
									{@const isCurrentWord = progress > 0 && progress < 1}
									<span
										class="inline-block transition-transform duration-100 ease-out {isWordEnd ? 'mr-[0.26em]' : ''} {isCurrentWord
											? 'scale-[1.03]'
											: ''}"
										style="
											background: linear-gradient(90deg, var(--foreground, #fff) {pct}%, rgba(255, 255, 255, 0.35) {pct}%);
											-webkit-background-clip: text;
											-webkit-text-fill-color: transparent;
											will-change: background, transform;
										"
									>
										{cleanText}
									</span>
								{:else}
									<span class="inline-block {isWordEnd ? 'mr-[0.26em]' : ''} {isPast ? 'text-muted-foreground/40' : 'text-muted-foreground/70'}">
										{cleanText}
									</span>
								{/if}
							{/each}
						</span>
					{:else}
						<span>{line.text || '♪'}</span>
					{/if}

					<!-- Translation line rendering -->
					{#if line.translation}
						<p class="mt-1 text-sm font-normal italic tracking-wide opacity-80 transition-opacity">
							{line.translation}
						</p>
					{/if}
				</button>
			{/each}
		</div>
	{:else if lyrics}
		<div
			class="space-y-2 leading-relaxed text-foreground/90 {expanded
				? 'mx-auto max-w-3xl text-xl'
				: 'text-[15px]'}"
		>
			{#each lyrics.lines as line, i (i)}
				{#if line.text}
					<div>
						<p>{line.text}</p>
						{#if line.translation}
							<p class="text-xs italic text-muted-foreground">{line.translation}</p>
						{/if}
					</div>
				{:else}
					<div class="h-4"></div>
				{/if}
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

