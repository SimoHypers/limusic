<script lang="ts">
	// The titlebar's integration buttons in one place, because the bar cannot keep them at every
	// width: below sm they are hidden (the window controls and the menu button need the room), and
	// the sidebar drawer carries them instead — to the right of its logo, where they are a click
	// away from the page rather than two clicks through Settings.
	// `itemClass` is the difference between the titlebar's full-height tabs and the drawer's square
	// buttons; everything else, including the Last.fm menu, is shared.
	import { onMount } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		CameraVideoIcon,
		CheckmarkCircle01Icon,
		Loading03Icon,
		HotspotOfflineIcon,
		MinimizeScreenIcon,
		Settings02Icon,
		UserGroup02Icon,
		Link04Icon
	} from '@hugeicons/core-free-icons';
	import LastFmIcon from './LastFmIcon.svelte';
	import DiscordIcon from './DiscordIcon.svelte';
	import { appearance } from '$lib/theme.svelte';
	import * as api from '$lib/api';
	import { openMiniPlayer, playback, prefs, toast, ui } from '$lib/player.svelte';
	import { lt } from '$lib/lt.svelte';
	import { anchorMenu, fitMenu, NO_ANCHOR } from '$lib/menu';
	import { t } from '$lib/i18n.svelte';
	import { connectLastfm, disconnectLastfm, lastfm, watchLastfm } from '$lib/lastfm.svelte';
	import { watchListenBrainz } from '$lib/listenbrainz.svelte';

	let {
		itemClass = 'flex h-full w-8 max-sm:hidden items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground'
	}: { itemClass?: string } = $props();

	// Which buttons render, from Appearance ▸ Interface ▸ Top bar. The titlebar's own narrow-width
	// rule (`max-sm:hidden`) is inside `itemClass`, so the drawer can ask for the same buttons
	// without it.
	const shown = (id: string) => !appearance.titlebarHidden.includes(id);

	// Last.fm connection state lives in `lastfm.svelte.ts`: the Scrobbling settings tab connects
	// and disconnects too.
	const connected = $derived(lastfm.connected);
	const username = $derived(lastfm.username);
	const connecting = $derived(lastfm.connecting);
	let menuOpen = $state(false);
	let anchor = $state(NO_ANCHOR);

	// Discord Rich Presence — a plain on/off toggle of the `discord_rpc` setting (the backend
	// connects/clears the presence the moment it flips). Optimistic; reverted on failure. The flag
	// lives in `prefs` because the Discord settings tab toggles the same thing.
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

<!-- Paste a YouTube Music link and go to it: the only way into a playlist that is shared by
     link and never appears in search or the library (#63). -->
{#if shown('link')}
	<button
		class={itemClass}
		onclick={() => (ui.linkOpen = true)}
		title={t('dialogs.link.title')}
		aria-label={t('dialogs.link.title')}
	>
		<HugeiconsIcon icon={Link04Icon} class="h-4 w-4" />
	</button>
{/if}

<!-- Opens the same modal as the home hero's button (one dialog, mounted in +layout). -->
{#if shown('listen_together')}
	<button
		class="{itemClass} {lt.role !== 'none' ? 'text-primary' : ''}"
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
{/if}

{#if shown('discord')}
	<button
		class="{itemClass} {discordOn ? 'text-foreground' : ''}"
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
{/if}

{#if shown('lastfm')}
	<button
		class="{itemClass} {connected ? 'text-foreground' : ''}"
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
{/if}

<!-- Theater mode: fullscreen, cover and lyrics, nothing else. Next to the mini player because
     the pair are the same idea in opposite directions (shrink the app / become the screen),
     and disabled with nothing playing, since there'd be nothing to show. -->
{#if shown('theater')}
	<button
		class="{itemClass} disabled:pointer-events-none disabled:opacity-25"
		onclick={() => (ui.theaterOpen = true)}
		disabled={!playback.now}
		title={t('player.theater_mode')}
		aria-label={t('player.theater_mode')}
	>
		<HugeiconsIcon icon={CameraVideoIcon} class="h-4 w-4" />
	</button>
{/if}

<!-- Mini player: hides the app to the tray and hands over to the floating widget (mini.rs).
     It sits with the integrations rather than the window controls because it swaps what
     you're using, not the size of this window. -->
{#if shown('mini')}
	<button
		class={itemClass}
		onclick={openMiniPlayer}
		title={t('a11y.toggle_mini')}
		aria-label={t('a11y.toggle_mini')}
	>
		<HugeiconsIcon icon={MinimizeScreenIcon} class="h-4 w-4" />
	</button>
{/if}

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
