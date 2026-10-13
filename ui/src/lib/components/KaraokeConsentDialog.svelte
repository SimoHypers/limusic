<script lang="ts">
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import {
		karaokeConsent,
		karaokeModel,
		confirmKaraokeModelDownload,
		cancelKaraokeModelDownload,
		dismissKaraokeConsent
	} from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	const sizeMB = $derived(
		karaokeModel.total > 0
			? `${(karaokeModel.total / (1024 * 1024)).toFixed(1)} MB`
			: '66.8 MB'
	);

	const downloadedMB = $derived((karaokeModel.downloaded / (1024 * 1024)).toFixed(1));
	const totalMB = $derived((karaokeModel.total / (1024 * 1024)).toFixed(1));
	const percent = $derived(
		karaokeModel.total > 0 ? Math.round((karaokeModel.downloaded / karaokeModel.total) * 100) : 0
	);
</script>

<Dialog.Root
	open={karaokeConsent.open}
	onOpenChange={(v) => {
		if (!v && karaokeModel.downloading) return;
		karaokeConsent.open = v;
		if (!v) dismissKaraokeConsent();
	}}
>
	<Dialog.Content class="z-[60] gap-5 sm:max-w-md">
		<div class="grid gap-1">
			<Dialog.Title class="text-lg font-semibold">{t('karaoke.consent.title')}</Dialog.Title>
			<Dialog.Description class="text-xs text-muted-foreground">
				{t('karaoke.consent.desc', { size: sizeMB })}
			</Dialog.Description>
		</div>

		{#if karaokeModel.error}
			<div class="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-xs text-destructive">
				{karaokeModel.error}
			</div>
		{/if}

		{#if karaokeModel.downloading}
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
			{#if karaokeModel.downloading}
				<Button variant="outline" size="sm" onclick={cancelKaraokeModelDownload}>
					{t('karaoke.cancel')}
				</Button>
			{:else if karaokeModel.error}
				<Button variant="outline" size="sm" onclick={dismissKaraokeConsent}>
					{t('karaoke.not_now')}
				</Button>
				<Button size="sm" onclick={confirmKaraokeModelDownload}>
					{t('karaoke.retry')}
				</Button>
			{:else}
				<Button variant="outline" size="sm" onclick={dismissKaraokeConsent}>
					{t('karaoke.not_now')}
				</Button>
				<Button size="sm" onclick={confirmKaraokeModelDownload}>
					{t('karaoke.download')}
				</Button>
			{/if}
		</div>
	</Dialog.Content>
</Dialog.Root>
