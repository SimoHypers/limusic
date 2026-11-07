<script lang="ts">
	// Ten-band equalizer, from the player bar. Pick a tuned preset or drag the bars yourself; every
	// change is heard while dragging and saved for the next launch, nothing waits for Done.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { AudioLinesIcon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Switch } from '$lib/components/ui/switch';
	import { Slider } from '$lib/components/ui/slider';
	import * as Dialog from '$lib/components/ui/dialog';
	import { playback, setEqualizer } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import {
		EQ_BANDS,
		EQ_MAX_DB,
		EQ_PRESETS,
		EQ_STEP,
		bandLabel,
		clampDb,
		matchPreset,
		signedDb,
		type EqPresetId
	} from '$lib/equalizer';

	let { open = $bindable(false) }: { open: boolean } = $props();

	const LABEL = 'mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground';
	const eq = $derived(playback.equalizer);
	const active = $derived(matchPreset(eq));
	const changed = $derived(eq.preamp !== 0 || eq.bands.some((g) => g !== 0));

	// Touching the curve means you want to hear it, so it switches the equalizer on.
	function setBand(i: number, v: number) {
		const bands = [...eq.bands];
		bands[i] = clampDb(v);
		setEqualizer({ ...eq, enabled: true, bands });
	}
	const setPreamp = (v: number) => setEqualizer({ ...eq, enabled: true, preamp: clampDb(v) });
	function applyPreset(id: EqPresetId) {
		const p = EQ_PRESETS.find((x) => x.id === id)!;
		setEqualizer({ enabled: id !== 'flat', preamp: p.preamp, bands: [...p.bands] });
	}
	const reset = () => applyPreset('flat');

	// A preset's curve drawn as ten little bars around a centre line.
	const barHeight = (g: number) => Math.max(1, Math.round((Math.abs(g) / EQ_MAX_DB) * 12));
</script>

{#snippet band(label: string, value: number, onchange: (v: number) => void, aria: string)}
	<div class="flex w-9 shrink-0 flex-col items-center gap-2">
		<span class="text-[11px] font-semibold tabular-nums {value === 0 ? 'text-muted-foreground' : ''}">
			{signedDb(value)}
		</span>
		<div class="h-40">
			<Slider
				type="single"
				orientation="vertical"
				min={-EQ_MAX_DB}
				max={EQ_MAX_DB}
				step={EQ_STEP}
				{value}
				onValueChange={onchange}
				aria-label={aria}
			/>
		</div>
		<button
			type="button"
			class="cursor-pointer rounded px-1 text-[11px] text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
			title={t('dialogs.equalizer.band_reset')}
			ondblclick={() => onchange(0)}
			onclick={(e) => e.detail === 0 && onchange(0)}
		>
			{label}
		</button>
	</div>
{/snippet}

<Dialog.Root bind:open>
	<Dialog.Content class="flex max-h-[85dvh] flex-col gap-0 overflow-hidden p-0 sm:max-w-2xl">
		<div class="flex items-start gap-3 px-6 pb-4 pr-14 pt-6">
			<span
				class="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary"
			>
				<HugeiconsIcon icon={AudioLinesIcon} class="h-5 w-5" />
			</span>
			<div class="min-w-0 flex-1">
				<Dialog.Title class="font-heading text-base font-semibold">
					{t('dialogs.equalizer.title')}
				</Dialog.Title>
				<Dialog.Description class="text-xs leading-relaxed text-muted-foreground">
					{t('dialogs.equalizer.desc')}
				</Dialog.Description>
			</div>
			<Switch
				checked={eq.enabled}
				onCheckedChange={(on) => setEqualizer({ ...eq, enabled: on })}
				aria-label={t('dialogs.equalizer.enable')}
			/>
		</div>

		<div class="min-h-0 flex-1 overflow-y-auto px-6 pb-6">
			<h3 class={LABEL}>{t('dialogs.equalizer.presets')}</h3>
			<div class="mb-6 grid grid-cols-2 gap-2 sm:grid-cols-4">
				{#each EQ_PRESETS as preset (preset.id)}
					{@const on = active === preset.id && (eq.enabled || preset.id === 'flat')}
					<button
						type="button"
						class="flex w-full cursor-pointer flex-col gap-1.5 rounded-lg border px-2.5 py-2 text-left text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring {on
							? 'border-primary bg-primary/8 text-foreground'
							: 'text-muted-foreground hover:border-foreground/25 hover:text-foreground'}"
						aria-pressed={on}
						title={t(`dialogs.equalizer.hints.${preset.id}`)}
						onclick={() => applyPreset(preset.id)}
					>
						<span class="truncate">{t(`dialogs.equalizer.preset_names.${preset.id}`)}</span>
						<span class="flex h-6 items-center gap-[3px]" aria-hidden="true">
							{#each preset.bands as g, i (i)}
								<span class="relative h-full w-1 flex-1">
									<span
										class="absolute left-0 right-0 rounded-sm {on ? 'bg-primary' : 'bg-muted-foreground/50'}"
										style="height:{barHeight(g)}px;{g >= 0 ? 'bottom:50%' : 'top:50%'}"
									></span>
								</span>
							{/each}
						</span>
					</button>
				{/each}
			</div>

			<div class="mb-2 flex items-center px-1">
				<h3 class="flex-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">
					{t('dialogs.equalizer.bands')}
				</h3>
				<Button size="sm" variant="ghost" class="h-7 text-xs" disabled={!changed} onclick={reset}>
					{t('common.reset')}
				</Button>
			</div>
			<div
				class="overflow-x-auto rounded-xl border bg-card px-3 py-4 transition-opacity {eq.enabled
					? ''
					: 'opacity-60'}"
			>
				<div class="mx-auto flex w-max items-start gap-1">
					{@render band(t('dialogs.equalizer.preamp'), eq.preamp, setPreamp, t('dialogs.equalizer.preamp'))}
					<span class="mx-1 h-48 w-px self-center bg-border"></span>
					{#each EQ_BANDS as hz, i (hz)}
						{@render band(
							bandLabel(hz),
							eq.bands[i],
							(v) => setBand(i, v),
							t('dialogs.equalizer.band_aria', { freq: hz >= 1000 ? `${hz / 1000} kHz` : `${hz} Hz` })
						)}
					{/each}
				</div>
			</div>
			<p class="mt-3 px-1 text-xs leading-relaxed text-muted-foreground">
				{t('dialogs.equalizer.footnote')}
			</p>
		</div>
	</Dialog.Content>
</Dialog.Root>
