<!--
	The column header over a playlist or album track list, and the picker that chooses its columns.

	The choice is one global set, like Spotify's: picked once, it holds on every list. Each list then
	shows the part of it that it can fill (`availableColumns`), so turning on Album doesn't leave an
	empty column on every album page. The picker lists only those too, rather than offering a column
	that would stay blank here.

	Sticky with an opaque background, so the labels stay over the rows on a long list. It lays out
	exactly as a row does (`TITLE`, `CELL`, `END` in `trackcolumns.ts`), which is what keeps every
	label over its cells.
-->
<script module lang="ts">
	import * as api from '$lib/api';
	import { parseColumns, type TrackColumn } from '$lib/trackcolumns';

	const KEY = 'track_columns';
	function stored(): string | null {
		try {
			return localStorage.getItem(KEY);
		} catch {
			return null; // a blocked store reads as nothing chosen yet: the default
		}
	}
	export const chosen = $state({ list: parseColumns(stored()) });

	function toggle(c: TrackColumn) {
		chosen.list = chosen.list.includes(c) ? chosen.list.filter((x) => x !== c) : [...chosen.list, c];
		try {
			localStorage.setItem(KEY, JSON.stringify(chosen.list));
		} catch {
			/* a full or disabled store only means the choice lasts until restart */
		}
	}

	/** The local listening history (videoId → plays, all time), for Your plays and "Most played".
	 *  Raw: the map runs to thousands of ids and is only ever replaced whole. */
	class Plays {
		counts = $state.raw<Record<string, number>>({});
	}
	export const myPlays = new Plays();
	let inflight: Promise<void> | null = null;
	/** Re-read on each call (it changes as you listen); concurrent calls share one read. */
	export function loadMyPlays(): Promise<void> {
		inflight ??= api
			.getPlayCounts()
			.then((c) => void (myPlays.counts = c))
			// An empty map reads as nothing played, which beats blocking the sort or the column.
			.catch(() => {})
			.finally(() => (inflight = null));
		return inflight;
	}
</script>

<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowDown01Icon, Clock01Icon } from '@hugeicons/core-free-icons';
	import { CELL, END, TITLE } from '$lib/trackcolumns';
	import { anchorMenu, fitMenu, NO_ANCHOR, toBody } from '$lib/menu';
	import { t } from '$lib/i18n.svelte';
	import { Checkbox } from './ui/checkbox';

	let {
		available,
		columns,
		selecting = false
	}: {
		/** What this list can fill: the picker's options. */
		available: TrackColumn[];
		/** What it shows: the chosen ones among `available`. The rows get the same list. */
		columns: TrackColumn[];
		/** The rows carry a checkbox in front (select mode), so the labels step right with them. */
		selecting?: boolean;
	} = $props();

	let open = $state(false);
	let anchor = $state(NO_ANCHOR);

	// Your plays reads the history when it comes on and when a track list opens with it on. A
	// boolean, not `columns`: that array is new with every page of a long playlist that lands.
	const mine = $derived(columns.includes('mine'));
	$effect(() => {
		if (mine) loadMyPlays();
	});
</script>

<svelte:window onkeydown={(e) => open && e.key === 'Escape' && (open = false)} />

<div
	class="sticky top-0 z-10 flex items-center gap-3 border-b bg-background px-2 py-1.5 text-xs font-medium text-muted-foreground"
>
	{#if selecting}
		<span class="w-4 shrink-0"></span>
	{/if}
	<div class="flex min-w-0 items-center gap-3 {TITLE}">
		<span class="w-5 shrink-0 text-center">#</span>
		<!-- Starts over the artwork, the way the title column reads in a table. -->
		<span class="min-w-0 truncate">{t('columns.title')}</span>
	</div>
	{#each columns as c (c)}
		{#if c !== 'duration'}
			<span class="truncate {CELL[c]}">{t(`columns.${c}`)}</span>
		{/if}
	{/each}
	<div class="flex shrink-0 items-center gap-2 {END}">
		{#if columns.includes('duration')}
			<!-- Over the duration, which the rows end on just before the ⋯, as the picker sits over it. -->
			<span class="flex min-w-[5ch] shrink-0 justify-end" title={t('columns.duration')}>
				<HugeiconsIcon icon={Clock01Icon} class="h-3.5 w-3.5" aria-label={t('columns.duration')} />
			</span>
		{/if}
		<button
			class="cursor-pointer rounded-md p-1.5 hover:bg-accent/20 hover:text-foreground {open
				? 'bg-accent/20 text-foreground'
				: ''}"
			aria-label={t('columns.choose')}
			title={t('columns.choose')}
			aria-expanded={open}
			onclick={(e) => {
				anchor = anchorMenu(e, { align: 'right' });
				open = true;
			}}
		>
			<HugeiconsIcon icon={ArrowDown01Icon} class="h-4 w-4" />
		</button>
	</div>
</div>

{#if open}
	<!-- Moved to <body>: the list is a size container, which `position: fixed` would otherwise
	     resolve against. Stays open while you tick, like any checklist; a click outside closes it. -->
	<button
		class="fixed inset-0 z-40 cursor-default"
		onclick={() => (open = false)}
		aria-label={t('a11y.close_menu')}
		{@attach toBody}
	></button>
	<div
		class="fixed z-50 min-w-48 animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style={anchor.style}
		{@attach toBody}
		{@attach fitMenu(anchor)}
	>
		<div class="px-2 pb-1 pt-1.5 text-xs font-medium text-muted-foreground">
			{t('columns.label')}
		</div>
		{#each available as c (c)}
			<label
				class="flex w-full cursor-pointer items-center justify-between gap-6 rounded-md px-2 py-1.5 text-sm hover:bg-accent/10"
			>
				{t(`columns.${c}`)}
				<Checkbox
					checked={chosen.list.includes(c)}
					onCheckedChange={() => toggle(c)}
					class="cursor-pointer"
				/>
			</label>
		{/each}
	</div>
{/if}
