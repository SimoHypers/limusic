<script lang="ts">
	// The Downloads manager: what the backend is fetching, and the finished library to play
	// offline. The list is the shared store (hydrated from `downloads_list()`, kept live by
	// `downloads-changed` / `download-progress`), so a row paused here changes everywhere.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Delete02Icon,
		Download01Icon,
		HardDriveIcon,
		PlayIcon,
		RefreshIcon
	} from '@hugeicons/core-free-icons';
	import { goto } from '$app/navigation';
	import { Button } from '$lib/components/ui/button';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import DownloadRow from '$lib/components/DownloadRow.svelte';
	import * as api from '$lib/api';
	import {
		downloads,
		refreshDownloads,
		removeDownloads,
		retryDownloads
	} from '$lib/downloads.svelte';
	import {
		downloadItemToSong,
		formatBytes,
		retryableDownloads,
		splitDownloads,
		storedBytes,
		summarize,
		type DownloadItem
	} from '$lib/downloads';
	import { openPlayer } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import { thumb } from '$lib/thumb';

	const sum = $derived(summarize(downloads.items));
	const parts = $derived(splitDownloads(downloads.items));
	// Covers for the header stack, from the finished files; same idea as the History page, because
	// the page has no artwork of its own.
	const covers = $derived([
		...new Set(parts.done.slice(0, 40).flatMap((d) => (d.thumbnail ? [d.thumbnail] : [])))
	]);
	let artFailed = $state(false);
	$effect(() => {
		covers[0]; // re-arm when the artwork changes
		artFailed = false;
	});

	const doneSongs = $derived(parts.done.map(downloadItemToSong));
	const subtitle = $derived(
		sum.total === 0
			? t('downloads.subtitle')
			: sum.unfinished > 0
				? t('downloads.summary', { active: sum.unfinished, done: sum.done })
				: t('downloads.downloaded_count', { count: sum.done })
	);
	// The header's two controls over the whole list: how much room the feature is taking, and the
	// one button that puts every stopped row back in flight (per-row retry stays on the rows).
	const stored = $derived(storedBytes(downloads.items));
	const retryable = $derived(retryableDownloads(downloads.items));
	/** One flag for the bulk calls: the rows are individually flagged too, this covers the buttons. */
	let bulkBusy = $state(false);

	async function retryFailed() {
		if (bulkBusy) return;
		bulkBusy = true;
		try {
			await retryDownloads(retryable.map((d) => d.id));
		} finally {
			bulkBusy = false;
		}
	}

	// Removing every finished download deletes every finished file, so it gets the same two-step
	// the single row uses, with the count spelled out before anything is touched.
	let confirmClear = $state(false);
	async function clearFinished() {
		if (bulkBusy) return;
		bulkBusy = true;
		try {
			if (await removeDownloads(parts.done.map((d) => d.id))) confirmClear = false;
		} finally {
			bulkBusy = false;
		}
	}

	// The finished library plays as one queue, in the order it is shown: a click resumes from that
	// row, "Play all" starts at the top.
	function play(row?: DownloadItem) {
		if (!doneSongs.length) return;
		const at = row ? parts.done.findIndex((d) => d.id === row.id) : 0;
		openPlayer();
		api.playPlaylist(doneSongs, at >= 0 ? at : 0, undefined, t('downloads.library_title'));
	}
</script>

<div class="p-6">
	<!-- The same rounded band the History page wears. -->
	<div class="relative mb-6 overflow-hidden rounded-2xl border">
		<div class="art-wash pointer-events-none absolute inset-0 overflow-hidden">
			{#if covers[0] && !artFailed}
				<img
					src={thumb(covers[0], 96)}
					alt=""
					class="absolute inset-0 h-full w-full scale-110 object-cover opacity-60 blur-2xl"
					onerror={() => (artFailed = true)}
				/>
			{/if}
			<div
				class="absolute inset-0 bg-gradient-to-r from-background via-background/80 to-background/40"
			></div>
		</div>
		<div class="relative flex flex-wrap items-center gap-4 p-4">
			{#if covers.length}
				<div class="flex shrink-0 items-center pl-1">
					{#each covers.slice(0, 5) as cover, i (cover)}
						<img
							src={thumb(cover, 400)}
							alt=""
							style="z-index:{5 - i}"
							class="relative -ml-5 h-20 w-20 rounded-xl object-cover shadow-lg ring-2 ring-background first:ml-0"
						/>
					{/each}
				</div>
			{:else}
				<div
					class="flex h-20 w-20 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary"
				>
					<HugeiconsIcon icon={Download01Icon} class="h-8 w-8" />
				</div>
			{/if}
			<div class="min-w-0 flex-1">
				<h1 class="font-heading text-2xl font-bold tracking-tight">{t('downloads.title')}</h1>
				<p class="mt-0.5 text-sm text-muted-foreground">{subtitle}</p>
				<div class="mt-3 flex flex-wrap items-center gap-2">
					<Button
						class="gap-2 rounded-full"
						disabled={!parts.done.length}
						onclick={() => play()}
					>
						<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" /> {t('downloads.play_all')}
					</Button>
					<!-- Bulk retry: only appears once something has actually stopped, and the count in
					     the label is the number of rows it will take. -->
					{#if retryable.length}
						<Button
							variant="outline"
							class="gap-2 rounded-full"
							disabled={bulkBusy}
							onclick={retryFailed}
						>
							<HugeiconsIcon icon={RefreshIcon} class="h-4 w-4" />
							{t('common.retry')} ({retryable.length})
						</Button>
					{/if}
					<!-- What the feature holds on disk, over every state: a finished file at full
					     size, a partial at however far it got. -->
					{#if downloads.items.length}
						<span
							data-storage
							class="inline-flex shrink-0 items-center gap-1.5 rounded-full border bg-muted/40 px-2.5 py-1 text-xs tabular-nums text-muted-foreground"
							title={t('downloads.storage_used', { bytes: formatBytes(stored) })}
						>
							<HugeiconsIcon icon={HardDriveIcon} class="h-3.5 w-3.5" />
							{formatBytes(stored)}
						</span>
					{/if}
				</div>
			</div>
		</div>
	</div>

	{#if !downloads.loaded}
		{#each Array(4) as _, i (i)}
			<Skeleton class="mb-1 h-14 w-full rounded-lg" />
		{/each}
	{:else if downloads.error && !downloads.items.length}
		<ErrorState message={downloads.error} onRetry={refreshDownloads} />
	{:else if !parts.queue.length && !parts.done.length}
		<div class="rounded-2xl border bg-card/40 p-6">
			<h2 class="font-heading text-lg font-bold tracking-tight">{t('downloads.empty_title')}</h2>
			<p class="mt-1 max-w-prose text-sm text-muted-foreground">{t('downloads.empty_hint')}</p>
			<Button variant="outline" class="mt-4 gap-2" onclick={() => goto('/library')}>
				{t('downloads.browse')}
			</Button>
		</div>
	{:else}
		{#if parts.queue.length}
			<section class="mb-6">
				<h2 class="mb-1 flex items-baseline gap-3 py-2">
					<span class="font-heading text-lg font-bold tracking-tight">{t('downloads.queue_title')}</span>
					<span class="h-px flex-1 bg-border"></span>
					<span class="text-xs text-muted-foreground">{parts.queue.length}</span>
				</h2>
				{#each parts.queue as item (item.id)}
					<DownloadRow {item} />
				{/each}
			</section>
		{/if}
		{#if parts.done.length}
			<section>
				<h2 class="mb-1 flex items-baseline gap-3 py-2">
					<span class="font-heading text-lg font-bold tracking-tight">{t('downloads.library_title')}</span>
					<span class="h-px flex-1 bg-border"></span>
					<span class="text-xs text-muted-foreground">{parts.done.length}</span>
					<!-- The finished library in one action, parked at the end of its own heading:
					     it deletes files, so it confirms first (the dialog is at the bottom). -->
					<button
						class="-my-1 cursor-pointer self-center rounded-md p-1.5 text-muted-foreground hover:bg-destructive/10 hover:text-destructive disabled:opacity-50"
						title={t('downloads.action_clear_finished')}
						aria-label={t('downloads.action_clear_finished')}
						disabled={bulkBusy || !parts.done.length}
						onclick={() => (confirmClear = true)}
					>
						<HugeiconsIcon icon={Delete02Icon} class="h-4 w-4" />
					</button>
				</h2>
				{#each parts.done as item (item.id)}
					<DownloadRow {item} onplay={play} />
				{/each}
			</section>
		{/if}
	{/if}
</div>

<!-- Bulk remove of the finished library: the copy says outright that files leave the machine and
     how many, and — like the single-row dialog — it stays open until the backend has answered. -->
<AlertDialog.Root bind:open={confirmClear}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('downloads.clear_confirm_title')}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('downloads.clear_confirm_desc', { count: parts.done.length })}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('common.cancel')}</AlertDialog.Cancel>
			<AlertDialog.Action variant="destructive" disabled={bulkBusy} onclick={clearFinished}>
				{t('downloads.action_remove')}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>