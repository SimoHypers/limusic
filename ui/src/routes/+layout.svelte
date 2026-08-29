<script lang="ts">
	import './layout.css';
	import favicon from '$lib/assets/favicon.svg';
	import { ModeWatcher } from 'mode-watcher';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		CheckmarkCircle02Icon,
		AlertCircleIcon,
		InformationCircleIcon
	} from '@hugeicons/core-free-icons';
	import { browser } from '$app/environment';
	import { onMount } from 'svelte';
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import { fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { appearance, applyArtworkAccent, prewarmArtworkAccent, initTheme } from '$lib/theme.svelte';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import { blockForeignDrag, dragScroll } from '$lib/dnd';
	import { suppressNative } from '$lib/menu';
	import Sidebar from '$lib/components/Sidebar.svelte';
	import Titlebar from '$lib/components/Titlebar.svelte';
	import ResizeBorders from '$lib/components/ResizeBorders.svelte';
	import PlayerBar from '$lib/components/PlayerBar.svelte';
	import QueuePanel from '$lib/components/QueuePanel.svelte';
	import LyricsPanel from '$lib/components/LyricsPanel.svelte';
	import AddToPlaylist from '$lib/components/AddToPlaylist.svelte';
	import SettingsDialog from '$lib/components/SettingsDialog.svelte';
	import ShareDialog from '$lib/components/ShareDialog.svelte';
	import ChannelPicker from '$lib/components/ChannelPicker.svelte';
	import ListenTogether from '$lib/components/ListenTogether.svelte';
	import LinkDialog from '$lib/components/LinkDialog.svelte';
	import MiniPlayer from '$lib/components/MiniPlayer.svelte';
	import NowPlaying from '$lib/components/NowPlaying.svelte';
	import TheaterMode from '$lib/components/TheaterMode.svelte';
	import VideoSurface from '$lib/components/VideoSurface.svelte';
	import CommandPalette from '$lib/components/CommandPalette.svelte';
	import KeyboardShortcuts from '$lib/components/KeyboardShortcuts.svelte';
	import { Button } from '$lib/components/ui/button';
	import { auth, initApp, np, playback, ui } from '$lib/player.svelte';
	import { win, initWin } from '$lib/win.svelte';
	import { initZoom } from '$lib/zoom';
	import { initShortcuts } from '$lib/shortcuts';
	import { initErrorLog } from '$lib/errlog';
	import {
		updateState,
		installUpdate,
		openDownloadPage,
		checkForUpdatesQuiet,
		QUIET_INTERVAL_MS
	} from '$lib/updater.svelte';

	let { children } = $props();
	// Queue and lyrics toggle independently and both float over the page rather than docking into
	// it — two docked columns squeezed the content down to an unusable strip. At lg+ they sit side
	// by side over the content; narrower, they stack (see QueuePanel / LyricsPanel).
	let queueOpen = $state(false);
	let lyricsOpen = $state(false);
	// Two ways the now-playing view and these panels can divide the same two buttons, picked in
	// settings (#62). Tabbed (the default): the view carries queue and lyrics itself, so the panels
	// step aside for it and the bar's buttons switch its tabs. Off: these are the only owner, the
	// buttons always mean the panels, and the panels float over that view like they float over a
	// page, so opening it costs you nothing you had open.
	const tabbed = $derived(np.open && appearance.tabbedPlayer);
	$effect(() => {
		if (tabbed) queueOpen = lyricsOpen = false;
	});

	// "Adapt colors to artwork": re-run on every track change and on the toggle itself. The 120px
	// cover is the one the player bar has already loaded, so this costs no extra request.
	$effect(() => {
		applyArtworkAccent(
			appearance.artworkAccent ? thumb(playback.now?.thumbnail, 120) : null
		);
	});
	// Same colour, one track early. Reading it off the queue instead of the track change means the
	// palette starts moving on the frame the artwork swaps, not after a fetch and a decode.
	$effect(() => {
		if (!appearance.artworkAccent) return;
		const q = playback.queue;
		prewarmArtworkAccent(thumb(q.items[q.currentIndex + 1]?.thumbnail, 120));
	});

	// The mini player runs this same SPA in a second window (Rust `mini.rs`), so the window label is
	// what tells the two apart: `mini` gets the widget instead of the app chrome, and none of the
	// routes below it are ever rendered. Constant for the window's lifetime.
	const isMini = browser && getCurrentWindow().label === 'mini';

	// Apply the saved accent color before the first paint (ssr=false → nothing renders until now).
	if (browser) initTheme();

	// Wire the Tauri event bridge once for the whole app; teardown on destroy. Check for an update
	// on every app open (silent unless one exists).
	onMount(() => {
		// Before the mini-window bail-out: both windows run this SPA and both can throw.
		initErrorLog();
		if (isMini) return initApp(true);
		// First: it reveals the window (see initWin).
		const teardownWin = initWin();
		checkForUpdatesQuiet();
		// Repeat while the app stays open: ✕ hides to tray by default, so this component can stay
		// mounted for days and a mount-only check would never see a release published in between.
		const updateTimer = setInterval(checkForUpdatesQuiet, QUIET_INTERVAL_MS);
		const teardownApp = initApp();
		const teardownZoom = initZoom();
		const teardownShortcuts = initShortcuts();
		return () => {
			clearInterval(updateTimer);
			teardownApp();
			teardownWin();
			teardownZoom();
			teardownShortcuts();
		};
	});
</script>

<!-- oncontextmenu: the app's own menus handle their right-click and stop the event, so anything
     that reaches the window is a place where WebKit would have offered back / reload / inspect.
     Text fields and selections keep the native menu (see `suppressNative`). -->
<svelte:window
	ondragover={blockForeignDrag}
	ondrop={blockForeignDrag}
	oncontextmenu={suppressNative}
/>

<svelte:head><link rel="icon" href={favicon} /></svelte:head>
<ModeWatcher />

<!-- The mini player is the whole window when it is the window: no titlebar, no sidebar, no routes,
     and no toasts (a banner would cover most of a 560x180 widget). -->
{#if isMini}
	<MiniPlayer />
{:else}
	<!-- The window itself is transparent; this root paints the background and, when not maximized,
	     rounds the corners (the compositor can't round an undecorated window for us). -->
	<div
		class="flex h-screen flex-col overflow-hidden bg-background text-foreground {win.maximized
			? ''
			: 'rounded-lg'}"
	>
		<ResizeBorders />
		<Titlebar />
		<!-- relative: the queue and lyrics panels are absolute overlays inside it (see QueuePanel). -->
		<div class="relative flex min-h-0 flex-1">
			<Sidebar />
			<!-- dragScroll: dragging a card up to home's Shortcuts grid has to be possible from anywhere in
			     the feed, so aiming at the top edge scrolls this container while the drag is in flight. -->
			<main class="min-w-0 flex-1 overflow-y-auto" {@attach dragScroll}>
				<!-- Remount the current page on sign-in/out so it refetches with the new account. -->
				{#key auth.epoch}
					{@render children()}
				{/key}
			</main>
			<!-- Always mounted, unlike the player view below it: it owns the one <video> element, which
			     has to keep playing while the view is closed. It renders nothing but a zero-sized
			     parking container until the view borrows the picture. -->
			<VideoSurface />
			{#if np.open && playback.now}<NowPlaying {queueOpen} {lyricsOpen} />{/if}
			<!-- Lyrics before queue: side by side over the page, lyrics on the left, queue on the right. -->
			{#if lyricsOpen}<LyricsPanel onClose={() => (lyricsOpen = false)} {queueOpen} />{/if}
			{#if queueOpen}<QueuePanel onClose={() => (queueOpen = false)} />{/if}
		</div>
		{#if playback.now}
			<!-- Slides up from its own height on first play; leaves instantly (bar removal is rare).
			     z-20 on the wrapper, not the bar: the intro's transform makes this a stacking context,
			     so a z on the footer inside would be trapped under it. The now-playing view is z-20 and
			     earlier in the DOM, which is what puts it behind the bar as it slides in and out. -->
			<div class="relative z-20" in:fly={{ y: 64, duration: 250, easing: cubicOut }}>
				<PlayerBar
					onToggleQueue={() => (tabbed ? (np.tab = 'queue') : (queueOpen = !queueOpen))}
					queueOpen={tabbed ? np.tab === 'queue' : queueOpen}
					onToggleLyrics={() => (tabbed ? (np.tab = 'lyrics') : (lyricsOpen = !lyricsOpen))}
					lyricsOpen={tabbed ? np.tab === 'lyrics' : lyricsOpen}
				/>
			</div>
		{/if}
	</div>

	<!-- Theater mode covers everything, titlebar included, and puts the window in fullscreen for as
	     long as it is mounted. Nothing playing means nothing to show, and that guard is also what
	     closes it (and leaves fullscreen) when the queue runs out. -->
	{#if ui.theaterOpen && playback.now}<TheaterMode />{/if}

	<CommandPalette />
	<KeyboardShortcuts />
	<AddToPlaylist />
	<ShareDialog />
	<SettingsDialog />
	<ChannelPicker />
	<ListenTogether />
	<LinkDialog />

	<!-- The two notification banners below run at z-[100]. Dialogs and menus sit at z-50 and portal to
	     <body>, so a z-50 banner loses the tie on DOM order and hides behind an open modal. -->
	{#if updateState.available}
		<div
			transition:fly={{ y: 16, duration: 220, easing: cubicOut }}
			class="fixed bottom-24 left-1/2 z-[100] flex -translate-x-1/2 items-center gap-3 rounded-lg border bg-card px-4 py-2 text-sm shadow-lg"
		>
			<span>{t('settings.about.update_available', { version: updateState.available.version })}</span>
			{#if updateState.canInstall}
				<Button size="sm" onclick={installUpdate} disabled={updateState.installing}>
					{updateState.installing ? t('common.loading') : t('settings.about.install_update')}
				</Button>
			{:else}
				<!-- Packaged build (.rpm, AUR): the updater can only rewrite an AppImage, so send them
				     to the releases page and let their package manager do it. -->
				<Button size="sm" onclick={openDownloadPage}>{t('settings.about.download_page')}</Button>
			{/if}
			{#if !updateState.installing}
				<button
					class="text-muted-foreground hover:text-foreground"
					aria-label={t('common.close')}
					onclick={() => (updateState.available = null)}>✕</button
				>
			{/if}
		</div>
	{/if}

	{#if ui.toast}
		{@const t = ui.toast}
		<div
			transition:fly={{ y: 16, duration: 220, easing: cubicOut }}
			class="fixed bottom-40 left-1/2 z-[100] flex -translate-x-1/2 items-center gap-2 rounded-lg border bg-card px-4 py-2 text-sm shadow-lg"
		>
			<!-- Three branches instead of a ternary on `icon`: HugeiconsIcon freezes `icon` at mount, so a
			     new toast replacing a visible one would keep the old glyph. -->
			{#if t.kind === 'success'}
				<HugeiconsIcon icon={CheckmarkCircle02Icon} class="h-4 w-4 shrink-0 text-primary" />
			{:else if t.kind === 'error'}
				<HugeiconsIcon icon={AlertCircleIcon} class="h-4 w-4 shrink-0 text-destructive" />
			{:else}
				<HugeiconsIcon
					icon={InformationCircleIcon}
					class="h-4 w-4 shrink-0 text-muted-foreground"
				/>
			{/if}
			{t.msg}
		</div>
	{/if}
{/if}
