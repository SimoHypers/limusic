<script lang="ts">
	// Enhanced lyrics (#591), in Playback ▸ Lyrics above the sources, and only there. The tuning rows
	// only exist while the switch is on, and each slider carries a reset to its default.
	import type { Snippet } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowTurnBackwardIcon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Switch } from '$lib/components/ui/switch';
	import { Slider } from '$lib/components/ui/slider';
	import { t, type TranslationKey } from '$lib/i18n.svelte';
	import {
		LYRICS_ANIMS,
		LYRICS_DEFAULTS,
		LYRICS_STYLES,
		appearance,
		setAppearance
	} from '$lib/theme.svelte';

	/** Only the sliders get a reset: a switch or a segmented pick is undone in one click anyway. */
	type Key = 'lyricsAnimStrength' | 'lyricsGlow';
</script>

{#snippet row(o: { title: string; desc: string; badge?: string; control?: Snippet; below?: Snippet })}
	<div class="px-4 py-3.5">
		<div class="flex items-center justify-between gap-6">
			<div class="min-w-0">
				<div class="flex items-center gap-2">
					<span class="text-sm font-medium">{o.title}</span>
					{#if o.badge}
						<span
							class="rounded-full bg-primary/12 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-primary"
						>
							{o.badge}
						</span>
					{/if}
				</div>
				<p class="mt-1 max-w-prose text-xs leading-relaxed text-muted-foreground">{o.desc}</p>
			</div>
			{#if o.control}
				<div class="flex shrink-0 items-center gap-1">{@render o.control()}</div>
			{/if}
		</div>
		{#if o.below}
			<div class="mt-3">{@render o.below()}</div>
		{/if}
	</div>
{/snippet}

<!-- Always shown, disabled at the default: hiding it left a gap that pushed the percentage in. Same
     icon as Appearance's tint and roundness resets. -->
{#snippet reset(key: Key)}
	<Button
		variant="ghost"
		size="icon-sm"
		class="text-muted-foreground"
		disabled={appearance[key] === LYRICS_DEFAULTS[key]}
		aria-label={t('common.reset')}
		title={t('common.reset')}
		onclick={() => setAppearance({ [key]: LYRICS_DEFAULTS[key] })}
	>
		<HugeiconsIcon icon={ArrowTurnBackwardIcon} size={15} strokeWidth={2} />
	</Button>
{/snippet}

<!-- Segmented, like audio quality: one exclusive choice. Wraps rather than overflowing when a
     translation runs long. -->
{#snippet segmented(key: 'lyricsAnim' | 'lyricsStyle', options: readonly string[], prefix: string)}
	<div class="inline-flex max-w-full flex-wrap rounded-lg bg-muted p-0.5">
		{#each options as opt (opt)}
			<button
				type="button"
				onclick={() => setAppearance({ [key]: opt })}
				aria-pressed={appearance[key] === opt}
				class="cursor-pointer rounded-md px-3 py-1.5 text-xs font-medium transition-colors {appearance[
					key
				] === opt
					? 'bg-background text-foreground shadow-sm'
					: 'text-muted-foreground hover:text-foreground'}"
			>
				{t(`${prefix}${opt}` as TranslationKey)}
			</button>
		{/each}
	</div>
{/snippet}

{#snippet percent(key: 'lyricsAnimStrength' | 'lyricsGlow')}
	<span class="w-9 text-right font-mono text-xs text-muted-foreground tabular-nums">
		{appearance[key]}%
	</span>
	{@render reset(key)}
{/snippet}

{#snippet slider(key: 'lyricsAnimStrength' | 'lyricsGlow', label: string, min: number, max: number)}
	<Slider
		type="single"
		aria-label={label}
		{min}
		{max}
		step={5}
		value={appearance[key]}
		onValueChange={(v: number) => setAppearance({ [key]: v })}
	/>
{/snippet}

{#snippet enabledSwitch()}<Switch
		aria-label={t('settings.themes.enhanced_lyrics')}
		checked={appearance.enhancedLyrics}
		onCheckedChange={(on) => setAppearance({ enhancedLyrics: on })}
	/>{/snippet}
{#snippet lineSyncSwitch()}<Switch
		aria-label={t('settings.themes.enhanced_lyrics_line_sync')}
		checked={appearance.enhancedLyricsLineSync}
		onCheckedChange={(on) => setAppearance({ enhancedLyricsLineSync: on })}
	/>{/snippet}
{#snippet animPicker()}{@render segmented('lyricsAnim', LYRICS_ANIMS, 'settings.themes.lyrics_anim_')}{/snippet}
{#snippet stylePicker()}{@render segmented('lyricsStyle', LYRICS_STYLES, 'settings.themes.lyrics_style_')}{/snippet}
{#snippet strengthValue()}{@render percent('lyricsAnimStrength')}{/snippet}
{#snippet strengthSlider()}{@render slider('lyricsAnimStrength', t('settings.themes.lyrics_anim_strength'), 25, 200)}{/snippet}
{#snippet glowValue()}{@render percent('lyricsGlow')}{/snippet}
{#snippet glowSlider()}{@render slider('lyricsGlow', t('settings.themes.lyrics_glow'), 0, 100)}{/snippet}

<div class="divide-y divide-border/60 overflow-hidden rounded-xl border bg-card">
	{@render row({
		title: t('settings.themes.enhanced_lyrics'),
		badge: t('settings.themes.experimental'),
		desc: t('settings.themes.enhanced_lyrics_hint'),
		control: enabledSwitch
	})}
	{#if appearance.enhancedLyrics}
		<!-- Tinted, so the tuning reads as belonging to the switch above it. -->
		<div class="divide-y divide-border/60 bg-muted/25">
			{@render row({
				title: t('settings.themes.enhanced_lyrics_line_sync'),
				desc: t('settings.themes.enhanced_lyrics_line_sync_hint'),
				control: lineSyncSwitch
			})}
			{@render row({
				title: t('settings.themes.lyrics_anim'),
				desc: t('settings.themes.lyrics_anim_hint'),
				below: animPicker
			})}
			{@render row({
				title: t('settings.themes.lyrics_style'),
				desc: t('settings.themes.lyrics_style_hint'),
				below: stylePicker
			})}
			{@render row({
				title: t('settings.themes.lyrics_anim_strength'),
				desc: t('settings.themes.lyrics_anim_strength_hint'),
				control: strengthValue,
				below: strengthSlider
			})}
			{@render row({
				title: t('settings.themes.lyrics_glow'),
				desc: t('settings.themes.lyrics_glow_hint'),
				control: glowValue,
				below: glowSlider
			})}
		</div>
	{/if}
</div>
