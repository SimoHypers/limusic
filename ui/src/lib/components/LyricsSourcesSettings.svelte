<script lang="ts">
	// The lyrics provider order (#23, #326): drag a source up to ask it sooner, switch it off to never
	// ask it. Saved as it changes, like Edit Home. The order is Rust's (`lyrics_providers`); a
	// reorder drops the cached lyrics it would have changed, sparing songs whose source was picked
	// by hand.
	import { tick } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowTurnBackwardIcon, DragDropVerticalIcon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Switch } from '$lib/components/ui/switch';
	import { LYRICS_SOURCE_MIME } from '$lib/dnd';
	import * as api from '$lib/api';
	import { t } from '$lib/i18n.svelte';
	import { KIND_CLASS, KIND_LABEL, SOURCE_KIND, aboutKey } from '$lib/lyricsSources';

	let { settings }: { settings: Record<string, string> } = $props();

	let rows = $state<api.LyricsProvider[]>([]);
	let dragging = $state<number | null>(null);
	let list = $state<HTMLElement | null>(null);
	/** Read out by screen readers after a keyboard move. */
	let announce = $state('');

	const customized = $derived(!!settings.lyrics_providers);
	const noneOn = $derived(rows.length > 0 && rows.every((r) => !r.on));
	/** Where each source sits among the ones that are on: the order they're actually asked in. */
	const rank = $derived.by(() => {
		let n = 0;
		return rows.map((r) => (r.on ? ++n : null));
	});

	const reload = async () => (rows = await api.lyricsProviders());
	reload();

	function commit() {
		settings.lyrics_providers = rows.map((r) => (r.on ? r.id : `-${r.id}`)).join(',');
		api.setSetting('lyrics_providers', settings.lyrics_providers);
	}

	async function reset() {
		settings.lyrics_providers = '';
		await api.setSetting('lyrics_providers', '');
		await reload();
	}

	/** Same live reorder as Edit Home: the row follows the pointer past the middle of its neighbour,
	 *  and the order is saved once, on dragend. */
	function moveTo(to: number) {
		if (dragging === null || dragging === to) return;
		const next = rows.slice();
		next.splice(to, 0, ...next.splice(dragging, 1));
		rows = next;
		dragging = to;
	}

	/** Arrow keys on a row's handle. Focus follows the row, so holding the key walks it along. */
	async function nudge(i: number, by: number) {
		const to = i + by;
		if (to < 0 || to >= rows.length) return;
		const next = rows.slice();
		next.splice(to, 0, ...next.splice(i, 1));
		rows = next;
		commit();
		const row = rows[to];
		announce = t('settings.playback.lyrics_moved_source', {
			name: row.name,
			position: to + 1,
			total: rows.length
		});
		await tick();
		list?.querySelector<HTMLElement>(`[data-grip="${row.id}"]`)?.focus();
	}

	function setOn(row: api.LyricsProvider, on: boolean) {
		row.on = on;
		commit();
	}
</script>

<div data-settings-card class="overflow-hidden rounded-xl border bg-card">
	<div class="flex items-start justify-between gap-6 px-4 py-3.5">
		<div class="min-w-0">
			<span class="text-sm font-medium">{t('settings.playback.lyrics_sources')}</span>
			<p class="mt-1 max-w-prose text-xs leading-relaxed text-muted-foreground">
				{t('settings.playback.lyrics_sources_hint')}
			</p>
		</div>
		<Button
			variant="ghost"
			size="sm"
			class="shrink-0 text-muted-foreground"
			disabled={!customized}
			onclick={reset}
		>
			<HugeiconsIcon icon={ArrowTurnBackwardIcon} size={15} strokeWidth={2} />
			{t('settings.playback.lyrics_sources_reset')}
		</Button>
	</div>

	<div bind:this={list} role="list" class="divide-y divide-border/60 border-t border-border/60">
		{#each rows as row, i (row.id)}
			{@const kind = SOURCE_KIND[row.id] ?? 'synced'}
			<!-- The whole row drags, the grip says where to grab it. No `animate:flip`, for the reason
			     HomeLayoutDialog gives: a moving hit box makes the list ping-pong under the pointer. -->
			<div
				role="listitem"
				draggable="true"
				ondragstart={(e) => {
					dragging = i;
					if (!e.dataTransfer) return;
					e.dataTransfer.setData(LYRICS_SOURCE_MIME, row.id);
					e.dataTransfer.effectAllowed = 'move';
				}}
				ondragover={(e) => {
					if (dragging === null || !e.dataTransfer?.types.includes(LYRICS_SOURCE_MIME)) return;
					e.preventDefault();
					e.dataTransfer.dropEffect = 'move';
					const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
					const past = e.clientY > r.top + r.height / 2;
					if (i > dragging ? past : !past) moveTo(i);
				}}
				ondragend={() => {
					dragging = null;
					commit();
				}}
				class="flex cursor-grab items-center gap-2 py-2.5 pl-1.5 pr-4 transition-colors {dragging === i
					? 'bg-muted opacity-60'
					: 'hover:bg-muted/40'}"
			>
				<button
					data-grip={row.id}
					aria-label={t('settings.playback.lyrics_move_source', { name: row.name })}
					onkeydown={(e) => {
						if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return;
						e.preventDefault();
						nudge(i, e.key === 'ArrowUp' ? -1 : 1);
					}}
					class="flex h-8 w-6 shrink-0 cursor-grab items-center justify-center rounded-md text-muted-foreground/50 outline-none transition-colors hover:text-foreground focus-visible:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
				>
					<HugeiconsIcon icon={DragDropVerticalIcon} class="h-4 w-4" />
				</button>
				<span
					class="flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-[11px] font-semibold tabular-nums transition-colors {row.on
						? 'bg-primary/12 text-primary'
						: 'bg-muted text-muted-foreground/50'}"
					aria-hidden="true"
				>
					{rank[i] ?? '–'}
				</span>
				<div class="min-w-0 flex-1 pl-1">
					<div class="flex items-center gap-2">
						<span class="truncate text-sm font-medium {row.on ? '' : 'text-muted-foreground'}">
							{row.name}
						</span>
						<span
							class="shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide {row.on
								? KIND_CLASS[kind]
								: 'bg-muted text-muted-foreground/60'}"
						>
							{t(KIND_LABEL[kind])}
						</span>
					</div>
					<p class="mt-0.5 text-xs leading-snug text-muted-foreground {row.on ? '' : 'opacity-60'}">
						{t(aboutKey(row.id))}
					</p>
				</div>
				<Switch
					size="sm"
					checked={row.on}
					onCheckedChange={(on) => setOn(row, on)}
					aria-label={t('settings.playback.lyrics_use_source', { name: row.name })}
				/>
			</div>
		{/each}
	</div>

	{#if noneOn}
		<p class="border-t border-border/60 bg-destructive/8 px-4 py-2.5 text-xs text-destructive">
			{t('settings.playback.lyrics_sources_none')}
		</p>
	{/if}
	<div class="sr-only" aria-live="polite">{announce}</div>
</div>
