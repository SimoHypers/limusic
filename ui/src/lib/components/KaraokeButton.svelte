<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowUp01Icon, MicVocalIcon } from '@hugeicons/core-free-icons';
	import * as Popover from '$lib/components/ui/popover';
	import KaraokePanel from './KaraokePanel.svelte';
	import { playback, karaokeStatus, karaokeAvailability, toggleKaraoke } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	/**
	 * Karaoke's switch, in the lyrics footer: singing is what the lyrics are open for, and the player
	 * bar had too many buttons (Apple Music keeps its Sing control in the lyrics view too). One footer
	 * covers the side panel, the player view's lyrics tab and theater mode's lyrics column.
	 *
	 * Two parts, like a call app's mute button. The icon turns karaoke on and off, always: vocals
	 * back for a verse and gone again is the thing people do most, so it stays one click. The chevron
	 * opens the panel with the queue, the quality and anything that went wrong.
	 *
	 * Styled like the footer's other toggles (translation, romanization). Nothing animates and
	 * nothing changes size with the state (docs/UI-PERFORMANCE.md).
	 */
	let open = $state(false);

	const on = $derived(playback.karaoke);
	const phase = $derived(karaokeStatus.state);
	const preparing = $derived(on && phase === 'preparing');
	// Couldn't start on this machine last time (runtime, VC++, proxy). Still clickable: turning it
	// on again is how a fix (a redist install) gets checked.
	const unavailable = $derived(!on && karaokeAvailability.reason !== undefined);

	// Same convention as the footer's other toggles: the state is the colour, never a background.
	const tone = $derived.by(() => {
		if (on && phase === 'error') return 'text-destructive';
		if (on && phase === 'too-long') return 'text-primary/50';
		if (on) return 'text-primary';
		if (unavailable) return 'text-muted-foreground/50';
		return 'hover:text-foreground';
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

<span class="flex shrink-0 items-center">
	<button
		type="button"
		onclick={toggleKaraoke}
		class="flex cursor-pointer items-center rounded-full px-2 py-0.5 transition-colors hover:bg-foreground/10 {tone}"
		aria-label={t('player.karaoke')}
		aria-pressed={on}
		title={label}
	>
		{#if preparing}
			<span class="relative flex size-[18px] items-center justify-center">
				<span
					class="absolute inset-0 rounded-full"
					style="background: {ring}; mask: radial-gradient(farthest-side, transparent calc(100% - 1.5px), var(--foreground) calc(100% - 1.5px));"
				></span>
				<HugeiconsIcon icon={MicVocalIcon} class="h-3 w-3 text-primary" />
			</span>
		{:else}
			<HugeiconsIcon icon={MicVocalIcon} class="h-3.5 w-3.5" />
		{/if}
	</button>
	<Popover.Root bind:open>
		<Popover.Trigger
			class="-ml-1 flex h-6 w-4 cursor-pointer items-center justify-center rounded-full outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring {open
				? 'text-foreground'
				: ''}"
			aria-label={t('karaoke.button.options')}
			title={t('karaoke.button.options')}
		>
			<HugeiconsIcon icon={ArrowUp01Icon} class="h-3 w-3" />
		</Popover.Trigger>
		<!-- Above theater mode, which sits over the page like a dialog does. -->
		<Popover.Content
			side="top"
			align="end"
			sideOffset={10}
			class="z-[60] w-80 max-w-[calc(100vw-2rem)] gap-0 overflow-hidden p-0"
		>
			<KaraokePanel onclose={() => (open = false)} />
		</Popover.Content>
	</Popover.Root>
</span>
