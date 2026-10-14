<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Mic02Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { playback, karaokeStatus, karaokeQueue, toggleKaraoke, retryKaraoke } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	let {
		size = 20
	}: {
		size?: number;
	} = $props();

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
</script>

<div class="flex items-center gap-1.5" title={karaokeTooltip}>
	<Button
		variant={playback.karaoke ? 'secondary' : 'ghost'}
		size="icon-sm"
		onclick={() => {
			if (karaokeStatus.state === 'error') {
				retryKaraoke();
			} else {
				toggleKaraoke();
			}
		}}
		aria-label={t('player.karaoke')}
		title={karaokeTooltip}
		aria-pressed={playback.karaoke}
	>
		{#if playback.karaoke && karaokeStatus.state === 'preparing'}
			{#if karaokeStatus.progress !== undefined}
				<!-- Progress ring: a conic gradient masked to a 2px band, with the mic icon inside. -->
				<div class="relative flex items-center justify-center" style="width: {size}px; height: {size}px;">
					<div
						class="absolute inset-0 rounded-full"
						style="background: conic-gradient(var(--primary) {karaokeStatus.progress * 100}%, var(--muted) 0); mask: radial-gradient(farthest-side, transparent calc(100% - 2px), var(--foreground) calc(100% - 2px));"
					></div>
					<HugeiconsIcon icon={Mic02Icon} size={size * 0.55} class="text-primary" />
				</div>
			{:else}
				<div style="width: {size * 0.8}px; height: {size * 0.8}px;" class="animate-spin rounded-full border-2 border-muted border-t-primary"></div>
			{/if}
		{:else if playback.karaoke && karaokeStatus.state === 'error'}
			<div class="relative flex items-center justify-center">
				<HugeiconsIcon icon={Mic02Icon} {size} class="text-destructive" />
				<span class="absolute -top-0.5 -right-0.5 h-2 w-2 rounded-full bg-destructive"></span>
			</div>
		{:else}
			<HugeiconsIcon icon={Mic02Icon} {size} class={playback.karaoke ? 'text-primary' : ''} />
		{/if}
	</Button>
	{#if playback.karaoke && karaokeQueue.slots.length > 0}
		<div class="flex flex-col gap-[3px]" aria-label={karaokeTooltip}>
			{#each karaokeQueue.slots as slot}
				{#if slot.state === 'ready'}
					<span class="h-1.5 w-1.5 rounded-full bg-primary" title=""></span>
				{:else if slot.state === 'preparing'}
					{#if slot.progress !== undefined}
						<span
							class="h-1.5 w-1.5 rounded-full"
							style="background: conic-gradient(var(--primary) {slot.progress * 100}%, var(--muted) 0);"
							title=""
						></span>
					{:else}
						<span class="h-1.5 w-1.5 rounded-full bg-primary animate-pulse" title=""></span>
					{/if}
				{:else if slot.state === 'queued'}
					<span class="h-1.5 w-1.5 rounded-full border border-muted-foreground/60 bg-transparent" title=""></span>
				{:else if slot.state === 'error'}
					<span class="h-1.5 w-1.5 rounded-full bg-destructive" title=""></span>
				{/if}
			{/each}
		</div>
	{/if}
</div>
