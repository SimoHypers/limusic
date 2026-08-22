<script lang="ts">
	// "Advanced": tempo and pitch for what's playing, from the player bar's ⋮ menu. Both apply on
	// every step (no Apply button), and neither is persisted: mpv comes up at 1.00x / 0 on every
	// launch, so `playback` holding them is the whole state.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		AudioWave02Icon,
		FastForwardIcon,
		MinusSignIcon,
		PlusSignIcon
	} from '@hugeicons/core-free-icons';
	import type { IconSvgElement } from '@hugeicons/svelte';
	import { Button } from '$lib/components/ui/button';
	import * as Dialog from '$lib/components/ui/dialog';
	import { inRoom } from '$lib/lt.svelte';
	import { playback, setTempoPitch as apply } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	let { open = $bindable(false) }: { open: boolean } = $props();

	// 0.25 → 2.00 in 0.05 steps, and the index is the source of truth. Storing the float and
	// searching the list back for it (Metrolist's version) walks off the end of the array the
	// moment anything sets a speed that isn't one of these 36 exact values.
	const SPEEDS = Array.from({ length: 36 }, (_, i) => Math.round((0.25 + i * 0.05) * 100) / 100);
	const SEMITONES = { min: -12, max: 12 };

	let speedIndex = $derived(Math.max(0, SPEEDS.indexOf(playback.speed)));
</script>

{#snippet stepper(
	icon: IconSvgElement,
	label: string,
	value: string,
	onStep: (dir: number) => void,
	atMin: boolean,
	atMax: boolean
)}
	<div class="flex items-center gap-4">
		<HugeiconsIcon {icon} class="h-5 w-5 shrink-0 text-muted-foreground" />
		<span class="flex-1 text-sm">{label}</span>
		<Button
			variant="ghost"
			size="icon-sm"
			disabled={atMin}
			onclick={() => onStep(-1)}
			aria-label="Decrease {label}"
		>
			<HugeiconsIcon icon={MinusSignIcon} class="h-4 w-4" />
		</Button>
		<span class="w-16 text-center font-medium tabular-nums">{value}</span>
		<Button
			variant="ghost"
			size="icon-sm"
			disabled={atMax}
			onclick={() => onStep(1)}
			aria-label="Increase {label}"
		>
			<HugeiconsIcon icon={PlusSignIcon} class="h-4 w-4" />
		</Button>
	</div>
{/snippet}

<Dialog.Root bind:open>
	<Dialog.Content class="gap-5 sm:max-w-sm">
		<div class="grid gap-1">
			<Dialog.Title class="text-lg font-semibold">{t('dialogs.tempo_pitch.title')}</Dialog.Title>
			<Dialog.Description class="text-xs text-muted-foreground">
				Applies to everything you play, until you change it back or restart the app.
			</Dialog.Description>
		</div>

		<div class="flex flex-col gap-4">
			<!-- Tempo moves you off the shared clock, so it's out while a room is on. Pitch is yours
			     alone: it changes nothing about when the next track starts. -->
			{#if !inRoom()}
				{@render stepper(
					FastForwardIcon,
					t('dialogs.tempo_pitch.tempo'),
					`${playback.speed.toFixed(2)}x`,
					(d) => apply(SPEEDS[speedIndex + d], playback.semitones),
					speedIndex === 0,
					speedIndex === SPEEDS.length - 1
				)}
			{/if}
			{@render stepper(
				AudioWave02Icon,
				t('dialogs.tempo_pitch.pitch'),
				playback.semitones > 0 ? `+${playback.semitones}` : String(playback.semitones),
				(d) => apply(playback.speed, playback.semitones + d),
				playback.semitones === SEMITONES.min,
				playback.semitones === SEMITONES.max
			)}
		</div>

		<div class="flex justify-end gap-2">
			<!-- Reset does not close: you're usually resetting to hear the difference. -->
			<Button variant="outline" size="sm" onclick={() => apply(1, 0)}>{t('common.reset')}</Button>
			<Button size="sm" onclick={() => (open = false)}>{t('common.done')}</Button>
		</div>
	</Dialog.Content>
</Dialog.Root>
