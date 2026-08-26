<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { CheckmarkCircle02Icon, UserCircleIcon } from '@hugeicons/core-free-icons';
	import * as Dialog from '$lib/components/ui/dialog';
	import { Button } from '$lib/components/ui/button';
	import * as api from '$lib/api';
	import type { AccountIdentity } from '$lib/api';
	import { toast, ui } from '$lib/player.svelte';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';

	let loading = $state(false);
	let switching = $state<string | null>(null);
	let cancelling = $state(false);
	let error = $state<string | null>(null);
	let loadedForOpen = $state(false);

	$effect(() => {
		if (!ui.channelPickerOpen) {
			loadedForOpen = false;
			error = null;
			return;
		}
		if (loadedForOpen) return;
		loadedForOpen = true;
		void loadIdentities();
	});

	async function loadIdentities() {
		loading = true;
		error = null;
		try {
			ui.channelIdentities = await api.getAccountIdentities();
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	async function choose(identity: AccountIdentity) {
		if (switching) return;
		switching = identity.selectionKey;
		error = null;
		try {
			const wasRequired = ui.channelPickerRequired;
			await api.switchAccount(identity.selectionKey);
			ui.channelPickerRequired = false;
			ui.channelPickerOpen = false;
			ui.channelIdentities = [];
			toast.success(wasRequired ? `Signed in as ${identity.name}` : `Switched to ${identity.name}`);
		} catch (e) {
			error = String(e);
			toast.error(error);
		} finally {
			switching = null;
		}
	}

	async function cancelSignIn() {
		if (cancelling || switching) return;
		cancelling = true;
		try {
			await api.signOut();
			ui.channelPickerRequired = false;
			ui.channelPickerOpen = false;
		} catch (e) {
			error = String(e);
		} finally {
			cancelling = false;
		}
	}

	// A new multi-channel login is intentionally unfinished until one usable identity is chosen, so
	// escape and click-outside are inert until then. Cancel sign-in is the way out.
	let dismissable: 'ignore' | 'close' = $derived(ui.channelPickerRequired ? 'ignore' : 'close');
</script>

<Dialog.Root bind:open={ui.channelPickerOpen}>
	<Dialog.Content
		class="gap-0 overflow-hidden p-0 sm:max-w-md"
		showCloseButton={!ui.channelPickerRequired}
		escapeKeydownBehavior={dismissable}
		interactOutsideBehavior={dismissable}
	>
		<div class="border-b px-5 py-4">
			<Dialog.Title class="text-lg font-semibold">{t('nav.choose_channel')}</Dialog.Title>
			<Dialog.Description class="mt-1 text-xs text-muted-foreground">
				{t('nav.choose_channel_desc')}
			</Dialog.Description>
		</div>

		<div class="max-h-[26rem] min-h-32 overflow-y-auto p-2">
			{#if loading}
				<p class="px-3 py-8 text-center text-sm text-muted-foreground">{t('common.loading')}</p>
			{:else if error}
				<div class="space-y-3 px-3 py-6 text-center">
					<p class="text-sm text-destructive">{error}</p>
					<Button variant="outline" size="sm" onclick={loadIdentities}>{t('common.retry')}</Button>
				</div>
			{:else if ui.channelIdentities.length === 0}
				<p class="px-3 py-8 text-center text-sm text-muted-foreground">
					YouTube did not return any selectable channels.
				</p>
			{:else}
				{#each ui.channelIdentities as identity (identity.selectionKey)}
					<button
						type="button"
						onclick={() => choose(identity)}
						disabled={switching !== null}
						class="flex w-full cursor-pointer items-center gap-3 rounded-lg px-3 py-2.5 text-left transition-colors hover:bg-muted disabled:cursor-wait disabled:opacity-60"
					>
						{#if identity.thumbnail}
							<img
								src={thumb(identity.thumbnail, 96)}
								alt=""
								class="h-10 w-10 shrink-0 rounded-full object-cover ring-1 ring-border"
							/>
						{:else}
							<span class="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-muted">
								<HugeiconsIcon icon={UserCircleIcon} class="h-6 w-6 text-muted-foreground" />
							</span>
						{/if}
						<span class="min-w-0 flex-1">
							<span class="block truncate text-sm font-medium">{identity.name}</span>
							{#if identity.handle || identity.email}
								<span class="block truncate text-xs text-muted-foreground">
									{identity.handle ?? identity.email}
								</span>
							{/if}
						</span>
						{#if identity.selected}
							<span class="flex shrink-0 items-center gap-1 text-xs text-primary">
								<HugeiconsIcon icon={CheckmarkCircle02Icon} class="h-4 w-4" />
								{t('library.in_library')}
							</span>
						{/if}
					</button>
				{/each}
			{/if}
		</div>

		<div class="flex justify-end border-t px-5 py-3">
			{#if ui.channelPickerRequired}
				<Button variant="outline" size="sm" onclick={cancelSignIn} disabled={cancelling || switching !== null}>
					{cancelling ? t('common.loading') : t('nav.cancel_sign_in')}
				</Button>
			{:else}
				<Button variant="outline" size="sm" onclick={() => (ui.channelPickerOpen = false)}>
					Cancel
				</Button>
			{/if}
		</div>
	</Dialog.Content>
</Dialog.Root>
