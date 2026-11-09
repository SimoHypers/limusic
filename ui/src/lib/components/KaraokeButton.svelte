<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowUp01Icon, SingLeftIcon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import * as Popover from '$lib/components/ui/popover';
	import KaraokePanel from './KaraokePanel.svelte';
	import { playback, karaokeStatus, karaokeAvailability, toggleKaraoke } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	/**
	 * Two parts, like a call app's mute button. The icon turns karaoke on and off, always: vocals
	 * back for a verse and gone again is the thing people do most, so it stays one click (it used to
	 * retry instead while the track had failed, which left no way to turn karaoke off). The chevron
	 * opens the panel with the queue, the quality and anything that went wrong.
	 *
	 * `bar`: the player bar's right cluster, ghost shadcn Buttons like its neighbours.
	 * `pill`: theater mode's title pill, raw buttons styled like the pill's own toggles.
	 * Nothing here animates and nothing changes size with the state (docs/UI-PERFORMANCE.md).
	 */
	let { variant = 'bar' }: { variant?: 'bar' | 'pill' } = $props();

	let open = $state(false);

	const on = $derived(playback.karaoke);
	const phase = $derived(karaokeStatus.state);
	const preparing = $derived(on && phase === 'preparing');
	// Couldn't start on this machine last time (runtime, VC++, proxy). Still clickable: turning it
	// on again is how a fix (a redist install) gets checked.
	const unavailable = $derived(!on && karaokeAvailability.reason !== undefined);

	// Same convention as shuffle and repeat: the state is the icon's colour, never a background.
	const tone = $derived.by(() => {
		if (on && phase === 'error') return 'text-destructive';
		if (on && phase === 'too-long') return 'text-primary/50';
		if (on) return 'text-primary';
		if (unavailable) return 'text-muted-foreground/50';
		return variant === 'pill' ? 'text-muted-foreground group-hover:text-foreground' : '';
	});

	const label = $derived.by(() => {
		if (!on) return t('player.karaoke');
		if (phase === 'error') return t('karaoke.button.failed');
		if (phase === 'too-long') return t('karaoke.button.too_long');
		if (phase === 'preparing') return t('karaoke.button.preparing');
		return t('karaoke.button.ready');
	});

	// While this track is being prepared the icon sits in a ring that fills with its progress.
	// Static: it only redraws when progress arrives.
	const ring = $derived(
		karaokeStatus.progress !== undefined
			? `conic-gradient(var(--primary) ${karaokeStatus.progress * 100}%, var(--muted) 0)`
			: 'var(--muted)'
	);
</script>

{#snippet face(iconClass: string, ringClass: string, smallIconClass: string)}
	{#if preparing}
		<span class="relative flex items-center justify-center {ringClass}">
			<span
				class="absolute inset-0 rounded-full"
				style="background: {ring}; mask: radial-gradient(farthest-side, transparent calc(100% - 2px), var(--foreground) calc(100% - 2px));"
			></span>
			<HugeiconsIcon icon={SingLeftIcon} class="{smallIconClass} text-primary" />
		</span>
	{:else}
		<HugeiconsIcon icon={SingLeftIcon} class="{iconClass} {tone}" />
	{/if}
{/snippet}

{#snippet panel()}
	<!-- Above theater mode, which sits over the page like a dialog does. -->
	<Popover.Content
		side="top"
		align="end"
		sideOffset={10}
		class="z-[60] w-80 max-w-[calc(100vw-2rem)] gap-0 overflow-hidden p-0"
	>
		<KaraokePanel onclose={() => (open = false)} />
	</Popover.Content>
{/snippet}

{#if variant === 'pill'}
	<span class="flex items-center">
		<button
			type="button"
			onclick={toggleKaraoke}
			class="group flex size-8 cursor-pointer items-center justify-center rounded-full transition-colors hover:bg-foreground/10"
			aria-label={t('player.karaoke')}
			aria-pressed={on}
			title={label}
		>
			{@render face('h-4 w-4', 'size-[22px]', 'h-3 w-3')}
		</button>
		<Popover.Root bind:open>
			<Popover.Trigger
				class="-ml-1 flex h-8 w-4 cursor-pointer items-center justify-center rounded-full outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring {open
					? 'text-foreground'
					: 'text-muted-foreground'}"
				aria-label={t('karaoke.button.options')}
				title={t('karaoke.button.options')}
			>
				<HugeiconsIcon icon={ArrowUp01Icon} class="h-3 w-3" />
			</Popover.Trigger>
			{@render panel()}
		</Popover.Root>
	</span>
{:else}
	<span class="flex items-center">
		<Button
			variant="ghost"
			size="icon-sm"
			onclick={toggleKaraoke}
			aria-label={t('player.karaoke')}
			aria-pressed={on}
			title={label}
		>
			{@render face('h-5 w-5', 'size-6', 'h-3.5 w-3.5')}
		</Button>
		<Popover.Root bind:open>
			<Popover.Trigger
				class="-ml-1 flex h-8 w-4 cursor-pointer items-center justify-center rounded-md outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring {open
					? 'text-foreground'
					: 'text-muted-foreground'}"
				aria-label={t('karaoke.button.options')}
				title={t('karaoke.button.options')}
			>
				<HugeiconsIcon icon={ArrowUp01Icon} class="h-3 w-3" />
			</Popover.Trigger>
			{@render panel()}
		</Popover.Root>
	</span>
{/if}
