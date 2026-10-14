<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { SingLeftIcon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { playback, karaokeStatus, karaokeQueue, toggleKaraoke, retryKaraoke } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	/**
	 * `bar`: the player bar's right cluster, a ghost shadcn Button like its neighbours.
	 * `pill`: theater mode's view pill, a raw button styled like the pill's own toggles.
	 * Nothing here animates, and the queue dots are absolutely positioned, so the control's box
	 * never changes size when they come and go (docs/UI-PERFORMANCE.md).
	 */
	let { variant = 'bar' }: { variant?: 'bar' | 'pill' } = $props();

	const preparing = $derived(playback.karaoke && karaokeStatus.state === 'preparing');
	const failed = $derived(playback.karaoke && karaokeStatus.state === 'error');

	// Same convention as shuffle/repeat: the state is the icon colour, never a background.
	const tone = $derived(
		failed
			? 'text-destructive'
			: playback.karaoke
				? 'text-primary'
				: variant === 'pill'
					? 'text-muted-foreground group-hover:text-foreground'
					: 'text-muted-foreground'
	);

	// Ring around the icon while preparing: progress as a conic sweep, or a plain muted ring when
	// no progress has been reported yet. Static either way.
	const ringBg = $derived(
		karaokeStatus.progress !== undefined
			? `conic-gradient(var(--primary) ${karaokeStatus.progress * 100}%, var(--muted) 0)`
			: 'var(--muted)'
	);

	const karaokeTooltip = $derived.by(() => {
		const base = !playback.karaoke
			? t('player.karaoke')
			: karaokeStatus.state === 'preparing'
				? t('karaoke.preparing')
				: karaokeStatus.state === 'error'
					? karaokeStatus.message ? `${karaokeStatus.message} — ${t('karaoke.error_retry')}` : t('karaoke.error_retry')
					: t('karaoke.active');

		if (!playback.karaoke || karaokeQueue.slots.length === 0) return base;

		const parts = karaokeQueue.slots.map((slot, i) => {
			const slotLabel = i === 0 ? t('karaoke.slot.now') : i === 1 ? t('karaoke.slot.next') : t('karaoke.slot.after');
			let stateDesc = '';
			if (slot.state === 'ready') {
				stateDesc = t('karaoke.state.ready');
			} else if (slot.state === 'preparing') {
				const pct = slot.progress !== undefined ? ` ${Math.round(slot.progress * 100)}%` : '';
				stateDesc = `${t('karaoke.state.preparing')}${pct}`;
			} else if (slot.state === 'queued') {
				stateDesc = t('karaoke.state.queued');
			} else if (slot.state === 'error') {
				stateDesc = t('karaoke.state.error');
			}
			return `${slotLabel}: ${stateDesc}`;
		});

		return `${base}\n${parts.join(' · ')}`;
	});

	function onclick() {
		if (karaokeStatus.state === 'error') {
			retryKaraoke();
		} else {
			toggleKaraoke();
		}
	}
</script>

{#snippet face(iconClass: string, ringClass: string, smallIconClass: string)}
	{#if preparing}
		<span class="relative flex items-center justify-center {ringClass}">
			<span
				class="absolute inset-0 rounded-full"
				style="background: {ringBg}; mask: radial-gradient(farthest-side, transparent calc(100% - 2px), var(--foreground) calc(100% - 2px));"
			></span>
			<HugeiconsIcon icon={SingLeftIcon} class="{smallIconClass} text-primary" />
		</span>
	{:else}
		<HugeiconsIcon icon={SingLeftIcon} class="{iconClass} {tone}" />
	{/if}
{/snippet}

{#snippet dots(position: string)}
	{#if playback.karaoke && karaokeQueue.slots.length > 0}
		<!-- Absolutely placed and pointer-transparent: showing or hiding them moves nothing. -->
		<span
			class="pointer-events-none absolute inset-x-0 flex justify-center gap-[3px] {position}"
			aria-hidden="true"
		>
			{#each karaokeQueue.slots as slot, i (`${i}:${slot.videoId}`)}
				{#if slot.state === 'ready'}
					<span class="h-1.5 w-1.5 rounded-full bg-primary"></span>
				{:else if slot.state === 'preparing'}
					<span
						class="h-1.5 w-1.5 rounded-full"
						style="background: conic-gradient(var(--primary) {(slot.progress ?? 0) * 100}%, var(--muted) 0);"
					></span>
				{:else if slot.state === 'queued'}
					<span class="h-1.5 w-1.5 rounded-full border border-foreground/50"></span>
				{:else if slot.state === 'error'}
					<span class="h-1.5 w-1.5 rounded-full bg-destructive"></span>
				{/if}
			{/each}
		</span>
	{/if}
{/snippet}

{#if variant === 'pill'}
	<!-- Same box and hover as the `column` toggles in TheaterMode's pill (size-8, rounded-full,
	     transition-colors). No "on" background: karaoke is a mode, shown by icon colour like
	     shuffle and repeat. The dots sit inside the button's lower edge so they stay in the pill. -->
	<button
		{onclick}
		class="group relative flex size-8 cursor-pointer items-center justify-center rounded-full transition-colors hover:bg-foreground/10"
		aria-label={t('player.karaoke')}
		aria-pressed={playback.karaoke}
		title={karaokeTooltip}
	>
		{@render face('h-4 w-4', 'size-[22px]', 'h-3 w-3')}
		{@render dots('bottom-px')}
	</button>
{:else}
	<!-- The dots hang just below the button, in the bar's vertical padding. -->
	<Button
		variant="ghost"
		size="icon-sm"
		class="relative"
		{onclick}
		aria-label={t('player.karaoke')}
		aria-pressed={playback.karaoke}
		title={karaokeTooltip}
	>
		{@render face('h-5 w-5', 'size-6', 'h-3.5 w-3.5')}
		{@render dots('top-full mt-0.5')}
	</Button>
{/if}
