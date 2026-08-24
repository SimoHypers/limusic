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
	import { np, playback, ui, wheelVolume } from '$lib/player.svelte';
	import { canVideo, claimVideo, parkVideo, showVideo, video } from '$lib/video.svelte';
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
	{#if appearance.artworkBackground && !showVideo() && srcs[2] && !bgFailed}
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
		class="relative flex w-full gap-6 xl:gap-10 {showVideo() ? 'max-w-[100rem]' : 'max-w-[80rem]'}"
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
					class="relative w-full {showVideo() ? 'max-w-[var(--vid)]' : 'max-w-[var(--art)]'}"
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
						<!-- The picture is not built here. VideoSurface owns it so it survives this view
						     being closed, and this is where it gets moved to while the view is open.
						     `display: contents` so the wrapper generates no box of its own and the video's
						     `w-full` still resolves against the button. -->
						<div
							class="contents"
							{@attach (box: HTMLElement) => {
								claimVideo(box);
								return parkVideo;
							}}
						></div>
						<!-- The artwork, when the video above isn't the picture. Both arms carry the same
						     guard rather than nesting, so the branch below keeps its indentation. -->
						{#if !showVideo() && src && attempt < srcs.length}
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
						{:else if !showVideo()}
							<div
								class="flex aspect-square w-full items-center justify-center rounded-2xl bg-muted text-muted-foreground/40"
							>
								<HugeiconsIcon icon={MusicNote01Icon} class="h-16 w-16" />
							</div>
						{/if}
					</button>
					{#if canVideo()}
						<!-- Both directions, or there is no way back to the video. On a plate, since it sits
						     over whatever frame happens to be showing. -->
						<button
							type="button"
							onclick={() => (video.want = !video.want)}
							aria-label={showVideo() ? 'Show artwork' : 'Show video'}
							class="absolute right-3 top-3 z-10 cursor-pointer rounded-md bg-black/40 p-1.5 text-white/70 transition-colors hover:text-white"
						>
							<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
							<HugeiconsIcon
								icon={Video01Icon}
								altIcon={VideoOffIcon}
								showAlt={showVideo()}
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
