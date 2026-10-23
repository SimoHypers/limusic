<script lang="ts">
	// The stats page's activity chart: a bar per day of the window (per month for a year, or a long
	// all time), the busiest one at full strength. No value axis: the hover says exactly how long, and
	// the page's header says the average and the busiest day in words. (An average line was tried:
	// next to the grid lines it read as one more of them.)
	// shadcn-svelte's chart (LayerChart) underneath.
	import { BarChart, type ChartState } from 'layerchart';
	import { cubicOut } from 'svelte/easing';
	import * as Chart from '$lib/components/ui/chart';
	import { listened, tagOf, type Bar } from '$lib/stats';
	import { currentLocale, t } from '$lib/i18n.svelte';

	let {
		bars,
		monthly,
		peak
	}: { bars: Bar[]; monthly: boolean; peak?: string } = $props();

	const tag = $derived(tagOf(currentLocale.id));
	const fmt = (opts: Intl.DateTimeFormatOptions) => new Intl.DateTimeFormat(tag, opts);

	// A label every few bars, counted back from today so the newest bar is always named. Seven a
	// row for days, twelve for months: about what fits at the narrowest width the page allows.
	const step = $derived(Math.ceil(bars.length / (monthly ? 12 : 7)));
	const ticks = $derived(bars.filter((_, i) => (bars.length - 1 - i) % step === 0).map((b) => b.key));
	const spansYears = $derived(
		bars.length > 1 && bars[0].date.getFullYear() !== bars[bars.length - 1].date.getFullYear()
	);
	const byKey = $derived(new Map(bars.map((b) => [b.key, b])));
	const dateOf = (key: unknown) => byKey.get(String(key))?.date ?? new Date();
	const tickLabel = $derived(
		monthly
			? fmt(spansYears ? { month: 'short', year: '2-digit' } : { month: 'short' })
			: bars.length <= 7
				? fmt({ weekday: 'short' })
				: fmt({ month: 'short', day: 'numeric' })
	);
	const tipLabel = $derived(
		monthly
			? fmt({ month: 'long', year: 'numeric' })
			: fmt({ weekday: 'long', month: 'long', day: 'numeric' })
	);

	// The tooltip only takes the formatter below for a series with a label; without one it prints
	// the raw seconds.
	const config = $derived({
		secs: { label: t('stats.activity'), color: 'var(--primary)' }
	} satisfies Chart.ChartConfig);

	let context = $state<ChartState>();
</script>

<Chart.Container {config} class="aspect-auto h-56 w-full">
	<BarChart
		bind:context
		data={bars}
		x="key"
		y="secs"
		bandPadding={bars.length > 40 ? 0.2 : 0.3}
		axis="x"
		rule={false}
		series={[{ key: 'secs', label: config.secs.label, color: config.secs.color }]}
		props={{
			bars: {
				stroke: 'none',
				radius: bars.length > 40 ? 2 : 5,
				rounded: 'top',
				// The busiest bar is where the eye should land; the rest stay readable, quieter.
				fillOpacity: (d: { key: string }) => (!peak || d.key === peak ? 1 : 0.55),
				// One rise from the baseline when the data lands or the range changes: a tween on a few
				// dozen rects, once, never a loop (docs/UI-PERFORMANCE.md).
				initialY: context?.height,
				initialHeight: 0,
				motion: {
					y: { type: 'tween', duration: 450, easing: cubicOut },
					height: { type: 'tween', duration: 450, easing: cubicOut }
				}
			},
			highlight: { area: { class: 'fill-foreground/5' } },
			xAxis: { ticks, format: (k: unknown) => tickLabel.format(dateOf(k)) }
		}}
	>
		{#snippet tooltip()}
			<Chart.Tooltip labelFormatter={(k: unknown) => tipLabel.format(dateOf(k))}>
				{#snippet formatter({ value })}
					<div class="flex items-center gap-2">
						<span class="h-2.5 w-2.5 shrink-0 rounded-[2px] bg-primary"></span>
						<span class="font-medium tabular-nums text-foreground">
							{listened(Number(value), tag)}
						</span>
					</div>
				{/snippet}
			</Chart.Tooltip>
		{/snippet}
	</BarChart>
</Chart.Container>
