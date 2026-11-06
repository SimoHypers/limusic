<script lang="ts">
	// Podcast speed, in shuffle's place while an episode plays (#500). The same session tempo the
	// effects dialog sets, offered as the steps a podcast app has, so it applies to what plays next
	// too and the effects button stays lit while it's off 1x.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Tick02Icon } from '@hugeicons/core-free-icons';
	import * as Popover from '$lib/components/ui/popover';
	import { inRoom } from '$lib/lt.svelte';
	import { playback, setTempoPitch } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	let { class: klass = '' }: { class?: string } = $props();

	const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2];
	const label = (s: number) => `${+s.toFixed(2)}x`;

	let open = $state(false);
	// Rust refuses tempo in a listening room, so the chip says why instead of failing on click.
	const locked = $derived(inRoom());
</script>

<Popover.Root bind:open>
	<Popover.Trigger
		disabled={locked}
		class="cursor-pointer font-semibold tabular-nums disabled:cursor-default disabled:opacity-50 {playback.speed !==
		1
			? 'text-primary'
			: 'text-muted-foreground hover:text-foreground'} {klass}"
		aria-label={t('player.speed')}
		title={locked ? t('dialogs.tempo_pitch.tempo_room_locked') : t('player.speed')}
	>
		<!-- Trimmed to the cap height, or the digits sit high: the line box leaves descender room
		     under them that they don't use, and the icons beside them are centred on their ink. -->
		<span class="[text-box:trim-both_cap_alphabetic]">{label(playback.speed)}</span>
	</Popover.Trigger>
	<Popover.Content side="top" sideOffset={8} class="w-36 gap-0 p-1.5">
		<p class="px-2.5 pb-1.5 pt-1 text-xs font-medium text-muted-foreground">{t('player.speed')}</p>
		{#each SPEEDS as s (s)}
			<button
				type="button"
				class="flex w-full cursor-pointer items-center rounded-lg px-2.5 py-1.5 text-left text-sm tabular-nums hover:bg-foreground/10 {s ===
				playback.speed
					? 'font-semibold text-primary'
					: ''}"
				onclick={() => {
					setTempoPitch(s, playback.semitones);
					open = false;
				}}
			>
				{s === 1 ? t('dialogs.tempo_pitch.normal') : label(s)}
				{#if s === playback.speed}
					<HugeiconsIcon icon={Tick02Icon} class="ml-auto h-4 w-4" />
				{/if}
			</button>
		{/each}
	</Popover.Content>
</Popover.Root>
