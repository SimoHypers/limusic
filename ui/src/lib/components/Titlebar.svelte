<script lang="ts">
	// Custom titlebar (the window runs undecorated — tauri.conf `decorations: false`). Everything
	// on the bar is a drag region except the buttons; double-click maximizes (handled by Tauri's
	// drag region itself). Right cluster: Last.fm scrobbler | separator | minimize / maximize /
	// close — per the design, the scrobbler lives with the window controls but visually apart.
	// Account (sign in/out) sits first in that cluster, in its own component.
	import { onMount } from 'svelte';
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		MinusSignIcon,
		SquareIcon,
		Cancel01Icon,
		CheckmarkCircle01Icon,
		Loading03Icon,
		HotspotOfflineIcon
	} from '@hugeicons/core-free-icons';
	import LastFmIcon from './LastFmIcon.svelte';
	import DiscordIcon from './DiscordIcon.svelte';
	import AccountMenu from './AccountMenu.svelte';
	import logo from '$lib/assets/favicon.svg';
	import * as api from '$lib/api';
	import { toast } from '$lib/player.svelte';

	const win = getCurrentWindow();

	// Last.fm connection state. `connecting` is UI-local: set on click, cleared by the
	// `lastfm-state` event (success, failure, or timeout) — the backend always answers.
	let connected = $state(false);
	let username = $state<string | null>(null);
	let connecting = $state(false);
	let menuOpen = $state(false);
	let mx = $state(0);
	let my = $state(0);

	// Discord Rich Presence — a plain on/off toggle of the `discord_rpc` setting (the backend
	// connects/clears the presence the moment it flips). Optimistic; reverted on failure.
	let discordOn = $state(false);

	async function toggleDiscord() {
		const next = !discordOn;
		discordOn = next;
		try {
			await api.setSetting('discord_rpc', next ? 'true' : 'false');
			toast(next ? 'Discord presence on' : 'Discord presence off');
		} catch (e) {
			discordOn = !next;
			toast(String(e));
		}
	}

	onMount(() => {
		api.getSettings()
			.then((s) => (discordOn = s.discord_rpc === 'true'))
			.catch(() => {});
		api.lastfmStatus()
			.then((s) => {
				connected = s.connected;
				username = s.username ?? null;
			})
			.catch(() => {});
		const sub = api.onLastfmState((s) => {
			const wasConnecting = connecting;
			connecting = false;
			connected = s.connected;
			username = s.username ?? null;
			if (s.error) toast(s.error);
			else if (s.connected) toast(`Scrobbling as ${s.username}`);
			else if (!wasConnecting) toast('Last.fm disconnected');
		});
		return () => sub.then((u) => u());
	});

	async function onScrobblerClick(e: MouseEvent) {
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
		connecting = true;
		try {
			await api.lastfmConnect();
			toast('Approve Limusic in your browser');
		} catch (err) {
			connecting = false;
			toast(String(err));
		}
	}

	function openMenu(e: MouseEvent) {
		const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
		mx = window.innerWidth - r.right;
		my = r.bottom + 6;
		menuOpen = true;
	}

	function disconnect() {
		menuOpen = false;
		api.lastfmDisconnect().catch((e) => toast(String(e)));
	}

	const scrobblerTitle = $derived(
		connecting
			? 'Connecting to Last.fm — click to cancel'
			: connected
				? `Scrobbling as ${username}`
				: 'Scrobble to Last.fm'
	);
</script>

<!-- `relative` makes this a stacking context, so the account/window dropdowns inside it are capped
     at this z — it must outrank the panels below (LyricsPanel/QueuePanel, z-30). -->
<header
	data-tauri-drag-region
	class="relative z-50 flex h-9 shrink-0 select-none items-center justify-between border-b border-border/60 bg-background"
>
	<span
		class="pointer-events-none absolute inset-x-0 text-center text-xs font-medium tracking-wide text-muted-foreground"
	>
		Limusic
	</span>

	<!-- pointer-events-none: the logo is decoration; clicks on it should drag the window. -->
	<img src={logo} alt="" class="pointer-events-none ml-3 h-4 w-4" />

	<div class="flex h-full items-center">
		<!-- Account first, then the integrations, then the window controls. The drag region lives on
		     <header> only, so these children are ordinary buttons — don't add the attribute here. -->
		<AccountMenu />
		<div class="mx-1.5 h-4 w-px bg-border"></div>

		<button
			class="flex h-full w-8 items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground {discordOn
				? 'text-foreground'
				: ''}"
			onclick={toggleDiscord}
			title={discordOn ? 'Discord presence on — click to turn off' : 'Show what you play on Discord'}
			aria-label="Discord Rich Presence"
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

		<div class="mx-1.5 h-4 w-px bg-border"></div>

		<button
			class="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground"
			onclick={() => win.minimize()}
			aria-label="Minimize"
		>
			<HugeiconsIcon icon={MinusSignIcon} class="h-4 w-4" />
		</button>
		<button
			class="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground"
			onclick={() => win.toggleMaximize()}
			aria-label="Maximize"
		>
			<HugeiconsIcon icon={SquareIcon} class="h-3.5 w-3.5" />
		</button>
		<button
			class="flex h-full w-11 items-center justify-center text-muted-foreground transition-colors hover:text-destructive"
			onclick={() => win.close()}
			aria-label="Close"
		>
			<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
		</button>
	</div>
</header>

{#if menuOpen}
	<button
		class="fixed inset-0 z-40 cursor-default"
		onclick={() => (menuOpen = false)}
		aria-label="Close menu"
	></button>
	<div
		class="fixed z-50 min-w-52 origin-top-right animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style="right:{mx}px; top:{my}px;"
	>
		<div class="flex items-center gap-2.5 px-2 py-2">
			<LastFmIcon class="h-4 w-4 shrink-0" />
			<div class="min-w-0">
				<div class="text-sm font-medium leading-tight">Last.fm</div>
				<div class="truncate text-xs text-muted-foreground">Scrobbling as {username}</div>
			</div>
		</div>
		<div class="mx-1 my-1 h-px bg-border"></div>
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-destructive hover:bg-destructive/10"
			onclick={disconnect}
		>
			<HugeiconsIcon icon={HotspotOfflineIcon} class="h-4 w-4" /> Disconnect
		</button>
	</div>
{/if}
