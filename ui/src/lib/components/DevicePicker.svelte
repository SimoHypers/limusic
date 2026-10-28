<script lang="ts">
	// Where the music plays, Spotify Connect style: this desktop or a linked device. Picking one
	// moves what plays there, where it is (src-tauri/src/devicelink.rs).
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		ComputerIcon,
		LaptopPhoneSyncIcon,
		SmartPhone01Icon,
		Tick02Icon
	} from '@hugeicons/core-free-icons';
	import * as Popover from '$lib/components/ui/popover';
	import * as api from '$lib/api';
	import { link } from '$lib/link.svelte';
	import { playback, toast, ui } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	let open = $state(false);

	const here = $derived(!link.remote);

	async function playOn(deviceId: string) {
		open = false;
		try {
			await api.linkPlayOn(deviceId);
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
</script>

	{#snippet row(icon: typeof ComputerIcon, name: string, status: string, active: boolean, disabled: boolean, pick: () => void)}
		<button
			class="flex w-full items-center gap-3 rounded-md px-2.5 py-2 text-left transition enabled:cursor-pointer enabled:hover:bg-muted disabled:opacity-50"
			{disabled}
			onclick={pick}
		>
			<span
				class="flex size-9 shrink-0 items-center justify-center rounded-full {active
					? 'bg-primary text-primary-foreground'
					: 'bg-muted text-muted-foreground'}"
			>
				<HugeiconsIcon {icon} class="h-4 w-4" />
			</span>
			<span class="min-w-0 flex-1">
				<span class="block truncate text-sm font-medium {active ? 'text-primary' : ''}">{name}</span>
				<span class="block truncate text-xs text-muted-foreground">{status}</span>
			</span>
			{#if active}<HugeiconsIcon icon={Tick02Icon} class="h-4 w-4 shrink-0 text-primary" />{/if}
		</button>
	{/snippet}

<Popover.Root bind:open>
	<Popover.Trigger
		class="inline-flex size-8 cursor-pointer items-center justify-center rounded-md transition hover:bg-muted {here
			? 'text-muted-foreground hover:text-foreground'
			: 'text-primary'}"
		aria-label={t('player.devices')}
		title={t('player.devices')}
	>
		<HugeiconsIcon icon={LaptopPhoneSyncIcon} class="h-5 w-5" />
	</Popover.Trigger>
	<Popover.Content side="top" align="end" sideOffset={10} class="w-72 p-1.5">
		<p class="px-2.5 pb-1.5 pt-1 text-xs font-medium text-muted-foreground">
			{t('player.devices_title')}
		</p>
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
			() => (here ? (open = false) : playOn(''))
		)}
		{#each link.devices as d (d.id)}
			{@render row(
				SmartPhone01Icon,
				d.name,
				deviceStatus(d),
				!here && link.player?.id === d.id,
				!d.online,
				() => (!here && link.player?.id === d.id ? (open = false) : playOn(d.id))
			)}
		{/each}
		{#if !link.devices.length}
			<button
				class="mt-1 w-full cursor-pointer rounded-md px-2.5 py-2 text-left text-sm text-primary transition hover:bg-muted"
				onclick={linkPhone}
			>
				{t('settings.devices.link_phone')}
			</button>
		{/if}
	</Popover.Content>
</Popover.Root>
