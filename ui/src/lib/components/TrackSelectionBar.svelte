<script lang="ts">
	import { fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		ArrowUpNarrowWideIcon,
		ArrowDownWideNarrowIcon,
		PlayListAddIcon,
		Cancel01Icon
	} from '@hugeicons/core-free-icons';
	import { Button } from './ui/button';
	import { enqueue, openAddManyToPlaylist, ui } from '$lib/player.svelte';
	import { isLocalId } from '$lib/api';
	import { t } from '$lib/i18n.svelte';
	import type { TrackSelection } from '$lib/selection.svelte';

	let { selection, from }: { selection: TrackSelection; from?: string } = $props();
	let busy = $state(false);
	const canAdd = $derived(selection.count > 0 && selection.songs.every((s) => !isLocalId(s.video_id)));
	const blocked = $derived(busy || selection.pending > 0);
	// One button, not a Select all beside a Clear: at a full list the only thing left to do is empty it.
	const allSelected = $derived(
		selection.visibleKeys.length > 0 && selection.visibleKeys.every((k) => selection.has(k))
	);

	function onKey(e: KeyboardEvent) {
		// Space activates these buttons, never the app-wide transport shortcut.
		if (e.key === ' ') e.stopPropagation();
		if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); selection.exit(); }
	}

	async function queue(next: boolean) {
		if (blocked || !selection.count) return;
		busy = true;
		try {
			// Snapshot before awaiting. Never append the source's continuation: only selected rows.
			await enqueue([...selection.songs], next, from);
		} finally {
			busy = false;
		}
	}
</script>

<!-- Floating, like the update banner and the toast above it, rather than a strip wedged between the
     header and row 1: the actions belong to the selection, not to a slot in the page, and a list
     that reflows the moment you tick a box is the thing you were aiming at moving. bottom-24 clears
     the player bar. Only in select mode; the header button is what turns that on. -->
{#if selection.active}
	<div
		transition:fly={{ y: 16, duration: 220, easing: cubicOut }}
		class="fixed bottom-24 left-1/2 z-40 flex max-w-[min(44rem,calc(100vw-2rem))] -translate-x-1/2 flex-col gap-1 rounded-2xl border bg-card px-2 py-2 shadow-lg"
		data-track-selection
	>
		<div class="flex flex-wrap items-center justify-center gap-1" role="group" aria-label={t('selection.actions')}>
			<span class="px-2 text-sm whitespace-nowrap" role="status">
				{#if selection.count}
					<span class="font-medium">{t('selection.count', { count: selection.count })}</span>
					{#if selection.hidden}
						<span class="text-muted-foreground"> · {t('selection.hidden', { count: selection.hidden })}</span>
					{/if}
				{:else}
					<span class="text-muted-foreground">{t('selection.hint')}</span>
				{/if}
			</span>

			{#if selection.count}
				<span class="mx-1 h-5 w-px shrink-0 bg-border"></span>
				<Button variant="ghost" size="icon" disabled={blocked} onkeydown={onKey}
					title={t('player.play_next')} aria-label={t('player.play_next')}
					onclick={() => queue(true)}>
					<HugeiconsIcon icon={ArrowUpNarrowWideIcon} class="h-4 w-4" />
				</Button>
				<Button variant="ghost" size="icon" disabled={blocked} onkeydown={onKey}
					title={t('player.add_to_queue')} aria-label={t('player.add_to_queue')}
					onclick={() => queue(false)}>
					<HugeiconsIcon icon={ArrowDownWideNarrowIcon} class="h-4 w-4" />
				</Button>
				{#if canAdd}
					<Button variant="ghost" size="icon" disabled={blocked || ui.addPending} onkeydown={onKey}
						title={t('player.add_to_playlist')} aria-label={t('player.add_to_playlist')}
						onclick={() => openAddManyToPlaylist([...selection.songs])}>
						<HugeiconsIcon icon={PlayListAddIcon} class="h-4 w-4" />
					</Button>
				{/if}
			{/if}

			<span class="mx-1 h-5 w-px shrink-0 bg-border"></span>
			{#if allSelected}
				<Button variant="ghost" size="sm" onkeydown={onKey} onclick={() => selection.clear()}>
					{t('selection.clear')}
				</Button>
			{:else}
				<Button variant="ghost" size="sm" disabled={!selection.visibleKeys.length} onkeydown={onKey}
					onclick={() => selection.selectAll()}>
					{t('selection.select_all', { count: selection.visibleKeys.length })}
				</Button>
			{/if}
			<Button variant="ghost" size="icon" onkeydown={onKey}
				title={t('selection.exit')} aria-label={t('selection.exit')}
				onclick={() => selection.exit()}>
				<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
			</Button>
		</div>

		{#if selection.pending || selection.lost}
			<p class="px-2 text-xs text-muted-foreground" role="status">
				{selection.pending ? t('selection.pending', { count: selection.pending }) : ''}
				{selection.lost ? t('selection.lost', { count: selection.lost }) : ''}
			</p>
		{/if}
	</div>
{/if}
