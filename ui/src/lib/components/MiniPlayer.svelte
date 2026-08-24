<script lang="ts">
	// The whole UI of the mini-player window (Rust `mini.rs`). It is the same SPA as the main
	// window — the root layout picks this instead of the app chrome when the window label is
	// `mini` — so it reads the same `playback` store and calls the same commands. Nothing here is
	// mini-specific state.
	//
	// The window is undecorated and transparent, so this component *is* the window: it paints the
	// rounded card, and `data-tauri-drag-region="deep"` makes every part of it a drag handle
	// except the controls (Tauri's drag script stops at buttons and inputs on its own).
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		PreviousIcon,
		NextIcon,
		PlayIcon,
		PauseIcon,
		ShuffleIcon,
		RepeatIcon,
		RepeatOne01Icon,
		FavouriteIcon,
		MusicNote01Icon,
		MaximizeScreenIcon,
		Mic01Icon,
		VolumeHighIcon,
		VolumeMute02Icon
	} from '@hugeicons/core-free-icons';
	import { fade } from 'svelte/transition';
	import * as api from '$lib/api';
	import {
		playback,
		commitVolume,
		cycleRepeat,
		dragVolume,
		toggleMute,
		toggleNowPlayingLike,
		wheelVolume
	} from '$lib/player.svelte';
	import { thumb } from '$lib/thumb';
	import LyricsView from './LyricsView.svelte';
	import Marquee from './Marquee.svelte';
	import { t } from '$lib/i18n.svelte';

	// Which of the two the right column is showing. Local, and reset when the widget is destroyed:
	// nothing here is worth persisting. The queue is the default because it is the cheaper view —
	// lyrics only fetch (and run the karaoke clock) while this is 'lyrics'.
	let tab = $state<'queue' | 'lyrics'>('queue');

	const now = $derived(playback.now);
	const shuffleOn = $derived(playback.queue.shuffle ?? false);
	const repeat = $derived(playback.queue.repeat ?? 'off');
	// A local file has no YouTube identity, so there is nothing to like (see api.isLocalId).
	const likeable = $derived(!!now && !api.isLocalId(now.videoId));

	// Three fit; the fourth is rendered on purpose and clipped by the list's mask, so the queue
	// reads as continuing rather than ending at whatever happens to fit. Real queue indices so a
	// click can jump to them.
	const upcoming = $derived.by(() => {
		const { items, currentIndex } = playback.queue;
		return items
			.slice(currentIndex + 1, currentIndex + 5)
			.map((item, k) => ({ item, index: currentIndex + 1 + k }));
	});

	// Every plain icon button. Fixed square boxes, flex-centred: left to inline layout, each glyph
	// sits wherever its own baseline puts it and neighbours don't line up.
	const artBtn =
		'flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-md text-white/70 transition hover:bg-white/15 hover:text-white';
	const panelBtn =
		'flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-md transition-colors hover:bg-muted';

	// Volume: the slider is revealed by hovering the control, and stays out for as long as it is
	// being dragged. Hover alone can't say the second part — the strip is 24px tall, so a pointer
	// that wanders off it mid-drag would collapse the slider under its own thumb.
	let volHover = $state(false);
	let volDragging = $state(false);
	const volOpen = $derived(volHover || volDragging);

	// Pop the heart once when favouriting (not when un-favouriting), same as the player bar.
	let justLiked = $state(false);
	function toggleLike() {
		if (playback.rating !== 'like') justLiked = true;
		toggleNowPlayingLike();
	}

	// Seek: hold the dragged value locally so incoming position ticks can't yank the thumb out
	// from under the pointer; only invoke the seek on release.
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
</script>

<!-- On the window, not the slider: a range drag that ends with the pointer somewhere else never
     delivers pointerup to the input, and the slider would stay out for good. -->
<svelte:window onpointerup={() => (volDragging = false)} />

<!-- h-screen/w-screen: the window has no chrome, so this fills it exactly and rounds its corners
     (the compositor can't round an undecorated window for us — same trick as the main window). -->
<div
	data-tauri-drag-region="deep"
	class="group relative flex h-screen w-screen select-none overflow-hidden rounded-2xl border border-border/60 bg-card text-foreground"
>
	<!-- Cover art under the left half, masked so it dissolves into the card instead of ending on a
	     seam. Keyed so a track change cross-fades. -->
	{#key now?.videoId}
		{#if now?.thumbnail}
			<img
				src={thumb(now.thumbnail, 480)}
				alt=""
				in:fade={{ duration: 300 }}
				class="pointer-events-none absolute inset-y-0 left-0 h-full w-[56%] object-cover"
				style="mask-image:linear-gradient(to right,#000 0,#000 70%,transparent 100%);-webkit-mask-image:linear-gradient(to right,#000 0,#000 70%,transparent 100%)"
			/>
		{/if}
	{/key}
	<!-- Enough shade to keep white text readable over a bright cover, following the same fade so it
	     never draws an edge of its own. The art stays plainly visible under it. -->
	<div
		class="pointer-events-none absolute inset-y-0 left-0 w-[56%]"
		style="background:linear-gradient(to right,rgb(0 0 0/0.72) 0%,rgb(0 0 0/0.58) 70%,rgb(0 0 0/0) 100%)"
	></div>

	<!-- Back to the app. Hidden until the pointer is over the widget: it is not part of the design,
	     it is the way out of it. The tray icon does the same thing. The command destroys this very
	     window, so its reply lands nowhere — the rejection is swallowed rather than left dangling. -->
	<button
		class="absolute left-2 top-2 z-10 flex size-6 cursor-pointer items-center justify-center rounded-md text-white/60 opacity-0 transition hover:bg-white/15 hover:text-white focus-visible:opacity-100 group-hover:opacity-100"
		onclick={() => api.closeMini().catch(() => {})}
		title={t('common.back')}
		aria-label={t('common.back')}
	>
		<HugeiconsIcon icon={MaximizeScreenIcon} class="h-3.5 w-3.5" />
	</button>

	<!-- Left: what's playing, over the art. -->
	<div class="relative flex min-w-0 flex-1 flex-col justify-between p-3.5 pl-4">
		<div class="flex items-center justify-end gap-0.5">
			<!-- Volume. The slider sits *in flow* to the left of its icon and grows from zero width:
			     the row is right-aligned, so it expands into the empty space on its left and the
			     heart never moves. In flow, and with no gap, so the wrapper's own box covers both —
			     absolute-positioned with a margin, the pointer left the hover target on its way to
			     the slider and the slider collapsed before it got there. -->
			<div
				class="flex items-center"
				role="group"
				aria-label={t('a11y.volume')}
				onpointerenter={() => (volHover = true)}
				onpointerleave={() => (volHover = false)}
			>
				<!-- min-w-0: a flex item defaults to min-width:auto, and a range input's intrinsic
				     width is not zero, so without it the slider never actually collapses. -->
				<input
					type="range"
					class="range on-art min-w-0 transition-[width,opacity] duration-150 {volOpen
						? 'w-20 opacity-100'
						: 'w-0 opacity-0'}"
					style="--pct:{playback.volume}%"
					min="0"
					max="100"
					value={playback.volume}
					onpointerdown={() => (volDragging = true)}
					oninput={(e) => dragVolume(Number(e.currentTarget.value))}
					onchange={(e) => commitVolume(Number(e.currentTarget.value))}
					onwheel={wheelVolume}
					aria-label={t('a11y.volume')}
				/>
				<button
					class={artBtn}
					onclick={toggleMute}
					aria-label={playback.volume === 0 ? 'Unmute' : 'Mute'}
				>
					<!-- icon swap via altIcon/showAlt — `icon` is frozen at mount -->
					<HugeiconsIcon
						icon={VolumeHighIcon}
						altIcon={VolumeMute02Icon}
						showAlt={playback.volume === 0}
						class="h-4 w-4"
					/>
				</button>
			</div>
			{#if likeable}
				<button
					class={artBtn}
					onclick={toggleLike}
					aria-label={playback.rating === 'like' ? t('player.remove_from_liked') : t('player.save_to_liked')}
				>
					<span
						class="flex"
						class:animate-heart-pop={justLiked}
						onanimationend={() => (justLiked = false)}
					>
						<!-- fill-current + text-primary is the same "liked" treatment the player bar uses. -->
						<HugeiconsIcon
							icon={FavouriteIcon}
							class="h-4 w-4 {playback.rating === 'like' ? 'fill-current text-primary' : ''}"
						/>
					</span>
				</button>
			{/if}
		</div>

		<div class="min-w-0 [text-shadow:0_1px_4px_rgb(0_0_0/0.7)]">
			<Marquee
				text={now?.title ?? t('player.not_playing')}
				class="font-heading text-[0.95rem] font-semibold leading-tight text-white"
			/>
			<Marquee text={now?.artists ?? ''} class="text-xs leading-snug text-white/75" />
		</div>

		<div class="flex items-center gap-2">
			<button class={artBtn} onclick={() => api.prevTrack()} aria-label={t('a11y.previous')}>
				<HugeiconsIcon icon={PreviousIcon} class="h-4 w-4" />
			</button>
			<input
				type="range"
				class="range on-art min-w-0 flex-1"
				style="--pct:{playback.duration ? (shownPosition / playback.duration) * 100 : 0}%"
				min="0"
				max={playback.duration || 0}
				value={shownPosition}
				oninput={onSeekInput}
				onchange={onSeekCommit}
				aria-label={t('a11y.seek')}
			/>
			<button class={artBtn} onclick={() => api.nextTrack()} aria-label={t('a11y.next')}>
				<HugeiconsIcon icon={NextIcon} class="h-4 w-4" />
			</button>
		</div>
	</div>

	<!-- Right: what's next, and the transport. -->
	<div class="relative flex w-64 shrink-0 flex-col gap-2 py-3 pl-1 pr-3">
		<!-- Takes whatever height is left above the controls, and the fourth row runs past that edge
		     and dissolves into it: the queue should look like it continues, not like it ends at
		     whatever happened to fit. Tuned by eye at 560x180. -->
		{#if tab === 'lyrics'}
			<!-- Faded top and bottom, unlike the queue: the active line is centred, so the lines
			     running off both edges should dissolve the same way. -->
			<div
				class="flex min-h-0 flex-1 flex-col overflow-hidden pl-2"
				style="mask-image:linear-gradient(to bottom,transparent 0,#000 20%,#000 80%,transparent 100%);-webkit-mask-image:linear-gradient(to bottom,transparent 0,#000 20%,#000 80%,transparent 100%)"
			>
				<LyricsView compact />
			</div>
		{:else}
		<div
			class="flex min-h-0 flex-1 flex-col gap-0.5 overflow-hidden"
			style="mask-image:linear-gradient(to bottom,#000 0,#000 78%,transparent 100%);-webkit-mask-image:linear-gradient(to bottom,#000 0,#000 78%,transparent 100%)"
		>
			{#each upcoming as { item, index } (item.video_id + index)}
				<button
					class="flex shrink-0 cursor-pointer items-center gap-2 rounded-md px-1.5 py-0.5 text-left transition-colors hover:bg-muted"
					onclick={() => api.playIndex(index)}
					title={item.title}
				>
					{#if item.thumbnail}
						<img
							src={thumb(item.thumbnail, 64)}
							alt=""
							style="max-width:none"
							class="h-6 w-6 shrink-0 rounded object-cover"
						/>
					{:else}
						<div
							class="flex h-6 w-6 shrink-0 items-center justify-center rounded bg-muted text-muted-foreground/50"
						>
							<HugeiconsIcon icon={MusicNote01Icon} class="h-3 w-3" />
						</div>
					{/if}
					<span class="truncate text-xs">{item.title}</span>
				</button>
			{:else}
				<p class="px-1.5 py-0.5 text-xs text-muted-foreground">{t('player.empty_queue')}</p>
			{/each}
		</div>
		{/if}

		<!-- The lyrics toggle rides the transport row absolutely rather than in a header of its
		     own: at 180px tall, a row for it would cost the queue a track and a half. -->
		<div class="relative flex shrink-0 items-center justify-center gap-2.5">
			<button
				class="{panelBtn} {shuffleOn ? 'text-primary' : 'text-muted-foreground'}"
				onclick={() => api.toggleShuffle()}
				aria-label={t('player.shuffle')}
				aria-pressed={shuffleOn}
			>
				<HugeiconsIcon icon={ShuffleIcon} class="h-4 w-4" />
			</button>
			<button
				class="flex size-9 shrink-0 cursor-pointer items-center justify-center rounded-full bg-primary text-primary-foreground transition-colors hover:bg-primary/80"
				onclick={() => api.togglePause()}
				aria-label={playback.paused ? t('player.play') : t('player.pause')}
			>
				<!-- HugeiconsIcon freezes `icon` at mount, so the swap has to go through
				     altIcon/showAlt — a ternary on `icon` would never repaint. -->
				<HugeiconsIcon icon={PauseIcon} altIcon={PlayIcon} showAlt={playback.paused} class="h-4 w-4" />
			</button>
			<button
				class="{panelBtn} {repeat !== 'off' ? 'text-primary' : 'text-muted-foreground'}"
				onclick={cycleRepeat}
				aria-label={t('player.repeat_state', {
					state: repeat === 'off' ? t('player.repeat_off') : repeat === 'one' ? t('player.repeat_one') : t('player.repeat_all')
				})}
				aria-pressed={repeat !== 'off'}
			>
				<HugeiconsIcon
					icon={RepeatIcon}
					altIcon={RepeatOne01Icon}
					showAlt={repeat === 'one'}
					class="h-4 w-4"
				/>
			</button>
			<button
				class="{panelBtn} absolute right-0 {tab === 'lyrics'
					? 'text-primary'
					: 'text-muted-foreground'}"
				onclick={() => (tab = tab === 'lyrics' ? 'queue' : 'lyrics')}
				aria-label={tab === 'lyrics' ? t('player.queue') : t('player.lyrics')}
				aria-pressed={tab === 'lyrics'}
			>
				<HugeiconsIcon icon={Mic01Icon} class="h-4 w-4" />
			</button>
		</div>
	</div>
</div>
