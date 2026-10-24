<script lang="ts">
	// Session effects from the player bar's ⋮ menu, reset on restart.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Add01Icon, ArrowTurnBackwardIcon, Delete02Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import * as Select from '$lib/components/ui/select';
	import { Switch } from '$lib/components/ui/switch';
	import { Slider } from '$lib/components/ui/slider';
	import { DEFAULT_REVERB } from '$lib/api';
	import * as Dialog from '$lib/components/ui/dialog';
	import { inRoom } from '$lib/lt.svelte';
	import { playback, setTempoPitch as apply } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import {
		EFFECT_PRESETS_KEY, PRESET_NAME_MAX, readEffectPresets, presetSettings, sameEffects,
		type EffectPreset
	} from '$lib/effect-presets';

	let { open = $bindable(false) }: { open: boolean } = $props();
	let presets = $state<EffectPreset[]>([]);
	let selectedName = $state('');
	let presetName = $state('');
	let addingPreset = $state(false);
	let presetError = $state('');
	let storageReady = $state(true);
	let selectedPreset = $derived(presets.find((p) => p.name === selectedName));
	let modified = $derived(!!selectedPreset && !sameEffects(presetSettings(selectedPreset, inRoom()), playback));
	let duplicateName = $derived(presets.some((p) => p.name.toLowerCase() === presetName.trim().toLowerCase()));

	// Presets persist on this device and load when the dialog opens.
	$effect(() => {
		if (!open) return;
		try {
			presets = readEffectPresets(localStorage.getItem(EFFECT_PRESETS_KEY));
			storageReady = true;
			presetError = '';
		} catch {
			storageReady = false;
		}
	});

	function persistPresets(next: EffectPreset[]) {
		if (!storageReady) return false;
		try {
			localStorage.setItem(EFFECT_PRESETS_KEY, JSON.stringify(next));
			presets = next;
			presetError = '';
			return true;
		} catch {
			presetError = t('dialogs.tempo_pitch.preset_save_error');
			return false;
		}
	}

	function savePreset(update = false) {
		const name = update ? selectedName : presetName.trim();
		if (!name || name.length > PRESET_NAME_MAX || (!update && duplicateName)) return;
		const preset: EffectPreset = {
			name, speed: update && inRoom() && selectedPreset ? selectedPreset.speed : playback.speed,
			semitones: playback.semitones, reverb: { ...playback.reverb }
		};
		const next = update ? presets.map((p) => p.name === name ? preset : p) : [...presets, preset];
		if (persistPresets(next)) {
			selectedName = name;
			if (!update) {
				presetName = '';
				addingPreset = false;
			}
		}
	}

	function selectPreset(name: string) {
		const preset = presets.find((p) => p.name === name);
		if (!preset) return;
		const settings = presetSettings(preset, inRoom());
		selectedName = name;
		apply(settings.speed, settings.semitones, settings.reverb);
	}

	function deletePreset() {
		if (persistPresets(presets.filter((p) => p.name !== selectedName))) selectedName = '';
	}

	const TEMPO = { min: 0.5, center: 1, max: 2, step: 0.01 };
	const TEMPO_SLIDER = { min: 0, midpoint: 100, max: 200 };
	const SEMITONES = { min: -12, max: 12 };

	let tempo = $derived(playback.speed);
	let pitch = $derived(playback.semitones);
	// Keep 1x at the midpoint while preserving the full tempo range.
	function tempoSliderValue(speed: number) {
		return Math.round(speed <= TEMPO.center
			? (speed - TEMPO.min) / (TEMPO.center - TEMPO.min) * TEMPO_SLIDER.midpoint
			: TEMPO_SLIDER.midpoint + (speed - TEMPO.center) / (TEMPO.max - TEMPO.center) * (TEMPO_SLIDER.max - TEMPO_SLIDER.midpoint));
	}
	function tempoFromSlider(value: number) {
		const speed = value <= TEMPO_SLIDER.midpoint
			? TEMPO.min + value / TEMPO_SLIDER.midpoint * (TEMPO.center - TEMPO.min)
			: TEMPO.center + (value - TEMPO_SLIDER.midpoint) / (TEMPO_SLIDER.max - TEMPO_SLIDER.midpoint) * (TEMPO.max - TEMPO.center);
		return Math.round(speed / TEMPO.step) / (1 / TEMPO.step);
	}
	const REVERB_TYPES = [
		'small_room', 'medium_room', 'large_room', 'medium_hall', 'large_hall', 'plate'
	] as const;
	const REVERB_CONTROLS = [
		{ key: 'dry', min: 0, max: 100, step: 1, unit: '%' },
		{ key: 'wet', min: 0, max: 100, step: 1, unit: '%' },
		{ key: 'decay', min: 0.1, max: 10, step: 0.1, unit: 's' },
		{ key: 'preDelay', min: 0, max: 200, step: 1, unit: 'ms' },
		{ key: 'damping', min: 500, max: 20000, step: 100, unit: 'Hz' },
		{ key: 'reflections', min: 0, max: 100, step: 1, unit: '%' }
	] as const;
	// Slider drafts are local; commit once on release or a keyboard step, then use backend values.
	let draft = $derived({ ...playback.reverb });
	function toggleReverb(on: boolean) {
		apply(playback.speed, playback.semitones, { ...playback.reverb, enabled: on });
	}
</script>

<Dialog.Root bind:open onOpenChange={(value) => { if (!value) { addingPreset = false; presetName = ''; } }}>
	<Dialog.Content class="max-h-[85dvh] overflow-y-auto gap-4 sm:max-w-4xl">
		<div class="grid gap-1">
			<Dialog.Title class="pr-10 text-lg font-semibold">{t('dialogs.tempo_pitch.title')}</Dialog.Title>
			<Dialog.Description class="text-xs text-muted-foreground">
				{t('dialogs.tempo_pitch.desc')}
			</Dialog.Description>
		</div>

		<div class="grid gap-2 rounded-2xl border bg-muted/20 p-3">
			<label for="effect-preset" class="text-sm font-medium">{t('dialogs.tempo_pitch.presets')}</label>
			<div class="flex flex-wrap items-center gap-2">
				<Select.Root type="single" value={modified ? '' : selectedName} disabled={!presets.length} onValueChange={selectPreset}>
					<Select.Trigger id="effect-preset" size="sm" class="w-64 max-w-full min-w-0">
						<span class="flex-1 truncate text-left">{selectedPreset ? `${selectedPreset.name}${modified ? ` · ${t('dialogs.tempo_pitch.preset_modified')}` : ''}` : t('dialogs.tempo_pitch.preset_choose')}</span>
					</Select.Trigger>
					<Select.Content>
						{#each presets as preset (preset.name)}
							<Select.Item value={preset.name} label={preset.name}>{preset.name}</Select.Item>
						{/each}
					</Select.Content>
				</Select.Root>
				{#if selectedPreset}
					{#if modified}
						<Button size="sm" variant="outline" disabled={!storageReady} onclick={() => savePreset(true)}>{t('dialogs.tempo_pitch.preset_update')}</Button>
					{/if}
					<Button size="icon-sm" variant="ghost" class="text-muted-foreground hover:bg-destructive/10 hover:text-destructive" aria-label={t('common.delete')} title={t('common.delete')} disabled={!storageReady} onclick={deletePreset}>
						<HugeiconsIcon icon={Delete02Icon} />
					</Button>
				{/if}
				<Button size="sm" variant="outline" class="sm:ml-auto" aria-expanded={addingPreset} aria-controls="add-effect-preset" disabled={!storageReady} onclick={() => (addingPreset = !addingPreset)}>
					<HugeiconsIcon icon={Add01Icon} />
					{t('dialogs.tempo_pitch.preset_add')}
				</Button>
			</div>
			{#if addingPreset}
				<form id="add-effect-preset" class="grid gap-2 border-t pt-3" onsubmit={(e) => { e.preventDefault(); savePreset(); }}>
					<div class="flex flex-wrap items-center gap-2">
						<Input bind:value={presetName} class="h-8 w-64 max-w-full" maxlength={PRESET_NAME_MAX} aria-label={t('dialogs.tempo_pitch.preset_name')}
							placeholder={t('dialogs.tempo_pitch.preset_name')} aria-invalid={duplicateName}
							aria-describedby={duplicateName ? 'preset-name-error' : undefined} disabled={!storageReady} />
						<Button type="submit" size="sm" disabled={!presetName.trim() || duplicateName || !storageReady}>{t('common.save')}</Button>
						<Button size="sm" variant="ghost" onclick={() => { addingPreset = false; presetName = ''; }}>{t('common.cancel')}</Button>
					</div>
					{#if duplicateName}
						<p id="preset-name-error" class="text-xs text-destructive" role="status">{t('dialogs.tempo_pitch.preset_duplicate')}</p>
					{/if}
					<p class="text-xs text-muted-foreground">{t('dialogs.tempo_pitch.presets_hint')}</p>
				</form>
			{/if}
			{#if !storageReady || presetError}
				<p class="text-xs text-destructive" role="alert">{!storageReady ? t('dialogs.tempo_pitch.preset_load_error') : presetError}</p>
			{/if}
			{#if inRoom()}
				<p class="text-xs text-muted-foreground">{t('dialogs.tempo_pitch.preset_room_hint')}</p>
			{/if}
		</div>

		<div class="grid gap-4 sm:grid-cols-2 sm:gap-x-6">
			<!-- Tempo moves you off the shared clock, so it's out while a room is on. Pitch is yours
			     alone: it changes nothing about when the next track starts. -->
			{#if !inRoom()}
				<div class="grid gap-2">
					<div class="flex justify-between gap-3 text-sm">
						<label for="tempo-slider">{t('dialogs.tempo_pitch.tempo')}</label>
						<span class="tabular-nums">{tempo.toFixed(2)}x</span>
					</div>
					<Slider id="tempo-slider" type="single"
						aria-label={t('dialogs.tempo_pitch.tempo')}
						min={TEMPO_SLIDER.min} max={TEMPO_SLIDER.max} step={1} value={tempoSliderValue(tempo)}
						onValueChange={(v) => (tempo = tempoFromSlider(v))}
						onValueCommit={(v) => void apply(tempoFromSlider(v), playback.semitones).finally(() => (tempo = playback.speed))} />
				</div>
			{/if}
			<div class="grid gap-2">
				<div class="flex justify-between gap-3 text-sm">
					<label for="pitch-slider">{t('dialogs.tempo_pitch.pitch')}</label>
					<span class="tabular-nums">{pitch > 0 ? `+${pitch}` : pitch}</span>
				</div>
				<Slider id="pitch-slider" type="single"
					aria-label={t('dialogs.tempo_pitch.pitch')}
					min={SEMITONES.min} max={SEMITONES.max} step={1} value={pitch}
					onValueChange={(v) => (pitch = v)}
					onValueCommit={(v) => void apply(playback.speed, v).finally(() => (pitch = playback.semitones))} />
			</div>
			<div class="border-t pt-4 sm:col-span-2">
				<div class="flex items-center justify-between gap-4">
					<label for="reverb-enabled" class="text-sm font-medium">{t('dialogs.tempo_pitch.reverb')}</label>
					<Switch id="reverb-enabled" checked={playback.reverb.enabled} onCheckedChange={toggleReverb} />
				</div>
				<p class="mt-1 text-xs text-muted-foreground">{t('dialogs.tempo_pitch.reverb_hint')}</p>
				{#if playback.reverb.enabled}
					<div class="mt-3 grid gap-2">
						<div class="flex flex-wrap items-center gap-2">
							<label for="reverb-type" class="text-sm">{t('dialogs.tempo_pitch.reverb_type')}</label>
							<Select.Root type="single" value={String(playback.reverb.preset)}
								onValueChange={(value) => { if (value) apply(playback.speed, playback.semitones, { ...playback.reverb, preset: Number(value) }); }}>
								<Select.Trigger id="reverb-type" size="sm" class="w-72 max-w-full min-w-0">
									<span class="min-w-0 flex-1 truncate text-left" title={t(`dialogs.tempo_pitch.reverb_types.${REVERB_TYPES[playback.reverb.preset - 1]}`)}>{t(`dialogs.tempo_pitch.reverb_types.${REVERB_TYPES[playback.reverb.preset - 1]}`)}</span>
								</Select.Trigger>
								<Select.Content>
									{#each REVERB_TYPES as type, i (type)}
										<Select.Item value={String(i + 1)} label={t(`dialogs.tempo_pitch.reverb_types.${type}`)}>{t(`dialogs.tempo_pitch.reverb_types.${type}`)}</Select.Item>
									{/each}
								</Select.Content>
							</Select.Root>
						</div>
						<div class="mt-2 grid gap-4 sm:grid-cols-2 sm:gap-x-6 md:grid-cols-3">
							{#each REVERB_CONTROLS as control (control.key)}
								<div class="grid content-start gap-2">
									<div class="flex justify-between gap-3 text-sm">
										<label for={`reverb-${control.key}`}>{t(`dialogs.tempo_pitch.reverb_controls.${control.key}`)}</label>
										<span class="tabular-nums">{control.key === 'decay' ? draft.decay.toFixed(1) : draft[control.key]} {control.unit}</span>
									</div>
									<Slider id={`reverb-${control.key}`} type="single"
										aria-label={t(`dialogs.tempo_pitch.reverb_controls.${control.key}`)}
										min={control.min} max={control.max} step={control.step}
										value={draft[control.key]}
										onValueChange={(v) => { draft = { ...draft, [control.key]: v }; }}
										onValueCommit={(v) => apply(playback.speed, playback.semitones, { ...draft, [control.key]: v })} />
									<p class="text-xs text-muted-foreground">{t(`dialogs.tempo_pitch.reverb_control_hints.${control.key}`)}</p>
								</div>
							{/each}
						</div>
					</div>
				{/if}
			</div>
		</div>

		<div class="flex justify-end gap-2">
			<Button variant="ghost" size="sm" class="h-7 w-fit gap-1.5 px-2 text-xs"
				onclick={() => { selectedName = ''; apply(1, 0, { ...DEFAULT_REVERB }); }}>
				<HugeiconsIcon icon={ArrowTurnBackwardIcon} class="h-3.5 w-3.5" />
				{t('settings.discord.reset')}
			</Button>
		</div>
	</Dialog.Content>
</Dialog.Root>
