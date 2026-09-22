<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		KeyboardIcon,
		Cancel01Icon,
		PlayCircleIcon,
		VolumeHighIcon,
		ComputerIcon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Switch } from '$lib/components/ui/switch';
	import { t } from '$lib/i18n.svelte';
	import {
		hotkeys,
		HOTKEY_ACTIONS,
		eventToShortcut,
		splitShortcut,
		type HotkeyActionDef
	} from '$lib/hotkeys.svelte';

	onMount(() => {
		hotkeys.load();
	});

	function handleKeyDown(e: KeyboardEvent) {
		if (!hotkeys.recordingAction) return;

		e.preventDefault();
		e.stopPropagation();

		// Cancel recording on single Escape press
		if (e.key === 'Escape' && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
			hotkeys.recordingAction = null;
			return;
		}

		const combo = eventToShortcut(e);
		if (combo) {
			hotkeys.setBinding(hotkeys.recordingAction, combo);
		}
	}

	$effect(() => {
		if (hotkeys.recordingAction) {
			window.addEventListener('keydown', handleKeyDown, { capture: true });
			return () => {
				window.removeEventListener('keydown', handleKeyDown, { capture: true });
			};
		}
	});

	onDestroy(() => {
		hotkeys.recordingAction = null;
	});

	const GROUP = 'mb-7 last:mb-1';
	const LABEL =
		'mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground';
	const CARD = 'divide-y divide-border/60 overflow-hidden rounded-xl border bg-card';

	const playbackActions = $derived(HOTKEY_ACTIONS.filter((a) => a.section === 'playback'));
	const audioActions = $derived(HOTKEY_ACTIONS.filter((a) => a.section === 'audio'));
	const windowActions = $derived(HOTKEY_ACTIONS.filter((a) => a.section === 'window'));
</script>

<div class="pb-2">
	<!-- Description & master toggle -->
	<section class={GROUP}>
		<div class={CARD}>
			<div class="flex items-center justify-between gap-4 p-4">
				<div class="min-w-0 flex-1 space-y-0.5">
					<div class="text-sm font-medium">{t('settings.hotkeys.enable')}</div>
					<div class="text-xs text-muted-foreground leading-relaxed">
						{t('settings.hotkeys.description')}
					</div>
					{#if hotkeys.wayland}
						<div class="text-xs text-muted-foreground leading-relaxed">
							{t('settings.hotkeys.wayland_hint')}
						</div>
					{/if}
				</div>
				<div class="shrink-0">
					<Switch
						checked={hotkeys.enabled}
						onCheckedChange={(val) => hotkeys.toggleEnabled(val)}
						disabled={hotkeys.saving}
					/>
				</div>
			</div>
		</div>
	</section>

	{#if !hotkeys.enabled}
		<div class="rounded-xl border border-dashed p-6 text-center text-xs text-muted-foreground">
			{t('settings.hotkeys.enable_hint')}
		</div>
	{:else}
		<!-- Playback Section -->
		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.hotkeys.section_playback')}</h3>
			<div class={CARD}>
				{#each playbackActions as action (action.id)}
					{@render hotkeyRow(action)}
				{/each}
			</div>
		</section>

		<!-- Audio Section -->
		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.hotkeys.section_audio')}</h3>
			<div class={CARD}>
				{#each audioActions as action (action.id)}
					{@render hotkeyRow(action)}
				{/each}
			</div>
		</section>

		<!-- Window Section -->
		<section class={GROUP}>
			<h3 class={LABEL}>{t('settings.hotkeys.section_window')}</h3>
			<div class={CARD}>
				{#each windowActions as action (action.id)}
					{@render hotkeyRow(action)}
				{/each}
			</div>
		</section>

		<!-- Reset button -->
		<div class="flex justify-end pt-2">
			<Button
				variant="outline"
				size="sm"
				class="text-xs text-muted-foreground hover:text-foreground"
				onclick={() => hotkeys.resetDefaults()}
				disabled={hotkeys.saving}
			>
				{t('settings.hotkeys.reset')}
			</Button>
		</div>
	{/if}
</div>

{#snippet hotkeyRow(action: HotkeyActionDef)}
	{@const combo = hotkeys.bindings[action.id]}
	{@const isRecording = hotkeys.recordingAction === action.id}
	{@const error = hotkeys.errors[action.id]}

	<div class="px-4 py-3 sm:py-3.5 transition-colors">
		<div class="flex flex-col gap-2.5 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
			<div class="min-w-0 flex-1">
				<div class="text-sm font-medium leading-snug">{t(action.titleKey)}</div>
				<div class="mt-0.5 text-xs text-muted-foreground leading-relaxed">{t(action.hintKey)}</div>
			</div>

			<div class="flex shrink-0 flex-wrap items-center gap-2 sm:justify-end">
				{#if isRecording}
					<div
						class="flex animate-pulse items-center gap-1.5 rounded-lg border border-primary/50 bg-primary/10 px-3 py-1.5 text-xs font-medium text-primary shadow-xs"
					>
						<HugeiconsIcon icon={KeyboardIcon} class="h-3.5 w-3.5 shrink-0" />
						<span class="text-xs">{t('settings.hotkeys.recording')}</span>
					</div>
					<Button
						variant="ghost"
						size="sm"
						class="h-7 px-2 text-xs"
						onclick={() => (hotkeys.recordingAction = null)}
					>
						{t('common.cancel')}
					</Button>
				{:else}
					{#if combo}
						<div class="flex flex-wrap items-center gap-1">
							{#each splitShortcut(combo) as key}
								<kbd
									class="inline-flex min-w-[20px] items-center justify-center rounded border bg-muted/80 px-2 py-0.5 font-mono text-[11px] font-semibold text-foreground shadow-xs select-none"
								>
									{key}
								</kbd>
							{/each}
						</div>
					{:else}
						<span class="text-xs italic text-muted-foreground/70">
							{t('settings.hotkeys.off')}
						</span>
					{/if}

					<div class="flex items-center gap-1.5">
						<Button
							variant="outline"
							size="sm"
							class="h-7 cursor-pointer px-2.5 text-xs font-medium"
							onclick={() => (hotkeys.recordingAction = action.id)}
							disabled={hotkeys.saving}
						>
							{t('settings.hotkeys.rebind')}
						</Button>

						{#if combo}
							<button
								type="button"
								class="flex h-7 w-7 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
								title={t('settings.hotkeys.clear')}
								onclick={() => hotkeys.clearBinding(action.id)}
								disabled={hotkeys.saving}
							>
								<HugeiconsIcon icon={Cancel01Icon} class="h-3.5 w-3.5" />
							</button>
						{/if}
					</div>
				{/if}
			</div>
		</div>

		{#if error}
			<div class="mt-1.5 text-[11px] font-medium text-destructive">
				{t('settings.hotkeys.failed_register')}
			</div>
		{/if}
	</div>
{/snippet}
