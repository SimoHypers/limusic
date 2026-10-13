<script lang="ts">
	// Theater mode: the window goes fullscreen and the app is replaced by one thing: the cover and
	// the controls on the left, the lyrics and/or the queue on the right (#297). Mounted by +layout
	// behind `ui.theaterOpen`, so mounting is what turns fullscreen on and unmounting is what turns it back off. That keeps
	// the flag the only switch: nothing else has to remember to undo the window state.
	//
	// PERFORMANCE, read before adding anything pretty. WebKitGTK re-runs a filter for the *damaged*
	// region on every repaint, and the region here holds lyrics that repaint every frame (the
	// karaoke sweep). Everything below has already been measured against that once, so the bans are
	// the design, not a preference:
	//   - no `filter` / `backdrop-filter` on anything full-screen (the cover wash is baked into a
	//     small canvas once per track instead, see `artroom.svelte.ts`, and upscaled, which is free);
	//   - no `mask-image` on the lyrics scroller (buffers the whole thing offscreen per frame);
	//   - no large-radius `box-shadow` over the backdrop (re-blurred on every repaint under it);
	//   - nothing full-screen animates. A moving viewport-sized layer damages the whole viewport
	//     every frame and drags every other repaint with it.
	// Depth comes from radial gradients, which are painted fills and cost nothing.
	import { onMount } from 'svelte';
	import { MediaQuery } from 'svelte/reactivity';
	import { beforeNavigate } from '$app/navigation';
	import { fade, fly, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Cancel01Icon,
		FavouriteIcon,
		Mic01Icon,
		MusicNote01Icon,
		NextIcon,
		PauseIcon,
		PlayIcon,
		PreviousIcon,
		Queue01Icon,
		RepeatIcon,
		RepeatOne01Icon,
		ShuffleIcon,
		VolumeHighIcon,
		VolumeMute02Icon
	} from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import KaraokeButton from './KaraokeButton.svelte';
	import {
		commitVolume,
		cycleRepeat,
		dragVolume,
		playback,
		toggleMute,
		toggleNowPlayingRating,
		ui,
		wheelVolume
	} from '$lib/player.svelte';
	import { artRoom } from '$lib/artroom.svelte';
	import { appearance, setAppearance } from '$lib/theme.svelte';
	import { isMissingThumb, thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import ArtistLine from './ArtistLine.svelte';
	import LyricsView from './LyricsView.svelte';
	import QueueList from './QueueList.svelte';

	const close = () => (ui.theaterOpen = false);

	// Clicking an artist goes to their page, and a fullscreen view over it would just hide it. Same
	// rule the now-playing view and the expanded lyrics panel use: navigating anywhere closes this.
	beforeNavigate(close);

	// Fullscreen goes through Rust, not `getCurrentWindow().setFullscreen`: on Windows the window
	// has to be un-maximized first and its frame recalculated after, in that order and on the main
	// thread, or it lands under the taskbar with a border around it (#139). Rust also puts the
	// maximized state back when theater closes.
	onMount(() => {
		api.theaterFullscreen(true).catch((e) => console.error('theater fullscreen failed', e));
		return () => {
			api.theaterFullscreen(false).catch(() => {});
		};
	});

	// Escape is the way out that needs no chrome on screen, which is what lets the chrome hide.
	function onKey(e: KeyboardEvent) {
		// A dialog or menu open over this one (the palette, the shortcut sheet, a track menu) already
		// took the key: bits-ui listens on the document and preventDefaults it, and its Escape should
		// only close itself, not the view underneath.
		if (e.defaultPrevented) return;
		if (e.key === 'Escape') {
			e.preventDefault();
			close();
		}
	}

	// The top strip (source line + exit) fades out after a few still seconds and the pointer goes
	// with it, the way every full-screen player does it. Everything you might actually want to
	// click stays put; this is only the frame around it. Any pointer movement brings it back.
	let idle = $state(false);
	let idleTimer: ReturnType<typeof setTimeout>;
	function wake() {
		idle = false;
		clearTimeout(idleTimer);
		idleTimer = setTimeout(() => (idle = true), 3500);
	}
	onMount(() => {
		wake();
		return () => clearTimeout(idleTimer);
	});

	// Same stepped-down artwork as the now-playing view: Google's CDN doesn't serve every rewritten
	// size for every image, and at this size a broken glyph is the whole screen.
	let attempt = $state(0);
	$effect(() => {
		playback.now?.thumbnail; // re-arm on every track change
		attempt = 0;
	});
	const srcs = $derived([720, 400, 120].map((px) => thumb(playback.now?.thumbnail, px)));
	const src = $derived(srcs[attempt]);

	// The cover's colour and its baked wash (`artroom.svelte.ts`, shared with the player view).
	const room = artRoom();

	const fmt = (secs: number) => {
		if (!secs || secs < 0) return '0:00';
		const s = Math.floor(secs);
		const h = Math.floor(s / 3600);
		const m = Math.floor((s % 3600) / 60);
		const mm = h ? String(m).padStart(2, '0') : `${m}`;
		return `${h ? `${h}:` : ''}${mm}:${String(s % 60).padStart(2, '0')}`;
	};

	// Seek, same hold-while-dragging trick as the player bar: incoming mpv ticks must not yank the
	// thumb back out from under the pointer.
	let seekDrag = $state<number | null>(null);
	const shownPosition = $derived(seekDrag ?? playback.position);
	const pct = $derived(playback.duration ? (shownPosition / playback.duration) * 100 : 0);

	const shuffleOn = $derived(playback.queue.shuffle ?? false);
	const repeat = $derived(playback.queue.repeat ?? 'off');
	const local = $derived(!!playback.now && api.isLocalId(playback.now.videoId));
	// Album isn't in now-playing, but the queue row usually carries it. Matched on videoId so a
	// mid-advance mismatch can't label the cover with the previous track's album.
	const album = $derived.by(() => {
		const cur = playback.queue.items[playback.queue.currentIndex];
		return cur?.video_id === playback.now?.videoId ? cur?.album : null;
	});

	// The slider is revealed by hovering the control, and stays out for as long as it is being
	// dragged: hover alone can't say the second part, since a pointer that wanders off the strip
	// mid-drag would collapse the slider under its own thumb.
	let volHover = $state(false);
	let volDragging = $state(false);
	// Scrolling the cover is the volume, as in Now Playing, and the slider opens while it is live
	// (plus a second to read it). Only the cover: on the whole view, scrolling the lyrics or the
	// empty backdrop changed the volume too (#344).
	let volWheel = $state(false);
	let volWheelTimer: ReturnType<typeof setTimeout>;
	function onArtWheel(e: WheelEvent) {
		wheelVolume(e);
		volWheel = true;
		clearTimeout(volWheelTimer);
		volWheelTimer = setTimeout(() => (volWheel = false), 1000);
	}
	const volOpen = $derived(volHover || volDragging || volWheel);

	// Lyrics and queue, each a sticky switch (lyrics on by default). Both fit beside the player only
	// from 90rem: below that the lyrics column would be ~20 characters wide at theater size, so the
	// two share one column and turning one on turns the other off. A pair saved on a wider screen
	// shows the queue here, the one that had to be asked for. Neither on, the player centres itself.
	const roomy = new MediaQuery('(min-width: 90rem)');
	const queueShown = $derived(appearance.theaterQueue);
	const lyricsShown = $derived(appearance.theaterLyrics && (!queueShown || roomy.current));
	const both = $derived(lyricsShown && queueShown);
	function toggleLyrics() {
		const on = !lyricsShown;
		setAppearance({ theaterLyrics: on, ...(on && !roomy.current && { theaterQueue: false }) });
	}
	function toggleQueue() {
		const on = !queueShown;
		setAppearance({ theaterQueue: on, ...(on && !roomy.current && { theaterLyrics: false }) });
	}
	// The queue hugs the player rather than taking the elastic half: a track row has nothing to do
	// with 60rem of width. Three columns tighten the gap so the lyrics keep what they can.
	const band = $derived(
		both
			? 'gap-12 grid-cols-[minmax(18rem,0.8fr)_minmax(0,1.2fr)_minmax(0,22rem)]'
			: lyricsShown
				? 'gap-10 xl:gap-20 lg:grid-cols-[minmax(20rem,0.85fr)_minmax(0,1.15fr)]'
				: queueShown
					? 'gap-10 xl:gap-20 lg:grid-cols-[minmax(20rem,32rem)_minmax(0,36rem)] lg:justify-center'
					: 'gap-10 xl:gap-20'
	);
	let justLiked = $state(false);
	function toggleLike() {
		if (playback.rating !== 'like') justLiked = true;
		toggleNowPlayingRating();
	}
</script>

<svelte:window onkeydown={onKey} onpointerup={() => (volDragging = false)} />

<!-- Above the whole app including the titlebar (the chrome tops out at z-20): fullscreen means
     fullscreen. Below the dialogs and menus (z-50 and up) and the toast/update banners (z-100),
     which have to stay reachable from in here: Ctrl+K opened the palette behind this view. -->
<!-- svelte-ignore a11y_no_static_element_interactions -- move only wakes the chrome -->
<section
	transition:fade={{ duration: 220 }}
	onpointermove={wake}
	class="theater fixed inset-0 z-40 flex flex-col overflow-hidden bg-background text-foreground {idle
		? 'cursor-none'
		: ''}"
>
	<!-- === Backdrop, three layers, none of which repaint with the content over them ===
	     1. the cover, pre-blurred into a 48px bitmap and upscaled. No live filter. Behind the
	        artwork-background setting, so it is still a one-click ablation. -->
	{#if appearance.artworkBackground && room.wash}
		{#key room.wash}
			<!-- Stretched to the window (`100% 100%`) rather than cropped to it. The source is a
			     square and the window is not, so `cover` throws away the top and bottom of the
			     cover's colour; at this blur radius nobody can see that it has been stretched. -->
			<div
				in:fade={{ duration: 700 }}
				style="background-image:url({room.wash});background-size:100% 100%"
				class="pointer-events-none absolute inset-0 opacity-50 dark:opacity-60"
			></div>
		{/key}
	{/if}
	<!-- 2. the room light: three soft blobs off the cover's own colour. Painted fills, no filter,
	        and STATIC. An earlier version drifted this on a keyframe; a full-viewport element moving
	        every frame damages the whole screen every frame, which drags every repaint on it (and
	        re-ran the wash's filter, back when the wash had one). Nothing here animates. -->
	<div class="pointer-events-none absolute -inset-[15%]" style="background-image:{room.mesh}"></div>
	<!-- 3. the vignette that puts the two columns back in the middle and keeps text legible over
	        whatever the cover happened to be. -->
	<div
		class="pointer-events-none absolute inset-0 bg-[radial-gradient(115%_90%_at_50%_42%,transparent_25%,var(--background)_100%)]"
	></div>
	<div
		class="pointer-events-none absolute inset-x-0 bottom-0 h-40 bg-gradient-to-t from-background to-transparent"
	></div>

	<!-- === Top strip. Where the queue came from on the left, the way out on the right. Fades with
	     the pointer; Esc works whether it is on screen or not. === -->
	<header
		class="relative z-10 flex shrink-0 items-center justify-between px-8 py-5 transition-opacity duration-500 xl:px-14 {idle
			? 'opacity-0'
			: 'opacity-100'}"
	>
		<div class="min-w-0">
			<p class="text-[10px] font-semibold uppercase tracking-[0.22em] text-muted-foreground">
				{playback.queue.sourceName ? t('player.playing_from') : t('player.now_playing')}
			</p>
			{#if playback.queue.sourceName}
				<p class="mt-1 truncate text-sm font-medium">{playback.queue.sourceName}</p>
			{/if}
		</div>
		<button
			onclick={close}
			class="flex h-10 w-10 cursor-pointer items-center justify-center rounded-full border border-border/50 bg-card/70 text-muted-foreground transition-colors hover:bg-card hover:text-foreground"
			title="{t('player.exit_theater')} (Esc)"
			aria-label={t('player.exit_theater')}
		>
			<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
		</button>
	</header>

	<!-- === The band. Up to three tracks, all vertically centred against the same height, so no
	     column dangles. Lyrics take the elastic share: they are the thing you read. Below lg there
	     is no room for two and the player wins. === -->
	<div
		class="relative z-10 mx-auto grid min-h-0 w-full max-w-[104rem] flex-1 grid-rows-[minmax(0,1fr)] px-8 pb-10 xl:px-14 {band}"
	>
		<!-- --art: the cover's side, whichever is smaller of the column and the height left once the
		     top strip, the meta and the controls have taken theirs.
		     ponytail: 25rem is those measured, not computed; raise it if the controls block grows. -->
		<div
			class="mx-auto w-full self-center {lyricsShown || queueShown ? 'max-w-[30rem]' : 'max-w-[34rem]'}"
			style="--art:min(100%, 100vh - 25rem)"
		>
			<!-- svelte-ignore a11y_no_static_element_interactions -- wheel is the volume gesture -->
			<div class="relative mx-auto" style="width:var(--art);max-width:100%" onwheel={onArtWheel}>
				<!-- The light the cover sits in. Bigger than the cover and behind it, so it reads as a
				     spill rather than an outline. -->
				<div
					class="pointer-events-none absolute -inset-[12%] -z-10 opacity-70"
					style="background-image:{room.glow}"
				></div>
				{#key playback.now?.videoId}
					<div in:scale={{ start: 0.94, duration: 420, easing: cubicOut }} class="relative">
						{#if src && attempt < srcs.length}
							<img
								{src}
								alt=""
								onerror={() => attempt++}
								onload={(e) => isMissingThumb(e) && attempt++}
								style={srcs[2] ? `background-image:url(${srcs[2]})` : undefined}
								class="aspect-square w-full rounded-2xl bg-cover object-cover ring-1 ring-white/10"
							/>
						{:else}
							<div
								class="flex aspect-square w-full items-center justify-center rounded-2xl bg-muted text-muted-foreground/40 ring-1 ring-white/10"
							>
								<HugeiconsIcon icon={MusicNote01Icon} class="h-20 w-20" />
							</div>
						{/if}
						<!-- A hairline of light along the top edge: the one thing that stops a flat square
						     from looking pasted on. Inset ring, so it costs a border and not a shadow. -->
						<div
							class="pointer-events-none absolute inset-0 rounded-2xl ring-1 ring-inset ring-white/10"
						></div>
					</div>
				{/key}

				<!-- Volume, on the art rather than in the layout: a slider sitting on its own under the
				     transport was a second horizontal bar competing with the scrubber. Collapsed to
				     the icon until the pointer is on it, same as the mini player, and it grows to the
				     right because it is pinned to the left edge.
				     In flow next to its icon with no gap, so the wrapper's box covers both: absolute
				     with a margin, the pointer leaves the hover target on its way to the slider and
				     the slider collapses before it gets there. -->
				<div
					class="absolute left-3 top-3 z-10 flex items-center rounded-full bg-black/40 px-1.5 py-1 text-white"
					role="group"
					aria-label={t('player.volume')}
					onpointerenter={() => (volHover = true)}
					onpointerleave={() => (volHover = false)}
				>
					<button
						class="flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-full text-white/75 transition-colors hover:text-white"
						onclick={toggleMute}
						aria-label={playback.volume === 0 ? t('player.unmute') : t('player.mute')}
					>
						<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
						<HugeiconsIcon
							icon={VolumeHighIcon}
							altIcon={VolumeMute02Icon}
							showAlt={playback.volume === 0}
							class="h-4 w-4"
						/>
					</button>
					<!-- min-w-0: a flex item defaults to min-width:auto and a range input's intrinsic
					     width is not zero, so without it the slider never actually collapses. -->
					<input
						type="range"
						class="range on-art min-w-0 transition-[width,opacity,margin] duration-150 {volOpen
							? 'ml-1.5 mr-1 w-24 opacity-100'
							: 'w-0 opacity-0'}"
						style="--pct:{playback.volume}%"
						min="0"
						max="100"
						value={playback.volume}
						onpointerdown={() => (volDragging = true)}
						oninput={(e) => dragVolume(Number(e.currentTarget.value))}
						onchange={(e) => commitVolume(Number(e.currentTarget.value))}
						aria-label={t('player.volume')}
					/>
				</div>
			</div>

			<!-- Meta. Title, artists, album — three steps of weight and colour, so the eye lands on the
			     title first and the album never competes with it. -->
			<div class="mt-8 flex items-start gap-4">
				<div class="min-w-0 flex-1">
					<h1
						class="truncate font-heading text-[1.75rem] font-bold leading-tight tracking-tight xl:text-4xl"
						title={playback.now?.title}
					>
						{playback.now?.title ?? t('player.not_playing')}
					</h1>
					<ArtistLine
						runs={playback.now?.artistRuns}
						text={playback.now?.artists ?? ''}
						class="mt-2 block text-base text-foreground/70"
					/>
					{#if album}
						<p class="mt-0.5 truncate text-[13px] text-muted-foreground">{album}</p>
					{/if}
				</div>
				<div class="flex shrink-0 items-center gap-1 pt-1.5">
					<!-- What sits beside the player, as one pill so it reads as a view switch rather than
					     two more actions next to like. Hidden below lg, where there is no second column. -->
					<div class="mr-1 hidden items-center gap-0.5 rounded-full bg-foreground/[0.06] p-1 lg:flex">
						{@render column(Mic01Icon, t('player.lyrics'), lyricsShown, toggleLyrics)}
						{@render column(Queue01Icon, t('player.queue'), queueShown, toggleQueue)}
					</div>
					{#if playback.now && !local}
						<button
							onclick={toggleLike}
							class="flex h-10 w-10 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground"
							aria-label={t('common.like')}
						>
							<span
								class="inline-flex"
								class:animate-heart-pop={justLiked}
								onanimationend={() => (justLiked = false)}
							>
								<HugeiconsIcon
									icon={FavouriteIcon}
									class="h-[18px] w-[18px] {playback.rating === 'like'
										? 'fill-current text-primary'
										: ''}"
								/>
							</span>
						</button>
					{/if}
				</div>
			</div>

			<!-- Scrubber. Times under the ends rather than beside them, so the bar runs the full width
			     of the cover and stays the widest thing on screen to aim at. -->
			<div class="mt-7">
				<input
					type="range"
					class="range theater-range w-full"
					style="--pct:{pct}%"
					min="0"
					max={playback.duration || 0}
					value={shownPosition}
					oninput={(e) => (seekDrag = Number(e.currentTarget.value))}
					onchange={(e) => {
						const v = Number(e.currentTarget.value);
						playback.position = v;
						seekDrag = null;
						api.seek(v);
					}}
					aria-label={t('player.seek')}
				/>
				<div class="mt-2 flex justify-between text-xs font-medium tabular-nums text-muted-foreground">
					<span>{fmt(shownPosition)}</span>
					<span>{fmt(playback.duration)}</span>
				</div>
			</div>

			<!-- Transport. Bigger than the player bar's: at arm's length this is the only control
			     surface on screen, and play is the one you reach for without looking. -->
			<div class="mt-6 flex items-center justify-center gap-2 xl:gap-3">
				<button
					onclick={() => api.toggleShuffle()}
					class="flex h-10 w-10 cursor-pointer items-center justify-center rounded-full transition-colors hover:bg-foreground/10 {shuffleOn
						? 'text-primary'
						: 'text-muted-foreground hover:text-foreground'}"
					aria-label={t('player.shuffle')}
					aria-pressed={shuffleOn}
				>
					<HugeiconsIcon icon={ShuffleIcon} class="h-[18px] w-[18px]" />
				</button>
				<KaraokeButton size={18} />
				<button
					onclick={() => api.prevTrack()}
					class="flex h-12 w-12 cursor-pointer items-center justify-center rounded-full text-foreground/90 transition-colors hover:bg-foreground/10 hover:text-foreground"
					aria-label={t('player.previous')}
				>
					<HugeiconsIcon icon={PreviousIcon} class="h-6 w-6" />
				</button>
				<button
					onclick={() => api.togglePause()}
					class="mx-1 flex h-[68px] w-[68px] cursor-pointer items-center justify-center rounded-full bg-primary text-primary-foreground transition-transform duration-150 hover:scale-[1.06] active:scale-95"
					aria-label={playback.paused ? t('player.play') : t('player.pause')}
				>
					<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
					<HugeiconsIcon
						icon={PauseIcon}
						altIcon={PlayIcon}
						showAlt={playback.paused}
						class="h-7 w-7"
					/>
				</button>
				<button
					onclick={() => api.nextTrack()}
					class="flex h-12 w-12 cursor-pointer items-center justify-center rounded-full text-foreground/90 transition-colors hover:bg-foreground/10 hover:text-foreground"
					aria-label={t('player.next')}
				>
					<HugeiconsIcon icon={NextIcon} class="h-6 w-6" />
				</button>
				<button
					onclick={cycleRepeat}
					class="flex h-10 w-10 cursor-pointer items-center justify-center rounded-full transition-colors hover:bg-foreground/10 {repeat !==
					'off'
						? 'text-primary'
						: 'text-muted-foreground hover:text-foreground'}"
					aria-label={t('player.repeat_state', {
						state:
							repeat === 'off'
								? t('player.repeat_off')
								: repeat === 'one'
									? t('player.repeat_one')
									: t('player.repeat_all')
					})}
					aria-pressed={repeat !== 'off'}
				>
					<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
					<HugeiconsIcon
						icon={RepeatIcon}
						altIcon={RepeatOne01Icon}
						showAlt={repeat === 'one'}
						class="h-[18px] w-[18px]"
					/>
				</button>
			</div>
		</div>

		<!-- === Lyrics, straight on the backdrop. No card: the wash is already dim and static behind
		     them, and a panel on a screen that has no other chrome is the one thing that made this
		     look like a page instead of a stage.
		     Never give this a `mask-image` for the end fades, however tempting: it buffers the whole
		     scroller offscreen on every scroll frame. Same for `backdrop-filter` (see the header).
		     The scrollbar is hidden from out here, since the child owns the scroller. === -->
		{#if lyricsShown}
			<!-- `in:` only, never `transition:`. An out transition keeps this in the DOM after the grid
			     has already dropped to one column, so it lands as a second *row* and the player
			     centres itself in two visible steps: sideways now, downwards 400ms later. -->
			<div
				in:fly={{ y: 24, duration: 400, easing: cubicOut }}
				class="hidden h-full min-h-0 flex-col overflow-hidden lg:flex [&_*]:[scrollbar-width:none] [&_*::-webkit-scrollbar]:hidden"
			>
				<LyricsView expanded />
			</div>
		{/if}
		<!-- === The queue, on the backdrop like the lyrics and for the same reason. It keeps its
		     scrollbar: a playlist queue runs to thousands of rows, and it is the only thing here you
		     scroll by hand.
		     `relative z-0` gives it its own stacking context, and that is what keeps the scrollbar on
		     screen. Without it, every transform transition in the lyrics next door (the word pop, the
		     active line's scale) blanked the thumb on WebKitGTK for as long as it ran, 100-300 ms each,
		     several times a second through a word-synced song (#343). perf/theaterprobe.py, real
		     screen captures: thumb missing in 10 of 30 before, 0 of 30 after. === -->
		{#if queueShown}
			<div
				in:fly={{ y: 24, duration: 400, easing: cubicOut }}
				class="relative z-0 hidden h-full min-h-0 flex-col overflow-hidden lg:flex"
			>
				<QueueList />
			</div>
		{/if}
	</div>
</section>

{#snippet column(icon: typeof Mic01Icon, label: string, on: boolean, toggle: () => void)}
	<button
		onclick={toggle}
		class="flex size-8 cursor-pointer items-center justify-center rounded-full transition-colors {on
			? 'bg-foreground/10 text-primary'
			: 'text-muted-foreground hover:bg-foreground/10 hover:text-foreground'}"
		aria-label={label}
		aria-pressed={on}
		title={label}
	>
		<HugeiconsIcon {icon} class="h-4 w-4" />
	</button>
{/snippet}
