<script lang="ts">
	// One row of the Downloads page. The controls are the backend actions, exactly: pause/resume,
	// cancel, retry, and remove (which deletes the saved file, so it confirms first).
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Cancel01Icon,
		Delete02Icon,
		MusicNote01Icon,
		PauseIcon,
		PlayIcon,
		RefreshIcon
	} from '@hugeicons/core-free-icons';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import { actOnDownload, dlBusy } from '$lib/downloads.svelte';
	import {
		DOWNLOAD_STATE_KEYS,
		formatBytes,
		progressFraction,
		type DownloadItem
	} from '$lib/downloads';

	let {
		item,
		/** Plays the finished file; the page owns the queue it plays out of. */
		onplay
	}: {
		item: DownloadItem;
		onplay?: (item: DownloadItem) => void;
	} = $props();

	const busy = $derived(!!dlBusy[item.id]);
	const frac = $derived(progressFraction(item));
	// The bar belongs to the two states that had bytes moving; a queued row has nothing to show yet,
	// and a failed one says so instead.
	const showBar = $derived(item.state === 'downloading' || item.state === 'paused');
	const active = $derived(
		item.state === 'downloading' || item.state === 'queued' || item.state === 'paused'
	);
	// Error and cancelled are the retryable ends; done holds the file, so it can be removed.
	const removable = $derived(
		item.state === 'done' || item.state === 'error' || item.state === 'cancelled'
	);
	const art = $derived(thumb(item.thumbnail, 96));
	const sub = $derived([item.artists, item.album].filter(Boolean).join(' • '));

	let confirmRemove = $state(false);
	async function remove() {
		if (await actOnDownload(item.id, 'remove')) confirmRemove = false;
	}
</script>

<div class="group flex items-center gap-3 rounded-lg p-2 hover:bg-accent/10" data-download-id={item.id}>
	{#if art}
		<img src={art} alt="" class="h-10 w-10 shrink-0 rounded-md object-cover" loading="lazy" />
	{:else}
		<div
			class="flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground/50"
		>
			<HugeiconsIcon icon={MusicNote01Icon} class="h-4 w-4" />
		</div>
	{/if}
	<div class="min-w-0 flex-1">
		<div class="flex min-w-0 items-center gap-2">
			<span class="min-w-0 truncate text-sm font-medium">{item.title}</span>
			<!-- State chip. Downloading wears a live dot: the row is the only place the queue's
			     work is visible without opening it. -->
			<span
				class="flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium {item.state ===
				'done'
					? 'bg-primary/10 text-primary'
					: item.state === 'error'
						? 'bg-destructive/10 text-destructive'
						: item.state === 'downloading'
							? 'bg-primary/10 text-primary'
							: 'bg-muted text-muted-foreground'}"
			>
				{#if item.state === 'downloading'}
					<span class="h-1.5 w-1.5 animate-pulse rounded-full bg-primary"></span>
				{/if}
				{t(DOWNLOAD_STATE_KEYS[item.state])}
			</span>
		</div>
		{#if sub}
			<div class="truncate text-xs text-muted-foreground">{sub}</div>
		{/if}
		{#if showBar}
			<div class="mt-1.5 flex max-w-md items-center gap-2">
				{#if frac !== null}
					<span class="h-1 flex-1 overflow-hidden rounded-full bg-primary/15">
						<span
							class="block h-full rounded-full bg-primary transition-[width] duration-300"
							style="width:{Math.round(frac * 100)}%"
						></span>
					</span>
				{/if}
				<span class="shrink-0 text-[11px] tabular-nums text-muted-foreground">
					{item.bytes_total != null
						? t('downloads.progress_of', {
								done: formatBytes(item.bytes_done),
								total: formatBytes(item.bytes_total)
							})
						: t('downloads.progress_done', { done: formatBytes(item.bytes_done) })}
				</span>
			</div>
		{/if}
		{#if item.state === 'error' && item.error}
			<p class="mt-0.5 truncate text-xs text-destructive" title={item.error}>{item.error}</p>
		{/if}
	</div>

	{#if item.quality_label && item.state !== 'cancelled'}
		<span
			class="hidden shrink-0 rounded-full bg-muted px-2 py-0.5 text-[10px] font-medium text-muted-foreground sm:inline"
		>
			{item.quality_label}
		</span>
	{/if}

	<div class="flex shrink-0 items-center gap-0.5">
		{#if item.state === 'done' && onplay}
			<button
				class="cursor-pointer rounded-md p-1.5 text-muted-foreground hover:bg-accent/20 hover:text-foreground disabled:opacity-50"
				title={t('downloads.action_play')}
				aria-label={t('downloads.action_play')}
				onclick={() => onplay(item)}
			>
				<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" />
			</button>
		{/if}
		{#if item.state === 'downloading'}
			<button
				class="cursor-pointer rounded-md p-1.5 text-muted-foreground hover:bg-accent/20 hover:text-foreground disabled:opacity-50"
				title={t('downloads.action_pause')}
				aria-label={t('downloads.action_pause')}
				disabled={busy}
				onclick={() => actOnDownload(item.id, 'pause')}
			>
				<HugeiconsIcon icon={PauseIcon} class="h-4 w-4" />
			</button>
		{:else if item.state === 'paused'}
			<button
				class="cursor-pointer rounded-md p-1.5 text-muted-foreground hover:bg-accent/20 hover:text-foreground disabled:opacity-50"
				title={t('downloads.action_resume')}
				aria-label={t('downloads.action_resume')}
				disabled={busy}
				onclick={() => actOnDownload(item.id, 'resume')}
			>
				<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" />
			</button>
		{:else if item.state === 'error' || item.state === 'cancelled'}
			<button
				class="cursor-pointer rounded-md p-1.5 text-muted-foreground hover:bg-accent/20 hover:text-foreground disabled:opacity-50"
				title={t('downloads.action_retry')}
				aria-label={t('downloads.action_retry')}
				disabled={busy}
				onclick={() => actOnDownload(item.id, 'retry')}
			>
				<HugeiconsIcon icon={RefreshIcon} class="h-4 w-4" />
			</button>
		{/if}
		{#if active}
			<button
				class="cursor-pointer rounded-md p-1.5 text-muted-foreground hover:bg-accent/20 hover:text-foreground disabled:opacity-50"
				title={t('downloads.action_cancel')}
				aria-label={t('downloads.action_cancel')}
				disabled={busy}
				onclick={() => actOnDownload(item.id, 'cancel')}
			>
				<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
			</button>
		{/if}
		{#if removable}
			<button
				class="cursor-pointer rounded-md p-1.5 text-muted-foreground hover:bg-destructive/10 hover:text-destructive disabled:opacity-50"
				title={t('downloads.action_remove')}
				aria-label={t('downloads.action_remove')}
				disabled={busy}
				onclick={() => (confirmRemove = true)}
			>
				<HugeiconsIcon icon={Delete02Icon} class="h-4 w-4" />
			</button>
		{/if}
	</div>
</div>

<!-- Removing a download deletes the file from disk: the same two-step the playlist delete uses.
     bits-ui's Action is a plain button (only Cancel closes), so the dialog stays up until the
     backend has answered. -->
<AlertDialog.Root bind:open={confirmRemove}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('downloads.remove_confirm_title')}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('downloads.remove_confirm_desc', { title: item.title })}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('common.cancel')}</AlertDialog.Cancel>
			<AlertDialog.Action variant="destructive" disabled={busy} onclick={remove}>
				{t('downloads.action_remove')}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>