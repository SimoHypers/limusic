<script lang="ts">
	// "When you listen": a week of hours, one cell per weekday and hour, shaded by how much was heard
	// then. Rows run Monday to Sunday; the hover says the exact time and amount. Cells are squares as
	// wide as the column allows, so the grid always reaches the window's edge; a fixed height made
	// them flat bars on a big screen. The gaps and corners grow with them. Plain CSS grid, no chart
	// library: 168 cells need nothing it offers. No transition on the cells, they are a repeated
	// element under the pointer (docs/UI-PERFORMANCE.md).
	import { heat, listened, tagOf } from '$lib/stats';
	import { currentLocale, t } from '$lib/i18n.svelte';

	let { hours }: { hours: number[] } = $props();

	const tag = $derived(tagOf(currentLocale.id));
	const max = $derived(Math.max(0, ...hours));
	const shortDay = $derived(new Intl.DateTimeFormat(tag, { weekday: 'short' }));
	const longDay = $derived(new Intl.DateTimeFormat(tag, { weekday: 'long' }));
	const hourFmt = $derived(new Intl.DateTimeFormat(tag, { hour: 'numeric' }));
	// 4 January 2026 was a Sunday, so weekday `d` (0 = Sunday) is the 4th plus `d`.
	const day = (d: number, h = 0) => new Date(2026, 0, 4 + d, h);
	const ROWS = [1, 2, 3, 4, 5, 6, 0];
	const SHADES = [
		'bg-foreground/[0.06]',
		'bg-primary/25',
		'bg-primary/50',
		'bg-primary/75',
		'bg-primary'
	];
</script>

<div class="grid grid-cols-[auto_repeat(24,minmax(0,1fr))] items-center gap-1 xl:gap-1.5">
	{#each ROWS as d (d)}
		<span class="pr-2 text-xs text-muted-foreground">{shortDay.format(day(d))}</span>
		{#each { length: 24 } as _, h (h)}
			{@const secs = hours[d * 24 + h] ?? 0}
			<div
				class="aspect-square rounded-[3px] xl:rounded-[5px] {SHADES[heat(secs, max)]}"
				title="{longDay.format(day(d))}, {hourFmt.format(day(d, h))} · {listened(secs, tag)}"
			></div>
		{/each}
	{/each}
	<span></span>
	{#each [0, 6, 12, 18] as h (h)}
		<span class="col-span-6 pt-1 text-xs text-muted-foreground">{hourFmt.format(day(0, h))}</span>
	{/each}
</div>
<div class="mt-3 flex items-center justify-end gap-1.5 text-xs text-muted-foreground">
	{t('stats.less')}
	{#each SHADES as shade (shade)}
		<span class="h-3 w-3 rounded-[3px] {shade}"></span>
	{/each}
	{t('stats.more')}
</div>
