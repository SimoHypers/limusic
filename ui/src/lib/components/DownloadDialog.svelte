<script lang="ts">
	// "Download playlist": the count picker. Opened from a playlist's ⋯ menu (PlaylistMenu and the
	// playlist page), one dialog for both so the count box, the quality choice and the copy read the
	// same everywhere. `ui.downloadPlaylist` carries the target.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Download01Icon } from '@hugeicons/core-free-icons';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import { t } from '$lib/i18n.svelte';
	import { ui } from '$lib/player.svelte';
	import { downloads, downloadPlaylist } from '$lib/downloads.svelte';
	import { parseCount, type DownloadQuality } from '$lib/downloads';

	// The three labels live in the `downloads` namespace, shared with Settings: one wording for
	// "High" wherever it is offered.
	const QUALITIES = [
		{ id: 'HIGH', label: 'downloads.quality_high', hint: 'downloads.quality_high_hint' },
		{ id: 'AUTO', label: 'downloads.quality_auto', hint: 'downloads.quality_auto_hint' },
		{ id: 'LOW', label: 'downloads.quality_low', hint: 'downloads.quality_low_hint' }
	] as const;
	const CONDITION = (q: DownloadQuality) => QUALITIES.find((o) => o.id === q) ?? QUALITIES[0];

	const target = $derived(ui.downloadPlaylist);
	// How many: the whole list, or the first N. All is the default — "download a playlist" usually
	// means all of it, and the number is the exception.
	let mode = $state<'all' | 'count'>('all');
	let countText = $state('');
	let quality = $state<DownloadQuality>(downloads.quality);
	let busy = $state(false);

	const check = $derived(parseCount(countText, target?.total));
	const qualityHint = $derived(t(CONDITION(quality).hint));
	// An empty box is "not answered yet" rather than an error, so it gets no red line.
	const countError = $derived(
		mode !== 'count' || check.error === null || check.error === 'empty'
			? null
			: check.error === 'too_big'
				? t('downloads.dialog.count_too_big', { max: target?.total ?? 0 })
				: t('downloads.dialog.count_invalid')
	);
	const canSubmit = $derived(!busy && (mode === 'all' || check.error === null));

	// A fresh dialog each time it opens: last visit's count and half-typed number are gone.
	$effect(() => {
		if (ui.downloadPlaylist) {
			mode = 'all';
			countText = '';
			quality = downloads.quality;
			busy = false;
		}
	});

	const close = () => (ui.downloadPlaylist = null);

	async function submit() {
		const t0 = target;
		if (!t0 || !canSubmit) return;
		const count = mode === 'all' ? null : check.count;
		if (mode === 'count' && count === null) return;
		busy = true;
		try {
			const queued = await downloadPlaylist(t0.id, count, quality);
			if (queued !== null) close();
		} finally {
			busy = false;
		}
	}
</script>

<Dialog.Root open={!!target} onOpenChange={(v) => !v && close()}>
	<Dialog.Content class="sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>
				{target?.kind === 'album' ? t('downloads.dialog.title_album') : t('downloads.dialog.title')}
			</Dialog.Title>
			<Dialog.Description>
				{t('downloads.dialog.desc', { title: target?.title ?? '' })}
			</Dialog.Description>
		</Dialog.Header>
		<form
			class="flex flex-col gap-4"
			onsubmit={(e) => {
				e.preventDefault();
				submit();
			}}
		>
			<div>
				<span class="mb-2 block px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">
					{t('downloads.dialog.count_label')}
				</span>
				<!-- Segmented like the quality picker: one exclusive choice. -->
				<div class="flex rounded-lg bg-muted p-0.5" role="radiogroup"
					aria-label={t('downloads.dialog.count_label')}>
					<button
						type="button"
						role="radio"
						aria-checked={mode === 'all'}
						onclick={() => (mode = 'all')}
						class="flex-1 cursor-pointer rounded-md px-3.5 py-1.5 text-xs font-medium transition-colors {mode ===
						'all'
							? 'bg-background text-foreground shadow-sm'
							: 'text-muted-foreground hover:text-foreground'}"
					>
						{target?.total
							? t('downloads.dialog.count_all_with_total', { total: target.total })
							: t('downloads.dialog.count_all')}
					</button>
					<button
						type="button"
						role="radio"
						aria-checked={mode === 'count'}
						onclick={() => (mode = 'count')}
						class="flex-1 cursor-pointer rounded-md px-3.5 py-1.5 text-xs font-medium transition-colors {mode ===
						'count'
							? 'bg-background text-foreground shadow-sm'
							: 'text-muted-foreground hover:text-foreground'}"
					>
						{t('downloads.dialog.count_first')}
					</button>
				</div>
				{#if mode === 'count'}
					<Input
						class="mt-2"
						bind:value={countText}
						inputmode="numeric"
						autocomplete="off"
						placeholder={t('downloads.dialog.count_placeholder')}
						aria-label={t('downloads.dialog.count_label')}
						aria-invalid={!!countError}
						autofocus
					/>
					{#if countError}
						<p class="mt-1.5 px-1 text-xs text-destructive">{countError}</p>
					{/if}
				{/if}
			</div>

			<div>
				<span class="mb-2 block px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">
					{t('downloads.dialog.quality_label')}
				</span>
				<div class="flex rounded-lg bg-muted p-0.5" role="radiogroup"
					aria-label={t('downloads.dialog.quality_label')}>
					{#each QUALITIES as q (q.id)}
						<button
							type="button"
							role="radio"
							aria-checked={quality === q.id}
							onclick={() => (quality = q.id)}
							class="flex-1 cursor-pointer rounded-md px-2.5 py-1.5 text-xs font-medium transition-colors {quality ===
							q.id
								? 'bg-background text-foreground shadow-sm'
								: 'text-muted-foreground hover:text-foreground'}"
						>
							{t(q.label)}
						</button>
					{/each}
				</div>
				<p class="mt-1.5 px-1 text-xs leading-relaxed text-muted-foreground">{qualityHint}</p>
			</div>

			<Dialog.Footer>
				<Button type="button" variant="outline" onclick={close}>{t('common.cancel')}</Button>
				<Button type="submit" class="gap-2" disabled={!canSubmit}>
					<HugeiconsIcon icon={Download01Icon} class="h-4 w-4" />
					{busy ? t('common.loading') : t('downloads.dialog.confirm')}
				</Button>
			</Dialog.Footer>
		</form>
	</Dialog.Content>
</Dialog.Root>