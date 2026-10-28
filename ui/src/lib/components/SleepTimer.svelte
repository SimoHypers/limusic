<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Timer01Icon } from '@hugeicons/core-free-icons';
	import * as Popover from '$lib/components/ui/popover';
	import { Button, buttonVariants } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { setSleepTimer, sleepTimer, toast } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	let open = $state(false);
	let busy = $state(false);
	let minutes = $state<number | undefined>(30);
	const inputId = $props.id();
	const snapshot = $derived(sleepTimer.snapshot);
	const active = $derived(snapshot.mode !== 'off');
	const clock = (seconds: number) => {
		const h = Math.floor(seconds / 3600);
		const m = Math.floor((seconds % 3600) / 60);
		const s = String(seconds % 60).padStart(2, '0');
		return h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
	};
	const status = $derived(snapshot.mode === 'duration'
		? t('sleep.remaining', { time: clock(snapshot.remainingSeconds) })
		: snapshot.mode === 'afterTrack' ? t('sleep.after_track') : t('sleep.off'));

	async function choose(duration: number | null, afterTrack = false) {
		busy = true;
		try {
			if (await setSleepTimer(duration, afterTrack)) open = false;
		} finally {
			busy = false;
		}
	}

	function custom(event: SubmitEvent) {
		event.preventDefault();
		if (minutes === undefined || !Number.isInteger(minutes) || minutes < 1 || minutes > 1440) {
			toast.error(t('sleep.invalid'));
			return;
		}
		choose(minutes);
	}
</script>

<Popover.Root bind:open>
	<Popover.Trigger
		class={buttonVariants({ variant: active ? 'secondary' : 'ghost', size: active ? 'sm' : 'icon-sm' })}
		aria-label={`${t('sleep.title')}: ${status}`}
		title={`${t('sleep.title')}: ${status}`}
	>
		<HugeiconsIcon icon={Timer01Icon} class="h-4 w-4" />
		{#if snapshot.mode === 'duration'}
			<span class="min-w-[4ch] text-xs tabular-nums">{clock(snapshot.remainingSeconds)}</span>
		{:else if snapshot.mode === 'afterTrack'}
			<span class="text-xs">{t('sleep.after_track_short')}</span>
		{/if}
	</Popover.Trigger>
	<Popover.Content side="top" align="end" class="w-72 max-w-[calc(100vw-2rem)] gap-3" aria-label={t('sleep.title')}>
		<div>
			<h3 class="font-semibold">{t('sleep.title')}</h3>
			<p class="mt-1 text-xs text-muted-foreground">{t('sleep.hint')}</p>
		</div>
		<p class="text-sm font-medium tabular-nums">{status}</p>
		<div class="grid grid-cols-3 gap-2">
			{#each [15, 30, 60] as preset}
				<Button variant="outline" size="sm" disabled={busy} onclick={() => choose(preset)}>
					{t('sleep.preset', { minutes: preset })}
				</Button>
			{/each}
		</div>
		<form onsubmit={custom} class="flex flex-col gap-2">
			<label for={inputId} class="text-xs text-muted-foreground">{t('sleep.custom')}</label>
			<div class="flex items-center gap-2">
				<Input id={inputId} type="number" min={1} max={1440} step={1} required bind:value={minutes} disabled={busy} class="w-24" />
				<Button type="submit" size="sm" disabled={busy} class="flex-1">{t('sleep.start')}</Button>
			</div>
		</form>
		<Button variant="outline" size="sm" disabled={busy} onclick={() => choose(null, true)}>
			{t('sleep.after_track')}
		</Button>
		{#if active}
			<Button variant="ghost" size="sm" disabled={busy} onclick={() => choose(null)}>{t('sleep.cancel')}</Button>
		{/if}
	</Popover.Content>
</Popover.Root>
