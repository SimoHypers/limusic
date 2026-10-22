<script lang="ts">
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import {
		karaokeConsent,
		karaokeModels,
		karaokeSelection,
		confirmKaraokeModelDownload,
		cancelKaraokeModelDownload,
		dismissKaraokeConsent
	} from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import { openExternal } from '$lib/api';
	import { browser } from '$app/environment';

	/** The UVR project, credited as its README asks ("providing credit to UVR and its developers"). */
	const UVR_URL = 'https://github.com/Anjok07/ultimatevocalremovergui';
	// DirectML (and its telemetry note) only exists on Windows.
	const IS_WINDOWS = browser && navigator.userAgent.includes('Windows');

	const currentModelId = $derived(karaokeConsent.model ?? karaokeSelection.model);
	const currentModelState = $derived(karaokeModels[currentModelId] ?? karaokeModels.kara);
	const modelLabel = $derived(
		currentModelId === 'kara_2'
			? t('karaoke.settings.model_medium')
			: currentModelId === 'inst_hq_3'
				? t('karaoke.settings.model_strong')
				: t('karaoke.settings.model_weak')
	);

	const sizeMB = $derived(
		currentModelState.total > 0
			? `${(currentModelState.total / (1024 * 1024)).toFixed(1)} MB`
			: currentModelId === 'inst_hq_3'
				? '66.8 MB'
				: currentModelId === 'kara_2'
					? '66.8 MB'
					: '66.8 MB'
	);

	const downloadedMB = $derived((currentModelState.downloaded / (1024 * 1024)).toFixed(1));
	const totalMB = $derived((currentModelState.total / (1024 * 1024)).toFixed(1));
	const percent = $derived(
		currentModelState.total > 0 ? Math.round((currentModelState.downloaded / currentModelState.total) * 100) : 0
	);
</script>

<Dialog.Root
	open={karaokeConsent.open}
	onOpenChange={(v) => {
		if (!v && currentModelState.downloading) return;
		karaokeConsent.open = v;
		if (!v) dismissKaraokeConsent();
	}}
>
	<Dialog.Content class="z-[60] gap-5 sm:max-w-md">
		<div class="grid gap-1">
			<Dialog.Title class="text-lg font-semibold">{t('karaoke.consent.title')} ({modelLabel})</Dialog.Title>
			<Dialog.Description class="text-xs text-muted-foreground">
				{t('karaoke.consent.desc', { size: sizeMB })}
			</Dialog.Description>
		</div>

		<ul class="grid list-disc gap-1 pl-4 text-xs text-muted-foreground">
			<li>{t('karaoke.consent.source')}</li>
			<li>
				{t('karaoke.consent.credit')}
				<button
					type="button"
					class="cursor-pointer text-foreground underline underline-offset-2 hover:text-primary"
					onclick={() => openExternal(UVR_URL)}
				>
					{t('karaoke.consent.credit_link')}
				</button>
			</li>
			<li>{t('karaoke.consent.redownload')}</li>
			{#if IS_WINDOWS}
				<li>{t('karaoke.consent.directml')}</li>
			{/if}
		</ul>

		{#if currentModelState.error}
			<div class="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-xs text-destructive">
				{currentModelState.error}
			</div>
		{/if}

		{#if currentModelState.downloading}
			<div class="flex flex-col gap-2">
				<div class="flex justify-between text-xs text-muted-foreground">
					<span>{downloadedMB} MB / {totalMB} MB</span>
					<span>{percent}%</span>
				</div>
				<div class="h-2 w-full overflow-hidden rounded-full bg-muted">
					<div class="h-full bg-primary transition-all duration-200" style="width: {percent}%"></div>
				</div>
			</div>
		{/if}

		<div class="flex justify-end gap-2">
			{#if currentModelState.downloading}
				<Button variant="outline" size="sm" onclick={cancelKaraokeModelDownload}>
					{t('karaoke.cancel')}
				</Button>
			{:else if currentModelState.error}
				<Button variant="outline" size="sm" onclick={dismissKaraokeConsent}>
					{t('karaoke.not_now')}
				</Button>
				<Button size="sm" onclick={() => confirmKaraokeModelDownload(karaokeConsent.model)}>
					{t('karaoke.retry')}
				</Button>
			{:else}
				<Button variant="outline" size="sm" onclick={dismissKaraokeConsent}>
					{t('karaoke.not_now')}
				</Button>
				<Button size="sm" onclick={() => confirmKaraokeModelDownload(karaokeConsent.model)}>
					{t('karaoke.download')}
				</Button>
			{/if}
		</div>
	</Dialog.Content>
</Dialog.Root>
