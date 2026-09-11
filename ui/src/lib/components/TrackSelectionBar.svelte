<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowUpNarrowWideIcon, PlayListAddIcon } from '@hugeicons/core-free-icons';
	import { Button } from './ui/button';
	import { enqueue, openAddManyToPlaylist, ui } from '$lib/player.svelte';
	import { isLocalId } from '$lib/api';
	import { t } from '$lib/i18n.svelte';
	import type { TrackSelection } from '$lib/selection.svelte';

	let { selection, from }: { selection: TrackSelection; from?: string } = $props();
	let busy = $state(false);
	const canAdd = $derived(selection.count > 0 && selection.songs.every((s) => !isLocalId(s.video_id)));

	function onKey(e: KeyboardEvent) {
		// Space activates these buttons, never the app-wide transport shortcut.
		if (e.key === ' ') e.stopPropagation();
		if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); selection.clear(); }
	}

	async function queue(next: boolean) {
		if (busy || selection.pending || !selection.count) return;
		busy = true;
		try {
			// Snapshot before awaiting. Never append the source's continuation: only selected rows.
			await enqueue([...selection.songs], next, from);
		} finally {
			busy = false;
		}
	}
</script>

<div class="sticky top-0 z-10 mb-2 rounded-lg border bg-background p-2" data-track-selection>
	<div class="flex flex-wrap items-center gap-2" role="group" aria-label={t('selection.actions')}>
		<span class="mr-auto text-sm text-muted-foreground" role="status">
			{selection.count ? t('selection.count', { count: selection.count }) : t('selection.hint')}
			{#if selection.hidden}
				<span> · {t('selection.hidden', { count: selection.hidden })}</span>
			{/if}
		</span>
		<Button variant="ghost" size="sm" disabled={!selection.visibleKeys.length}
			onkeydown={onKey}
			onclick={() => selection.selectAll()}>
			{t('selection.select_all', { count: selection.visibleKeys.length })}
		</Button>
		{#if selection.count}
			<Button variant="outline" size="sm" disabled={busy || selection.pending > 0} onkeydown={onKey} onclick={() => queue(true)}>
				<HugeiconsIcon icon={ArrowUpNarrowWideIcon} class="h-4 w-4" /> {t('player.play_next')}
			</Button>
			<Button variant="outline" size="sm" disabled={busy || selection.pending > 0} onkeydown={onKey} onclick={() => queue(false)}>
				<HugeiconsIcon icon={PlayListAddIcon} class="h-4 w-4" /> {t('player.add_to_queue')}
			</Button>
			{#if canAdd}
				<Button variant="outline" size="sm" disabled={busy || ui.addPending || selection.pending > 0}
					onkeydown={onKey}
					onclick={() => openAddManyToPlaylist([...selection.songs])}>
					{t('player.add_to_playlist')}
				</Button>
			{/if}
		{/if}
		{#if selection.count || selection.lost}
			<Button variant="ghost" size="sm" onkeydown={onKey} onclick={() => selection.clear()}>{t('selection.clear')}</Button>
		{/if}
	</div>
	{#if selection.pending}
		<p class="mt-1 text-xs text-muted-foreground" role="status">
			{t('selection.pending', { count: selection.pending })}
		</p>
	{/if}
	{#if selection.lost}
		<p class="mt-1 text-xs text-muted-foreground" role="status">
			{t('selection.lost', { count: selection.lost })}
		</p>
	{/if}
</div>
