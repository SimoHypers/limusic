<script lang="ts">
	// "When you listen", in four: morning, afternoon, evening and night as a donut, the part with the
	// most listening named in its middle, and what that makes you underneath. shadcn-svelte's "donut
	// with text", in the app's accent like the rest of the page. Seconds come folded from the same
	// weekday x hour grid as the heatmap.
	import { PieChart, Text, Tooltip } from 'layerchart';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Moon02Icon, Sun03Icon, SunriseIcon, SunsetIcon } from '@hugeicons/core-free-icons';
	import * as Chart from '$lib/components/ui/chart';
	import { DAYPARTS, dayparts, listened, tagOf, type Daypart } from '$lib/stats';
	import { currentLocale, t } from '$lib/i18n.svelte';

	let { hours }: { hours: number[] } = $props();

	const tag = $derived(tagOf(currentLocale.id));
	const percent = $derived(new Intl.NumberFormat(tag, { style: 'percent' }));
	const hourFmt = $derived(new Intl.DateTimeFormat(tag, { hour: 'numeric' }));
	const at = (h: number) => hourFmt.format(new Date(2026, 0, 4, h));

	// Four shades of the accent (so the artwork tint moves it too): toward the text colour for
	// morning, the accent itself for afternoon, then sinking into the page for evening and night. On
	// a dark theme that reads as light at dawn, darkest at night.
	const shade = (to: string, pct: number) => `color-mix(in oklch, var(--primary) ${pct}%, ${to})`;
	const config = $derived({
		morning: { label: t('stats.dayparts.morning'), color: shade('var(--foreground)', 55) },
		afternoon: { label: t('stats.dayparts.afternoon'), color: 'var(--primary)' },
		evening: { label: t('stats.dayparts.evening'), color: shade('var(--background)', 70) },
		night: { label: t('stats.dayparts.night'), color: shade('var(--background)', 45) }
	} satisfies Chart.ChartConfig);
	const ICONS = { morning: SunriseIcon, afternoon: Sun03Icon, evening: SunsetIcon, night: Moon02Icon };

	const secs = $derived(dayparts(hours));
	const total = $derived(DAYPARTS.reduce((sum, p) => sum + secs[p.key], 0));
	// In the day's order around the ring (`sort: null` below), not largest first.
	const data = $derived(
		DAYPARTS.map((p) => ({ key: p.key, secs: secs[p.key], color: `var(--color-${p.key})` }))
	);
	const top = $derived(DAYPARTS.reduce((a, b) => (secs[b.key] > secs[a.key] ? b : a)));
	const share = (k: Daypart) => percent.format(total ? secs[k] / total : 0);
</script>

<!-- h-full: as tall as the heatmap beside it on a wide window, its content centred in the space. -->
<div class="flex h-full flex-col justify-center rounded-2xl bg-foreground/[0.04] p-5">
	<Chart.Container {config} class="mx-auto aspect-square w-full max-w-[13.5rem]">
		<PieChart
			{data}
			key="key"
			value="secs"
			c="color"
			innerRadius={0.62}
			cornerRadius={5}
			padAngle={0.02}
			padding={6}
			props={{ pie: { sort: null, motion: 'tween' } }}
		>
			{#snippet aboveMarks()}
				<Text
					value={share(top.key)}
					textAnchor="middle"
					verticalAnchor="middle"
					class="fill-foreground font-heading text-3xl! font-bold"
					dy={-6}
				/>
				<Text
					value={config[top.key].label}
					textAnchor="middle"
					verticalAnchor="middle"
					class="fill-muted-foreground! text-sm!"
					dy={18}
				/>
			{/snippet}
			{#snippet tooltip()}
				<Tooltip.Root variant="none">
					{#snippet children({ data: d })}
						<div
							class="flex min-w-32 items-center gap-2 rounded-lg border border-border/50 bg-background px-2.5 py-1.5 text-xs shadow-xl"
						>
							<span
								class="h-2.5 w-2.5 shrink-0 rounded-[2px]"
								style="background:{config[d.key as Daypart].color}"
							></span>
							<span class="text-muted-foreground">{config[d.key as Daypart].label}</span>
							<span class="ml-auto pl-3 font-medium tabular-nums text-foreground">
								{listened(d.secs, tag)}
							</span>
						</div>
					{/snippet}
				</Tooltip.Root>
			{/snippet}
		</PieChart>
	</Chart.Container>

	<div class="mt-4 grid grid-cols-2 gap-x-5 gap-y-2">
		{#each DAYPARTS as p (p.key)}
			<div class="flex min-w-0 items-center gap-2 text-xs" title="{at(p.from)}–{at(p.to)}">
				<span class="h-2.5 w-2.5 shrink-0 rounded-[3px]" style="background:{config[p.key].color}"></span>
				<span class="truncate text-muted-foreground">{config[p.key].label}</span>
				<span class="ml-auto font-medium tabular-nums">{share(p.key)}</span>
			</div>
		{/each}
	</div>

	<div class="mt-5">
		<div class="flex items-center gap-2 text-sm font-semibold">
			<!-- Keyed: `icon` is read once at mount, and a range switch can change the part. -->
			{#key top.key}
				<HugeiconsIcon icon={ICONS[top.key]} strokeWidth={2} class="h-4 w-4 shrink-0 text-primary" />
			{/key}
			{t(`stats.persona.${top.key}`)}
		</div>
		<p class="mt-1 text-xs text-muted-foreground">
			{t('stats.persona_hint', { from: at(top.from), to: at(top.to) })}
		</p>
	</div>
</div>
