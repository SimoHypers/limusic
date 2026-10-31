<script lang="ts">
	// Tempo, pitch and reverb, from the player bar. Two parts, in the order people use them: pick a
	// ready-made sound, or adjust one yourself. Every change applies on release, nothing waits for
	// Done, and the reverb's engineering controls stay folded away until asked for.
	import { untrack } from 'svelte';
	import { RadioGroup } from 'bits-ui';
	import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/svelte';
	import {
		ArrowDown01Icon,
		ArrowTurnBackwardIcon,
		ArrowUp01Icon,
		Bookmark02Icon,
		BookmarkAdd02Icon,
		Cancel01Icon,
		GuitarIcon,
		Moon02Icon,
		MusicNote01Icon,
		PreferenceVerticalIcon,
		SparklesIcon,
		TheaterIcon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { Switch } from '$lib/components/ui/switch';
	import { Slider } from '$lib/components/ui/slider';
	import * as Dialog from '$lib/components/ui/dialog';
	import type { Reverb } from '$lib/api';
	import { inRoom } from '$lib/lt.svelte';
	import { playback, setTempoPitch } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import {
		BUILTIN_PRESETS,
		EFFECT_PRESETS_KEY,
		ORIGINAL,
		PRESET_NAME_MAX,
		ROOMS,
		ROOM_DECAYS,
		presetSettings,
		readEffectPresets,
		sameEffects,
		sameSound,
		withRoom,
		type BuiltinId,
		type EffectPreset,
		type EffectSettings
	} from '$lib/effect-presets';

	let { open = $bindable(false) }: { open: boolean } = $props();

	const BUILTIN_ICONS: Record<BuiltinId, IconSvgElement> = {
		original: MusicNote01Icon,
		slowed: Moon02Icon,
		nightcore: SparklesIcon,
		practice: GuitarIcon,
		hall: TheaterIcon
	};
	const TEMPO = { min: 0.5, max: 2, step: 0.05 };
	const FINE = [
		{ key: 'dry', min: 0, max: 100, step: 1 },
		{ key: 'decay', min: 0.1, max: 10, step: 0.1 },
		{ key: 'preDelay', min: 0, max: 200, step: 1 },
		{ key: 'damping', min: 500, max: 20000, step: 100 },
		{ key: 'reflections', min: 0, max: 100, step: 1 }
	] as const;
	type FineKey = (typeof FINE)[number]['key'];
	const LABEL = 'mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground';
	// Each room's tail drawn as bars over 3.6 s, so a hall reads as longer than a room at a glance.
	const TAILS = ROOM_DECAYS.map((decay) =>
		Array.from({ length: 9 }, (_, i) =>
			Math.max(2, Math.round(16 * Math.max(0, 1 - (i * 0.4) / (decay * 1.25)) ** 1.6))
		)
	);

	// --- what the dialog shows --------------------------------------------------------------------
	// The backend's confirmed values, overlaid by a change sent but not yet confirmed (`pending`)
	// and by a slider mid-drag.
	let presets = $state<EffectPreset[]>([]); // saved on this device, read on open
	let pending = $state<EffectSettings | null>(null);
	let draft = $state<{ speed?: number; semitones?: number }>({});
	let reverbDraft = $state<Partial<Reverb>>({});
	const base: EffectSettings = $derived(
		pending ?? { speed: playback.speed, semitones: playback.semitones, reverb: playback.reverb }
	);
	const view: EffectSettings = $derived({
		speed: draft.speed ?? base.speed,
		semitones: draft.semitones ?? base.semitones,
		reverb: { ...base.reverb, ...reverbDraft }
	});
	// Every confirmation replaces `playback.reverb`, and so does a refused change (the store resets
	// it), so either one ends the wait.
	$effect(() => {
		void playback.reverb;
		untrack(() => (pending = null));
	});

	const roomLocked = $derived(inRoom());
	/** A preset as it plays here: a listening room keeps its shared clock at normal tempo. */
	const here = (s: EffectSettings) => presetSettings(s, roomLocked);
	const activeBuiltin = $derived(BUILTIN_PRESETS.find((p) => sameSound(view, here(p)))?.id ?? null);
	const activeSaved = $derived(presets.find((p) => sameSound(view, here(p)))?.name ?? null);
	const nothingToSave = $derived(!!activeBuiltin || !!activeSaved);

	function commit(next: EffectSettings) {
		const settings = {
			speed: roomLocked ? 1 : next.speed,
			semitones: next.semitones,
			reverb: { ...next.reverb }
		};
		pending = settings;
		draft = {};
		reverbDraft = {};
		setTempoPitch(settings.speed, settings.semitones, settings.reverb);
	}
	const setReverb = (patch: Partial<Reverb>) => commit({ ...base, reverb: { ...base.reverb, ...patch } });

	// --- presets ----------------------------------------------------------------------------------
	let storageReady = $state(true);
	let presetError = $state('');
	let naming = $state(false);
	let presetName = $state('');
	const trimmed = $derived(presetName.trim());
	const existing = $derived(presets.find((p) => p.name.toLowerCase() === trimmed.toLowerCase()));

	$effect(() => {
		if (!open) {
			untrack(closeNaming);
			return;
		}
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

	function savePreset() {
		if (!trimmed || trimmed.length > PRESET_NAME_MAX) return;
		const preset: EffectPreset = {
			name: trimmed,
			// In a room the tempo shown is the room's, so a replaced preset keeps its own.
			speed: roomLocked ? (existing?.speed ?? 1) : view.speed,
			semitones: view.semitones,
			reverb: { ...view.reverb }
		};
		const next = existing
			? presets.map((p) => (p.name === existing.name ? preset : p))
			: [...presets, preset];
		if (persistPresets(next)) closeNaming();
	}
	function closeNaming() {
		naming = false;
		presetName = '';
	}
	const deletePreset = (name: string) => persistPresets(presets.filter((p) => p.name !== name));

	// --- wording ----------------------------------------------------------------------------------
	const signed = (n: number) => (n > 0 ? `+${n}` : n < 0 ? `−${-n}` : '0');
	const roomName = (preset: number) => t(`dialogs.tempo_pitch.reverb_types.${ROOMS[preset - 1]}`);
	/** What a saved preset does, in place of the description a built-in has. */
	function summary(p: EffectSettings) {
		const parts = [];
		if (p.speed !== 1) parts.push(`${p.speed.toFixed(2)}×`);
		if (p.semitones) parts.push(t('dialogs.tempo_pitch.pitch_short', { n: signed(p.semitones) }));
		if (p.reverb.enabled) parts.push(roomName(p.reverb.preset));
		return parts.join(', ') || t('dialogs.tempo_pitch.builtin.original');
	}
	const fmt = (secs: number) => {
		const s = Math.round(secs);
		const h = Math.floor(s / 3600);
		const m = Math.floor((s % 3600) / 60);
		const ss = String(s % 60).padStart(2, '0');
		return h ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
	};
	const tempoNote = $derived(
		roomLocked
			? t('dialogs.tempo_pitch.tempo_room_locked')
			: view.speed !== 1 && playback.duration > 0
				? t('dialogs.tempo_pitch.song_length', { time: fmt(playback.duration / view.speed) })
				: ''
	);
	function fineValue(key: FineKey, v: number) {
		if (key === 'decay') return `${v.toFixed(1)} s`;
		if (key === 'damping') return `${(v / 1000).toFixed(1)} kHz`;
		return key === 'preDelay' ? `${v} ms` : `${v}%`;
	}
	let fineOpen = $state(false);
</script>

{#snippet card(
	label: string,
	desc: string,
	icon: IconSvgElement,
	active: boolean,
	onclick: () => void,
	disabled = false,
	dashed = false,
	extraClass = ''
)}
	<!-- Sized like the room tiles below, so the two pickers read as one family. The description is
	     the tooltip: the name already says enough to choose by. -->
	<button
		type="button"
		class="flex w-full cursor-pointer items-center gap-2 rounded-lg border px-2.5 py-2 text-left text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default disabled:opacity-50 {dashed
			? 'border-dashed'
			: ''} {active
			? 'border-primary bg-primary/8 text-foreground'
			: 'text-muted-foreground enabled:hover:border-foreground/25 enabled:hover:text-foreground'} {extraClass}"
		aria-pressed={dashed ? undefined : active}
		title={desc}
		{disabled}
		{onclick}
	>
		<HugeiconsIcon {icon} class="h-4 w-4 shrink-0 {active ? 'text-primary' : ''}" />
		<span class="min-w-0 truncate">{label}</span>
	</button>
{/snippet}

<!-- A slider's plain words at both ends, and its normal value marked where it sits. -->
{#snippet scale(low: string, high: string, normalAt: number)}
	<div class="relative mt-2 h-5 text-[11px] text-muted-foreground">
		<span class="absolute left-0 top-1">{low}</span>
		<span class="absolute top-0 -translate-x-1/2 text-center" style="left:{normalAt}%">
			<span class="mx-auto block h-1 w-px bg-muted-foreground/50"></span>
			{t('dialogs.tempo_pitch.normal')}
		</span>
		<span class="absolute right-0 top-1">{high}</span>
	</div>
{/snippet}

{#snippet valueRow(label: string, value: string, changed: boolean, reset?: () => void)}
	<div class="mb-3 flex items-center gap-2">
		<span class="flex-1 text-sm font-medium">{label}</span>
		{#if reset}
			<!-- Kept in the layout when hidden, so the value beside it doesn't jump. -->
			<button
				type="button"
				class="cursor-pointer rounded px-1 text-xs text-primary outline-none transition-colors hover:text-primary/80 focus-visible:ring-2 focus-visible:ring-ring {changed
					? ''
					: 'invisible'}"
				onclick={reset}
			>
				{t('common.reset')}
			</button>
		{/if}
		<span class="min-w-[4.5rem] rounded-md bg-muted px-2 py-0.5 text-center text-sm font-semibold">
			{value}
		</span>
	</div>
{/snippet}

<Dialog.Root bind:open>
	<Dialog.Content class="flex max-h-[85dvh] flex-col gap-0 overflow-hidden p-0 sm:max-w-xl">
		<div class="flex items-start gap-3 px-6 pb-4 pr-14 pt-6">
			<span
				class="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary"
			>
				<HugeiconsIcon icon={PreferenceVerticalIcon} class="h-5 w-5" />
			</span>
			<div class="min-w-0">
				<Dialog.Title class="font-heading text-base font-semibold">
					{t('dialogs.tempo_pitch.title')}
				</Dialog.Title>
				<Dialog.Description class="text-xs leading-relaxed text-muted-foreground">
					{t('dialogs.tempo_pitch.desc')}
				</Dialog.Description>
			</div>
		</div>

		<div class="min-h-0 flex-1 overflow-y-auto px-6 pb-6">
			<h3 class={LABEL}>{t('dialogs.tempo_pitch.presets')}</h3>
			<div class="mb-6">
				<div class="grid grid-cols-3 gap-2">
					{#each BUILTIN_PRESETS as preset (preset.id)}
						{@render card(
							t(`dialogs.tempo_pitch.builtin.${preset.id}`),
							t(`dialogs.tempo_pitch.builtin_hints.${preset.id}`),
							BUILTIN_ICONS[preset.id],
							activeBuiltin === preset.id,
							() => commit(here(preset))
						)}
					{/each}
					{#each presets as preset (preset.name)}
						<!-- The delete button shows on hover or focus, beside the card rather than in it:
						     a button can't hold another one. -->
						<div class="group relative">
							{@render card(
								preset.name,
								summary(preset),
								Bookmark02Icon,
								activeSaved === preset.name,
								() => commit(here(preset)),
								false,
								false,
								'pr-8'
							)}
							<button
								type="button"
								class="invisible absolute right-1.5 top-1/2 flex size-5 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:visible focus-visible:ring-2 focus-visible:ring-ring group-hover:visible group-focus-within:visible disabled:pointer-events-none"
								aria-label={t('dialogs.tempo_pitch.preset_delete', { name: preset.name })}
								disabled={!storageReady}
								onclick={() => deletePreset(preset.name)}
							>
								<HugeiconsIcon icon={Cancel01Icon} class="h-3 w-3" />
							</button>
						</div>
					{/each}
					{@render card(
						t('dialogs.tempo_pitch.preset_save'),
						nothingToSave
							? t('dialogs.tempo_pitch.preset_save_none')
							: t('dialogs.tempo_pitch.preset_save_hint'),
						BookmarkAdd02Icon,
						naming,
						() => (naming = true),
						!storageReady || nothingToSave,
						true
					)}
				</div>
				{#if naming}
					<form
						class="mt-3 flex items-center gap-2"
						onsubmit={(e) => {
							e.preventDefault();
							savePreset();
						}}
					>
						<Input
							bind:value={presetName}
							class="h-9 min-w-0 flex-1"
							maxlength={PRESET_NAME_MAX}
							placeholder={t('dialogs.tempo_pitch.preset_name')}
							aria-label={t('dialogs.tempo_pitch.preset_name')}
							onkeydown={(e) => {
								// Escape here backs out of naming, not out of the whole dialog.
								if (e.key === 'Escape') {
									e.stopPropagation();
									closeNaming();
								}
							}}
							{@attach (node: HTMLElement) => node.focus()}
						/>
						<Button type="submit" size="sm" disabled={!trimmed}>
							{existing ? t('dialogs.tempo_pitch.preset_replace') : t('common.save')}
						</Button>
						<Button type="button" size="sm" variant="ghost" onclick={closeNaming}>
							{t('common.cancel')}
						</Button>
					</form>
					{#if existing}
						<p class="mt-1.5 px-1 text-xs text-muted-foreground">
							{t('dialogs.tempo_pitch.preset_duplicate')}
						</p>
					{/if}
				{/if}
				{#if !storageReady || presetError}
					<p class="mt-2 px-1 text-xs text-destructive" role="alert">
						{!storageReady ? t('dialogs.tempo_pitch.preset_load_error') : presetError}
					</p>
				{/if}
				{#if roomLocked}
					<p class="mt-2 px-1 text-xs leading-relaxed text-muted-foreground">
						{t('dialogs.tempo_pitch.preset_room_hint')}
					</p>
				{/if}
			</div>

			<h3 class={LABEL}>{t('dialogs.tempo_pitch.adjust')}</h3>
			<div class="divide-y divide-border/60 overflow-hidden rounded-xl border bg-card">
				<div class="px-4 py-4">
					{@render valueRow(
						t('dialogs.tempo_pitch.tempo'),
						view.speed === 1 ? t('dialogs.tempo_pitch.normal') : `${view.speed.toFixed(2)}×`,
						view.speed !== 1 && !roomLocked,
						() => commit({ ...base, speed: 1 })
					)}
					<Slider
						type="single"
						min={TEMPO.min}
						max={TEMPO.max}
						step={TEMPO.step}
						value={view.speed}
						disabled={roomLocked}
						onValueChange={(v) => (draft = { speed: Math.round(v * 20) / 20 })}
						onValueCommit={(v) => commit({ ...base, speed: Math.round(v * 20) / 20 })}
						aria-label={t('dialogs.tempo_pitch.tempo')}
					/>
					{@render scale(
						t('dialogs.tempo_pitch.slower'),
						t('dialogs.tempo_pitch.faster'),
						((1 - TEMPO.min) / (TEMPO.max - TEMPO.min)) * 100
					)}
					{#if tempoNote}
						<p class="mt-2 text-xs leading-relaxed text-muted-foreground">{tempoNote}</p>
					{/if}
				</div>

				<div class="px-4 py-4">
					{@render valueRow(
						t('dialogs.tempo_pitch.pitch'),
						view.semitones === 0 ? t('dialogs.tempo_pitch.normal') : signed(view.semitones),
						view.semitones !== 0,
						() => commit({ ...base, semitones: 0 })
					)}
					<Slider
						type="single"
						min={-12}
						max={12}
						step={1}
						value={view.semitones}
						onValueChange={(v) => (draft = { semitones: v })}
						onValueCommit={(v) => commit({ ...base, semitones: v })}
						aria-label={t('dialogs.tempo_pitch.pitch')}
					/>
					{@render scale(t('dialogs.tempo_pitch.lower'), t('dialogs.tempo_pitch.higher'), 50)}
				</div>

				<div class="px-4 py-4">
					<div class="flex items-center justify-between gap-4">
						<div class="min-w-0">
							<div class="text-sm font-medium">{t('dialogs.tempo_pitch.reverb')}</div>
							<p class="mt-0.5 text-xs leading-relaxed text-muted-foreground">
								{t('dialogs.tempo_pitch.reverb_hint')}
							</p>
						</div>
						<Switch
							checked={view.reverb.enabled}
							onCheckedChange={(on) => setReverb({ enabled: on })}
							aria-label={t('dialogs.tempo_pitch.reverb')}
						/>
					</div>
					{#if view.reverb.enabled}
						<div class="mt-4 text-xs font-medium text-muted-foreground">
							{t('dialogs.tempo_pitch.reverb_room')}
						</div>
						<RadioGroup.Root
							value={String(view.reverb.preset)}
							onValueChange={(v) => v && commit({ ...base, reverb: withRoom(base.reverb, Number(v)) })}
							aria-label={t('dialogs.tempo_pitch.reverb_room')}
							class="mt-2 grid grid-cols-3 gap-2"
						>
							{#each ROOMS as room, i (room)}
								<RadioGroup.Item
									value={String(i + 1)}
									class="group flex cursor-pointer items-center gap-2.5 rounded-lg border px-2.5 py-2 text-xs font-medium text-muted-foreground outline-none transition-colors hover:border-foreground/25 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-checked:border-primary data-checked:bg-primary/8 data-checked:text-foreground"
								>
									<span class="flex h-4 shrink-0 items-end gap-[2px]" aria-hidden="true">
										{#each TAILS[i] as height, j (j)}
											<span
												class="w-[2px] rounded-full bg-muted-foreground/35 transition-colors group-data-checked:bg-primary"
												style="height:{height}px"
											></span>
										{/each}
									</span>
									<span class="min-w-0 truncate">{roomName(i + 1)}</span>
								</RadioGroup.Item>
							{/each}
						</RadioGroup.Root>

						<div class="mt-4">
							{@render valueRow(t('dialogs.tempo_pitch.reverb_amount'), `${view.reverb.wet}%`, false)}
							<Slider
								type="single"
								min={0}
								max={100}
								step={1}
								value={view.reverb.wet}
								onValueChange={(v) => (reverbDraft = { wet: v })}
								onValueCommit={(v) => setReverb({ wet: v })}
								aria-label={t('dialogs.tempo_pitch.reverb_amount')}
							/>
						</div>

						<button
							type="button"
							class="-mx-1 mt-4 flex cursor-pointer items-center gap-1 rounded px-1 text-xs font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
							aria-expanded={fineOpen}
							onclick={() => (fineOpen = !fineOpen)}
						>
							{t('dialogs.tempo_pitch.reverb_fine_tune')}
							<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
							<HugeiconsIcon
								icon={ArrowDown01Icon}
								altIcon={ArrowUp01Icon}
								showAlt={fineOpen}
								class="h-3.5 w-3.5"
							/>
						</button>
						{#if fineOpen}
							<div class="mt-3 grid gap-5 rounded-lg bg-muted/40 p-3">
								{#each FINE as control (control.key)}
									<div>
										<div class="mb-2.5 flex items-baseline justify-between gap-3">
											<span class="text-sm">
												{t(`dialogs.tempo_pitch.reverb_controls.${control.key}`)}
											</span>
											<span class="text-xs text-muted-foreground">
												{fineValue(control.key, view.reverb[control.key])}
											</span>
										</div>
										<Slider
											type="single"
											min={control.min}
											max={control.max}
											step={control.step}
											value={view.reverb[control.key]}
											onValueChange={(v) => (reverbDraft = { [control.key]: v })}
											onValueCommit={(v) =>
												setReverb({ [control.key]: control.key === 'decay' ? Math.round(v * 10) / 10 : v })}
											aria-label={t(`dialogs.tempo_pitch.reverb_controls.${control.key}`)}
										/>
										<p class="mt-1.5 text-xs leading-relaxed text-muted-foreground">
											{t(`dialogs.tempo_pitch.reverb_control_hints.${control.key}`)}
										</p>
									</div>
								{/each}
							</div>
						{/if}
					{/if}
				</div>
			</div>
		</div>

		<div class="flex items-center justify-between gap-2 border-t px-6 py-4">
			<Button
				variant="ghost"
				size="sm"
				disabled={sameEffects(view, ORIGINAL)}
				onclick={() => commit(ORIGINAL)}
			>
				<HugeiconsIcon icon={ArrowTurnBackwardIcon} class="h-4 w-4" />
				{t('dialogs.tempo_pitch.reset_all')}
			</Button>
			<Button size="sm" onclick={() => (open = false)}>{t('common.done')}</Button>
		</div>
	</Dialog.Content>
</Dialog.Root>
