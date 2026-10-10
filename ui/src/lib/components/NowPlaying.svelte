<script lang="ts">
	import { fade, fly, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { beforeNavigate, goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		ArrowDown01Icon,
		FavouriteIcon,
		Comment01Icon,
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
		np,
		openAddToPlaylist,
		playback,
		prefs,
		toggleNowPlayingRating,
		ui,
		wheelVolume
	} from '$lib/player.svelte';
	import {
		canVideo,
		claimVideo,
		parkVideo,
		setHole,
		showVideo,
		video
	} from '$lib/video.svelte';
	import { artRoom } from '$lib/artroom.svelte';
	import { typing } from '$lib/shortcuts';
	import { appearance } from '$lib/theme.svelte';
	import { t } from '$lib/i18n.svelte';
	import { thumb } from '$lib/thumb';
	import ArtistLine from './ArtistLine.svelte';
	import QueueList from './QueueList.svelte';
	import LyricsView from './LyricsView.svelte';
	import TrackMenu from './TrackMenu.svelte';
	import CommentsView from './CommentsView.svelte';
	import Ambient from './Ambient.svelte';

	// Off in settings, this view drops its tabs and the queue/lyrics panels stay in charge of both
	// (see +layout): they paint above this (z-30 over z-20), so all this needs is to hand back the
	// width they take at lg+ instead of letting them cover a third of the artwork. Below lg they're
	// a scrimmed overlay and there's nothing to shrink into. In tabbed mode both are always closed.
	// `lyricsOpen` is the lyrics/comments slot: the comments panel takes the lyrics' place, so either
	// one open counts as that one panel.
	let { queueOpen, lyricsOpen }: { queueOpen: boolean; lyricsOpen: boolean } = $props();
	const tabbed = $derived(appearance.tabbedPlayer);
	// The two layouts (Settings > Appearance > New player layout). One component, so both share
	// every behaviour below: the video hole, the backdrop click, the wheel volume, Esc. Only the
	// markup differs. Off is the original, opt-in while people get used to the new one.
	const stage = $derived(appearance.stagePlayer);
	// ponytail: mirrors QueuePanel / LyricsPanel's w-80, keep in sync if those change.
	const panels = $derived(Number(queueOpen) + Number(lyricsOpen));
	const inset = $derived(['', 'lg:right-80', 'lg:right-[40rem]'][panels]);

	// Going somewhere means the user wants that page, not this one: minimise. The player bar brings
	// it back. beforeNavigate (not a pathname effect) so clicking the tab you're already on counts.
	beforeNavigate(() => (np.open = false));

	// Clicking the empty space around the artwork closes the view (#302), the way a dialog's
	// backdrop does. `data-np-keep` marks the regions that own their clicks: the picture (with its
	// title, in the new layout), the Song/Video switch, and the queue/lyrics column. Anything added
	// straight to the view outside those closes it.
	// Same press-not-release rule as the player bar: dragging a queue row (or a lyrics scroll) and
	// releasing over the backdrop retargets the click at the common ancestor, which is this. Both
	// ends of the drag are checked, because the retarget happens whichever way it ran: a selection
	// started on the backdrop and finished over the lyrics lands the click here too.
	const keeps = (t: EventTarget | null) => !!(t as Element | null)?.closest?.('[data-np-keep]');
	let pressedKeep = false;
	let releasedKeep = false;
	function onBackdropClick(e: MouseEvent) {
		if (pressedKeep || releasedKeep || keeps(e.target)) return;
		np.open = false;
	}

	// Enlarged lyrics take the whole view, the picture and the tab strip included. A class swap
	// rather than unmounting the tabs: LyricsView must survive it or it refetches and loses its
	// scroll position.
	let big = $state(false);
	/** The picture's box, for the ambient light to glow around. */
	let picBox = $state<HTMLElement | null>(null);
	$effect(() => {
		if (np.tab !== 'lyrics') big = false; // nothing to enlarge on the queue tab
	});

	// Esc steps back one level: out of the enlarged lyrics, then out of the view. A dialog or menu
	// over this one already took the key (bits-ui preventDefaults it), and theater mode, which
	// covers this view, has its own Esc.
	function onKey(e: KeyboardEvent) {
		if (e.key !== 'Escape' || e.defaultPrevented || ui.theaterOpen || typing(e.target)) return;
		e.preventDefault();
		if (big) big = false;
		else np.open = false;
	}

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

	// New layout: the cover lights the room, a wash baked from it, a mesh off its colour, and a
	// glow behind the picture. Shared with theater mode (`artroom.svelte.ts`). Idle in the original
	// layout, which blurs the 120px cover live instead.
	const room = artRoom(() => stage);

	// New layout: the picture's side. A square spends whichever is smaller of the column's width and the
	// height the title block leaves; a video spends that height at 16:9 instead, which a square's
	// width would leave half empty. The caps stop a 720px cover being stretched across a 4K
	// window, and a video drifting into an empty half.
	// ponytail: 9.5rem is the block under the picture at its largest (two lines of title, the
	// artist and album lines, the gap above them) measured, not computed. Raise it if that grows.
	// `wantedVideoHeight` in player.svelte.ts mirrors the video budget.
	const ART_SIDE = 'min(100cqw, 100cqh - 9.5rem, 44rem)';
	const VIDEO_SIDE = 'min(100cqw, (100cqh - 9.5rem) * 16 / 9, 80rem)';

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

	// The queue row for what's playing: it carries the album link and what the ⋮ menu needs, which
	// the now-playing event does not. Matched on videoId so a mid-advance mismatch can't point
	// either at the previous song.
	const currentSong = $derived.by(() => {
		const cur = playback.queue.items[playback.queue.currentIndex];
		return cur?.video_id === playback.now?.videoId ? cur : null;
	});
	const local = $derived(!!playback.now && api.isLocalId(playback.now.videoId));
	const album = $derived(playback.now?.album ?? currentSong?.album);
	const albumId = $derived(!local ? currentSong?.album_id : undefined);

	// Pop the heart once on a like (not an unlike); the animation's end re-arms it.
	let justLiked = $state(false);
	function toggleLike() {
		if (playback.rating !== 'like') justLiked = true;
		toggleNowPlayingRating();
	}

	// Linux: the picture is drawn by mpv underneath the page, and the box below is a hole it shows
	// through (src-tauri/src/nativevideo.rs). Not while the view flies in or out: the box moves every
	// frame then, and the picture underneath cannot follow. The end of the fly is what settles it,
	// never a timer: the fly starts only once the view's first frame is out, so after a slow one a
	// 340 ms timer measured the box mid-flight, and a transform fires no observer to correct it
	// (the picture sat 186 px low). Reopened mid-flight out, the view is the same component flying
	// back in, and that fly's end settles it the same way.
	// Closing pauses this component, and a paused component runs no effects or class updates until
	// the fly is over, so the picture stayed up for all of it. The outro's start is an event, which
	// still fires: it takes the hole away itself, and `inert:` (Svelte sets it on the view for the
	// fly out) paints the view and the box over the picture meanwhile.
	let settled = $state(false);
	let view: HTMLElement;
	$effect(() => {
		// Mounted without a fly (the intro only plays when the view itself opens), so no introend
		// is coming. By the first frame Svelte has started the fly if there is one.
		const frame = requestAnimationFrame(() => {
			if (!view.getAnimations().length) settled = true;
		});
		return () => cancelAnimationFrame(frame);
	});

	/** Report the hole's box whenever it moves, and none while the window is hidden (the tray, the
	 *  mini player): mpv then stops decoding the picture, and picks it up in step on the way back. */
	function holeFor(el: HTMLElement) {
		// Theater mode paints over the whole window, so nobody would see the picture.
		if (!settled || ui.theaterOpen) return;
		const measure = () => {
			if (document.hidden) return setHole(null);
			const r = el.getBoundingClientRect();
			setHole({ x: r.left, y: r.top, w: r.width, h: r.height });
		};
		// The box, the window, and this view: every layout change that moves the box also resizes
		// one of them. The view is the one that only moves it: collapsing the sidebar widens the view
		// and slides the box over at the same size, which left the picture where the box had been.
		const ro = new ResizeObserver(measure);
		ro.observe(el);
		ro.observe(document.documentElement);
		const view = el.closest('[data-np-view]');
		if (view) ro.observe(view);
		document.addEventListener('visibilitychange', measure);
		return () => {
			ro.disconnect();
			document.removeEventListener('visibilitychange', measure);
			setHole(null);
		};
	}
</script>

<svelte:window onkeydown={onKey} />

{#snippet mode(on: boolean, label: string)}
	<button
		type="button"
		onclick={() => (video.want = on)}
		aria-pressed={video.want === on}
		class="cursor-pointer rounded-full px-4 py-1 text-sm font-medium transition-colors {video.want ===
		on
			? 'bg-background text-foreground shadow-sm'
			: 'text-muted-foreground hover:text-foreground'}"
	>
		{label}
	</button>
{/snippet}

<!-- Both layouts: the volume level while the wheel is live, middle left of the picture, on a plate:
     it sits over whatever the picture is, so it needs its own background to stay readable. Theme
     tokens, same primary-on-muted as the volume slider in the player bar. -->
{#snippet volPlate()}
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
		<!-- Three digits wide at every level, or the plate grows a few px at 100 (#336). -->
		<span class="w-[3ch] text-center text-[10px] tabular-nums">{playback.volume}</span>
	</div>
{/snippet}

<!-- Both layouts: the play/pause button over the picture, and the picture in it. -->
{#snippet picture()}
	<button type="button" onclick={toggle} aria-label={t('a11y.play_pause')} class="block w-full cursor-pointer">
		{#if flash}
			<!-- No backdrop-blur: re-blurring the plate on every frame of the scale is what made this
			     stutter on WebKitGTK. Transform and opacity only. -->
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
		<!-- The picture is not built here. VideoSurface owns it so it survives this view being
		     closed, and this is where it gets moved to while the view is open. `display: contents`
		     so the wrapper generates no box of its own and the video's `w-full` still resolves
		     against the button. -->
		{#if prefs.nativeVideo}
			<!-- Transparent once the picture is up (+layout paints everything around it), black until
			     then, and black again while the view flies out. -->
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
					return parkVideo;
				}}
			></div>
		{/if}
		<!-- The artwork, when the video above isn't the picture. In the new layout a new track's cover
		     settles in from a little smaller: the one motion there, and it says the song changed. -->
		{#if !showVideo()}
			{#if stage}
				{#key playback.now?.videoId}
					<div in:scale={{ start: 0.96, duration: 360, easing: cubicOut }} class="relative">
						{@render cover()}
						<!-- A hairline round the edge, so a cover that is mostly the background's colour
						     still has one. A ring, not a shadow: the glow behind it is the depth. -->
						<div
							class="pointer-events-none absolute inset-0 rounded-2xl ring-1 ring-inset ring-foreground/10"
						></div>
					</div>
				{/key}
			{:else}
				{@render cover()}
			{/if}
		{/if}
	</button>
{/snippet}

{#snippet cover()}
	{#if src && attempt < srcs.length}
		<!-- The 120 underneath is the one the player bar already has for this track, so it paints on
		     the frame the track changes. Without it an <img> keeps showing the *previous* track's
		     picture for as long as this one's fetch takes (#77): 720 is a size nothing else in the
		     app asks for, so it is always a cold request. -->
		<img
			{src}
			alt=""
			onerror={imgFailed}
			style={srcs[2] ? `background-image:url(${srcs[2]})` : undefined}
			class="aspect-square w-full rounded-2xl bg-cover object-cover {stage ? '' : 'shadow-2xl'}"
		/>
	{:else}
		<div
			class="flex aspect-square w-full items-center justify-center rounded-2xl bg-muted text-muted-foreground/40"
		>
			<HugeiconsIcon icon={MusicNote01Icon} class="h-16 w-16" />
		</div>
	{/if}
{/snippet}

<!-- Both layouts: queue and lyrics as tabs. The new layout makes the column a full-height side of
     the view, a tint of the background rather than a card, so it reads as part of the room and the
     room's colour still comes through. No backdrop-filter: it would re-run on every lyrics frame.
     `relative z-0` there gives the scroller inside its own stacking context: without one, the
     cover's scale-in next door blanks its scrollbar on WebKitGTK while it runs
     (docs/UI-PERFORMANCE.md). -->
{#snippet tabs()}
	<div
		class="flex min-h-0 flex-col {big
			? 'flex-1'
			: `w-full md:w-[22rem] xl:w-[26rem] ${stage ? 'border-l border-foreground/10 bg-background/55' : ''}`} {stage
			? 'relative z-0'
			: ''}"
		data-np-keep
	>
		<Tabs.Root
			value={np.tab}
			onValueChange={(v) => (np.tab = v as typeof np.tab)}
			class="min-h-0 flex-1 {stage ? 'gap-0' : ''}"
		>
			<div
				class="flex items-center gap-2 {stage ? 'shrink-0 px-4 pt-3' : ''} {big ? 'justify-end' : ''}"
			>
				<!-- Same two glyphs the player bar uses for the queue and lyrics buttons. -->
				<Tabs.List
					variant={stage ? 'line' : 'default'}
					class={big ? 'hidden' : stage ? 'flex-1 justify-start gap-5' : 'flex-1'}
				>
					<Tabs.Trigger value="queue" class={stage ? 'flex-none gap-2 px-0.5' : 'gap-2.5'}>
						<HugeiconsIcon icon={Queue01Icon} class="h-4 w-4" />
						{t('player.queue')}
					</Tabs.Trigger>
					<Tabs.Trigger value="lyrics" class={stage ? 'flex-none gap-2 px-0.5' : 'gap-2.5'}>
						<HugeiconsIcon icon={Mic01Icon} class="h-4 w-4" />
						{t('player.lyrics')}
					</Tabs.Trigger>
					{#if !api.isLocalId(playback.now?.videoId)}
						<Tabs.Trigger value="comments" class={stage ? 'flex-none gap-2 px-0.5' : 'gap-2.5'}>
							<HugeiconsIcon icon={Comment01Icon} class="h-4 w-4" />
							{t('player.comments')}
						</Tabs.Trigger>
					{/if}
				</Tabs.List>
				{#if np.tab === 'lyrics'}
					<button
						type="button"
						onclick={() => (big = !big)}
						class="flex cursor-pointer items-center justify-center text-muted-foreground transition-colors hover:text-foreground {stage
							? 'size-8 rounded-full hover:bg-foreground/10'
							: 'rounded-md p-1.5'}"
						aria-label={big ? t('player.shrink_lyrics') : t('player.enlarge_lyrics')}
						title={big ? t('player.shrink_lyrics') : t('player.enlarge_lyrics')}
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
			{:else if np.tab === 'comments'}
				<Tabs.Content value="comments" class="flex min-h-0 flex-col">
					<CommentsView />
				</Tabs.Content>
			{:else}
				<Tabs.Content value="lyrics" class="flex min-h-0 flex-col">
					<LyricsView expanded={big} />
				</Tabs.Content>
			{/if}
		</Tabs.Root>
	</div>
{/snippet}

<!-- Covers the page but not the sidebar (you navigate away to minimise) and not the player bar,
     which stays in charge of transport and paints above this on the way in and out. So nothing
     here plays, pauses or seeks: the bar right underneath already does.
     z-20 matches the highest a page uses for its own chrome (home's sticky mood chips) and wins the
     tie on DOM order, since <main> is static and its z-indexes land in the same stacking context.
     The player bar and the queue/lyrics panels come later/higher, so they still paint above.
     ponytail: left offsets mirror Sidebar's w-16/lg:w-60 (and its manual collapse), keep in sync
     if those change. -->
<!-- The player bar's chevron, E, Esc (and the new layout's collapse button) are the keyboard
     equivalents of clicking the backdrop, so this stays a plain region rather than a control
     wrapping the whole view. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div
	transition:fly={{ y: '100%', duration: 320, easing: cubicOut }}
	onoutrostart={() => ((settled = false), setHole(null))}
	onintroend={() => (settled = true)}
	bind:this={view}
	onpointerdown={(e) => ((pressedKeep = keeps(e.target)), (releasedKeep = false))}
	onpointerup={(e) => (releasedKeep = keeps(e.target))}
	onclick={onBackdropClick}
	data-np-view
	class="absolute inset-y-0 left-16 right-0 z-20 flex overflow-hidden inert:bg-background {video.hole
		? ''
		: 'bg-background'} {stage ? '' : 'justify-center px-4 py-4 sm:px-6 sm:py-6 lg:px-10'} {ui.sidebarCollapsed
		? ''
		: 'lg:left-60'} {inset}"
>
	<!-- The ambient light (Settings > Video): the video's colours spilling into the view around it. Not
	     under theater mode, which covers this view. -->
	{#if prefs.ambient && showVideo() && picBox && !ui.theaterOpen}
		<Ambient box={picBox} />
	{/if}

	{#if stage}
		<!-- === New layout ===
		     The room, lit by the cover. Three layers, none of which repaint with the content over
		     them: the wash (the cover pre-blurred into a small bitmap once per track and stretched, no
		     live filter), a mesh of soft blobs off the cover's colour (painted fills), and a vignette
		     back to the background so the title and the queue read over whatever the cover was.
		     Chromium 4x, 1920x1080, lyrics playing, frames over 20 ms: 1/1/1% with them, 0/0/0%
		     without, 1/0/1% for the original layout's live blur.
		     Not while a video plays. The video fills the view on its own, and on Linux everything
		     around mpv's picture is painted by +layout, so a layer here would cover the picture. -->
		{#if appearance.artworkBackground && !showVideo()}
			{#if room.wash}
				{#key room.wash}
					<!-- Stretched to the view rather than cropped to it: at this blur nobody can see
					     the square has been stretched, and `cover` would throw away a third of its
					     colour. -->
					<div
						in:fade={{ duration: 700 }}
						style="background-image:url({room.wash});background-size:100% 100%"
						class="pointer-events-none absolute inset-0 opacity-40 dark:opacity-50"
					></div>
				{/key}
			{/if}
			<div
				class="pointer-events-none absolute -inset-[15%]"
				style="background-image:{room.mesh}"
			></div>
			<div
				class="pointer-events-none absolute inset-0 bg-[radial-gradient(100%_95%_at_35%_42%,transparent_25%,var(--background)_100%)]"
			></div>
		{/if}

		<!-- The stage: the picture and what's playing. Untabbed there is no column beside it, so it
		     is the whole view. `relative z-0` is a stacking context of its own, so the glow behind
		     the picture (-z-10) stays above the room instead of sinking under it. -->
		{#if !big}
			<section
				class="relative z-0 min-w-0 flex-1 flex-col px-6 pt-4 pb-6 lg:px-10 {tabbed
					? 'hidden md:flex'
					: 'flex'}"
			>
				<!-- The way out on the left, the Song/Video switch in the middle. -->
				<div class="grid h-10 shrink-0 grid-cols-[1fr_auto_1fr] items-center">
					<button
						type="button"
						onclick={() => (np.open = false)}
						class="flex size-9 cursor-pointer items-center justify-center justify-self-start rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground"
						aria-label={t('player.minimize_player')}
						title={t('player.minimize_player')}
					>
						<HugeiconsIcon icon={ArrowDown01Icon} class="h-5 w-5" />
					</button>
					<!-- The user's choice, not what is on screen: "Video" stays picked while the
					     picture is still loading, and the artwork stands in until it arrives. -->
					{#if canVideo()}
						<div class="flex rounded-full bg-foreground/[0.07] p-1" role="group" data-np-keep>
							{@render mode(false, t('player.song'))}
							{@render mode(true, t('player.video'))}
						</div>
					{/if}
				</div>

				<!-- A size container, so the picture can be sized against the space actually left
				     for it (cqw/cqh) instead of guessing at the window's chrome. -->
				<div class="flex min-h-0 flex-1 items-center justify-center [container-type:size]">
					<div
						style="--side:{showVideo() ? VIDEO_SIDE : ART_SIDE};width:var(--side)"
						data-np-keep
						data-ctx
					>
						<!-- svelte-ignore a11y_no_static_element_interactions -- wheel is the volume gesture -->
						<div bind:this={picBox} class="relative" onwheel={onWheel}>
							{#if !showVideo()}
								<!-- The light the cover sits in: bigger than it and behind it, so it reads
								     as a spill, where a shadow this size would be re-blurred on every
								     repaint. -->
								<div
									class="pointer-events-none absolute -inset-[14%] -z-10 opacity-60 dark:opacity-75"
									style="background-image:{room.glow}"
								></div>
							{/if}
							{#if volFlash}{@render volPlate()}{/if}
							{@render picture()}
						</div>

						<!-- What's playing. Title, artists, album: three steps of size and colour, so
						     the eye lands on the title and the album never competes with it. The title
						     scales with the picture, so it reads as part of it rather than a caption. -->
						<div class="mt-6 flex items-start gap-3">
							<div class="min-w-0 flex-1">
								<h1
									class="line-clamp-2 font-heading leading-[1.15] font-bold tracking-tight [overflow-wrap:anywhere]"
									style="font-size:clamp(1.25rem, var(--side) * 0.06, 2rem)"
									title={playback.now?.title}
								>
									{playback.now?.title ?? t('player.not_playing')}
								</h1>
								<ArtistLine
									runs={playback.now?.artistRuns}
									text={playback.now?.artists ?? ''}
									class="mt-1.5 block text-base text-foreground/75"
								/>
								{#if album}
									<!-- There is no page per song, so the album is where "more about
									     this" goes. Local files have none to go to. -->
									{#if albumId}
										<button
											type="button"
											onclick={() => goto(`/album/${encodeURIComponent(albumId)}`)}
											class="mt-0.5 block max-w-full cursor-pointer truncate text-left text-sm text-muted-foreground transition-colors hover:text-foreground hover:underline"
										>
											{album}
										</button>
									{:else}
										<p class="mt-0.5 truncate text-sm text-muted-foreground">{album}</p>
									{/if}
								{/if}
							</div>
							<!-- Like, and the same ⋮ menu as the player bar's (save to playlist, radio,
							     go to, tempo). Right-clicking the picture or the title opens it too. -->
							<div class="flex shrink-0 items-center gap-0.5">
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
												class="h-5 w-5 {playback.rating === 'like'
													? 'fill-current text-primary'
													: ''}"
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
							</div>
						</div>
					</div>
				</div>
			</section>
		{/if}
		{#if tabbed}{@render tabs()}{/if}
	{:else}
		<!-- === Original layout ===
		     The artwork itself, blurred to a wash, is the background: same trick as HomeHero, and it
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

		<!-- Capped and centred, so a wide window doesn't park the artwork in the middle of an empty
		     half with the tabs glued to the right edge. --art is the artwork's side: whichever is
		     smaller of the column's width and the height left over once the titlebar, the player bar
		     and this padding have had theirs, at 75% so the square doesn't dominate the view.
		     ponytail: 11rem is those three measured, not computed. The 0.75 leaves it plenty of
		     slack now, so only a much taller player bar would need it raised.

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
				<!-- Centred against the full height of the column on the right. Below md there isn't
				     room for both columns, and the queue wins. Untabbed there is no second column, so
				     the artwork is the whole view at every width. -->
				<div
					class="min-w-0 flex-1 items-center justify-center {tabbed ? 'hidden md:flex' : 'flex'}"
				>
					<!-- A div, not a button: the video-mode toggle has to be a sibling of the play/pause
					     button rather than nested inside it (nested buttons are invalid HTML and the
					     inner one never reliably gets the click). -->
					<div
						bind:this={picBox}
						class="relative w-full {showVideo() ? 'max-w-[var(--vid)]' : 'max-w-[var(--art)]'}"
						onwheel={onWheel}
						data-np-keep
					>
						{#if volFlash}{@render volPlate()}{/if}
						{@render picture()}
						{#if canVideo()}
							<!-- Both directions, or there is no way back to the video. On a plate, since
							     it sits over whatever frame happens to be showing. -->
							<button
								type="button"
								onclick={() => (video.want = !video.want)}
								aria-label={showVideo() ? t('a11y.show_artwork') : t('a11y.show_video')}
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
			{#if tabbed}{@render tabs()}{/if}
		</div>
	{/if}
</div>
