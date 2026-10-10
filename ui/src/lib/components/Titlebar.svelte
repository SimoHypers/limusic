<script lang="ts">
	// Custom titlebar. The window runs undecorated by default (tauri.conf `decorations: false`);
	// with a system frame (`win.chrome`, issue #65) this bar stays as a toolbar and only drops its
	// own window buttons, since everything else on it is app navigation, not window management.
	// Everything on the bar is a drag region except the buttons; double-click maximizes (Tauri's
	// drag region itself). Right cluster: Last.fm scrobbler | separator | minimize / maximize /
	// close — per the design, the scrobbler lives with the window controls but visually apart.
	// Account (sign in/out) sits first in that cluster, in its own component.
	import { onMount } from 'svelte';
	import { afterNavigate } from '$app/navigation';
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		ArrowLeft01Icon,
		ArrowRight01Icon,
		Refresh03Icon,
		MinusSignIcon,
		SquareIcon,
		Cancel01Icon,
		MinimizeScreenIcon,
		CameraVideoIcon,
		CheckmarkCircle01Icon,
		Loading03Icon,
		HotspotOfflineIcon,
		UserGroup02Icon,
		Link04Icon,
		Settings02Icon
	} from '@hugeicons/core-free-icons';
	import LastFmIcon from './LastFmIcon.svelte';
	import DiscordIcon from './DiscordIcon.svelte';
	import AccountMenu from './AccountMenu.svelte';
	import { appIcon } from '$lib/appicon.svelte';
	import * as api from '$lib/api';
	import { openMiniPlayer, playback, prefs, refreshView, toast, ui } from '$lib/player.svelte';
	import { win } from '$lib/win.svelte';
	import { lt } from '$lib/lt.svelte';
	import { anchorMenu, fitMenu, NO_ANCHOR } from '$lib/menu';
	import { t } from '$lib/i18n.svelte';
	import { connectLastfm, disconnectLastfm, lastfm, watchLastfm } from '$lib/lastfm.svelte';
	import { watchListenBrainz } from '$lib/listenbrainz.svelte';

	// `w` is this window; `win` (imported) is the shared frame state.
	const w = getCurrentWindow();

	// Back/forward. `depth` is how many history entries deep the session is, `deepest` how far it
	// has ever been, so both buttons grey out instead of doing nothing. popstate carries a signed
	// delta (the mouse's side buttons come through here); anything else is a push, which wipes the
	// entries ahead of us.
	let depth = $state(0);
	let deepest = $state(0);
	afterNavigate((nav) => {
		if (nav.type === 'enter') depth = deepest = 0;
		else if (nav.delta !== undefined) depth = Math.max(0, depth + nav.delta);
		else deepest = depth += 1;
	});

	// Last.fm connection state lives in `lastfm.svelte.ts`: the Scrobbling settings tab connects
	// and disconnects too.
	const connected = $derived(lastfm.connected);
	const username = $derived(lastfm.username);
	const connecting = $derived(lastfm.connecting);
	let menuOpen = $state(false);
	let anchor = $state(NO_ANCHOR);

	// Discord Rich Presence — a plain on/off toggle of the `discord_rpc` setting (the backend
	// connects/clears the presence the moment it flips). Optimistic; reverted on failure. The flag
	// lives in `prefs` because the Discord settings tab toggles the same thing: a local copy here
	// went stale the moment the other one was used.
	const discordOn = $derived(prefs.discordRpc);

	async function toggleDiscord() {
		const next = !discordOn;
		prefs.discordRpc = next;
		try {
			await api.setSetting('discord_rpc', next ? 'true' : 'false');
			toast.success(next ? t('integrations.discord_on') : t('integrations.discord_off'));
		} catch (e) {
			prefs.discordRpc = !next;
			toast.error(String(e));
		}
	}

	onMount(watchLastfm);
	onMount(watchListenBrainz);

	function onScrobblerClick(e: MouseEvent) {
		if (connecting) {
			// A second click cancels the pending browser authorization. The `lastfm-state` event it
			// triggers clears the spinner (and, arriving while `connecting`, stays toast-silent).
			api.lastfmDisconnect().catch(() => {});
			return;
		}
		if (connected) {
			openMenu(e);
			return;
		}
		connectLastfm();
	}

	function openMenu(e: MouseEvent) {
		anchor = anchorMenu(e, { align: 'right' });
		menuOpen = true;
	}

	function disconnect() {
		menuOpen = false;
		disconnectLastfm();
	}

	// #327: this menu is where people look for scrobbling settings.
	function openScrobbleSettings() {
		menuOpen = false;
		ui.settingsFocus = 'scrobbling';
		ui.settingsOpen = true;
	}

	const scrobblerTitle = $derived(
		connecting
			? t('integrations.lastfm_connecting')
			: connected
				? t('integrations.lastfm_scrobbling_as', { user: username ?? '' })
				: t('integrations.lastfm_scrobble_to')
	);
</script>

<!-- `relative` makes this a stacking context, so the account/window dropdowns inside it are capped
     at this z — it must outrank the panels below (LyricsPanel/QueuePanel, z-30). Theater mode is
     the exception: it sits at z-40 so dialogs (z-50) can open over it, so the bar has to duck
     under it instead of the other way round. -->
<header
	data-tauri-drag-region
	class="relative {ui.theaterOpen ? 'z-0' : 'z-50'} flex h-9 shrink-0 select-none items-center justify-between border-b border-border/60 bg-background"
>
	<span
		class="pointer-events-none absolute inset-x-0 text-center text-xs font-medium tracking-wide text-muted-foreground"
	>
		Limusic
	</span>

	<!-- macOS overlay style floats the traffic lights over the top-left of the webview, so the row
	     starts clear of them. `trafficLightPosition` in tauri.macos.conf.json centres them in this
	     36px bar and insets them 14px (#189); they end near 68px, so 70px plus the logo's margin
	     leaves about as much space after them as before them. -->
	<div class="flex h-full items-center {win.chrome === 'overlay' ? 'pl-[70px]' : ''}">
		<!-- pointer-events-none: the logo is decoration; clicks on it should drag the window. -->
		<img src={appIcon.src} alt="" class="pointer-events-none ml-3 mr-1 h-4 w-4" />
		<!-- Bigger and heavier than the icons on the right: these are navigation, and at their
		     weight the arrow read as decoration and got missed. -->
		<button
			class="flex h-full w-9 items-center justify-center text-foreground/80 transition-colors hover:bg-accent/10 hover:text-foreground disabled:pointer-events-none disabled:opacity-25"
			onclick={() => history.back()}
			disabled={depth === 0}
			title={t('common.back')}
			aria-label={t('common.back')}
		>
			<HugeiconsIcon icon={ArrowLeft01Icon} strokeWidth={2.5} class="h-5 w-5" />
		</button>
		<button
			class="flex h-full w-9 items-center justify-center text-foreground/80 transition-colors hover:bg-accent/10 hover:text-foreground disabled:pointer-events-none disabled:opacity-25"
			onclick={() => history.forward()}
			disabled={depth === deepest}
			title={t('common.forward')}
			aria-label={t('common.forward')}
		>
			<HugeiconsIcon icon={ArrowRight01Icon} strokeWidth={2.5} class="h-5 w-5" />
		</button>
		<!-- Third in the browser's own control group, and read as such without a label. It drops the
		     browse cache and remounts the page, which is the only way to ask YouTube for a different
		     home feed: refetching rotates roughly a third of "Quick picks" (#177). Never disabled,
		     since the page it reloads is always there; the route's own skeletons are the progress. -->
		<button
			class="flex h-full w-9 items-center justify-center text-foreground/80 transition-colors hover:bg-accent/10 hover:text-foreground"
			onclick={refreshView}
			title={t('common.refresh')}
			aria-label={t('common.refresh')}
		>
			<HugeiconsIcon icon={Refresh03Icon} strokeWidth={2.5} class="h-4 w-4" />
		</button>
	</div>

	<div class="flex h-full items-center">
		<!-- Account first, then the integrations, then the window controls. The drag region lives on
		     <header> only, so these children are ordinary buttons — don't add the attribute here. -->
		<AccountMenu />
		<div class="mx-1.5 h-4 w-px bg-border"></div>

		<!-- Paste a YouTube Music link and go to it: the only way into a playlist that is shared by
		     link and never appears in search or the library (#63). -->
		<button
			class="flex h-full w-8 items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground"
			onclick={() => (ui.linkOpen = true)}
			title={t('dialogs.link.title')}
			aria-label={t('dialogs.link.title')}
		>
			<HugeiconsIcon icon={Link04Icon} class="h-4 w-4" />
		</button>

		<!-- Opens the same modal as the home hero's button (one dialog, mounted in +layout). -->
		<button
			class="flex h-full w-8 items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground {lt.role !==
			'none'
				? 'text-primary'
				: ''}"
			onclick={() => (ui.ltOpen = true)}
			title={t('nav.listen_together')}
			aria-label={t('nav.listen_together')}
		>
			<span class="relative">
				<HugeiconsIcon icon={UserGroup02Icon} class="h-4 w-4" />
				{#if lt.role !== 'none'}
					<!-- Discord's status dot with a ping behind it: two layers, because animate-ping
					     scales and fades the element it's on, so a lone dot would blink out. -->
					<span class="absolute -right-0.5 -top-0.5 h-1.5 w-1.5">
						<span class="absolute inset-0 animate-ping rounded-full bg-emerald-500 opacity-75"
						></span>
						<span class="absolute inset-0 rounded-full bg-emerald-500 ring-[1.5px] ring-background"
						></span>
					</span>
				{/if}
			</span>
		</button>

		<button
			class="flex h-full w-8 items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground {discordOn
				? 'text-foreground'
				: ''}"
			onclick={toggleDiscord}
			title={discordOn ? t('integrations.discord_tooltip_on') : t('integrations.discord_tooltip_off')}
			aria-label={t('settings.general.discord_rpc')}
		>
			<span class="relative">
				<DiscordIcon class="h-4 w-4" />
				<!-- Presence status dot, Discord-style: green = live, red = off. -->
				<span
					class="absolute -right-0.5 -top-0.5 h-1.5 w-1.5 rounded-full ring-[1.5px] ring-background {discordOn
						? 'bg-emerald-500'
						: 'bg-red-500'}"
				></span>
			</span>
		</button>

		<button
			class="flex h-full w-8 items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground {connected
				? 'text-foreground'
				: ''}"
			onclick={onScrobblerClick}
			title={scrobblerTitle}
			aria-label={scrobblerTitle}
		>
			<span class="relative">
				<LastFmIcon class="h-4 w-4 {connecting ? 'animate-pulse opacity-60' : ''}" />
				{#if connecting}
					<HugeiconsIcon
						icon={Loading03Icon}
						strokeWidth={2.5}
						class="absolute -bottom-1.5 -right-2 h-3.5 w-3.5 animate-spin text-primary"
					/>
				{:else if connected}
					<!-- bg-background ring so the badge reads over the icon's stroke. -->
					<HugeiconsIcon
						icon={CheckmarkCircle01Icon}
						strokeWidth={2.5}
						class="absolute -bottom-1.5 -right-2 h-3.5 w-3.5 rounded-full bg-background text-primary"
					/>
				{/if}
			</span>
		</button>

		<!-- Theater mode: fullscreen, cover and lyrics, nothing else. Next to the mini player because
		     the pair are the same idea in opposite directions (shrink the app / become the screen),
		     and disabled with nothing playing, since there'd be nothing to show. -->
		<button
			class="flex h-full w-8 items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground disabled:pointer-events-none disabled:opacity-25"
			onclick={() => (ui.theaterOpen = true)}
			disabled={!playback.now}
			title={t('player.theater_mode')}
			aria-label={t('player.theater_mode')}
		>
			<HugeiconsIcon icon={CameraVideoIcon} class="h-4 w-4" />
		</button>

		<!-- Mini player: hides the app to the tray and hands over to the floating widget (mini.rs).
		     It sits with the integrations rather than the window controls because it swaps what
		     you're using, not the size of this window. -->
		<button
			class="flex h-full w-8 items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground"
			onclick={openMiniPlayer}
			title={t('a11y.toggle_mini')}
			aria-label={t('a11y.toggle_mini')}
		>
			<HugeiconsIcon icon={MinimizeScreenIcon} class="h-4 w-4" />
		</button>

		{#if win.chrome === 'off'}
			<div class="mx-1.5 h-4 w-px bg-border"></div>

			<button
				class="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground"
				onclick={() => w.minimize()}
				aria-label={t('common.minimize')}
			>
				<HugeiconsIcon icon={MinusSignIcon} class="h-4 w-4" />
			</button>
			<button
				class="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground"
				onclick={() => w.toggleMaximize()}
				aria-label={t('common.maximize')}
			>
				<HugeiconsIcon icon={SquareIcon} class="h-3.5 w-3.5" />
			</button>
			<button
				class="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:text-destructive"
				onclick={() => w.close()}
				aria-label={t('common.close')}
			>
				<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
			</button>
		{:else}
			<!-- The system frame owns closing; leave a little air before its own buttons. -->
			<div class="w-2"></div>
		{/if}
	</div>
</header>

{#if menuOpen}
	<button
		class="fixed inset-0 z-40 cursor-default"
		onclick={() => (menuOpen = false)}
		aria-label={t('common.close')}
	></button>
	<div
		class="fixed z-50 min-w-52 animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style={anchor.style}
		{@attach fitMenu(anchor)}
	>
		<div class="flex items-center gap-2.5 px-2 py-2">
			<LastFmIcon class="h-4 w-4 shrink-0" />
			<div class="min-w-0">
				<div class="text-sm font-medium leading-tight">Last.fm</div>
				<div class="truncate text-xs text-muted-foreground">
					{t('integrations.lastfm_scrobbling_as', { user: username ?? '' })}
				</div>
			</div>
		</div>
		<div class="mx-1 my-1 h-px bg-border"></div>
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={openScrobbleSettings}
		>
			<HugeiconsIcon icon={Settings02Icon} class="h-4 w-4" /> {t('integrations.lastfm_settings')}
		</button>
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-destructive hover:bg-destructive/10"
			onclick={disconnect}
		>
			<HugeiconsIcon icon={HotspotOfflineIcon} class="h-4 w-4" /> {t('integrations.disconnect')}
		</button>
	</div>
{/if}
