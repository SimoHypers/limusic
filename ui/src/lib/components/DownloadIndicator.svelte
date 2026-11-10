<script lang="ts">
	// Titlebar badge for background downloads. It only exists while there are jobs to talk about;
	// clicking it opens a small popover with each track, its byte-level progress, size and speed.
	// The popover is `fixed` and positioned at the button, same as the scrobbler menu next to it.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Alert02Icon,
		CheckmarkCircle02Icon,
		Download01Icon,
		Loading03Icon
	} from '@hugeicons/core-free-icons';
	import { anchorMenu, fitMenu, NO_ANCHOR } from '$lib/menu';
	import { activeJobs, clearFinished, downloads } from '$lib/downloads.svelte';
	import { formatBytes, formatSpeed } from '$lib/download';
	import { t } from '$lib/i18n.svelte';

	let menuOpen = $state(false);
	let anchor = $state(NO_ANCHOR);

	const active = $derived(activeJobs());
	const hasFinished = $derived(
		downloads.jobs.some((job) => job.state === 'done' || job.state === 'failed')
	);
	// Working jobs first, so the popover opens on what is happening; finished ones newest first.
	/** Active jobs first, then finished ones newest first. */
const shown = $derived([
		...downloads.jobs.filter((job) => job.state === 'downloading'),
		...downloads.jobs.filter((job) => job.state === 'queued'),
		...downloads.jobs
			.filter((job) => job.state === 'done' || job.state === 'failed')
			.reverse()
	]);
	/** A downloading job at 100% is in yt-dlp's postprocessing phase. */
const processing = (percent: number) => percent >= 100;

	/** Opens the popover at the button. */
function openMenu(e: MouseEvent) {
		e.preventDefault();
		e.stopPropagation();
		anchor = anchorMenu(e, { align: 'right' });
		menuOpen = true;
	}

	/** Closes the popover. */
function close() {
		menuOpen = false;
	}
</script>

{#if downloads.jobs.length}
	<button
		class="relative mx-1 flex h-full w-9 items-center justify-center rounded-md transition-colors hover:bg-accent/10
			{active.length ? 'bg-primary/10 text-primary' : 'text-muted-foreground'}"
		onclick={openMenu}
		aria-label={t('player.download_indicator')}
		title={t('player.download_indicator')}
		aria-expanded={menuOpen}
	>
		<HugeiconsIcon icon={Download01Icon} class="h-[18px] w-[18px]" />
		{#if active.length}
			<!-- Ringed and offset so the count sits clear of the arrow instead of on top of it. -->
			<span
				class="absolute right-0.5 top-0.5 flex h-3.5 min-w-3.5 items-center justify-center rounded-full bg-primary px-1 text-[10px] font-bold leading-none text-primary-foreground ring-2 ring-background"
			>
				{active.length}
			</span>
		{/if}
	</button>

	{#if menuOpen}
		<button
			class="fixed inset-0 z-40 cursor-default"
			onclick={close}
			aria-label={t('common.close')}
		></button>
		<div
			class="fixed z-50 w-96 animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
			style={anchor.style}
			{@attach fitMenu(anchor)}
		>
			<div class="flex items-center justify-between gap-3 px-2.5 py-2">
				<div class="flex items-center gap-2">
					<HugeiconsIcon icon={Download01Icon} class="h-4 w-4 text-primary" />
					<span class="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
						{t('player.downloads')}
					</span>
					{#if active.length}
						<span class="rounded-full bg-primary/12 px-1.5 py-0.5 text-[10px] font-semibold text-primary">
							{t('player.download_active', { count: active.length })}
						</span>
					{/if}
				</div>
				{#if hasFinished}
					<button
						class="cursor-pointer rounded px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent/10 hover:text-foreground"
						onclick={clearFinished}
					>
						{t('player.download_clear')}
					</button>
				{/if}
			</div>

			<div class="max-h-80 divide-y divide-border/50 overflow-y-auto">
				{#each shown as job (job.id)}
					<div class="px-3 py-2.5">
						<div class="flex items-center gap-3">
							<span class="min-w-0 flex-1 truncate text-xs font-medium">{job.title}</span>
							{#if job.state === 'downloading'}
								{#if processing(job.percent)}
									<span class="shrink-0 pr-0.5 text-[11px] text-muted-foreground">
										{t('player.download_processing')}
									</span>
								{:else}
									<span class="shrink-0 pr-0.5 text-xs font-semibold tabular-nums text-primary">
										{Math.round(job.percent)}%
									</span>
								{/if}
							{:else if job.state === 'queued'}
								<span class="shrink-0 pr-0.5 text-[11px] text-muted-foreground">
									{t('player.download_queued')}
								</span>
							{:else if job.state === 'done'}
								<HugeiconsIcon
									icon={CheckmarkCircle02Icon}
									class="mr-0.5 h-4 w-4 shrink-0 text-primary"
								/>
							{:else}
								<HugeiconsIcon
									icon={Alert02Icon}
									class="mr-0.5 h-4 w-4 shrink-0 text-destructive"
								/>
							{/if}
						</div>

						{#if job.artists}
							<div class="mt-0.5 truncate pr-1 text-[11px] text-muted-foreground">
								{job.artists}
							</div>
						{/if}

						{#if job.state === 'downloading'}
							<div class="mt-1.5 h-1.5 overflow-hidden rounded-full bg-muted" aria-hidden="true">
								<div
									class="h-full rounded-full bg-primary transition-[width] duration-200 {processing(
										job.percent
									)
										? 'animate-pulse'
										: ''}"
									style="width: {job.percent}%"
								></div>
							</div>
							<div
								class="mt-1 flex items-center justify-between gap-3 pr-0.5 text-[11px] text-muted-foreground"
							>
								<span class="tabular-nums">
									{formatBytes(job.downloaded)}{job.total ? ` / ${formatBytes(job.total)}` : ''}
								</span>
								<span class="flex shrink-0 items-center gap-1 tabular-nums">
									{#if job.speed && !processing(job.percent)}
										<HugeiconsIcon icon={Loading03Icon} class="h-2.5 w-2.5 animate-spin" />
									{/if}
									{processing(job.percent) ? '' : formatSpeed(job.speed)}
								</span>
							</div>
						{:else if job.state === 'done'}
							<div class="mt-1 pr-0.5 text-[11px] tabular-nums text-muted-foreground">
								{formatBytes(job.total ?? job.downloaded)}
							</div>
						{:else if job.state === 'failed' && job.error}
							<div
								class="mt-1 max-h-12 overflow-auto pr-0.5 text-[11px] leading-snug text-destructive"
							>
								{job.error}
							</div>
						{/if}
					</div>
				{/each}
			</div>
		</div>
	{/if}
{/if}
