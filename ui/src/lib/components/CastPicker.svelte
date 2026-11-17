<script lang="ts">
	// Where the music plays, Spotify Connect style: this desktop or a device on the network
	// (a Chromecast, a Google Home speaker, a DLNA renderer). src-tauri/src/cast/.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		CastIcon,
		ComputerIcon,
		RefreshIcon,
		Speaker01Icon,
		Tv01Icon
	} from '@hugeicons/core-free-icons';
	import * as Popover from '$lib/components/ui/popover';
	import { Button } from '$lib/components/ui/button';
	import * as api from '$lib/api';
	import { castState } from '$lib/cast.svelte';
	import { deviceSubtitle, isPlaying } from '$lib/cast';
	import { playback, toast } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	let open = $state(false);
	// The button waits for the device while it connects, so it is not clicked twice.
	let busy = $state(false);

	const casting = $derived(castState.active && castState.device !== null);
	const device = $derived(castState.device);
	const label = $derived(
		casting
			? t('cast.playing_on', { name: device?.name ?? '' })
			: t('cast.button')
	);

	function iconFor(kind: string): typeof Speaker01Icon {
		return kind === 'dlna' ? Tv01Icon : Speaker01Icon;
	}

	const subtitle = (d: api.CastDevice) =>
		deviceSubtitle(d, t('cast.chromecast'), t('cast.dlna'));

	async function playOn(deviceId: string) {
		if (busy) return;
		busy = true;
		try {
			await api.castStart(deviceId);
			open = false;
		} catch (e) {
			toast.error(String(e));
		} finally {
			busy = false;
		}
	}

	async function stopCasting() {
		busy = true;
		try {
			await api.castStop();
			open = false;
		} catch (e) {
			toast.error(String(e));
		} finally {
			busy = false;
		}
	}
</script>

<Popover.Root bind:open>
	<!-- While a device is playing, the button names it: the only marker that this bar is not
	     playing here. One trigger whose contents change, never two in an if/else: bits-ui keeps
	     a single trigger node per popover. -->
	<Popover.Trigger
		class={casting
			? 'inline-flex h-8 max-w-44 cursor-pointer items-center gap-1.5 rounded-full bg-primary/15 px-3 text-xs font-medium text-primary transition hover:bg-primary/25'
			: 'inline-flex size-8 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition hover:bg-muted hover:text-foreground'}
		aria-label={label}
		title={label}
	>
		{#if casting}
			<HugeiconsIcon icon={CastIcon} class="size-4 shrink-0" />
			<span class="truncate">{device?.name}</span>
		{:else}
			<HugeiconsIcon icon={CastIcon} class="h-5 w-5" />
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
				<p class="font-heading text-sm font-semibold">{t('cast.title')}</p>
				<p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">
					{playback.now ? t('cast.hint') : t('cast.nothing_playing')}
				</p>
			</div>
			<Button
				variant="ghost"
				size="icon-sm"
				class="-mr-1.5 -mt-1 shrink-0 text-muted-foreground"
				aria-label={t('cast.refresh')}
				title={t('cast.refresh')}
				onclick={() =>
					api.castRefresh().then((d) => (castState.devices = d)).catch(() => {})}
			>
				<HugeiconsIcon icon={RefreshIcon} class="h-4 w-4" />
			</Button>
		</div>
		<div class="flex flex-col gap-0.5 p-1.5">
			{#if casting}
				<!-- The music comes back here: the same row the device picker uses for this
				     computer, so the way back is where the way out was. -->
				<button
					class="flex w-full cursor-pointer items-center gap-3 rounded-lg px-2.5 py-2 text-left transition enabled:hover:bg-foreground/10 disabled:opacity-50"
					disabled={busy}
					onclick={stopCasting}
				>
					<span
						class="flex size-8 shrink-0 items-center justify-center rounded-md bg-primary/15 text-primary"
					>
						<HugeiconsIcon icon={ComputerIcon} class="size-4" />
					</span>
					<span class="min-w-0 flex-1">
						<span class="block truncate text-sm font-medium">{t('cast.back_here')}</span>
						<span class="block truncate text-xs text-muted-foreground">
							{t('cast.playing_on', { name: device?.name ?? '' })}
						</span>
					</span>
				</button>
			{/if}
			{#each castState.devices as d (d.id)}
				<button
					class="flex w-full cursor-pointer items-center gap-3 rounded-lg px-2.5 py-2 text-left transition enabled:hover:bg-foreground/10 disabled:opacity-50"
					disabled={busy}
					aria-current={casting && isPlaying(d, device?.id ?? null)}
					onclick={() => playOn(d.id)}
				>
					<span
						class="flex size-8 shrink-0 items-center justify-center rounded-md {casting &&
						device?.id === d.id
							? 'bg-primary/15 text-primary'
							: 'bg-muted text-muted-foreground'}"
					>
						<HugeiconsIcon icon={iconFor(d.kind)} class="size-4" />
					</span>
					<span class="min-w-0 flex-1">
						<span class="block truncate text-sm font-medium">{d.name}</span>
						<span class="block truncate text-xs text-muted-foreground">
							{subtitle(d)}
						</span>
					</span>
				</button>
			{/each}
			{#if !castState.devices.length}
				<div class="px-2.5 py-3 text-center text-xs leading-relaxed text-muted-foreground">
					{t('cast.no_devices')}
				</div>
			{/if}
		</div>
	</Popover.Content>
</Popover.Root>
