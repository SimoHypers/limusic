<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		PreviousIcon,
		NextIcon,
		PlayIcon,
		PauseIcon,
		ShuffleIcon,
		RepeatIcon,
		RepeatOne01Icon,
		Queue01Icon,
		Mic01Icon,
		VolumeHighIcon,
		VolumeMute02Icon,
		FavouriteIcon,
		Add01Icon,
		InfinityIcon,
		MinimizeScreenIcon,
		MusicNote01Icon,
		ArrowUp01Icon,
		ArrowDown01Icon
	} from '@hugeicons/core-free-icons';
	import { fade } from 'svelte/transition';
	import { Button } from '$lib/components/ui/button';
	import * as api from '$lib/api';
	import {
		np,
		playback,
		commitVolume,
		cycleRepeat,
		dragVolume,
		openAddToPlaylist,
		openMiniPlayer,
		toggleMute,
		toggleNowPlayingLike,
		wheelVolume
	} from '$lib/player.svelte';
	import { thumb } from '$lib/thumb';
	import ArtistLine from './ArtistLine.svelte';
	import Marquee from './Marquee.svelte';
	import TrackMenu from './TrackMenu.svelte';
	import { t } from '$lib/i18n.svelte';

	let {
		onToggleQueue,
		queueOpen,
		onToggleLyrics,
		lyricsOpen
	}: {
		onToggleQueue: () => void;
		queueOpen: boolean;
		onToggleLyrics: () => void;
		lyricsOpen: boolean;
	} = $props();

	// Pop the heart once when the user favourites (not when un-favouriting). Reset on animation end
	// so the next like can replay it.
	let justLiked = $state(false);

	function toggleLike() {
		if (playback.rating !== 'like') justLiked = true;
		toggleNowPlayingLike();
	}

	const fmt = (secs: number) => {
		if (!secs || secs < 0) return '0:00';
		const t = Math.floor(secs);
		const h = Math.floor(t / 3600);
		const m = Math.floor((t % 3600) / 60);
		const s = t % 60;
		const mm = h ? m.toString().padStart(2, '0') : `${m}`;
		return `${h ? `${h}:` : ''}${mm}:${s.toString().padStart(2, '0')}`;
	};

	const shuffleOn = $derived(playback.queue.shuffle ?? false);
	const repeat = $derived(playback.queue.repeat ?? 'off');

	// The current track was appended by autoplay → show the subtle ∞ badge next to the title.
	// Matched against the now-playing videoId so a transient queue/now-playing mismatch (mid
	// gapless advance) can't flash the badge on the wrong song.
	const autoplayTrack = $derived.by(() => {
		const cur = playback.queue.items[playback.queue.currentIndex];
		return !!cur?.autoplay && cur.video_id === playback.now?.videoId;
	});

	// The ⋮ menu needs the full SongItem — NowPlaying carries no album_id. Take it from the queue
	// row, matched on videoId so a mid-advance mismatch can't point the menu at the wrong song.
	const currentSong = $derived.by(() => {
		const cur = playback.queue.items[playback.queue.currentIndex];
		return cur?.video_id === playback.now?.videoId ? cur : null;
	});

	// Seek: while dragging, hold a local value so incoming mpv position ticks can't yank the thumb
	// back under the pointer; only invoke the (expensive) seek on release.
	let seekDrag = $state<number | null>(null);
	const shownPosition = $derived(seekDrag ?? playback.position);

	function onSeekInput(e: Event) {
		seekDrag = Number((e.target as HTMLInputElement).value);
	}
	function onSeekCommit(e: Event) {
		const v = Number((e.target as HTMLInputElement).value);
		playback.position = v;
		seekDrag = null;
		api.seek(v);
	}

	const onVolume = (e: Event) => dragVolume(Number((e.target as HTMLInputElement).value));
	const onVolumeCommit = (e: Event) => commitVolume(Number((e.target as HTMLInputElement).value));

	const isControl = (t: EventTarget | null) =>
		!!(t as HTMLElement | null)?.closest?.('button, a, input, [role="button"]');

	// Dragging a slider past its end and releasing outside it retargets the click at the bar (the
	// click lands on the common ancestor of press and release), which used to toggle the view.
	// So judge by where the press started, not where the release happened.
	let pressedControl = false;

	// Anywhere on the bar that isn't a control opens (or closes) the now-playing view: the bar is
	// what's left of it once it's minimised, so it's the way back in. Deliberately no pointer
	// cursor, because this is the whole bar, not a button, and every real button keeps its own click.
	function onBarClick(e: MouseEvent) {
		if (pressedControl || isControl(e.target)) return;
		np.open = !np.open;
	}
</script>

<!-- The chevron button below is the keyboard equivalent of clicking the bar, so the bar itself
     stays a plain region rather than becoming a focusable control wrapping every other control. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions, a11y_no_noninteractive_element_interactions -->
<footer
	onpointerdown={(e) => (pressedControl = isControl(e.target))}
	onclick={onBarClick}
	class="flex items-center gap-2 border-t bg-card px-2 py-2.5 sm:gap-4 sm:px-4 sm:py-3"
>
	<!-- Now playing. data-ctx: right-clicking the cover or the title opens the ⋮ menu for the track
	     that's playing (not the buttons beside them — those keep their own meaning). -->
	<div class="flex min-w-0 flex-1 items-center gap-3" data-ctx>
		{#key playback.now?.videoId}
			{#if playback.now?.thumbnail}
				<img
					src={thumb(playback.now.thumbnail, 120)}
					alt=""
					style="max-width:none"
					class="h-12 w-12 shrink-0 rounded-lg object-cover"
					in:fade={{ duration: 250 }}
				/>
			{:else}
				<div
					class="flex h-12 w-12 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground/50"
				>
					<HugeiconsIcon icon={MusicNote01Icon} class="h-5 w-5" />
				</div>
			{/if}
		{/key}
		<div class="min-w-0">
			<div class="flex items-center gap-1.5">
				<Marquee
					text={playback.now?.title ?? t('player.not_playing')}
					class="text-sm font-medium"
				/>
				{#if autoplayTrack}
					<span
						class="shrink-0 text-muted-foreground"
						title={t('player.autoplay_notice')}
						in:fade={{ duration: 200 }}
					>
						<HugeiconsIcon icon={InfinityIcon} class="h-3.5 w-3.5" />
					</span>
				{/if}
			</div>
			<ArtistLine
				runs={playback.now?.artistRuns}
				text={playback.now?.artists ?? ''}
				marquee
				class="block max-w-full text-xs text-muted-foreground"
			/>
		</div>
		{#if playback.now}
			<div class="flex items-center">
				<!-- A local file has no YouTube identity (see api.isLocalId): nothing to like, and no
				     YTM playlist to add it to. Below lg both drop and the ⋮ menu carries them instead:
				     on a narrow window three buttons here leave the title almost no room. lg, not md:
				     the window's minWidth is 900 (tauri.conf.json), so md never fires. -->
				{#if !api.isLocalId(playback.now.videoId)}
					<Button
						variant="ghost"
						size="icon-sm"
						class="hidden lg:inline-flex"
						onclick={toggleLike}
						aria-label={t('common.like')}
					>
						<span
							class="inline-flex"
							class:animate-heart-pop={justLiked}
							onanimationend={() => (justLiked = false)}
						>
							<HugeiconsIcon
								icon={FavouriteIcon}
								class="h-4 w-4 {playback.rating === 'like' ? 'fill-current text-primary' : 'text-muted-foreground'}"
							/>
						</span>
					</Button>
					<Button
						variant="ghost"
						size="icon-sm"
						class="hidden lg:inline-flex"
						onclick={() => {
							const now = playback.now!;
							openAddToPlaylist({
								video_id: now.videoId,
								title: now.title,
								artists: now.artists,
								artist_id: now.artistId,
								thumbnail: now.thumbnail,
								duration: now.duration
							});
						}}
						aria-label={t('player.save_to_playlist')}
					>
						<HugeiconsIcon icon={Add01Icon} class="h-4 w-4 text-muted-foreground" />
					</Button>
				{/if}
				{#if currentSong}
					<TrackMenu
						song={currentSong}
						linksOnly
						onAdd={() => openAddToPlaylist(currentSong!)}
						triggerClass="inline-flex size-8 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition hover:bg-muted hover:text-foreground"
					/>
				{/if}
			</div>
		{/if}
	</div>

	<!-- Transport -->
	<div class="flex flex-[1.5] flex-col items-center gap-1">
		<div class="flex items-center gap-1">
			<Button
				variant="ghost"
				size="icon-sm"
				onclick={() => api.toggleShuffle()}
				aria-label={t('player.shuffle')}
				aria-pressed={shuffleOn}
			>
				<HugeiconsIcon
					icon={ShuffleIcon}
					class="h-4 w-4 {shuffleOn ? 'text-primary' : 'text-muted-foreground'}"
				/>
			</Button>
			<Button variant="ghost" size="icon-sm" onclick={() => api.prevTrack()} aria-label={t('player.previous')}>
				<HugeiconsIcon icon={PreviousIcon} class="h-5 w-5" />
			</Button>
			<Button
				variant="default"
				size="icon"
				class="rounded-full"
				onclick={() => api.togglePause()}
				aria-label={playback.paused ? t('player.play') : t('player.pause')}
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
			<Button variant="ghost" size="icon-sm" onclick={() => api.nextTrack()} aria-label={t('player.next')}>
				<HugeiconsIcon icon={NextIcon} class="h-5 w-5" />
			</Button>
			<Button
				variant="ghost"
				size="icon-sm"
				onclick={cycleRepeat}
				aria-label={t('player.repeat_state', {
					state: repeat === 'off' ? t('player.repeat_off') : repeat === 'one' ? t('player.repeat_one') : t('player.repeat_all')
				})}
				aria-pressed={repeat !== 'off'}
			>
				<!-- icon swap via altIcon/showAlt — `icon` is frozen at mount (see play/pause above) -->
				<HugeiconsIcon
					icon={RepeatIcon}
					altIcon={RepeatOne01Icon}
					showAlt={repeat === 'one'}
					class="h-4 w-4 {repeat !== 'off' ? 'text-primary' : 'text-muted-foreground'}"
				/>
			</Button>
		</div>
		<div class="flex w-full max-w-md items-center gap-2 text-xs text-muted-foreground">
			<span class="tabular-nums">{fmt(shownPosition)}</span>
			<input
				type="range"
				class="range flex-1"
				style="--pct:{playback.duration ? (shownPosition / playback.duration) * 100 : 0}%"
				min="0"
				max={playback.duration || 0}
				value={shownPosition}
				oninput={onSeekInput}
				onchange={onSeekCommit}
				aria-label={t('player.seek')}
			/>
			<span class="tabular-nums">{fmt(playback.duration)}</span>
		</div>
	</div>

	<!-- Volume + queue -->
	<div class="flex flex-1 items-center justify-end gap-2">
		<!-- Volume is the first control to drop on a narrow window (OS volume still works). -->
		<div class="hidden items-center gap-1 md:flex">
			<Button
				variant="ghost"
				size="icon-sm"
				class="text-muted-foreground"
				onclick={toggleMute}
				aria-label={playback.volume === 0 ? t('player.unmute') : t('player.mute')}
			>
				<!-- icon swap via altIcon/showAlt — `icon` is frozen at mount (see play/pause above) -->
				<HugeiconsIcon
					icon={VolumeHighIcon}
					altIcon={VolumeMute02Icon}
					showAlt={playback.volume === 0}
					class="h-4 w-4"
				/>
			</Button>
			<input
				type="range"
				class="range w-24"
				style="--pct:{playback.volume}%"
				min="0"
				max="100"
				value={playback.volume}
				oninput={onVolume}
				onchange={onVolumeCommit}
				onwheel={wheelVolume}
				aria-label={t('player.volume')}
			/>
		</div>
		<!-- One cluster, so they sit tighter to each other than to the volume slider. -->
		<div class="flex items-center gap-0.5">
			<Button variant="ghost" size="icon-sm" onclick={openMiniPlayer} aria-label={t('player.mini_player')}>
				<HugeiconsIcon icon={MinimizeScreenIcon} class="h-5 w-5" />
			</Button>
			<Button
				variant={lyricsOpen ? 'secondary' : 'ghost'}
				size="icon-sm"
				onclick={onToggleLyrics}
				aria-label={t('player.lyrics')}
			>
				<HugeiconsIcon icon={Mic01Icon} class="h-5 w-5" />
			</Button>
			<Button
				variant={queueOpen ? 'secondary' : 'ghost'}
				size="icon-sm"
				onclick={onToggleQueue}
				aria-label={t('player.queue')}
			>
				<HugeiconsIcon icon={Queue01Icon} class="h-5 w-5" />
			</Button>
			<!-- The keyboard (and discoverable) way in and out of the now-playing view; clicking the
			     bar's empty space does the same thing. -->
			<Button
				variant="ghost"
				size="icon-sm"
				onclick={() => (np.open = !np.open)}
				aria-label={np.open ? t('player.minimize_player') : t('player.open_player')}
				aria-expanded={np.open}
			>
				<!-- icon swap via altIcon/showAlt — `icon` is frozen at mount (see play/pause above) -->
				<HugeiconsIcon
					icon={ArrowUp01Icon}
					altIcon={ArrowDown01Icon}
					showAlt={np.open}
					class="h-5 w-5"
				/>
			</Button>
		</div>
	</div>
</footer>
