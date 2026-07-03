<script lang="ts">
	import { onMount } from 'svelte';
	import { Input } from '$lib/components/ui/input';
	import { Button } from '$lib/components/ui/button';
	import * as api from '$lib/api';
	import type { SongItem, NowPlaying, QueueState } from '$lib/api';

	let query = $state('');
	let results = $state<SongItem[]>([]);
	let searching = $state(false);
	let error = $state<string | null>(null);

	let now = $state<NowPlaying | null>(null);
	let queue = $state<QueueState>({ items: [], currentIndex: 0 });
	let paused = $state(false);
	let position = $state(0);
	let duration = $state(0);
	let volume = $state(100);

	onMount(() => {
		const unlisteners = [
			api.onNowPlaying((n) => (now = n)),
			api.onQueueChanged((q) => (queue = q)),
			api.onPosition((p) => (position = p)),
			api.onDuration((d) => (duration = d)),
			api.onPlaybackState((s) => (paused = s === 'paused')),
			api.onPlaybackError((msg) => (error = msg))
		];
		api.getQueue().then((q) => (queue = q)).catch(() => {});
		return () => unlisteners.forEach((u) => u.then((f) => f()));
	});

	async function runSearch() {
		if (!query.trim()) return;
		searching = true;
		error = null;
		try {
			results = await api.search(query);
		} catch (e) {
			error = String(e);
		} finally {
			searching = false;
		}
	}

	const fmt = (secs: number) => {
		if (!secs || secs < 0) return '0:00';
		const m = Math.floor(secs / 60);
		const s = Math.floor(secs % 60);
		return `${m}:${s.toString().padStart(2, '0')}`;
	};

	async function onSeek(e: Event) {
		const v = Number((e.target as HTMLInputElement).value);
		position = v;
		await api.seek(v);
	}
	async function onVolume(e: Event) {
		const v = Number((e.target as HTMLInputElement).value);
		volume = v;
		await api.setVolume(v);
	}
</script>

<div class="flex h-screen flex-col bg-background text-foreground">
	<!-- Search -->
	<header class="border-b p-4">
		<h1 class="mb-3 font-[Oxanium] text-xl font-semibold tracking-tight">Limusic</h1>
		<form class="flex gap-2" onsubmit={(e) => { e.preventDefault(); runSearch(); }}>
			<Input bind:value={query} placeholder="Search YouTube Music…" class="max-w-md" />
			<Button type="submit" disabled={searching}>{searching ? 'Searching…' : 'Search'}</Button>
		</form>
		{#if error}<p class="mt-2 text-sm text-destructive">{error}</p>{/if}
	</header>

	<!-- Results + Queue -->
	<main class="flex min-h-0 flex-1">
		<section class="min-h-0 flex-1 overflow-y-auto p-2">
			{#each results as item (item.video_id)}
				<button
					class="flex w-full items-center gap-3 rounded-md p-2 text-left hover:bg-muted"
					onclick={() => api.play(item)}
				>
					{#if item.thumbnail}
						<img src={item.thumbnail} alt="" class="h-10 w-10 rounded object-cover" />
					{:else}
						<div class="h-10 w-10 rounded bg-muted"></div>
					{/if}
					<div class="min-w-0 flex-1">
						<div class="truncate text-sm font-medium">{item.title}</div>
						<div class="truncate text-xs text-muted-foreground">{item.artists}</div>
					</div>
					{#if item.duration}<span class="text-xs text-muted-foreground">{item.duration}</span>{/if}
				</button>
			{:else}
				<p class="p-4 text-sm text-muted-foreground">Search for a song to start.</p>
			{/each}
		</section>

		<aside class="min-h-0 w-72 overflow-y-auto border-l p-2">
			<h2 class="px-2 py-1 text-xs font-semibold uppercase text-muted-foreground">Up next</h2>
			{#each queue.items as item, i (item.video_id + i)}
				<button
					class="flex w-full items-center gap-2 rounded-md p-2 text-left hover:bg-muted {i === queue.currentIndex ? 'bg-muted' : ''}"
					onclick={() => api.playIndex(i)}
				>
					<span class="w-4 text-xs text-muted-foreground">{i + 1}</span>
					<div class="min-w-0 flex-1">
						<div class="truncate text-sm {i === queue.currentIndex ? 'font-semibold text-primary' : ''}">{item.title}</div>
						<div class="truncate text-xs text-muted-foreground">{item.artists}</div>
					</div>
				</button>
			{/each}
		</aside>
	</main>

	<!-- Player bar -->
	<footer class="flex items-center gap-4 border-t p-3">
		<div class="flex min-w-0 flex-1 items-center gap-3">
			{#if now?.thumbnail}
				<img src={now.thumbnail} alt="" class="h-12 w-12 rounded object-cover" />
			{:else}
				<div class="h-12 w-12 rounded bg-muted"></div>
			{/if}
			<div class="min-w-0">
				<div class="truncate text-sm font-medium">{now?.title ?? 'Nothing playing'}</div>
				<div class="truncate text-xs text-muted-foreground">{now?.artists ?? ''}</div>
			</div>
		</div>

		<div class="flex flex-[2] flex-col items-center gap-1">
			<div class="flex items-center gap-2">
				<Button variant="ghost" size="sm" onclick={() => api.prevTrack()} aria-label="Previous">⏮</Button>
				<Button variant="ghost" size="sm" onclick={() => api.togglePause()} aria-label="Play/pause">{paused ? '▶' : '⏸'}</Button>
				<Button variant="ghost" size="sm" onclick={() => api.nextTrack()} aria-label="Next">⏭</Button>
			</div>
			<div class="flex w-full items-center gap-2 text-xs text-muted-foreground">
				<span>{fmt(position)}</span>
				<input type="range" class="flex-1 accent-primary" min="0" max={duration || 0} value={position} oninput={onSeek} aria-label="Seek" />
				<span>{fmt(duration)}</span>
			</div>
		</div>

		<div class="flex flex-1 items-center justify-end gap-2">
			<span class="text-xs text-muted-foreground">Vol</span>
			<input type="range" class="w-24 accent-primary" min="0" max="100" value={volume} oninput={onVolume} aria-label="Volume" />
		</div>
	</footer>
</div>
