<script lang="ts">
	// What karaoke is doing, from the player bar's and theater mode's karaoke button: the song
	// playing and the next two, and the quality. The button itself only turns karaoke on and off,
	// so the common thing (vocals back for a verse, then off again) stays one click.
	import { RadioGroup } from 'bits-ui';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Alert02Icon, Download04Icon, Settings02Icon, Tick02Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Switch } from '$lib/components/ui/switch';
	import * as api from '$lib/api';
	import {
		playback,
		karaokeStatus,
		karaokeQueue,
		karaokeAvailability,
		karaokeModels,
		karaokeSelection,
		karaokeTrack,
		toggleKaraoke,
		retryKaraoke,
		chooseKaraokeModel,
		openKaraokeSettings
	} from '$lib/player.svelte';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import { KARAOKE_TIERS, tierMB } from '$lib/karaoke-tiers';
	import ExperimentalBadge from './ExperimentalBadge.svelte';

	let { onclose }: { onclose: () => void } = $props();

	const on = $derived(playback.karaoke);
	// Why karaoke couldn't start last time it was tried. Shown only while it is off: turning it on
	// clears the reason, and a failure turns it straight back off.
	const reason = $derived(on ? undefined : karaokeAvailability.reason);
	const now = $derived(playback.now);
	const phase = $derived(karaokeStatus.state);
	const pct = $derived(Math.round((karaokeStatus.progress ?? 0) * 100));
	const eta = $derived(karaokeTrack(now?.videoId)?.etaSecs);
	const selected = $derived(
		KARAOKE_TIERS.find((tier) => tier.id === karaokeSelection.model) ?? KARAOKE_TIERS[0]
	);

	const nowLine = $derived.by(() => {
		if (phase === 'ready') return t('karaoke.panel.now_ready');
		if (phase === 'too-long') return t('karaoke.panel.now_too_long');
		if (phase === 'error') return t(`karaoke.error.${karaokeStatus.code ?? 'failed'}`);
		if (karaokeStatus.progress === undefined) return t('karaoke.panel.now_starting');
		if (eta === undefined) return t('karaoke.panel.now_progress', { pct });
		if (eta < 45) return t('karaoke.panel.now_progress_soon', { pct });
		return t('karaoke.panel.now_progress_eta', { pct, min: Math.round(eta / 60) || 1 });
	});

	// The slots are the wanted tracks in order (this one first, unless it is too long to prepare);
	// the rows name them from the queue. A track that isn't in the queue any more has no row.
	const upNext = $derived(
		karaokeQueue.slots
			.filter((slot) => slot.videoId !== now?.videoId)
			.map((slot) => ({
				slot,
				item: playback.queue.items.find((i) => i.video_id === slot.videoId)
			}))
			.filter((row) => row.item)
	);

	function flip() {
		// Not set up yet: turning on opens the setup dialog, which shouldn't sit behind this.
		if (!on && !karaokeModels[karaokeSelection.model]?.installed) onclose();
		toggleKaraoke();
	}

	function pick(v: string) {
		const model = v as api.KaraokeModelId;
		if (model === karaokeSelection.model) return;
		if (!karaokeModels[model]?.installed) onclose();
		void chooseKaraokeModel(model);
	}
</script>

{#snippet art(url: string | undefined, size: string)}
	<span class="{size} shrink-0 overflow-hidden rounded-md bg-muted">
		{#if url}<img src={thumb(url, 96)} alt="" class="size-full object-cover" />{/if}
	</span>
{/snippet}

<div class="flex items-start gap-3 px-4 pb-3 pt-3.5">
	<div class="min-w-0 flex-1">
		<p class="flex items-center gap-2 font-heading text-sm font-semibold">
			{t('karaoke.panel.title')}
			<ExperimentalBadge tooltip={t('karaoke.experimental_tooltip')} />
		</p>
		<p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">{t('karaoke.panel.hint')}</p>
	</div>
	<Switch checked={on} onCheckedChange={flip} aria-label={t('player.karaoke')} class="mt-0.5" />
</div>

{#if reason}
	<div class="border-t px-4 py-3">
		<div class="flex gap-2.5 rounded-lg bg-muted/60 p-3">
			<HugeiconsIcon icon={Alert02Icon} class="mt-px size-4 shrink-0 text-muted-foreground" />
			<div class="min-w-0">
				<p class="text-xs leading-relaxed">{t(`karaoke.unavailable.${reason}`)}</p>
				{#if reason === 'vc_runtime' || (reason !== 'proxy' && reason !== 'not_installed')}
					<div class="mt-2.5 flex flex-wrap gap-2">
						{#if reason === 'vc_runtime'}
							<Button
								variant="outline"
								size="sm"
								class="h-7 text-xs"
								onclick={() => api.openExternal(api.VC_REDIST_URL)}
							>
								{t('karaoke.settings.get_vc_redist')}
							</Button>
						{/if}
						<!-- Turning on again is the check: it retries the load and reports anew. -->
						<Button variant="outline" size="sm" class="h-7 text-xs" onclick={toggleKaraoke}>
							{t('karaoke.panel.try_again')}
						</Button>
					</div>
				{/if}
			</div>
		</div>
	</div>
{:else if !on}
	<p class="border-t px-4 py-3 text-xs leading-relaxed text-muted-foreground">
		{t('karaoke.panel.off')}
	</p>
{/if}

{#if on && now}
	<div class="border-t px-4 py-3">
		<div class="flex items-center gap-3">
			{@render art(now.thumbnail, 'size-10')}
			<div class="min-w-0 flex-1">
				<p class="truncate text-sm font-medium">{now.title}</p>
				<p
					class="truncate text-xs {phase === 'ready'
						? 'text-primary'
						: phase === 'error'
							? 'text-destructive'
							: 'text-muted-foreground'}"
				>
					{nowLine}
				</p>
			</div>
			{#if phase === 'ready'}
				<HugeiconsIcon icon={Tick02Icon} class="size-4 shrink-0 text-primary" />
			{/if}
		</div>
		{#if phase === 'preparing'}
			<!-- Progress lands every 250 ms; no width transition, it would only lag behind it. -->
			<div
				class="mt-3 h-1 overflow-hidden rounded-full bg-muted"
				role="progressbar"
				aria-valuemin={0}
				aria-valuemax={100}
				aria-valuenow={pct}
				aria-label={t('karaoke.panel.now_starting')}
			>
				<div class="h-full rounded-full bg-primary" style="width:{pct}%"></div>
			</div>
			<p class="mt-2 text-xs leading-relaxed text-muted-foreground">{t('karaoke.panel.now_swap')}</p>
		{:else if phase === 'error'}
			<Button variant="outline" size="sm" class="mt-3 h-7 text-xs" onclick={() => retryKaraoke()}>
				{t('karaoke.panel.try_again')}
			</Button>
		{/if}
	</div>

	<div class="border-t px-4 py-3">
		<p class="mb-2 text-xs font-medium text-muted-foreground">{t('karaoke.panel.up_next')}</p>
		{#if upNext.length === 0}
			<p class="text-xs text-muted-foreground">{t('karaoke.panel.up_next_empty')}</p>
		{:else}
			<ul class="flex flex-col gap-2">
				{#each upNext as row (row.slot.videoId)}
					<li class="flex items-center gap-2.5">
						{@render art(row.item?.thumbnail, 'size-8')}
						<span class="min-w-0 flex-1 truncate text-xs">{row.item?.title}</span>
						<span class="flex shrink-0 items-center gap-1.5 text-xs">
							{#if row.slot.state === 'ready'}
								<HugeiconsIcon icon={Tick02Icon} class="size-3.5 text-primary" />
								<span class="text-primary">{t('karaoke.panel.slot_ready')}</span>
							{:else if row.slot.state === 'preparing'}
								<span class="h-1 w-10 overflow-hidden rounded-full bg-muted" aria-hidden="true">
									<span
										class="block h-full rounded-full bg-primary"
										style="width:{Math.round((row.slot.progress ?? 0) * 100)}%"
									></span>
								</span>
								<span class="w-8 text-right tabular-nums text-muted-foreground">
									{Math.round((row.slot.progress ?? 0) * 100)}%
								</span>
							{:else if row.slot.state === 'error'}
								<span class="text-destructive">{t('karaoke.panel.slot_failed')}</span>
							{:else}
								<span class="text-muted-foreground">{t('karaoke.panel.slot_waiting')}</span>
							{/if}
						</span>
					</li>
				{/each}
			</ul>
		{/if}
	</div>
{/if}

{#if !reason}
	<div class="border-t px-4 py-3">
		<p class="mb-2 text-xs font-medium text-muted-foreground">{t('karaoke.panel.quality')}</p>
		<RadioGroup.Root
			value={karaokeSelection.model}
			onValueChange={(v) => v && pick(v)}
			aria-label={t('karaoke.panel.quality')}
			class="grid grid-cols-3 gap-1.5"
		>
			{#each KARAOKE_TIERS as tier (tier.id)}
				<RadioGroup.Item
					value={tier.id}
					class="group flex cursor-pointer flex-col items-start gap-1.5 rounded-lg border px-2.5 py-2 text-left outline-none transition-colors hover:border-foreground/25 focus-visible:ring-2 focus-visible:ring-ring data-checked:border-primary data-checked:bg-primary/8"
				>
					<!-- One to three bars: the tiers' place on the speed-for-quality trade. -->
					<span class="flex h-3 items-end gap-[2px]" aria-hidden="true">
						{#each [1, 2, 3] as n (n)}
							<span
								class="w-[3px] rounded-full {n <= tier.level
									? 'bg-muted-foreground/60 group-data-checked:bg-primary'
									: 'bg-muted-foreground/20'}"
								style="height:{3 + n * 3}px"
							></span>
						{/each}
					</span>
					<span class="text-xs font-medium">{t(`karaoke.settings.model_${tier.key}`)}</span>
					{#if !karaokeModels[tier.id].installed}
						<span class="flex items-center gap-1 text-[11px] text-muted-foreground">
							<HugeiconsIcon icon={Download04Icon} class="size-3" />
							{tierMB(tier.id)}
						</span>
					{/if}
				</RadioGroup.Item>
			{/each}
		</RadioGroup.Root>
		<p class="mt-2 text-xs leading-relaxed text-muted-foreground">
			{t(`karaoke.settings.model_${selected.key}_desc`)}
		</p>
	</div>
{/if}

<div class="border-t p-1.5">
	<button
		type="button"
		class="flex w-full cursor-pointer items-center gap-3 rounded-lg px-2.5 py-2 text-left text-sm text-muted-foreground transition hover:bg-foreground/10 hover:text-foreground"
		onclick={() => {
			onclose();
			openKaraokeSettings();
		}}
	>
		<HugeiconsIcon icon={Settings02Icon} class="size-4" />
		{t('karaoke.panel.settings')}
	</button>
</div>
