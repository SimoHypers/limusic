<script lang="ts">
	// Theater mode: the window goes fullscreen and the app is replaced by one thing — the cover and
	// the controls on the left, the lyrics on the right. Mounted by +layout behind `ui.theaterOpen`,
	// so mounting is what turns fullscreen on and unmounting is what turns it back off. That keeps
	// the flag the only switch: nothing else has to remember to undo the window state.
	//
	// PERFORMANCE, read before adding anything pretty. WebKitGTK re-runs a filter for the *damaged*
	// region on every repaint, and the region here holds lyrics that repaint every frame (the
	// karaoke sweep). Everything below has already been measured against that once, so the bans are
	// the design, not a preference:
	//   - no `filter` / `backdrop-filter` on anything full-screen (the cover wash is baked into a
	//     48px canvas once per track instead — see `wash` — and upscaled, which is free);
	//   - no `mask-image` on the lyrics scroller (buffers the whole thing offscreen per frame);
	//   - no large-radius `box-shadow` over the backdrop (re-blurred on every repaint under it);
	//   - nothing full-screen animates. A moving viewport-sized layer damages the whole viewport
	//     every frame and drags every other repaint with it.
	// Depth comes from radial gradients, which are painted fills and cost nothing.
	import { onMount } from 'svelte';
	import { beforeNavigate } from '$app/navigation';
	import { fade, fly, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { getCurrentWindow } from '@tauri-apps/api/window';
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
		playback,
		toggleMute,
		toggleNowPlayingLike,
		ui,
		wheelVolume
	} from '$lib/player.svelte';
	import { artworkAccent } from '$lib/artcolor';
	import { hexToHsv } from '$lib/color';
	import { appearance } from '$lib/theme.svelte';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import ArtistLine from './ArtistLine.svelte';
	import LyricsView from './LyricsView.svelte';

	const close = () => (ui.theaterOpen = false);

	// Clicking an artist goes to their page, and a fullscreen view over it would just hide it. Same
	// rule the now-playing view and the expanded lyrics panel use: navigating anywhere closes this.
	beforeNavigate(close);

	onMount(() => {
		const w = getCurrentWindow();
		w.setFullscreen(true).catch((e) => console.error('theater fullscreen failed', e));
		return () => {
			w.setFullscreen(false).catch(() => {});
		};
	});

	// Escape is the way out that needs no chrome on screen, which is what lets the chrome hide.
	function onKey(e: KeyboardEvent) {
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

	// The cover's own colour, lighting the room. Memoized in artcolor, and read here directly rather
	// than through the "adapt colors to artwork" setting: that setting repaints the whole app, this
	// is one screen the user opened to look at one album.
	let accent = $state<string | null>(null);
	$effect(() => {
		const url = thumb(playback.now?.thumbnail, 120);
		if (!url) {
			accent = null;
			return;
		}
		let alive = true;
		artworkAccent(url).then((hex) => {
			if (alive) accent = hex;
		});
		return () => {
			alive = false;
		};
	});
	// The cover wash, baked ONCE per track into a small canvas and then upscaled by CSS. A live
	// `filter: blur()` on a full-screen element is re-run for the damaged region on every repaint,
	// which is what made this view crawl; a bitmap upscale is something the compositor does for free.
	// Same CORS story as artcolor: googleusercontent and ytimg send `access-control-allow-origin: *`,
	// and a host that doesn't taints the canvas, throws on toDataURL, and lands in the same `null`.
	const washes = new Map<string, string>();
	let wash = $state<string | null>(null);
	$effect(() => {
		const url = thumb(playback.now?.thumbnail, 400);
		if (!url) {
			wash = null;
			return;
		}
		const hit = washes.get(url);
		if (hit !== undefined) {
			wash = hit;
			return;
		}
		let alive = true;
		bake(url).then((data) => {
			if (!alive || !data) return;
			// ponytail: one small data URL per cover for the life of the view. It is cleared with the
			// component, and nobody sits in theater mode through a thousand tracks.
			washes.set(url, data);
			wash = data;
		});
		return () => {
			alive = false;
		};
	});

	// 160px square, blurred at 28px. The size matters in both directions: too small (an earlier
	// version used 48) and the upscale to a 1080p-plus screen is 40x, where bilinear interpolation
	// draws its own diamond pattern over whatever the cover was; too big and the one-time blur
	// starts to cost something. At 160 the source is already smooth, so the upscale has nothing to
	// invent. Saturation goes up in the same pass because a heavy blur averages colour away.
	// PNG, not JPEG: at this size lossless costs a few KB, and JPEG's 8x8 blocks on a 160px buffer
	// arrive on screen as 8x8 *tiles* once CSS has stretched them across the window.
	const WASH = 160;
	const WASH_BLUR = 28;
	async function bake(url: string): Promise<string | null> {
		try {
			const img = new Image();
			img.crossOrigin = 'anonymous';
			img.src = url;
			await img.decode();
			const canvas = document.createElement('canvas');
			canvas.width = canvas.height = WASH;
			const ctx = canvas.getContext('2d');
			if (!ctx) return null;
			ctx.imageSmoothingQuality = 'high';
			// Overdrawn past every edge by more than the blur radius. Without it the blur samples the
			// transparent pixels outside the drawing and leaves a dark frame all the way round, which
			// the upscale then turns into a vignette nobody asked for.
			const over = WASH_BLUR * 1.6;
			const canFilter = typeof ctx.filter === 'string';
			if (canFilter) {
				ctx.filter = `blur(${WASH_BLUR}px) saturate(1.5)`;
				ctx.drawImage(img, -over, -over, WASH + over * 2, WASH + over * 2);
			} else {
				// No canvas filters: downscale hard and let the upscale do the smoothing instead.
				// Rougher, but it is a wash at 45% opacity behind a mesh, not the subject.
				const small = document.createElement('canvas');
				small.width = small.height = 20;
				small.getContext('2d')?.drawImage(img, 0, 0, 20, 20);
				ctx.drawImage(small, -over, -over, WASH + over * 2, WASH + over * 2);
			}
			return canvas.toDataURL('image/png');
		} catch {
			return null; // offline, 404, throttled, tainted — the mesh below is the backdrop on its own
		}
	}

	// Falls back to the theme's own accent hue, so a greyscale cover (or a cover that hasn't been
	// read yet) still gets a lit room rather than a flat one.
	const hue = $derived(accent ? (hexToHsv(accent)?.h ?? null) : null);
	// Three blobs off one hue: the near-complement keeps it from reading as a single flat tint, and
	// staggered sizes/positions are what make it look lit rather than gradient-filled.
	const mesh = $derived.by(() => {
		const h = hue ?? 265;
		const a = (deg: number) => (h + deg + 360) % 360;
		return [
			`radial-gradient(70% 60% at 12% 18%, hsl(${a(0)} 72% 48% / 0.34), transparent 68%)`,
			`radial-gradient(60% 55% at 88% 82%, hsl(${a(38)} 70% 45% / 0.28), transparent 68%)`,
			`radial-gradient(55% 50% at 72% 8%, hsl(${a(-46)} 65% 52% / 0.2), transparent 70%)`
		].join(',');
	});
	// The glow behind the cover, so it sits in light instead of on top of a picture. A radial
	// gradient, deliberately: this is the shape a big soft box-shadow would draw, at no filter cost.
	const glow = $derived(`radial-gradient(closest-side, hsl(${hue ?? 265} 80% 55% / 0.5), transparent)`);

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
	const volOpen = $derived(volHover || volDragging);

	// Lyrics on or off. On by default, and local to the view: turning them off is "I want to look at
	// the cover for this song", not a setting. Off, the player is the only thing on screen and
	// centres itself.
	let showLyrics = $state(true);

	let justLiked = $state(false);
	function toggleLike() {
		if (playback.rating !== 'like') justLiked = true;
		toggleNowPlayingLike();
	}
</script>

<svelte:window onkeydown={onKey} onpointerup={() => (volDragging = false)} />

<!-- Below the toast/update banners (z-100) so a notification still reaches the user, above
     everything else including the titlebar: fullscreen means fullscreen. -->
<!-- svelte-ignore a11y_no_static_element_interactions -- wheel is the volume gesture, move only wakes the chrome -->
<section
	transition:fade={{ duration: 220 }}
	onwheel={wheelVolume}
	onpointermove={wake}
	class="theater fixed inset-0 z-[90] flex flex-col overflow-hidden bg-background text-foreground {idle
		? 'cursor-none'
		: ''}"
>
	<!-- === Backdrop, three layers, none of which repaint with the content over them ===
	     1. the cover, pre-blurred into a 48px bitmap and upscaled. No live filter. Behind the
	        artwork-background setting, so it is still a one-click ablation. -->
	{#if appearance.artworkBackground && wash}
		{#key wash}
			<!-- Stretched to the window (`100% 100%`) rather than cropped to it. The source is a
			     square and the window is not, so `cover` throws away the top and bottom of the
			     cover's colour; at this blur radius nobody can see that it has been stretched. -->
			<div
				in:fade={{ duration: 700 }}
				style="background-image:url({wash});background-size:100% 100%"
				class="pointer-events-none absolute inset-0 opacity-50 dark:opacity-60"
			></div>
		{/key}
	{/if}
	<!-- 2. the room light: three soft blobs off the cover's own colour. Painted fills, no filter,
	        and STATIC. An earlier version drifted this on a keyframe; a full-viewport element moving
	        every frame damages the whole screen every frame, which drags every repaint on it (and
	        re-ran the wash's filter, back when the wash had one). Nothing here animates. -->
	<div class="pointer-events-none absolute -inset-[15%]" style="background-image:{mesh}"></div>
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

	<!-- === The band. Two tracks, both vertically centred against the same height, so neither column
	     dangles. Lyrics take the elastic half: they are the thing you read. Below lg there is no
	     room for two and the player wins. === -->
	<div
		class="relative z-10 mx-auto grid min-h-0 w-full max-w-[104rem] flex-1 grid-rows-[minmax(0,1fr)] gap-10 px-8 pb-10 xl:gap-20 xl:px-14 {showLyrics
			? 'lg:grid-cols-[minmax(20rem,0.85fr)_minmax(0,1.15fr)]'
			: ''}"
	>
		<!-- --art: the cover's side, whichever is smaller of the column and the height left once the
		     top strip, the meta and the controls have taken theirs.
		     ponytail: 25rem is those measured, not computed; raise it if the controls block grows. -->
		<div
			class="mx-auto w-full self-center {showLyrics ? 'max-w-[30rem]' : 'max-w-[34rem]'}"
			style="--art:min(100%, 100vh - 25rem)"
		>
			<div class="relative mx-auto" style="width:var(--art);max-width:100%">
				<!-- The light the cover sits in. Bigger than the cover and behind it, so it reads as a
				     spill rather than an outline. -->
				<div
					class="pointer-events-none absolute -inset-[12%] -z-10 opacity-70"
					style="background-image:{glow}"
				></div>
				{#key playback.now?.videoId}
					<div in:scale={{ start: 0.94, duration: 420, easing: cubicOut }} class="relative">
						{#if src && attempt < srcs.length}
							<img
								{src}
								alt=""
								onerror={() => attempt++}
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
					<!-- Hidden below lg, where there is no second column for the lyrics to be in. -->
					<button
						onclick={() => (showLyrics = !showLyrics)}
						class="hidden h-10 w-10 cursor-pointer items-center justify-center rounded-full transition-colors hover:bg-foreground/10 lg:flex {showLyrics
							? 'text-primary'
							: 'text-muted-foreground hover:text-foreground'}"
						aria-label={t('player.lyrics')}
						aria-pressed={showLyrics}
						title={t('player.lyrics')}
					>
						<HugeiconsIcon icon={Mic01Icon} class="h-[18px] w-[18px]" />
					</button>
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
		{#if showLyrics}
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
	</div>
</section>

<style>
	/* A thicker track than the app-wide `.range`: this bar is the width of the cover and is being
	   read from across a room, and 4px on that width reads as a hairline. */
	.theater-range::-webkit-slider-runnable-track {
		height: 6px;
	}
	.theater-range::-webkit-slider-thumb {
		margin-top: -4px;
		height: 14px;
		width: 14px;
	}
</style>
