<script lang="ts">
	import { fade, fly, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { beforeNavigate } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Maximize01Icon,
		Minimize01Icon,
		Mic01Icon,
		MusicNote01Icon,
		PlayIcon,
		PauseIcon,
		Queue01Icon,
		Video01Icon,
		VideoOffIcon,
		VolumeHighIcon,
		VolumeMute02Icon
	} from '@hugeicons/core-free-icons';
	import * as Tabs from '$lib/components/ui/tabs';
	import * as api from '$lib/api';
	import {
		forgetVideoUrl,
		np,
		playback,
		prefs,
		ui,
		videoUrlFor,
		wheelVolume
	} from '$lib/player.svelte';
	import { appearance } from '$lib/theme.svelte';
	import { thumb } from '$lib/thumb';
	import QueueList from './QueueList.svelte';
	import LyricsView from './LyricsView.svelte';

	// Off in settings, this view drops its tabs and the queue/lyrics panels stay in charge of both
	// (see +layout): they paint above this (z-30 over z-20), so all this needs is to hand back the
	// width they take at lg+ instead of letting them cover a third of the artwork. Below lg they're
	// a scrimmed overlay and there's nothing to shrink into. In tabbed mode both are always closed.
	let { queueOpen, lyricsOpen }: { queueOpen: boolean; lyricsOpen: boolean } = $props();
	const tabbed = $derived(appearance.tabbedPlayer);
	// ponytail: mirrors QueuePanel / LyricsPanel's w-80, keep in sync if those change.
	const panels = $derived(Number(queueOpen) + Number(lyricsOpen));
	const inset = $derived(['', 'lg:right-80', 'lg:right-[40rem]'][panels]);

	// Going somewhere means the user wants that page, not this one: minimise. The player bar brings
	// it back. beforeNavigate (not a pathname effect) so clicking the tab you're already on counts.
	beforeNavigate(() => (np.open = false));

	// Enlarged lyrics take the whole view, artwork column and tab strip included. A class swap
	// rather than unmounting the tabs: LyricsView must survive it or it refetches and loses its
	// scroll position.
	let big = $state(false);
	$effect(() => {
		if (np.tab !== 'lyrics') big = false; // nothing to enlarge on the queue tab
	});

	// Google's CDN doesn't serve every rewritten size for every image (see MediaCard), and at this
	// size a broken-image glyph *is* the page. So step down until one loads: crisp, then the size
	// proven everywhere else in the app, then the 120 the player bar is already showing for this
	// very track, and only then a music note.
	let attempt = $state(0);
	let bgFailed = $state(false);
	$effect(() => {
		playback.now?.thumbnail; // re-arm on every track change
		attempt = 0;
		bgFailed = false;
	});
	const srcs = $derived([720, 400, 120].map((px) => thumb(playback.now?.thumbnail, px)));
	const src = $derived(srcs[attempt]);
	const imgFailed = () => attempt++;

	// Clicking the artwork toggles playback, and flashes the action just taken over it so the click
	// visibly did something. Read `paused` before the toggle: the backend event that flips it is a
	// round trip away, and the icon has to be right on the frame the user clicked.
	let flash: 'play' | 'pause' | null = $state(null);
	let flashTimer: ReturnType<typeof setTimeout>;
	function toggle() {
		flash = playback.paused ? 'play' : 'pause';
		clearTimeout(flashTimer);
		flashTimer = setTimeout(() => (flash = null), 220);
		api.togglePause();
	}

	// Scrolling the artwork is the volume, same step as the slider. The level only draws while the
	// gesture is live (plus a second to read it) so the artwork is otherwise untouched.
	let volFlash = $state(false);
	let volTimer: ReturnType<typeof setTimeout>;
	function onWheel(e: WheelEvent) {
		wheelVolume(e);
		volFlash = true;
		clearTimeout(volTimer);
		volTimer = setTimeout(() => (volFlash = false), 1000);
	}

	// --- Music videos (plan 031) -------------------------------------------------------------
	// When the track *is* a music video, this box draws the video instead of the artwork. mpv stays
	// the audio master: the element is muted and its clock is stapled to mpv's position. Bytes come
	// from Rust over a loopback proxy, so nothing here ever sees a googlevideo URL.
	//
	// `wantVideo` is session-sticky on purpose: someone who hits "show artwork" wants artwork now
	// and almost certainly on the next video too, but a permanent no is what the setting is for.
	let wantVideo = $state(true);
	let videoUrl = $state<string | null>(null);
	let videoEl = $state<HTMLVideoElement | null>(null);
	const canVideo = $derived(prefs.musicVideos && !!playback.now?.isVideo);
	/** The element exists for the whole track once it has a URL, whether or not it is on screen:
	 *  unmounting it on "show artwork" meant coming back rebuilt it, refetched from byte 0 and
	 *  re-seeked, which is the black frame in #107. Hidden it keeps decoding in step with mpv, so
	 *  the way back is a class swap.
	 *  ponytail: that is one muted decode the user cannot see. It only runs while this view is
	 *  open (+layout unmounts the whole component on minimise), so it is bounded. */
	const hasVideo = $derived(canVideo && !!videoUrl);
	const showVideo = $derived(hasVideo && wantVideo);

	// How the picture is kept in step with mpv. Measured against mpv actually playing the same
	// track, because the guesses before it were all wrong in the same direction:
	//
	//   - Drift is *flat*. Once the two agree they stay agreeing over minutes, so nothing needs
	//     continuous correction.
	//   - A seek is expensive. A tiny buffered one loses ~85 ms of motion; a real mid-stream one
	//     costs about a second of re-buffering, and mpv plays on throughout. A seek issued to close
	//     a small gap therefore *opens* a gap of roughly its own cost, and a loop that seeks
	//     whenever it is out seeks forever. That was the stutter, twice, and it is also where the
	//     steady half-second offset came from: the seek landed exactly its own cost behind.
	//   - A `playbackRate` change is nearly free and holds exactly: 1.5x measured a dead-steady
	//     ratio of 1.500 with no stalls, on a picture that is muted anyway.
	//
	// So the rate does the work and seeking is the exception. A seek is only for a gap too big to
	// trim out in reasonable time (opening the view mid-track, a scrub, coming back from the tray),
	// and it aims past the target by what the re-buffer will cost. The first sync of a fresh element
	// is one of those cases by definition: resolving the video costs a round trip, so mpv is already
	// a second or two in before the element can seek at all.
	/** Trim bands. A rate write is *not* free on WebKitGTK, whatever the steady-state measurement
	 *  above says: WebKit's GStreamer backend implements a `playbackRate` change as a flushing
	 *  seek, so every write drops the sink's buffers and shows as a flicker (#107). So the rate is
	 *  written twice per correction at most, once to start it and once to end it, and never for
	 *  the wobble that ~4 Hz sampling puts on `drift`. TRIM_TICKS is that guard: about two seconds
	 *  of sustained out-of-band drift before a correction starts at all. */
	const TRIM_FROM = 0.35;
	const TRIM_TO = 0.15;
	const TRIM_TICKS = 8;
	/** Above this, trimming would take too long to sit through, so pay for one seek instead. */
	const SEEK_FROM = 2.5;
	/** The lead on the *first* seek only, the one that is unbuffered: it aims past the re-buffer
	 *  that seek is about to cost. Every seek after it lands inside the buffer, where a lead would
	 *  only overshoot. */
	const SEEK_COST = 1;
	const SEEK_COOLDOWN = 6000;
	/** Getting a fresh element onto the music. The first seek is unbuffered, so where it lands
	 *  cannot be predicted, only measured: it costs about a second of re-buffering and mpv plays on
	 *  through it. So seek once with a lead, wait for the seek to land *and* for the element to
	 *  have enough data to play on, then measure again. A correction at that point is a buffered
	 *  seek (~85 ms), cheap enough to spend and accurate to a frame or two. Bounded, because a
	 *  stream that will not converge must not seek forever, and deadlined, because a stall must not
	 *  leave the picture frozen. Only once this is done does the steady-state trim below take over,
	 *  and by then it only ever sees the slow drift it was designed for. */
	const SYNC_SEEKS = 3;
	const SYNC_DEADLINE = 5000;
	let synced = false;
	let syncSeeks = 0;
	let syncStart = 0;
	let lastSeek = 0;
	/** A correction in progress, the speed it was picked against, and how many ticks running the
	 *  drift has been out of band. Plain lets: nothing renders off them. */
	let trimming = false;
	let trimSpeed = 1;
	let outOfBand = 0;

	/** Which track the URL below was fetched for. A plain let, so the fetch effect can read it
	 *  without depending on itself. */
	let fetchedId: string | null = null;

	// Two effects, not one: the reset must key on the *track*, or toggling back to artwork would
	// throw the URL away and the way back would be a fresh download (#107). Effects run in
	// creation order, so this one clears before the next one fetches.
	$effect(() => {
		playback.now?.videoId;
		canVideo;
		videoUrl = null;
		fetchedId = null;
		synced = false;
		syncSeeks = 0;
		syncStart = 0;
		lastSeek = 0; // a new element may sync at once, whatever the last one was doing
	});

	$effect(() => {
		const id = playback.now?.videoId;
		// wantVideo is a dependency so turning video on mid-track starts the fetch; fetchedId is
		// what stops turning it off and on again from refetching what we already have.
		if (!id || !canVideo || !wantVideo || fetchedId === id) return;
		fetchedId = id;
		let cancelled = false;
		// Usually already resolved (the store warms it when the track starts, and keeps it across
		// this component being unmounted), in which case this settles without touching the network.
		videoUrlFor(id).then((u) => {
			if (cancelled) return;
			// Open the stream where the music already is. Without this the element opens at byte 0,
			// buffers, and then pays for an unbuffered seek to get to the same place: two
			// connections and two re-buffers to reach a position it could have started at.
			// `mpvNow()` here is read outside the effect's tracking (this callback is async), so it
			// does not make the src reload on every position tick, which is exactly what must not
			// happen: the fragment is fixed for the life of this element.
			// ponytail: silently ignored if WebKitGTK's GStreamer backend does not honour media
			// fragments, in which case this is exactly today's behaviour and the sync still lands it.
			videoUrl = u && `${u}#t=${Math.max(0, mpvNow()).toFixed(2)}`;
		});
		// Cancelled with nothing to show for it (toggled off mid-flight): let it be tried again.
		return () => {
			cancelled = true;
			if (!videoUrl) fetchedId = null;
		};
	});

	/** Where mpv is *now*, not where it was when the last tick was emitted. Ticks land at ~4 Hz
	 *  (src-tauri/src/lib.rs), so `playback.position` alone is up to 250 ms old, and lining the
	 *  picture up against it parks it that far behind the music every time. */
	function mpvNow() {
		if (playback.paused) return playback.position;
		const since = (performance.now() - playback.positionAt) / 1000;
		// Past a couple of tick intervals the stream has stopped rather than slowed: a stall, a
		// backgrounded window, or the instant after unpausing, where the newest sample is still the
		// one from the pause. Extrapolating there would be a guess that overshoots.
		if (since > 0.4) return playback.position;
		return playback.position + since * playback.speed;
	}

	/** Catch-up rate for a gap, picked once when the correction starts and held until it closes:
	 *  re-picking it as the gap shrinks would cost a flush per band crossed. */
	function trimFor(drift: number) {
		const a = Math.abs(drift);
		const k = a > 1.2 ? 0.5 : a > 0.5 ? 0.25 : 0.1;
		return playback.speed * (1 + Math.sign(drift) * k);
	}

	/** Start the picture, if the music is going and this window can see it. Not before the converge
	 *  phase has finished: playing during it means showing real motion from the wrong moment of the
	 *  video, which reads as broken in a way a still frame does not. */
	function resumeVideo() {
		const el = videoEl;
		if (el && synced && !playback.paused && !document.hidden) el.play().catch(() => {});
	}

	function syncVideo() {
		const el = videoEl;
		// readyState 0 has no clock to compare against, and mid-seek the comparison is meaningless.
		// Anything above that is fair game: re-buffering sits at 2 for long stretches and the
		// picture keeps moving perfectly well there.
		if (!el || !hasVideo || el.seeking || el.readyState < 1) return;
		const drift = mpvNow() - el.currentTime;
		if (!synced) {
			if (!syncStart) syncStart = performance.now();
			const overdue = performance.now() - syncStart > SYNC_DEADLINE;
			// readyState < 3 means the seek target is not playable yet, so `drift` is still moving
			// by whatever the re-buffer costs. Measuring now would be measuring the re-buffer.
			if (syncSeeks > 0 && el.readyState < 3 && !overdue) return;
			// In band, out of seeks, or out of time: hand over to the trim, which can close
			// whatever is left without a flush.
			if ((syncSeeks > 0 && Math.abs(drift) <= TRIM_TO) || syncSeeks >= SYNC_SEEKS || overdue) {
				synced = true;
				lastSeek = performance.now();
				resumeVideo();
				return;
			}
			syncSeeks++;
			trimming = false;
			outOfBand = 0;
			el.playbackRate = playback.speed;
			// Only the first seek is unbuffered and therefore worth leading past its own cost. The
			// corrections after it land inside the buffer, where a lead would only overshoot.
			// Never while paused: mpv is not moving, so there is nothing to lead.
			el.currentTime = mpvNow() + (playback.paused || syncSeeks > 1 ? 0 : SEEK_COST);
			return;
		}
		if (Math.abs(drift) > SEEK_FROM) {
			const now = performance.now();
			if (now - lastSeek < SEEK_COOLDOWN) return; // let the last one finish re-buffering
			lastSeek = now;
			trimming = false;
			outOfBand = 0;
			el.playbackRate = playback.speed;
			// Aim past the re-buffer this is about to cost, or it lands behind by exactly that.
			// Not while paused: mpv is not moving, so the lead would only overshoot.
			el.currentTime = mpvNow() + (playback.paused ? 0 : SEEK_COST);
			return;
		}
		if (playback.paused) return; // nothing is moving, so there is nothing to trim
		// A tempo change makes the held rate meaningless: it was picked against the old speed.
		if (trimming && trimSpeed !== playback.speed) trimming = false;
		if (trimming) {
			if (Math.abs(drift) > TRIM_TO) return; // still closing: hold the rate, write nothing
			trimming = false;
		}
		if (el.playbackRate !== playback.speed) {
			el.playbackRate = playback.speed; // ends a correction, or follows a tempo change
			return;
		}
		outOfBand = Math.abs(drift) > TRIM_FROM ? outOfBand + 1 : 0;
		if (outOfBand < TRIM_TICKS) return;
		outOfBand = 0;
		trimming = true;
		trimSpeed = playback.speed;
		el.playbackRate = trimFor(drift);
	}

	$effect(() => {
		playback.position; // the tick this runs on
		playback.paused; // and a pause/resume, which position ticks do not cover
		playback.speed; // and a tempo change, which must reach the picture without waiting for drift
		syncVideo();
	});

	$effect(() => {
		const paused = playback.paused;
		const el = videoEl;
		if (!el || !hasVideo) return;
		// document.hidden for the same reason: unpausing from the mini player must not start a
		// hidden window decoding again. The visibilitychange handler picks it up on the way back.
		// `synced` is a plain let, so this effect does not re-run when it flips; `syncVideo` calls
		// `resumeVideo` itself at that moment, which is what starts a freshly converged picture.
		if (paused || document.hidden || !synced) el.pause();
		else resumeVideo();
	});

	// Minimised to the tray, the window still decodes video unless we stop it. Nothing here touches
	// mpv, so the audio carries on.
	$effect(() => {
		const onVisibility = () => {
			if (!videoEl) return;
			if (document.hidden) {
				videoEl?.pause();
				return;
			}
			// Seconds out by definition: the picture stood still while the music carried on. Run the
			// whole converge phase again rather than sitting out the cooldown and then trimming, and
			// keep the picture still until it lands.
			synced = false;
			syncSeeks = 0;
			syncStart = 0;
			lastSeek = 0;
			videoEl.pause();
			// The picture restarts from inside resumeVideo, which is also where the "only if mpv is
			// still going" check now lives.
			syncVideo();
		};
		document.addEventListener('visibilitychange', onVisibility);
		return () => document.removeEventListener('visibilitychange', onVisibility);
	});
</script>

<!-- Covers the page but not the sidebar (you navigate away to minimise) and not the player bar,
     which stays in charge of transport and paints above this on the way in and out.
     z-20 matches the highest a page uses for its own chrome (home's sticky mood chips) and wins the
     tie on DOM order, since <main> is static and its z-indexes land in the same stacking context.
     The player bar and the queue/lyrics panels come later/higher, so they still paint above.
     ponytail: left offsets mirror Sidebar's w-16/lg:w-60 (and its manual collapse) — keep in sync
     if those change. -->
<div
	transition:fly={{ y: '100%', duration: 320, easing: cubicOut }}
	class="absolute inset-y-0 left-16 right-0 z-20 flex justify-center overflow-hidden bg-background px-4 py-4 sm:px-6 sm:py-6 lg:px-10 {ui.sidebarCollapsed
		? ''
		: 'lg:left-60'} {inset}"
>
	<!-- The artwork itself, blurred to a wash, is the background: same trick as HomeHero, and it
	     needs no colour extraction (which a remote image would taint the canvas for anyway). The
	     120px variant is the one the player bar has already loaded for this track, so this costs
	     no request and nothing new to decode.
	     Two opacities because the wash sits on opposite grounds: over white it has to stay pale
	     enough for dark text, over near-black it can carry more colour before muted-foreground
	     stops reading. Turn them up together if it's too subtle.

	     Not while a video is playing. WebKitGTK re-runs this 40px blur for the damaged region on
	     every video frame, and the damaged region is the video, so the cost grows with the window.
	     Measured 2026-08-20 on a 1100px box, 720p30: with the wash 13 fps of video and 74ms UI
	     frames, without it 30 fps and 17ms. Layer promotion does not help (will-change,
	     translateZ(0) and contain:paint all measured as noise), and the cost tracks the blur
	     radius rather than the image. A video fills the view on its own, so there is nothing to
	     replace it with. -->
	{#if appearance.artworkBackground && !showVideo && srcs[2] && !bgFailed}
		<img
			src={srcs[2]}
			alt=""
			onerror={() => (bgFailed = true)}
			class="pointer-events-none absolute inset-0 h-full w-full art-wash scale-110 object-cover opacity-30 blur-2xl dark:opacity-40"
		/>
	{/if}

	<!-- Capped and centred, so a wide window doesn't park the artwork in the middle of an empty half
	     with the tabs glued to the right edge. --art is the artwork's side: whichever is smaller of
	     the column's width and the height left over once the titlebar, the player bar and this
	     padding have had theirs, at 75% so the square doesn't dominate the view.
	     ponytail: 11rem is those three measured, not computed. The 0.75 leaves it plenty of slack
	     now, so only a much taller player bar would need it raised.

	     A video gets its own budget, and a wider cap to spend it in. 16:9 in the square's width
	     leaves half the height empty, so --vid is the width that spends the same leftover height
	     instead: height * 16 / 9, capped by the column (max-width can only shrink `w-full`). The
	     80rem cap exists to stop a square drifting into an empty half, which a video this wide
	     never does. -->
	<div
		class="relative flex w-full gap-6 xl:gap-10 {showVideo ? 'max-w-[100rem]' : 'max-w-[80rem]'}"
		style="--art:calc(min(100%,100vh - 11rem) * 0.75); --vid:calc((100vh - 11rem) * 0.85 * 16 / 9)"
	>
		{#if !big}
			<!-- Centred against the full height of the column on the right. Below md there isn't room
			     for both columns, and the queue wins. Untabbed there is no second column, so the
			     artwork is the whole view at every width. -->
			<div
				class="min-w-0 flex-1 items-center justify-center {tabbed ? 'hidden md:flex' : 'flex'}"
			>
				<!-- A div, not a button: the video-mode toggle has to be a sibling of the play/pause
				     button rather than nested inside it (nested buttons are invalid HTML and the
				     inner one never reliably gets the click). -->
				<div
					class="relative w-full {showVideo ? 'max-w-[var(--vid)]' : 'max-w-[var(--art)]'}"
					onwheel={onWheel}
				>
					{#if volFlash}
						<!-- Middle left of the artwork, on a plate: it sits over whatever the picture is,
						     so it needs its own background to stay readable. Theme tokens, same
						     primary-on-muted as the volume slider in the player bar. -->
						<div
							transition:fade={{ duration: 120 }}
							class="pointer-events-none absolute left-3 top-1/2 z-10 flex -translate-y-1/2 flex-col items-center gap-2 rounded-full border bg-popover/90 px-2 py-3 text-popover-foreground"
						>
							<HugeiconsIcon
								icon={VolumeHighIcon}
								altIcon={VolumeMute02Icon}
								showAlt={playback.volume === 0}
								class="h-4 w-4"
							/>
							<div class="relative h-24 w-1 overflow-hidden rounded-full bg-muted">
								<div
									class="absolute inset-x-0 bottom-0 rounded-full bg-primary"
									style="height:{playback.volume}%"
								></div>
							</div>
							<span class="text-[10px] tabular-nums">{playback.volume}</span>
						</div>
					{/if}
					<button
						type="button"
						onclick={toggle}
						aria-label="Play/pause"
						class="block w-full cursor-pointer"
					>
						{#if flash}
							<!-- No backdrop-blur: re-blurring the plate on every frame of the scale is what made
							     this stutter on WebKitGTK. Transform and opacity only. -->
							<div
								in:scale={{ start: 0.7, duration: 150, easing: cubicOut }}
								out:scale={{ start: 1.3, duration: 320, easing: cubicOut }}
								class="pointer-events-none absolute inset-0 z-10 flex items-center justify-center"
							>
								<div class="rounded-full bg-black/55 p-3.5 text-white">
									<!-- icon is frozen at mount, so swap via showAlt, not a ternary. -->
									<HugeiconsIcon
										icon={PauseIcon}
										altIcon={PlayIcon}
										showAlt={flash === 'play'}
										class="h-7 w-7"
									/>
								</div>
							</div>
						{/if}
						{#if hasVideo}
							<!-- Muted and never seeked by the user: mpv is the clock (see the sync effects).
							     16:9 in --vid, the width that spends the height the square leaves empty.

							     No shadow, unlike the artwork: WebKitGTK re-blurs a resting box-shadow over
							     the whole tile on every repaint, and a video repaints 24 times a second at
							     this size. It dragged the whole app, not just the picture. Same cause as
							     the card-hover cost measured on 2026-08-06. -->
							<!-- svelte-ignore a11y_media_has_caption -->
							<video
								bind:this={videoEl}
								src={videoUrl}
								muted
								playsinline
								preload="auto"
								onloadedmetadata={syncVideo}
								oncanplay={syncVideo}
								onseeked={syncVideo}
								onerror={() => {
									if (playback.now?.videoId) forgetVideoUrl(playback.now.videoId);
									videoUrl = null;
								}}
								class="w-full rounded-2xl bg-black object-contain {showVideo
									? 'aspect-video'
									: 'pointer-events-none absolute inset-0 h-full opacity-0'}"
							></video>
						{/if}
						<!-- The artwork, when the video above isn't the picture. Both arms carry the same
						     guard rather than nesting, so the branch below keeps its indentation. -->
						{#if !showVideo && src && attempt < srcs.length}
							<!-- The 120 underneath is the one the player bar already has for this track, so it
							     paints on the frame the track changes. Without it an <img> keeps showing the
							     *previous* track's picture for as long as this one's fetch takes (#77): 720 is
							     a size nothing else in the app asks for, so it is always a cold request. -->
							<img
								{src}
								alt=""
								onerror={imgFailed}
								style={srcs[2] ? `background-image:url(${srcs[2]})` : undefined}
								class="aspect-square w-full rounded-2xl bg-cover object-cover shadow-2xl"
							/>
						{:else if !showVideo}
							<div
								class="flex aspect-square w-full items-center justify-center rounded-2xl bg-muted text-muted-foreground/40"
							>
								<HugeiconsIcon icon={MusicNote01Icon} class="h-16 w-16" />
							</div>
						{/if}
					</button>
					{#if canVideo}
						<!-- Both directions, or there is no way back to the video. On a plate, since it sits
						     over whatever frame happens to be showing. -->
						<button
							type="button"
							onclick={() => (wantVideo = !wantVideo)}
							aria-label={showVideo ? 'Show artwork' : 'Show video'}
							class="absolute right-3 top-3 z-10 cursor-pointer rounded-md bg-black/40 p-1.5 text-white/70 transition-colors hover:text-white"
						>
							<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
							<HugeiconsIcon
								icon={Video01Icon}
								altIcon={VideoOffIcon}
								showAlt={showVideo}
								class="h-4 w-4"
							/>
						</button>
					{/if}
				</div>
			</div>
		{/if}

		{#if tabbed}
			<div class="flex min-h-0 flex-col {big ? 'flex-1' : 'w-full md:w-[22rem] xl:w-[26rem]'}">
				<Tabs.Root
					value={np.tab}
					onValueChange={(v) => (np.tab = v as typeof np.tab)}
					class="min-h-0 flex-1"
				>
					<div class="flex items-center gap-2 {big ? 'justify-end' : ''}">
						<!-- Same two glyphs the player bar uses for the queue and lyrics buttons. -->
						<Tabs.List class={big ? 'hidden' : 'flex-1'}>
							<Tabs.Trigger value="queue" class="gap-2.5">
								<HugeiconsIcon icon={Queue01Icon} class="h-4 w-4" /> Queue
							</Tabs.Trigger>
							<Tabs.Trigger value="lyrics" class="gap-2.5">
								<HugeiconsIcon icon={Mic01Icon} class="h-4 w-4" /> Lyrics
							</Tabs.Trigger>
						</Tabs.List>
						{#if np.tab === 'lyrics'}
							<button
								onclick={() => (big = !big)}
								class="cursor-pointer rounded-md p-1.5 text-muted-foreground transition-colors hover:text-foreground"
								aria-label={big ? 'Shrink lyrics' : 'Enlarge lyrics'}
							>
								<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
								<HugeiconsIcon
									icon={Maximize01Icon}
									altIcon={Minimize01Icon}
									showAlt={big}
									class="h-4 w-4"
								/>
							</button>
						{/if}
					</div>
					<!-- Only the open tab is mounted: bits-ui keeps inactive content in the DOM, which would
					     leave LyricsView fetching lyrics for every track you never asked to see. -->
					{#if np.tab === 'queue'}
						<Tabs.Content value="queue" class="flex min-h-0 flex-col">
							<QueueList />
						</Tabs.Content>
					{:else}
						<Tabs.Content value="lyrics" class="flex min-h-0 flex-col">
							<LyricsView expanded={big} />
						</Tabs.Content>
					{/if}
				</Tabs.Root>
			</div>
		{/if}
	</div>
</div>
