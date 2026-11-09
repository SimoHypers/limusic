<script lang="ts">
	// First use of karaoke, or a quality tier that isn't downloaded yet: what it is, which quality,
	// what downloading means. Opened by turning karaoke on, by the panel's quality picker and by
	// Settings, always through `karaokeConsent`.
	import { RadioGroup } from 'bits-ui';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		ComputerIcon,
		Download04Icon,
		HardDriveIcon,
		SingLeftIcon,
		Tick02Icon
	} from '@hugeicons/core-free-icons';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import {
		playback,
		karaokeConsent,
		karaokeModels,
		chooseKaraokeModel,
		confirmKaraokeModelDownload,
		cancelKaraokeModelDownload,
		dismissKaraokeConsent,
		toggleKaraoke
	} from '$lib/player.svelte';
	import { KARAOKE_TIERS, tierMB } from '$lib/karaoke-tiers';
	import ExperimentalBadge from './ExperimentalBadge.svelte';
	import { t } from '$lib/i18n.svelte';
	import { openExternal } from '$lib/api';
	import { browser } from '$app/environment';

	/** The UVR project, credited as its README asks ("providing credit to UVR and its developers"). */
	const UVR_URL = 'https://github.com/Anjok07/ultimatevocalremovergui';
	// DirectML (and its telemetry note) only exists on Windows.
	const IS_WINDOWS = browser && navigator.userAgent.includes('Windows');
	const LABEL = 'mb-2 px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground';

	const tier = $derived(KARAOKE_TIERS.find((x) => x.id === karaokeConsent.model) ?? KARAOKE_TIERS[0]);
	const model = $derived(karaokeModels[tier.id]);
	const downloading = $derived(Object.values(karaokeModels).some((m) => m.downloading));
	const percent = $derived(
		model.total > 0 ? Math.min(100, Math.round((model.downloaded / model.total) * 100)) : 0
	);
	const mb = (bytes: number) => (bytes / 1048576).toFixed(1);

	/** Already downloaded: nothing to fetch, just switch to it (and turn karaoke on if that's how
	 *  the dialog was reached). */
	async function useInstalled() {
		const enable = karaokeConsent.enableAfter;
		await chooseKaraokeModel(tier.id);
		dismissKaraokeConsent();
		if (enable && !playback.karaoke) toggleKaraoke();
	}
</script>

<Dialog.Root
	open={karaokeConsent.open}
	onOpenChange={(v) => {
		// A download in flight ends with Cancel, not by clicking past the dialog.
		if (!v && downloading) return;
		if (!v) dismissKaraokeConsent();
	}}
>
	<Dialog.Content class="z-[60] flex max-h-[85dvh] flex-col gap-0 overflow-hidden p-0 sm:max-w-md">
		<div class="flex items-start gap-3 px-6 pb-4 pr-14 pt-6">
			<div
				class="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary"
			>
				<HugeiconsIcon icon={SingLeftIcon} class="h-5 w-5" />
			</div>
			<div class="min-w-0">
				<Dialog.Title class="flex items-center gap-2 font-heading text-base font-semibold">
					{t('karaoke.setup.title')}
					<ExperimentalBadge tooltip={t('karaoke.experimental_tooltip')} />
				</Dialog.Title>
				<Dialog.Description class="text-xs leading-relaxed text-muted-foreground">
					{t('karaoke.setup.desc')}
				</Dialog.Description>
			</div>
		</div>

		<div class="min-h-0 flex-1 overflow-y-auto px-6 pb-5">
			<h3 class={LABEL}>{t('karaoke.panel.quality')}</h3>
			<RadioGroup.Root
				value={tier.id}
				onValueChange={(v) => v && !downloading && (karaokeConsent.model = v as typeof tier.id)}
				aria-label={t('karaoke.panel.quality')}
				class="grid gap-2"
			>
				{#each KARAOKE_TIERS as option (option.id)}
					<RadioGroup.Item
						value={option.id}
						disabled={downloading}
						class="group flex cursor-pointer items-center gap-3 rounded-xl border px-3.5 py-3 text-left outline-none transition-colors hover:border-foreground/25 focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default data-checked:border-primary data-checked:bg-primary/8"
					>
						<span class="flex h-4 w-4 shrink-0 items-end gap-[2px]" aria-hidden="true">
							{#each [1, 2, 3] as n (n)}
								<span
									class="w-[3px] rounded-full {n <= option.level
										? 'bg-muted-foreground/60 group-data-checked:bg-primary'
										: 'bg-muted-foreground/20'}"
									style="height:{4 + n * 4}px"
								></span>
							{/each}
						</span>
						<span class="min-w-0 flex-1">
							<span class="block text-sm font-medium">
								{t(`karaoke.settings.model_${option.key}`)}
							</span>
							<span class="mt-0.5 block text-xs leading-relaxed text-muted-foreground">
								{t(`karaoke.settings.model_${option.key}_desc`)}
							</span>
						</span>
						<span class="shrink-0 text-xs text-muted-foreground">
							{#if karaokeModels[option.id].installed}
								<span class="flex items-center gap-1 text-primary">
									<HugeiconsIcon icon={Tick02Icon} class="size-3.5" />
									{t('karaoke.setup.downloaded')}
								</span>
							{:else}
								{tierMB(option.id)}
							{/if}
						</span>
					</RadioGroup.Item>
				{/each}
			</RadioGroup.Root>

			<ul class="mt-5 grid gap-2.5 text-xs leading-relaxed text-muted-foreground">
				<li class="flex gap-2.5">
					<HugeiconsIcon icon={ComputerIcon} class="mt-px size-4 shrink-0" />
					{t('karaoke.setup.local')}
				</li>
				<li class="flex gap-2.5">
					<HugeiconsIcon icon={Download04Icon} class="mt-px size-4 shrink-0" />
					{t('karaoke.consent.redownload')}
				</li>
				<li class="flex gap-2.5">
					<HugeiconsIcon icon={HardDriveIcon} class="mt-px size-4 shrink-0" />
					{t('karaoke.setup.cache')}
				</li>
			</ul>

			<!-- Where everything comes from, and who made the models: small print, but all of it. -->
			<p class="mt-5 text-[11px] leading-relaxed text-muted-foreground/80">
				{t('karaoke.consent.source')}
				{IS_WINDOWS ? t('karaoke.consent.runtime_windows') : t('karaoke.consent.runtime')}
				{#if IS_WINDOWS}{t('karaoke.consent.directml')}{/if}
				{t('karaoke.consent.credit')}
				<button
					type="button"
					class="cursor-pointer text-foreground underline underline-offset-2 hover:text-primary"
					onclick={() => openExternal(UVR_URL)}
				>
					{t('karaoke.consent.credit_link')}
				</button>
			</p>

			{#if model.error}
				<p
					class="mt-4 rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2.5 text-xs text-destructive"
					role="alert"
				>
					{t(`karaoke.download_error.${model.error}`)}
				</p>
			{/if}
		</div>

		<div class="flex items-center gap-3 border-t px-6 py-4">
			{#if model.downloading}
				<div class="min-w-0 flex-1">
					<div
						class="h-1.5 overflow-hidden rounded-full bg-muted"
						role="progressbar"
						aria-valuemin={0}
						aria-valuemax={100}
						aria-valuenow={percent}
					>
						<div class="h-full rounded-full bg-primary" style="width:{percent}%"></div>
					</div>
					<p class="mt-1.5 text-xs tabular-nums text-muted-foreground">
						{t('karaoke.setup.progress', {
							done: mb(Math.min(model.downloaded, model.total)),
							total: mb(model.total)
						})}
					</p>
				</div>
				<Button variant="outline" size="sm" onclick={cancelKaraokeModelDownload}>
					{t('karaoke.cancel')}
				</Button>
			{:else}
				<span class="flex-1"></span>
				<Button variant="ghost" size="sm" onclick={dismissKaraokeConsent}>
					{t('karaoke.not_now')}
				</Button>
				{#if model.installed}
					<Button size="sm" onclick={useInstalled}>
						{t('karaoke.setup.use', { name: t(`karaoke.settings.model_${tier.key}`) })}
					</Button>
				{:else}
					<Button size="sm" onclick={() => confirmKaraokeModelDownload(tier.id)}>
						<HugeiconsIcon icon={Download04Icon} class="h-4 w-4" />
						{model.error
							? t('karaoke.retry')
							: tierMB(tier.id)
								? t('karaoke.setup.download', { size: tierMB(tier.id) })
								: t('karaoke.download')}
					</Button>
				{/if}
			{/if}
		</div>
	</Dialog.Content>
</Dialog.Root>
