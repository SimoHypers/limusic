<script lang="ts">
	// Where the music plays, Spotify Connect style: this desktop, a linked phone, or a Cast device
	// on the network. Picking one moves what plays there, where it is (src-tauri/src/devicelink.rs,
	// src-tauri/src/castout.rs).
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		CastIcon,
		ComputerIcon,
		ComputerPhoneSyncIcon,
		Loading03Icon,
		PlusSignIcon,
		Settings02Icon,
		SmartPhone01Icon,
		Speaker01Icon,
		Tv01Icon,
		VolumeHighIcon
	} from '@hugeicons/core-free-icons';
	import * as Popover from '$lib/components/ui/popover';
	import { Button } from '$lib/components/ui/button';
	import * as api from '$lib/api';
	import { link } from '$lib/link.svelte';
	import { applyCastDevices, cast, castMessage } from '$lib/cast.svelte';
	import { playback, toast, ui } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	let open = $state(false);
	// Cast devices answer within a second or two of the search starting; until then an empty list
	// means "still looking", not "nothing here".
	let searching = $state(false);
	let searchTimer: ReturnType<typeof setTimeout> | undefined;

	const remotePlayer = $derived(link.remote ? link.player : null);
	const casting = $derived(cast.device);
	const here = $derived(!link.remote && !casting);
	const label = $derived(
		remotePlayer
			? link.isPlaying
				? t('player.link_playing_on', { name: remotePlayer.name })
				: t('player.link_paused_on', { name: remotePlayer.name })
			: casting
				? playback.paused
					? t('player.link_paused_on', { name: casting.name })
					: t('player.link_playing_on', { name: casting.name })
				: t('player.devices')
	);

	// Discovery runs only while the picker is open (and while a cast plays): the first search is
	// what makes Windows Firewall and macOS ask, so nobody is asked who never opens this.
	function onOpenChange(o: boolean) {
		clearTimeout(searchTimer);
		searching = o && cast.devices.length === 0;
		if (o) searchTimer = setTimeout(() => (searching = false), 5000);
		api.castPicker(o)
			.then((devices) => o && applyCastDevices(devices))
			.catch(() => {});
	}

	async function playOn(deviceId: string) {
		open = false;
		try {
			await api.linkPlayOn(deviceId);
		} catch (e) {
			toast.error(String(e));
		}
	}

	async function castTo(d: api.CastDevice) {
		if (cast.connecting) return;
		if (casting?.id === d.id) {
			open = false;
			return;
		}
		try {
			await api.castStart(d.id);
			open = false;
		} catch (e) {
			toast.error(castMessage(String(e), d.name));
		}
	}

	async function selectHere() {
		open = false;
		try {
			if (casting) await api.castStop();
			else if (link.remote) await api.linkPlayOn('');
		} catch (e) {
			toast.error(String(e));
		}
	}

	function linkPhone() {
		open = false;
		ui.settingsFocus = 'devices';
		ui.settingsOpen = true;
	}

	const deviceStatus = (d: api.LinkDevice) => {
		if (!d.online) return t('settings.devices.offline');
		if (link.player?.id === d.id) return link.isPlaying ? t('settings.devices.playing') : t('player.devices_paused');
		return t('settings.devices.online');
	};

	const castIcon = (d: api.CastDevice) =>
		d.kind === 'tv' ? Tv01Icon : d.kind === 'group' ? CastIcon : Speaker01Icon;

	const castStatus = (d: api.CastDevice) => {
		if (cast.connecting === d.id) return t('cast.connecting');
		if (casting?.id === d.id) return playback.paused ? t('player.devices_paused') : t('settings.devices.playing');
		return d.model || t(`cast.kind_${d.kind}`);
	};
</script>

	{#snippet row(icon: typeof ComputerIcon, name: string, status: string, active: boolean, disabled: boolean, pick: () => void, busy = false)}
		<button
			class="flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-left transition enabled:cursor-pointer disabled:opacity-50 {active
				? 'bg-primary/10'
				: 'enabled:hover:bg-foreground/10'}"
			{disabled}
			aria-current={active}
			onclick={pick}
		>
			<span
				class="flex size-8 shrink-0 items-center justify-center rounded-md {active
					? 'bg-primary/15 text-primary'
					: 'bg-muted text-muted-foreground'}"
			>
				<HugeiconsIcon {icon} class="size-4" />
			</span>
			<span class="min-w-0 flex-1">
				<span class="block truncate text-sm font-medium {active ? 'text-primary' : ''}">{name}</span>
				<span class="block truncate text-xs text-muted-foreground">{status}</span>
			</span>
			{#if busy}
				<HugeiconsIcon icon={Loading03Icon} class="size-4 shrink-0 animate-spin text-muted-foreground" />
			{:else if active}
				<HugeiconsIcon icon={VolumeHighIcon} class="size-4 shrink-0 text-primary" />
			{/if}
		</button>
	{/snippet}

<Popover.Root bind:open {onOpenChange}>
	<!-- While the bar is another device's remote, the button names that device: the only marker
	     that this bar is not playing here. One trigger whose contents change, never two in an
	     if/else: bits-ui keeps a single trigger node per popover, and the one unmounting on a
	     handover clears it after the new one set it, so the popover stopped opening.
	     No title: on Hyprland and niri the GTK tooltip opens under the pointer at the bottom
	     edge, the hover drops, the tooltip hides, and it loops into a flicker. -->
	<Popover.Trigger
		class={remotePlayer || casting
			? 'inline-flex h-8 max-w-44 cursor-pointer items-center gap-1.5 rounded-full bg-primary/15 px-3 text-xs font-medium text-primary transition hover:bg-primary/25'
			: `inline-flex size-8 cursor-pointer items-center justify-center rounded-md transition hover:bg-muted ${here
					? 'text-muted-foreground hover:text-foreground'
					: 'text-primary'}`}
		aria-label={label}
	>
		{#if remotePlayer}
			<HugeiconsIcon icon={SmartPhone01Icon} class="size-4 shrink-0" />
			<span class="truncate">{remotePlayer.name}</span>
		{:else if casting}
			<HugeiconsIcon icon={castIcon(casting)} class="size-4 shrink-0" />
			<span class="truncate">{casting.name}</span>
		{:else}
			<HugeiconsIcon icon={ComputerPhoneSyncIcon} class="h-5 w-5" />
		{/if}
	</Popover.Trigger>
	<Popover.Content
		side="top"
		align="end"
		sideOffset={10}
		class="w-80 max-w-[calc(100vw-2rem)] gap-0 overflow-hidden p-0"
	>
		<div class="flex items-start gap-3 border-b px-4 pb-3 pt-3.5">
			<div class="min-w-0 flex-1">
				<p class="font-heading text-sm font-semibold">{t('player.devices_title')}</p>
				<p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">{t('player.devices_hint')}</p>
			</div>
			<Button
				variant="ghost"
				size="icon-sm"
				class="-mr-1.5 -mt-1 shrink-0 text-muted-foreground"
				aria-label={t('player.devices_settings')}
				title={t('player.devices_settings')}
				onclick={linkPhone}
			>
				<HugeiconsIcon icon={Settings02Icon} class="h-4 w-4" />
			</Button>
		</div>
		<div class="flex flex-col gap-0.5 p-1.5">
			{@render row(
				ComputerIcon,
				t('player.this_computer'),
				here && playback.now
					? playback.paused
						? t('player.devices_paused')
						: t('player.devices_playing_here')
					: t('player.devices_this_computer_hint'),
				here,
				false,
				selectHere
			)}
			{#each link.devices as d (d.id)}
				{@render row(
					SmartPhone01Icon,
					d.name,
					deviceStatus(d),
					!!remotePlayer && link.player?.id === d.id,
					!d.online,
					() => (remotePlayer && link.player?.id === d.id ? (open = false) : playOn(d.id))
				)}
			{/each}
			{#each cast.devices as d (d.id)}
				{@render row(
					castIcon(d),
					d.name,
					castStatus(d),
					casting?.id === d.id,
					!!cast.connecting && cast.connecting !== d.id,
					() => castTo(d),
					cast.connecting === d.id
				)}
			{/each}
			{#if cast.devices.length === 0}
				<p class="flex items-center gap-2 px-2.5 py-2 text-xs text-muted-foreground">
					{#if searching}
						<HugeiconsIcon icon={Loading03Icon} class="size-3.5 shrink-0 animate-spin" />
						{t('cast.searching')}
					{:else}
						{t('cast.none_found')}
					{/if}
				</p>
			{/if}
		</div>
		<div class="border-t p-1.5">
			<button
				class="flex w-full cursor-pointer items-center gap-3 rounded-lg px-2.5 py-2 text-left text-sm text-muted-foreground transition hover:bg-foreground/10 hover:text-foreground"
				onclick={linkPhone}
			>
				<span class="flex size-8 shrink-0 items-center justify-center">
					<HugeiconsIcon icon={PlusSignIcon} class="size-4" />
				</span>
				{link.devices.length ? t('settings.devices.link_another') : t('settings.devices.link_phone')}
			</button>
		</div>
	</Popover.Content>
</Popover.Root>
