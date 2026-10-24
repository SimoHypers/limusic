<script lang="ts">
	// Theater mode: the window goes fullscreen and the app is replaced by a stage. The stage holds
	// the picture (the cover, or the music video) and the words, nothing else; every control lives
	// in the dock along the bottom, and the dock, the top strip and the pointer fade out after a few
	// still seconds while the music plays. Beside the picture, the lyrics, the queue, or both (#297).
	// Neither is the spotlight, the picture alone in the middle of the screen.
	// Mounted by +layout behind `ui.theaterOpen`, so mounting is what turns fullscreen on and
	// unmounting is what turns it back off. That keeps the flag the only switch: nothing else has
	// to remember to undo the window state.
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
	import { beforeNavigate, goto } from '$app/navigation';
	import { fade, fly, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Cancel01Icon,
		FavouriteIcon,
		FullScreenIcon,
		Mic01Icon,
		MinimizeScreenIcon,
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
	import {
		commitVolume,
		cycleRepeat,
		dragVolume,
		openAddToPlaylist,
		playback,
		prefs,
		toggleMute,
		toggleNowPlayingRating,
		ui,
		wheelVolume
	} from '$lib/player.svelte';
	import {
		canSwitch,
		cinema as cinemaOn,
		claimVideo,
		parkVideo,
		pickVersion,
		setHole,
		showVideo,
		video,
		videoSide
	} from '$lib/video.svelte';
	import { artRoom } from '$lib/artroom.svelte';
	import { appearance, setAppearance } from '$lib/theme.svelte';
	import { isMissingThumb, thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import { keyOf, typing } from '$lib/shortcuts';
	import ArtistLine from './ArtistLine.svelte';
	import LyricsView from './LyricsView.svelte';
	import QueueList from './QueueList.svelte';
	import TrackMenu from './TrackMenu.svelte';
	import Ambient from './Ambient.svelte';

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
			// Cinema is per opening, not a setting: theater always starts in its normal layout.
			video.cinema = false;
		};
	});

	// Calm: after a few still seconds the dock, the top strip and the pointer fade out and the stage
	// is all that is left, the way every full-screen player does it. Not while paused (a paused
	// screen with no controls on it reads as frozen) and not while the pointer rests on the dock.
	// Any movement or key brings it all back; Esc works whether the chrome is on screen or not.
	let idle = $state(false);
	let overDock = $state(false);
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
	const calm = $derived(idle && !playback.paused && !overDock);
	// Cinema: the picture fills the screen, the title, lyrics and queue step aside and the header
	// and dock float over it.
	const cinema = $derived(cinemaOn());
	// Over a video the ink has to be light whatever the theme: the scrims under the header and the
	// dock are black. Only the neutrals change, so the accent (custom, artwork or the theme's own)
	// stays what it is everywhere else. `.dark` would reset it to the default palette's.
	const CINEMA_INK =
		'[--foreground:oklch(0.985_0_0)] [--muted-foreground:oklch(0.985_0_0/0.8)] [--background:oklch(0.145_0_0)] [--muted:oklch(1_0_0/0.3)]';
	function toggleCinema() {
		video.cinema = !video.cinema;
		wake();
	}

	// The dock floats over the bottom of the stage. Once it has faded out, the lyrics and the queue
	// grow down into its room until they end as far from the bottom edge as they start from the top
	// (the header's height), instead of stopping over an empty band (#468). Not to the edge: that cut
	// lines in half against the progress line. The picture keeps its size. Growing the columns at
	// the bottom moves nothing already on screen, it only shows more of them. Waits out the dock's
	// fade so the two never overlap, and gives the room back the moment the dock comes back.
	let dockH = $state(0);
	let roomFree = $state(false);
	$effect(() => {
		if (!calm) {
			roomFree = false;
			return;
		}
		const timer = setTimeout(() => (roomFree = true), 500);
		return () => clearTimeout(timer);
	});

	// A focused slider (the seek bar or volume, after a click) is not a text field: F must still
	// work there.
	const textTarget = (el: EventTarget | null) =>
		typing(el) && !(el instanceof HTMLInputElement && el.type === 'range');

	function onKey(e: KeyboardEvent) {
		wake();
		// A dialog or menu open over this one (the palette, the shortcut sheet, a track menu) already
		// took the key: bits-ui listens on the document and preventDefaults it, and its Escape should
		// only close itself, not the view underneath.
		if (e.defaultPrevented) return;
		if (e.key === 'Escape') {
			e.preventDefault();
			close();
		}
		// F toggles cinema. keyOf reads the physical key for non-Latin layouts, like the app-wide
		// shortcuts do.
		if (
			keyOf(e) === 'f' &&
			!e.ctrlKey &&
			!e.metaKey &&
			!e.altKey &&
			!e.repeat &&
			showVideo() &&
			!textTarget(e.target)
		) {
			e.preventDefault();
			toggleCinema();
		}
	}

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
	// Display playback seconds at the selected tempo; mpv seeks still take track seconds.
	const shownPosition = $derived((seekDrag ?? playback.position) / playback.speed);
	const shownDuration = $derived(playback.duration / playback.speed);
	const pct = $derived(shownDuration ? (shownPosition / shownDuration) * 100 : 0);

	const shuffleOn = $derived(playback.queue.shuffle ?? false);
	const repeat = $derived(playback.queue.repeat ?? 'off');
	const local = $derived(!!playback.now && api.isLocalId(playback.now.videoId));
	// The queue row for what's playing: it carries the album link and what the ⋮ menu needs, which
	// the now-playing event does not. Matched on videoId so a mid-advance mismatch can't point
	// either at the previous song.
	const currentSong = $derived.by(() => {
		const cur = playback.queue.items[playback.queue.currentIndex];
		return cur?.video_id === playback.now?.videoId ? cur : null;
	});
	const album = $derived(playback.now?.album ?? currentSong?.album);
	const albumId = $derived(!local ? currentSong?.album_id : undefined);

	// Lyrics and queue, each a sticky switch (lyrics on by default). Below lg there is no room for
	// either beside a picture this size, so the spotlight is all there is and the switches hide.
	// Both fit only from 90rem: below that the lyrics column would be ~20 characters wide at this
	// size, so the two share one column and turning one on turns the other off. A pair saved on a
	// wider screen shows the queue here, the one that had to be asked for.
	const wide = new MediaQuery('(min-width: 64rem)');
	const roomy = new MediaQuery('(min-width: 90rem)');
	// A short screen (768 and 800px laptops) gets a slimmer dock and top strip: at full size the
	// two took a third of the height away from the picture and the lyrics (#469).
	const short = new MediaQuery('(max-height: 50rem)');
	const headerH = $derived(short.current ? '4rem' : '5rem');
	const queueShown = $derived(wide.current && appearance.theaterQueue);
	const lyricsShown = $derived(
		wide.current && appearance.theaterLyrics && (!queueShown || roomy.current)
	);
	// A song with nothing to read (no lyrics found, or instrumental) gets the spotlight instead of a
	// half-screen column with one line at its top. The lyrics stay mounted, hidden, to fetch the next
	// song's, and the layout moves only once a fetch has answered, never for the loading in between.
	// Forgotten while no lyrics are mounted to keep it current.
	let noLyrics = $state(false);
	$effect(() => {
		if (!lyricsShown) noLyrics = false;
	});
	const lyricsColumn = $derived(lyricsShown && !noLyrics);
	// The column rises in when it appears, as the queue does with `in:fly`, and again each time a
	// song with lyrics follows one without, which `in:` can't do for a column that was only hidden.
	// Transform and opacity only (tw-animate's `animate-in` also animates `filter`).
	let lyricsCol: HTMLElement | undefined = $state();
	$effect(() => {
		if (lyricsColumn && lyricsCol)
			lyricsCol.animate([{ opacity: 0, transform: 'translateY(24px)' }, {}], {
				duration: 400,
				easing: 'cubic-bezier(0.33, 1, 0.68, 1)' // cubicOut
			});
	});
	const both = $derived(lyricsColumn && queueShown);
	/** A column beside the picture: it moves over to the left and its title gets smaller. */
	const columns = $derived(lyricsColumn || queueShown);
	function toggleLyrics() {
		const on = !lyricsShown;
		setAppearance({ theaterLyrics: on, ...(on && !roomy.current && { theaterQueue: false }) });
	}
	function toggleQueue() {
		const on = !queueShown;
		setAppearance({ theaterQueue: on, ...(on && !roomy.current && { theaterLyrics: false }) });
	}

	// The picture's side. A square spends whichever is smaller of its column's width and the height
	// the title block leaves; a video spends that height at 16:9 instead. The spotlight is bigger
	// and its title is set larger, so it budgets more for it.
	// ponytail: 8rem / 6.5rem are the title blocks at their largest, measured, not computed. Raise
	// them if the block under the picture grows.
	// A title too long for one line gets a second instead of an ellipsis (the "feat." credits are
	// what the ellipsis used to eat), and only then does the picture give up a line's height for it:
	// 2.25rem / 3rem at leading 1.15. Measured at one line (does it overflow?) or at two (did it
	// wrap?), so the answer can't flip back and forth: the room it takes only narrows the line.
	let titleEl: HTMLElement | undefined = $state();
	let twoLines = $state(false);
	$effect(() => {
		const el = titleEl;
		if (!el) return;
		const ro = new ResizeObserver(() => {
			twoLines = twoLines
				? el.scrollHeight > parseFloat(getComputedStyle(el).lineHeight) * 1.5
				: el.scrollWidth > el.clientWidth + 1;
		});
		ro.observe(el);
		return () => ro.disconnect();
	});
	const titleRoom = $derived(
		`${columns ? 6.5 : 8}rem - ${twoLines ? (columns ? 2.6 : 3.5) : 0}rem`
	);
	const side = $derived(
		columns
			? showVideo()
				? `min(100cqw, (100cqh - ${titleRoom}) * 16 / 9)`
				: `min(100cqw, 100cqh - ${titleRoom}, 34rem)`
			: showVideo()
				? `min(100cqw, (100cqh - ${titleRoom}) * 16 / 9, 96rem)`
				: `min(100cqw, 100cqh - ${titleRoom}, 44rem)`
	);

	// Clicking the picture toggles playback and flashes the action just taken over it. Read `paused`
	// before the toggle: the event that flips it is a round trip away.
	let flash: 'play' | 'pause' | null = $state(null);
	let flashTimer: ReturnType<typeof setTimeout>;
	function toggle() {
		flash = playback.paused ? 'play' : 'pause';
		clearTimeout(flashTimer);
		flashTimer = setTimeout(() => (flash = null), 220);
		api.togglePause();
	}

	// Scrolling the picture is the volume, as in Now Playing. Only the picture: on the whole view,
	// scrolling the lyrics or the empty backdrop changed the volume too (#344). It wakes the dock,
	// whose slider then shows the level.
	function onArtWheel(e: WheelEvent) {
		wheelVolume(e);
		wake();
	}

	let justLiked = $state(false);
	function toggleLike() {
		if (playback.rating !== 'like') justLiked = true;
		toggleNowPlayingRating();
	}

	// The last seconds of a track say what comes after it, in a corner, so a screen nobody is
	// touching still answers "what's next". Not with the queue on screen, which already says it,
	// and not on repeat-one, where nothing does come next.
	const next = $derived.by(() => {
		if (repeat === 'one') return null;
		const { items, currentIndex } = playback.queue;
		return items[currentIndex + 1] ?? (repeat === 'all' ? items[0] : null);
	});
	const upNext = $derived(
		!queueShown &&
			playback.duration > 30 &&
			playback.duration - playback.position <= 15
			? next
			: null
	);

	/** The picture's box, for the ambient light to glow around. */
	let picBox = $state<HTMLElement | null>(null);

	// Linux and Windows: mpv draws the music video under the page and the box below is a hole it
	// shows through (see `setHole` and the player view, which does the same). Not while the view
	// fades in: +layout hides the app under theater once there is a hole, and doing that while the
	// fade still shows the app through would flash the desktop. Taken away when the fade out starts,
	// for the same reason, and only by this view: by the time this is destroyed the player view
	// underneath may already hold the hole again.
	let view: HTMLElement;
	let settled = $state(false);
	let released = false;
	let holding = false;
	$effect(() => {
		// Mounted without a fade (none is coming), so no introend either.
		const frame = requestAnimationFrame(() => {
			if (!view.getAnimations().length) settled = true;
		});
		return () => cancelAnimationFrame(frame);
	});
	function leave() {
		released = true;
		if (holding) setHole(null);
	}
	function holeFor(el: HTMLElement) {
		if (!settled) return;
		holding = true;
		const measure = () => {
			if (released) return;
			if (document.hidden) return setHole(null);
			const r = el.getBoundingClientRect();
			setHole({ x: r.left, y: r.top, w: r.width, h: r.height });
		};
		// The box, the window (fullscreen lands after mount), and the picture's column, whose size
		// changes with every switch of the side column and is what moves the box without resizing it.
		const ro = new ResizeObserver(measure);
		ro.observe(el);
		ro.observe(document.documentElement);
		const col = el.closest('[data-theater-pic]');
		if (col) ro.observe(col);
		document.addEventListener('visibilitychange', measure);
		return () => {
			ro.disconnect();
			document.removeEventListener('visibilitychange', measure);
			holding = false;
			if (!released) setHole(null);
		};
	}
</script>

<svelte:window onkeydown={onKey} onpointermove={wake} />

<!-- Above the whole app including the titlebar (the chrome tops out at z-20): fullscreen means
     fullscreen. Below the dialogs and menus (z-50 and up) and the toast/update banners (z-100),
     which have to stay reachable from in here: Ctrl+K opened the palette behind this view.
     Transparent while mpv's picture shows through it; `inert:` is the fade out, when Svelte stops
     updating classes here and the background has to come back on its own. -->
<section
	bind:this={view}
	transition:fade={{ duration: 220 }}
	onintroend={() => (settled = true)}
	onoutrostart={leave}
	class="fixed inset-0 z-40 flex flex-col overflow-hidden text-foreground inert:bg-background {video.hole
		? ''
		: cinema
			? 'bg-black'
			: 'bg-background'} {cinema ? CINEMA_INK : ''} {calm ? 'cursor-none' : ''}"
>
	{#if showVideo()}
		<!-- The video lights the room itself (Settings > Video > ambient light), and nothing else
		     does: on Linux every layer here would paint over mpv's picture. -->
		{#if prefs.ambient && picBox && !cinema}<Ambient box={picBox} />{/if}
	{:else}
		<!-- === Backdrop, three layers, none of which repaint with the content over them ===
		     1. the cover, pre-blurred into a small bitmap once per track and upscaled. No live
		        filter. Behind the artwork-background setting, so it is still a one-click ablation.
		        Stretched to the window (`100% 100%`) rather than cropped: at this blur nobody can see
		        the square has been stretched, and `cover` would throw away its top and bottom.
		        A new track's wash crossfades with the last one's: an entrance alone dropped the old
		        one at once and dipped the room to the bare background on every song. -->
		{#if appearance.artworkBackground && room.wash}
			{#key room.wash}
				<div
					transition:fade={{ duration: 700 }}
					style="background-image:url({room.wash});background-size:100% 100%;{room.washVars}"
					class="pointer-events-none absolute inset-0 opacity-[min(0.5,var(--wash-light))] dark:opacity-[min(0.6,var(--wash-dark))]"
				></div>
			{/key}
		{/if}
		<!-- 2. the room light: soft blobs off the cover's own colour. Painted fills and STATIC: an
		        earlier version drifted this on a keyframe, and a full-viewport element moving every
		        frame damages the whole screen every frame. -->
		<div class="pointer-events-none absolute -inset-[15%]" style="background-image:{room.mesh}"></div>
		<!-- 3. the vignette, centred on the picture, and the floor under the dock: the two things that
		        keep text legible over whatever the cover happened to be. -->
		<div
			class="pointer-events-none absolute inset-0"
			style="background-image:radial-gradient(110% 90% at {columns
				? '28%'
				: '50%'} 42%, transparent 25%, var(--background) 100%)"
		></div>
		<div
			class="pointer-events-none absolute inset-x-0 bottom-0 h-48 bg-gradient-to-t from-background to-transparent"
		></div>
	{/if}

	<!-- === Top strip: where the queue came from, and the way out. === -->
	<header
		class="{cinema
			? 'absolute inset-x-0 top-0 z-10 bg-gradient-to-b from-black/75 via-black/35 to-transparent'
			: 'relative z-10'} flex shrink-0 items-center justify-between gap-6 px-8 transition-opacity duration-500 xl:px-14 {calm
			? 'opacity-0'
			: ''}"
		style="height:{headerH}"
	>
		<div class="min-w-0">
			<!-- The queue column says this itself. -->
			{#if !queueShown}
				<p class="text-[10px] font-semibold uppercase tracking-[0.22em] text-muted-foreground">
					{playback.queue.sourceName ? t('player.playing_from') : t('player.now_playing')}
				</p>
				{#if playback.queue.sourceName}
					<p class="mt-1 truncate text-sm font-medium">{playback.queue.sourceName}</p>
				{/if}
			{/if}
		</div>
		<button
			type="button"
			onclick={close}
			class="flex size-10 shrink-0 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground"
			title="{t('player.exit_theater')} (Esc)"
			aria-label={t('player.exit_theater')}
		>
			<HugeiconsIcon icon={Cancel01Icon} class="h-5 w-5" />
		</button>
	</header>

	<!-- === The stage. The picture and what's playing, and beside it at most one column. ===
	     A stacking context of its own (z-10), so the light behind the picture (-z-10) stays above the
	     backdrop instead of sinking under it. -->
	{#if !cinema}
		<div
			class="relative z-10 mx-auto flex min-h-0 w-full max-w-[110rem] flex-1 gap-10 px-8 xl:px-14 {both
				? 'xl:gap-12'
				: 'xl:gap-20'}"
		>
			<!-- A size container, so the picture is sized against the space actually left for it
			     (cqw/cqh) instead of guessing at the window. -->
			<div
				class="flex min-w-0 flex-1 items-center justify-center [container-type:size]"
				style="padding-bottom:{dockH}px"
				data-theater-pic
			>
				<div style="--side:{side};width:var(--side)">
					<!-- svelte-ignore a11y_no_static_element_interactions -- wheel is the volume gesture -->
					<div bind:this={picBox} class="relative" onwheel={onArtWheel}>
						{#if !showVideo()}
							<!-- The light the cover sits in: bigger than it and behind it, so it reads as a
							     spill, where a shadow this size would be re-blurred on every repaint. -->
							<div
								class="pointer-events-none absolute -inset-[14%] -z-10 opacity-70"
								style="background-image:{room.glow}"
							></div>
						{/if}
						<button
							type="button"
							onclick={toggle}
							aria-label={t('a11y.play_pause')}
							class="relative block w-full cursor-pointer"
						>
							{#if flash}
								<!-- Transform and opacity only: no backdrop-blur on the plate. -->
								<div
									in:scale={{ start: 0.7, duration: 150, easing: cubicOut }}
									out:scale={{ start: 1.3, duration: 320, easing: cubicOut }}
									class="pointer-events-none absolute inset-0 z-10 flex items-center justify-center"
								>
									<div class="rounded-full bg-black/55 p-4 text-white">
										<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
										<HugeiconsIcon
											icon={PauseIcon}
											altIcon={PlayIcon}
											showAlt={flash === 'play'}
											class="h-8 w-8"
										/>
									</div>
								</div>
							{/if}
							<!-- The music video. VideoSurface owns the element (or mpv draws it, on Linux and
							     Windows) and this borrows it while theater is up, the same way the player
							     view does; the player view hands it over and takes it back. -->
							{#if prefs.nativeVideo}
								{#if showVideo()}
									<div
										class="aspect-video w-full rounded-2xl inert:bg-black {video.hole ? '' : 'bg-black'}"
										{@attach holeFor}
									></div>
								{/if}
							{:else}
								<div
									class="contents"
									{@attach (box: HTMLElement) => {
										claimVideo(box);
										return () => parkVideo(box);
									}}
								></div>
							{/if}
							{#if !showVideo()}
								<!-- A new track's cover settles in from a little smaller: the one motion on
								     the stage, and it says the song changed. -->
								{#key playback.now?.videoId}
									<div in:scale={{ start: 0.94, duration: 420, easing: cubicOut }} class="relative">
										{#if src && attempt < srcs.length}
											<!-- The 120 underneath is the one the player bar already has, so it
											     paints on the frame the track changes (#77). -->
											<img
												{src}
												alt=""
												onerror={() => attempt++}
												onload={(e) => isMissingThumb(e) && attempt++}
												style={srcs[2] ? `background-image:url(${srcs[2]})` : undefined}
												class="aspect-square w-full rounded-2xl bg-cover object-cover"
											/>
										{:else}
											<div
												class="flex aspect-square w-full items-center justify-center rounded-2xl bg-muted text-muted-foreground/40"
											>
												<HugeiconsIcon icon={MusicNote01Icon} class="h-20 w-20" />
											</div>
										{/if}
										<!-- A hairline round the edge, so a cover that is mostly the background's
										     colour still has one. A ring, not a shadow. -->
										<div
											class="pointer-events-none absolute inset-0 rounded-2xl ring-1 ring-inset ring-foreground/10"
										></div>
									</div>
								{/key}
							{/if}
						</button>
					</div>

					<!-- What's playing. The title scales with the picture so it reads as part of it, set
					     big enough to read from across the room. Rises in with each new track. -->
					{#key playback.now?.videoId}
						<div
							in:fly={{ y: 10, duration: 420, easing: cubicOut }}
							class={columns ? 'mt-6' : 'mt-8 text-center'}
						>
							<h1
								bind:this={titleEl}
								class="font-heading leading-[1.15] font-bold tracking-tight {twoLines
									? 'line-clamp-2 [overflow-wrap:anywhere]'
									: 'truncate'}"
								style="font-size:clamp(1.5rem, var(--side) * {columns ? 0.065 : 0.07}, {columns
									? '2.25rem'
									: '3rem'})"
								title={playback.now?.title}
							>
								{playback.now?.title ?? t('player.not_playing')}
							</h1>
							<p
								class="mt-2 flex min-w-0 items-baseline gap-2 {columns
									? 'text-base'
									: 'justify-center text-lg'}"
							>
								<ArtistLine
									runs={playback.now?.artistRuns}
									text={playback.now?.artists ?? ''}
									class="text-foreground/75"
								/>
								{#if album}
									<span class="shrink-0 text-muted-foreground/60" aria-hidden="true">·</span>
									<!-- There is no page per song, so the album is where "more about this" goes. -->
									{#if albumId}
										<button
											type="button"
											onclick={() => goto(`/album/${encodeURIComponent(albumId)}`)}
											class="min-w-0 cursor-pointer truncate text-muted-foreground transition-colors hover:text-foreground hover:underline"
										>
											{album}
										</button>
									{:else}
										<span class="min-w-0 truncate text-muted-foreground">{album}</span>
									{/if}
								{/if}
							</p>
						</div>
					{/key}
				</div>
			</div>

			<!-- === Lyrics, straight on the backdrop. No card: a panel on a screen with no other chrome
			     is the one thing that makes this look like a page instead of a stage. One size up on big
			     screens, where this is read from the sofa. That size is `em`, not `text-4xl`, so the
			     lyrics text size control still scales it.
			     Never give this a `mask-image` for the end fades, however tempting: it buffers the whole
			     scroller offscreen on every scroll frame. Same for `backdrop-filter` (see the header).
			     The scrollbar is hidden from out here, since the child owns the scroller.
			     An entrance only, never an exit: one keeps this in the DOM after the stage has already
			     given its room back, and the picture recentres in two visible steps. The entrance is
			     `lyricsCol.animate` (script), since `noLyrics` hides this rather than unmounting it. === -->
			{#if lyricsShown}
				<div
					bind:this={lyricsCol}
					class="min-h-0 min-w-0 flex-[1.2] flex-col overflow-hidden 2xl:[&_[data-line]]:text-[2.25em] [&_*]:[scrollbar-width:none] [&_*::-webkit-scrollbar]:hidden {noLyrics
						? 'hidden'
						: 'flex'}"
					style="padding-bottom:{roomFree ? headerH : `${dockH}px`}"
				>
					<LyricsView expanded bind:missing={noLyrics} />
				</div>
			{/if}
			<!-- === The queue, on the backdrop like the lyrics and for the same reason. It keeps its
			     scrollbar: a playlist queue runs to thousands of rows, and it is the only thing here you
			     scroll by hand. It hugs a track row's width rather than taking half the screen, and
			     narrows further when the lyrics are on screen too.
			     `relative z-0` gives it its own stacking context, and that is what keeps the scrollbar on
			     screen: without it, every transform transition on the stage (the cover settling in, the
			     flash) blanks the thumb on WebKitGTK for as long as it runs (#343, docs/UI-PERFORMANCE.md). -->
			{#if queueShown}
				<div
					in:fly={{ y: 24, duration: 400, easing: cubicOut }}
					class="relative z-0 flex min-h-0 shrink-0 flex-col overflow-hidden {both
						? 'w-[22rem]'
						: 'w-[28rem] xl:w-[34rem]'}"
					style="padding-bottom:{roomFree ? headerH : `${dockH}px`}"
				>
					<QueueList />
				</div>
			{/if}

			<!-- What comes next, in the last seconds of a track. Stays up through calm: it is for the
			     screen nobody is touching. -->
			{#if upNext}
				<button
					type="button"
					transition:fly={{ x: 24, duration: 350, easing: cubicOut }}
					onclick={() => api.nextTrack()}
					style="bottom:{dockH + 16}px"
					class="absolute right-8 z-20 flex w-80 cursor-pointer items-center gap-3 rounded-2xl border border-foreground/10 bg-background/80 p-2.5 pr-4 text-left transition-colors hover:bg-background xl:right-14"
					aria-label="{t('player.up_next')}: {upNext.title}"
				>
					{#if upNext.thumbnail}
						<img
							src={thumb(upNext.thumbnail, 120)}
							alt=""
							class="size-12 shrink-0 rounded-lg bg-muted object-cover"
						/>
					{/if}
					<div class="min-w-0">
						<p class="text-[10px] font-semibold uppercase tracking-[0.2em] text-primary">
							{t('player.up_next')}
						</p>
						<p class="truncate text-sm font-semibold">{upNext.title}</p>
						<p class="truncate text-xs text-muted-foreground">{upNext.artists}</p>
					</div>
				</button>
			{/if}
		</div>
	{:else}
		<!--
			Cinema. This wrapper has to stay transparent while mpv draws under the page
			(video.hole), or its black would hide the picture.
		-->
		<!-- svelte-ignore a11y_no_static_element_interactions -- wheel is the volume gesture -->
		<div class="absolute inset-0 z-0 {video.hole ? '' : 'bg-black'}" onwheel={onArtWheel}>
			<button
				type="button"
				onclick={toggle}
				class="block h-full w-full cursor-pointer"
				aria-label={t('a11y.play_pause')}
			>
				{#if prefs.nativeVideo}
					<div class="h-full w-full inert:bg-black {video.hole ? '' : 'bg-black'}" {@attach holeFor}></div>
				{:else}
					<div
						class="contents"
						{@attach (box: HTMLElement) => {
							claimVideo(box);
							return () => parkVideo(box);
						}}
					></div>
				{/if}
			</button>
		</div>
	{/if}

	<!-- === The dock: every control on one strip along the bottom. The scrubber runs the full width,
	     the widest thing on screen to aim at; transport in the middle, what is playing on the left,
	     what the stage shows on the right. Fades with calm and floats over the stage, which lends its
	     room to the lyrics and the queue while it is gone (see `roomFree`). === -->
	{#if cinema}
		<!-- The floor under the dock, the full width of the screen (the dock itself stops at
		     110rem). Strong enough that the controls read over a white frame. -->
		<div
			class="pointer-events-none absolute inset-x-0 bottom-0 z-10 h-56 bg-gradient-to-t from-black/85 via-black/50 to-transparent transition-opacity duration-500 {calm
				? 'opacity-0'
				: ''}"
		></div>
	{/if}
	<div
		role="group"
		bind:clientHeight={dockH}
		onpointerenter={() => (overDock = true)}
		onpointerleave={() => (overDock = false)}
		class="absolute inset-x-0 bottom-0 z-10 mx-auto w-full max-w-[110rem] px-8 transition-opacity duration-500 xl:px-14 {short.current
			? 'pt-2 pb-4'
			: 'pt-3 pb-7'} {calm ? 'opacity-0' : ''}"
	>
		<!-- Fixed-width slots for the times: the font may have no tabular figures, and the bar would
		     shift a few px per tick (#463). -->
		<div class="flex items-center gap-4 text-xs font-medium tabular-nums text-muted-foreground">
			<span class="min-w-[5ch] text-right">{fmt(shownPosition)}</span>
			<input
				type="range"
				class="range theater-range min-w-0 flex-1 {cinema ? 'on-art' : ''}"
				style="--pct:{pct}%"
				min="0"
				max={shownDuration || 0}
				value={shownPosition}
				oninput={(e) => (seekDrag = Number(e.currentTarget.value) * playback.speed)}
				onchange={(e) => {
					const v = Number(e.currentTarget.value) * playback.speed;
					playback.position = v;
					seekDrag = null;
					api.seek(v);
				}}
				aria-label={t('player.seek')}
			/>
			<span class="min-w-[5ch]">{fmt(shownDuration)}</span>
		</div>

		<div
			class="grid grid-cols-[1fr_auto_1fr] items-center gap-6 {short.current ? 'mt-1.5' : 'mt-3'}"
		>
			<!-- What's playing: like, the player bar's own ⋮ menu, and Song/Video. -->
			<div class="flex min-w-0 items-center gap-1">
				{#if playback.now && !local}
					<button
						type="button"
						onclick={toggleLike}
						class="flex size-10 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground"
						aria-label={t('common.like')}
						aria-pressed={playback.rating === 'like'}
					>
						<span
							class="inline-flex"
							class:animate-heart-pop={justLiked}
							onanimationend={() => (justLiked = false)}
						>
							<HugeiconsIcon
								icon={FavouriteIcon}
								class="h-5 w-5 {playback.rating === 'like' ? 'fill-current text-primary' : ''}"
							/>
						</span>
					</button>
				{/if}
				{#if currentSong}
					<TrackMenu
						song={currentSong}
						linksOnly
						playlistId={playback.queue.sourceId}
						queueIndex={playback.queue.currentIndex}
						onAdd={() => openAddToPlaylist(currentSong!)}
						triggerClass="flex size-10 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground"
					/>
				{/if}
				{#if canSwitch()}
					<div class="ml-2 flex rounded-full bg-foreground/[0.07] p-1" role="group">
						{@render mode(false, t('player.song'))}
						{@render mode(true, t('player.video'))}
					</div>
				{/if}
			</div>

			<!-- Transport. Bigger than the player bar's: at arm's length this is the only control
			     surface on screen, and play is the one you reach for without looking. -->
			<div class="flex items-center gap-2 xl:gap-3">
				<button
					type="button"
					onclick={() => api.toggleShuffle()}
					class="flex size-10 cursor-pointer items-center justify-center rounded-full transition-colors hover:bg-foreground/10 {shuffleOn
						? 'text-primary'
						: 'text-muted-foreground hover:text-foreground'}"
					aria-label={t('player.shuffle')}
					aria-pressed={shuffleOn}
				>
					<HugeiconsIcon icon={ShuffleIcon} class="h-[18px] w-[18px]" />
				</button>
				<button
					type="button"
					onclick={() => api.prevTrack()}
					class="flex cursor-pointer items-center justify-center rounded-full text-foreground/90 transition-colors hover:bg-foreground/10 hover:text-foreground {short.current
						? 'size-10'
						: 'size-12'}"
					aria-label={t('player.previous')}
				>
					<HugeiconsIcon icon={PreviousIcon} class={short.current ? 'h-5 w-5' : 'h-6 w-6'} />
				</button>
				<button
					type="button"
					onclick={() => api.togglePause()}
					class="mx-1 flex cursor-pointer items-center justify-center rounded-full bg-primary text-primary-foreground transition-transform duration-150 hover:scale-[1.06] active:scale-95 {short.current
						? 'size-12'
						: 'size-16'}"
					aria-label={playback.paused ? t('player.play') : t('player.pause')}
				>
					<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
					<HugeiconsIcon
						icon={PauseIcon}
						altIcon={PlayIcon}
						showAlt={playback.paused}
						class={short.current ? 'h-6 w-6' : 'h-7 w-7'}
					/>
				</button>
				<button
					type="button"
					onclick={() => api.nextTrack()}
					class="flex cursor-pointer items-center justify-center rounded-full text-foreground/90 transition-colors hover:bg-foreground/10 hover:text-foreground {short.current
						? 'size-10'
						: 'size-12'}"
					aria-label={t('player.next')}
				>
					<HugeiconsIcon icon={NextIcon} class={short.current ? 'h-5 w-5' : 'h-6 w-6'} />
				</button>
				<button
					type="button"
					onclick={cycleRepeat}
					class="flex size-10 cursor-pointer items-center justify-center rounded-full transition-colors hover:bg-foreground/10 {repeat !==
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

			<!-- What the stage shows, and the volume. -->
			<div class="flex min-w-0 items-center justify-end gap-3">
				<!-- One pill, so it reads as a view switch rather than two more actions. Not in cinema,
				     where there is no stage for either column. -->
				<div
					class="hidden items-center gap-0.5 rounded-full bg-foreground/[0.06] p-1 {cinema
						? ''
						: 'lg:flex'}"
				>
					{@render sideButton(Mic01Icon, t('player.lyrics'), lyricsShown, toggleLyrics)}
					{@render sideButton(Queue01Icon, t('player.queue'), queueShown, toggleQueue)}
				</div>
				<!-- svelte-ignore a11y_no_static_element_interactions -- wheel is the volume gesture -->
				<div class="flex items-center gap-1" onwheel={wheelVolume}>
					<button
						type="button"
						class="flex size-9 shrink-0 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground"
						onclick={toggleMute}
						aria-label={playback.volume === 0 ? t('player.unmute') : t('player.mute')}
					>
						<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
						<HugeiconsIcon
							icon={VolumeHighIcon}
							altIcon={VolumeMute02Icon}
							showAlt={playback.volume === 0}
							class="h-[18px] w-[18px]"
						/>
					</button>
					<input
						type="range"
						class="range w-24 xl:w-28 {cinema ? 'on-art' : ''}"
						style="--pct:{playback.volume}%"
						min="0"
						max="100"
						value={playback.volume}
						oninput={(e) => dragVolume(Number(e.currentTarget.value))}
						onchange={(e) => commitVolume(Number(e.currentTarget.value))}
						aria-label={t('player.volume')}
					/>
				</div>
				<!-- Last, where every video player keeps it. -->
				{#if showVideo()}
					<button
						type="button"
						onclick={toggleCinema}
						class="flex size-10 shrink-0 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground"
						aria-label={cinema ? t('player.exit_fullscreen_video') : t('player.fullscreen_video')}
						title="{cinema ? t('player.exit_fullscreen_video') : t('player.fullscreen_video')} (F)"
					>
						<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
						<HugeiconsIcon
							icon={FullScreenIcon}
							altIcon={MinimizeScreenIcon}
							showAlt={cinema}
							class="h-5 w-5"
						/>
					</button>
				{/if}
			</div>
		</div>
	</div>

	<!-- In calm, the dock's scrubber comes down to a hairline along the bottom edge: how far into the
	     song, with nothing to click. A transform, so a tick moves a layer instead of repainting. -->
	<div
		class="pointer-events-none absolute inset-x-0 bottom-0 z-10 h-[3px] bg-foreground/10 transition-opacity duration-500 {calm
			? ''
			: 'opacity-0'}"
	>
		<div class="h-full origin-left bg-primary" style="transform:scaleX({pct / 100})"></div>
	</div>
</section>

{#snippet mode(on: boolean, label: string)}
	<button
		type="button"
		onclick={() => pickVersion(on)}
		aria-pressed={videoSide() === on}
		class="cursor-pointer rounded-full px-3.5 py-1 text-sm font-medium transition-colors {videoSide() ===
		on
			? 'bg-background text-foreground shadow-sm'
			: 'text-muted-foreground hover:text-foreground'}"
	>
		{label}
	</button>
{/snippet}

{#snippet sideButton(icon: typeof Mic01Icon, label: string, on: boolean, toggle: () => void)}
	<button
		type="button"
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
