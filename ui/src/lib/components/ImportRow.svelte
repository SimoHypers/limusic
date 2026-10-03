<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		CheckmarkCircle02Icon,
		Alert02Icon,
		Cancel01Icon,
		MusicNote01Icon
	} from '@hugeicons/core-free-icons';
	import type { ImportTrack, ImportMatch } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import * as Popover from '$lib/components/ui/popover';
	import { importerState, togglePicked, setChoice } from '$lib/importer.svelte';

	let {
		index,
		track,
		result,
		choice = null
	}: {
		index: number;
		track: ImportTrack;
		result?: ImportMatch;
		choice?: number | null;
	} = $props();

	function formatDuration(secs?: number | null): string {
		if (!secs || secs < 0) return '';
		const m = Math.floor(secs / 60);
		const s = secs % 60;
		return `${m}:${s.toString().padStart(2, '0')}`;
	}

	const sourceDuration = $derived(formatDuration(track.duration_secs));
	const artistsStr = $derived(track.artists.join(', '));

	const candidate = $derived.by(() => {
		if (!result || choice === null || choice === undefined) return null;
		return result.candidates[choice] ?? null;
	});

	const diffText = $derived.by(() => {
		if (!candidate || candidate.duration_diff_secs === undefined || candidate.duration_diff_secs === null || candidate.duration_diff_secs === 0) {
			return null;
		}
		const diff = candidate.duration_diff_secs;
		return diff > 0 ? `+${diff}s` : `${diff}s`;
	});

	const statusIcon = $derived.by(() => {
		if (!result) return null;
		if (result.status === 'matched') {
			return { icon: CheckmarkCircle02Icon, class: 'text-primary' };
		}
		if (result.status === 'review') {
			return { icon: Alert02Icon, class: 'text-primary' };
		}
		return { icon: Cancel01Icon, class: 'text-muted-foreground' };
	});

	const hasNoCandidate = $derived(!result || result.status === 'not_found' || result.candidates.length === 0);
	let popoverOpen = $state(false);
</script>

<div
	class="grid grid-cols-1 md:grid-cols-2 gap-4 items-center p-3 rounded-lg border bg-card/20 {result?.status === 'review' ? 'border-primary/40' : ''} {result?.status === 'not_found' ? 'opacity-60' : ''} [content-visibility:auto] [contain-intrinsic-size:auto_64px]"
>
	<!-- LEFT: Checkbox + Source track -->
	<div class="flex items-center gap-3 min-w-0">
		<Checkbox
			checked={importerState.picked[index]}
			disabled={hasNoCandidate}
			aria-label={t('import.select_track', { title: track.title })}
			class="cursor-pointer shrink-0"
			onclick={(e) => {
				e.preventDefault();
				e.stopPropagation();
				togglePicked(index);
			}}
		/>
		<div class="flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground/50">
			<HugeiconsIcon icon={MusicNote01Icon} class="h-4 w-4" />
		</div>
		<div class="min-w-0 flex-1">
			<div class="truncate text-sm font-medium text-foreground">{track.title}</div>
			<div class="truncate text-xs text-muted-foreground">
				{artistsStr}{track.album ? ` · ${track.album}` : ''}{sourceDuration ? ` · ${sourceDuration}` : ''}
			</div>
		</div>
	</div>

	<!-- RIGHT: Chosen candidate, pending skeleton, not found, and Change button -->
	<div class="flex items-center justify-between min-w-0 gap-3">
		{#if result === undefined}
			<div class="w-full space-y-2">
				<Skeleton class="h-4 w-3/4 rounded" />
				<Skeleton class="h-3 w-1/2 rounded" />
			</div>
		{:else if result.status === 'not_found' || !candidate}
			<span class="text-sm text-muted-foreground italic">{t('import.not_found')}</span>
		{:else}
			<div class="flex items-center gap-3 min-w-0 flex-1">
				{#if candidate.song.thumbnail}
					<img
						src={thumb(candidate.song.thumbnail, 96)}
						alt=""
						class="h-10 w-10 shrink-0 rounded-md object-cover"
						loading="lazy"
					/>
				{:else}
					<div class="flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground/50">
						<HugeiconsIcon icon={MusicNote01Icon} class="h-4 w-4" />
					</div>
				{/if}
				<div class="min-w-0 flex-1">
					<div class="flex items-center gap-2">
						<span class="truncate text-sm font-medium text-foreground">{candidate.song.title}</span>
						{#if diffText}
							<span class="shrink-0 rounded bg-muted px-1 py-0.5 text-[10px] tabular-nums text-muted-foreground">
								{diffText}
							</span>
						{/if}
					</div>
					<div class="truncate text-xs text-muted-foreground">
						{candidate.song.artists}{candidate.song.duration ? ` · ${candidate.song.duration}` : ''}
					</div>
				</div>
			</div>
		{/if}

		<div class="flex items-center gap-2 shrink-0">
			{#if result && result.candidates.length > 0}
				<Popover.Root bind:open={popoverOpen}>
					<Popover.Trigger
						class="inline-flex items-center justify-center rounded-full border border-input bg-background px-3 py-1 text-xs font-medium shadow-sm hover:bg-accent hover:text-accent-foreground transition-colors"
						aria-label={t('import.change_match', { title: track.title })}
					>
						{t('import.change')}
					</Popover.Trigger>
					<Popover.Content side="bottom" align="end" class="w-80 p-2 space-y-1">
						<div class="px-2 py-1.5 text-xs font-medium text-muted-foreground">{t('import.candidates')}</div>
						{#each result.candidates as cand, k}
							{@const isChosen = choice === k}
							{@const candDiff = cand.duration_diff_secs === undefined || cand.duration_diff_secs === null || cand.duration_diff_secs === 0 ? null : (cand.duration_diff_secs > 0 ? `+${cand.duration_diff_secs}s` : `${cand.duration_diff_secs}s`)}
							<button
								type="button"
								class="w-full text-left flex items-center justify-between gap-2 rounded-md p-2 text-xs hover:bg-accent/10 transition-colors {isChosen ? 'bg-primary/10 font-medium' : ''}"
								onclick={() => {
									setChoice(index, k);
									popoverOpen = false;
								}}
							>
								<div class="min-w-0 flex-1">
									<div class="truncate text-foreground">{cand.song.title}</div>
									<div class="truncate text-[11px] text-muted-foreground">
										{cand.song.artists}{cand.song.duration ? ` · ${cand.song.duration}` : ''}
									</div>
								</div>
								{#if candDiff}
									<span class="shrink-0 rounded bg-muted px-1 py-0.5 text-[10px] tabular-nums text-muted-foreground">
										{candDiff}
									</span>
								{/if}
							</button>
						{/each}
						<div class="border-t my-1"></div>
						<button
							type="button"
							class="w-full text-left rounded-md p-2 text-xs text-muted-foreground hover:bg-accent/10 hover:text-foreground transition-colors"
							onclick={() => {
								setChoice(index, null);
								popoverOpen = false;
							}}
						>
							{t('import.skip_track')}
						</button>
					</Popover.Content>
				</Popover.Root>
			{/if}

			{#if statusIcon}
				{@const iconObj = statusIcon}
				<div class="flex items-center">
					<HugeiconsIcon icon={iconObj.icon} class="h-4 w-4 {iconObj.class}" />
				</div>
			{/if}
		</div>
	</div>
</div>
