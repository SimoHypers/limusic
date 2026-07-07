<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		PreviousIcon,
		NextIcon,
		PlayIcon,
		PauseIcon,
		Queue01Icon,
		VolumeHighIcon,
		FavouriteIcon,
		Add01Icon
	} from '@hugeicons/core-free-icons';
	import { goto } from '$app/navigation';
	import { Button } from '$lib/components/ui/button';
	import * as api from '$lib/api';
	import { playback, toast, openAddToPlaylist } from '$lib/player.svelte';

	let { onToggleQueue, queueOpen }: { onToggleQueue: () => void; queueOpen: boolean } = $props();

	async function toggleLike() {
		if (!playback.now) return;
		const next = !playback.liked;
		playback.liked = next; // optimistic
		try {
			await api.like(playback.now.videoId, next);
			toast(next ? 'Added to liked songs' : 'Removed from liked songs');
		} catch (e) {
			playback.liked = !next; // revert on failure
			toast(String(e));
		}
	}

	const fmt = (secs: number) => {
		if (!secs || secs < 0) return '0:00';
		const m = Math.floor(secs / 60);
		const s = Math.floor(secs % 60);
		return `${m}:${s.toString().padStart(2, '0')}`;
	};

	function onSeek(e: Event) {
		const v = Number((e.target as HTMLInputElement).value);
		playback.position = v;
		api.seek(v);
	}
	function onVolume(e: Event) {
		const v = Number((e.target as HTMLInputElement).value);
		playback.volume = v;
		api.setVolume(v);
	}
</script>

<footer class="flex items-center gap-2 border-t bg-card px-2 py-2.5 sm:gap-4 sm:px-4 sm:py-3">
	<!-- Now playing -->
	<div class="flex min-w-0 flex-1 items-center gap-3">
		{#if playback.now?.thumbnail}
			<img
				src={playback.now.thumbnail}
				alt=""
				style="max-width:none"
				class="h-12 w-12 shrink-0 rounded-lg object-cover"
			/>
		{:else}
			<div class="h-12 w-12 shrink-0 rounded-lg bg-muted"></div>
		{/if}
		<div class="min-w-0">
			<div class="truncate text-sm font-medium">{playback.now?.title ?? 'Nothing playing'}</div>
			{#if playback.now?.artistId}
				<button
					class="block max-w-full cursor-pointer truncate text-left text-xs text-muted-foreground hover:text-foreground hover:underline"
					onclick={() => goto(`/artist/${encodeURIComponent(playback.now!.artistId!)}`)}
				>
					{playback.now.artists}
				</button>
			{:else}
				<div class="truncate text-xs text-muted-foreground">{playback.now?.artists ?? ''}</div>
			{/if}
		</div>
		{#if playback.now}
			<div class="flex items-center">
				<Button variant="ghost" size="icon-sm" onclick={toggleLike} aria-label="Like">
					<HugeiconsIcon
						icon={FavouriteIcon}
						class="h-4 w-4 {playback.liked ? 'text-primary' : 'text-muted-foreground'}"
					/>
				</Button>
				<Button
					variant="ghost"
					size="icon-sm"
					onclick={() => openAddToPlaylist(playback.now!.videoId)}
					aria-label="Add to playlist"
				>
					<HugeiconsIcon icon={Add01Icon} class="h-4 w-4 text-muted-foreground" />
				</Button>
			</div>
		{/if}
	</div>

	<!-- Transport -->
	<div class="flex flex-[1.5] flex-col items-center gap-1">
		<div class="flex items-center gap-1">
			<Button variant="ghost" size="icon-sm" onclick={() => api.prevTrack()} aria-label="Previous">
				<HugeiconsIcon icon={PreviousIcon} class="h-5 w-5" />
			</Button>
			<Button
				variant="default"
				size="icon"
				class="rounded-full"
				onclick={() => api.togglePause()}
				aria-label="Play/pause"
			>
				<!-- HugeiconsIcon only re-renders `altIcon`/`showAlt`, not `icon` (frozen at mount) —
			     so toggle via showAlt, not a ternary on `icon`. -->
			<HugeiconsIcon
				icon={PauseIcon}
				altIcon={PlayIcon}
				showAlt={playback.paused}
				class="h-5 w-5"
			/>
			</Button>
			<Button variant="ghost" size="icon-sm" onclick={() => api.nextTrack()} aria-label="Next">
				<HugeiconsIcon icon={NextIcon} class="h-5 w-5" />
			</Button>
		</div>
		<div class="flex w-full max-w-md items-center gap-2 text-xs text-muted-foreground">
			<span class="tabular-nums">{fmt(playback.position)}</span>
			<input
				type="range"
				class="flex-1 accent-primary"
				min="0"
				max={playback.duration || 0}
				value={playback.position}
				oninput={onSeek}
				aria-label="Seek"
			/>
			<span class="tabular-nums">{fmt(playback.duration)}</span>
		</div>
	</div>

	<!-- Volume + queue -->
	<div class="flex flex-1 items-center justify-end gap-2">
		<!-- Volume is the first control to drop on a narrow window (OS volume still works). -->
		<div class="hidden items-center gap-2 md:flex">
			<HugeiconsIcon icon={VolumeHighIcon} class="h-4 w-4 text-muted-foreground" />
			<input
				type="range"
				class="w-24 accent-primary"
				min="0"
				max="100"
				value={playback.volume}
				oninput={onVolume}
				aria-label="Volume"
			/>
		</div>
		<Button
			variant={queueOpen ? 'secondary' : 'ghost'}
			size="icon-sm"
			onclick={onToggleQueue}
			aria-label="Toggle queue"
		>
			<HugeiconsIcon icon={Queue01Icon} class="h-5 w-5" />
		</Button>
	</div>
</footer>
