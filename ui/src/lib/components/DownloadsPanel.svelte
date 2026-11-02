<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Cancel01Icon,
		Download04Icon,
		Folder01Icon,
		Delete02Icon,
		CheckmarkCircle01Icon,
		Alert02Icon,
		MusicNote01Icon,
		Loading03Icon,
		StopCircleIcon
	} from '@hugeicons/core-free-icons';
	import { downloadQueue, cancelTask, cancelAll, clearCompleted } from '$lib/download.svelte';
	import * as api from '$lib/api';
	import { t } from '$lib/i18n.svelte';

	function formatBytes(bytes: number) {
		if (bytes === 0) return '0 B';
		const k = 1024;
		const sizes = ['B', 'KB', 'MB', 'GB'];
		const i = Math.floor(Math.log(bytes) / Math.log(k));
		return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
	}
</script>

{#if downloadQueue.isOpen}
	<!-- Backdrop to close on click outside -->
	<div
		class="fixed inset-0 z-50 bg-black/40 backdrop-blur-[2px]"
		onclick={() => (downloadQueue.isOpen = false)}
		role="presentation"
	></div>

	<!-- Downloads Modal / Panel -->
	<div
		class="fixed right-4 top-12 z-50 flex h-[480px] w-96 flex-col rounded-xl border border-border/80 bg-background/95 p-4 shadow-2xl backdrop-blur-md animate-in fade-in zoom-in-95 duration-150"
	>
		<!-- Header -->
		<div class="flex items-center justify-between border-b border-border/60 pb-3">
			<div class="flex items-center gap-2">
				<HugeiconsIcon icon={Download04Icon} class="h-5 w-5 text-primary" />
				<h2 class="text-base font-semibold text-foreground">{t('download.title')}</h2>
			</div>
			<div class="flex items-center gap-1">
				<button
					class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition hover:bg-accent/10 hover:text-foreground"
					onclick={() => api.openDownloadDir()}
					title={t('download.open_folder')}
					aria-label={t('download.open_folder')}
				>
					<HugeiconsIcon icon={Folder01Icon} class="h-4 w-4" />
				</button>
				{#if downloadQueue.activeCount > 0}
					<button
						class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition hover:bg-destructive/20 hover:text-destructive"
						onclick={cancelAll}
						title={t('download.cancel_all')}
						aria-label={t('download.cancel_all')}
					>
						<HugeiconsIcon icon={StopCircleIcon} class="h-4 w-4" />
					</button>
				{/if}
				<button
					class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition hover:bg-accent/10 hover:text-foreground"
					onclick={clearCompleted}
					title={t('download.clear_completed')}
					aria-label={t('download.clear_completed')}
				>
					<HugeiconsIcon icon={Delete02Icon} class="h-4 w-4" />
				</button>
				<button
					class="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition hover:bg-accent/10 hover:text-foreground"
					onclick={() => (downloadQueue.isOpen = false)}
					title={t('common.close')}
					aria-label={t('common.close')}
				>
					<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
				</button>
			</div>
		</div>

		<!-- Task List -->
		<div class="flex-1 overflow-y-auto py-3 space-y-2.5">
			{#if downloadQueue.tasks.filter((t) => t.status !== 'cancelled').length === 0}
				<div class="flex h-full flex-col items-center justify-center gap-2 text-muted-foreground">
					<HugeiconsIcon icon={Download04Icon} class="h-8 w-8 opacity-40" />
					<p class="text-sm">{t('download.empty')}</p>
				</div>
			{:else}
				{#each [...downloadQueue.tasks].filter((t) => t.status !== 'cancelled').reverse() as task (task.id)}
					<div
						class="flex flex-col gap-1.5 rounded-lg border border-border/40 bg-accent/5 p-2.5 transition hover:bg-accent/10"
					>
						<div class="flex items-center gap-2.5">
							{#if task.thumbnail}
								<img
									src={task.thumbnail}
									alt=""
									class="h-10 w-10 shrink-0 rounded-md object-cover shadow-sm"
								/>
							{:else}
								<div
									class="flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground"
								>
									<HugeiconsIcon icon={MusicNote01Icon} class="h-5 w-5" />
								</div>
							{/if}

							<div class="min-w-0 flex-1">
								<p class="truncate text-sm font-medium text-foreground" title={task.title}>
									{task.title}
								</p>
								<p class="truncate text-xs text-muted-foreground" title={task.artist}>
									{task.artist}
								</p>
							</div>

							<div class="shrink-0">
								{#if task.status === 'completed'}
									<span class="text-emerald-500" title={t('download.status_completed')}>
										<HugeiconsIcon icon={CheckmarkCircle01Icon} class="h-4 w-4" />
									</span>
								{:else if task.status === 'failed'}
									<span class="text-destructive" title={task.error || t('download.status_failed')}>
										<HugeiconsIcon icon={Alert02Icon} class="h-4 w-4" />
									</span>
								{:else if task.status === 'downloading' || task.status === 'queued'}
									<button
										class="flex h-6 w-6 items-center justify-center rounded text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
										onclick={() => cancelTask(task.id)}
										title={t('common.cancel')}
										aria-label={t('common.cancel')}
									>
										<HugeiconsIcon icon={Cancel01Icon} class="h-3.5 w-3.5" />
									</button>
								{/if}
							</div>
						</div>

						<!-- Progress or Status info -->
						{#if task.status === 'downloading'}
							{#if task.bytes_downloaded === 0 && task.total_bytes === 0}
								<div class="flex items-center gap-2 py-0.5 text-xs text-muted-foreground">
									<HugeiconsIcon icon={Loading03Icon} class="h-3.5 w-3.5 animate-spin text-primary" />
									<span>{t('download.initializing')}</span>
								</div>
							{:else}
								<div class="space-y-1">
									<div class="h-1.5 w-full overflow-hidden rounded-full bg-border/40">
										<div
											class="h-full bg-primary transition-all duration-200"
											style="width: {Math.max(5, Math.min(100, task.progress))}%"
										></div>
									</div>
									<div class="flex justify-between text-[10px] text-muted-foreground">
										<span>{Math.round(task.progress)}%</span>
										{#if task.total_bytes > 0}
											<span>{formatBytes(task.bytes_downloaded)} / {formatBytes(task.total_bytes)}</span>
										{/if}
									</div>
								</div>
							{/if}
						{:else if task.status === 'tagging'}
							<div class="flex items-center gap-1.5 text-xs text-amber-500">
								<span class="h-1.5 w-1.5 animate-ping rounded-full bg-amber-500"></span>
								<span>{t('download.status_tagging')}</span>
							</div>
						{:else if task.status === 'queued'}
							<div class="flex items-center gap-1.5 text-xs text-muted-foreground">
								<HugeiconsIcon icon={Loading03Icon} class="h-3.5 w-3.5 animate-spin opacity-60" />
								<span>{t('download.status_queued')}</span>
							</div>
						{:else if task.status === 'failed'}
							<p class="truncate text-xs text-destructive" title={task.error}>
								{task.error || t('download.status_failed')}
							</p>
						{:else if task.status === 'cancelled'}
							<p class="text-xs text-muted-foreground">{t('download.status_cancelled')}</p>
						{/if}
					</div>
				{/each}
			{/if}
		</div>
	</div>
{/if}
