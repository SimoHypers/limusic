<script lang="ts">
	// Contributing to Limusic Lyrics (see `limusic.rs`): anonymous by default, so this card says so
	// and offers to link Discord or GitHub, which is optional. A linked account shows its name; the
	// code the confirm page in the browser shows appears while a link is waiting.
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Loading03Icon, UserIcon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { ui } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import {
		cancelSignIn,
		limusic,
		refreshAccount,
		rename,
		signIn,
		signOut
	} from '$lib/limusic.svelte';
	import { providerName } from '$lib/limusic';
	import { waitingLabel } from '$lib/review';
	import type { TranslationKey } from '$lib/i18n.svelte';

	onMount(refreshAccount);

	let renaming = $state(false);
	let draft = $state('');
	let error = $state<TranslationKey | null>(null);
	let busy = $state(false);

	function startRename() {
		draft = limusic.handle ?? '';
		error = null;
		renaming = true;
	}
	async function saveRename() {
		const name = draft.trim();
		if (busy) return;
		if (name === limusic.handle) {
			renaming = false;
			return;
		}
		busy = true;
		error = await rename(name);
		busy = false;
		if (!error) renaming = false;
	}

	const a = $derived(limusic.account);
	/** A session with a public name: a linked account. */
	const linked = $derived(limusic.signedIn && !limusic.anonymous);
	const unlinked = $derived(!linked);
</script>

<div data-settings-card class="overflow-hidden rounded-xl border bg-card">
	<div class="flex items-center gap-3.5 px-4 py-4">
		<div
			class="flex size-11 shrink-0 items-center justify-center overflow-hidden rounded-full bg-muted ring-1 ring-border"
		>
			{#if linked && a?.avatar_url}
				<img src={a.avatar_url} alt="" class="size-full object-cover" draggable="false" />
			{:else if linked && limusic.handle}
				<span class="text-base font-semibold text-muted-foreground uppercase">{limusic.handle[0]}</span>
			{:else}
				<HugeiconsIcon icon={UserIcon} size={20} class="text-muted-foreground" />
			{/if}
		</div>
		<div class="min-w-0 flex-1">
			{#if linked && renaming}
				<form
					class="flex items-center gap-2"
					onsubmit={(e) => {
						e.preventDefault();
						saveRename();
					}}
				>
					<input
						bind:value={draft}
						maxlength="20"
						aria-label={t('limusic.rename')}
						class="min-w-0 flex-1 border-b border-border bg-transparent py-0.5 text-sm font-semibold outline-none focus:border-primary"
					/>
					<Button type="submit" size="sm" disabled={busy}>{t('common.save')}</Button>
					<Button type="button" variant="ghost" size="sm" onclick={() => (renaming = false)}>
						{t('common.cancel')}
					</Button>
				</form>
				<p class="mt-1 text-xs {error ? 'text-destructive' : 'text-muted-foreground'}">
					{error ? t(error) : t('limusic.rename_hint')}
				</p>
			{:else if limusic.signingIn}
				<p class="text-sm font-semibold">{t('limusic.waiting')}</p>
				<p class="text-xs text-muted-foreground">{t('limusic.code_hint')}</p>
			{:else if linked}
				<p class="truncate text-sm font-semibold">{limusic.handle}</p>
				{#if a}
					<p class="truncate text-xs text-muted-foreground">
						{t('limusic.linked_points', { count: String(a.score) })}
					</p>
				{/if}
			{:else if limusic.signedIn}
				<p class="text-sm font-semibold">{t('limusic.anonymous_title')}</p>
				{#if a}
					<p class="truncate text-xs text-muted-foreground">
						{t('limusic.anonymous_points', { count: String(a.score) })}
					</p>
				{/if}
			{:else}
				<p class="text-sm font-semibold">{t('limusic.anonymous_none')}</p>
				<p class="text-xs text-muted-foreground">{t('limusic.anonymous_none_hint')}</p>
			{/if}
		</div>
		{#if linked && !renaming && !limusic.signingIn}
			<Button variant="ghost" size="sm" onclick={startRename}>{t('limusic.rename')}</Button>
			<Button variant="ghost" size="sm" class="text-destructive hover:text-destructive" onclick={signOut}>
				{t('limusic.sign_out')}
			</Button>
		{:else if limusic.signingIn}
			<Button variant="ghost" size="sm" class="gap-1.5" onclick={cancelSignIn}>
				<HugeiconsIcon icon={Loading03Icon} size={15} class="animate-spin" />
				{t('common.cancel')}
			</Button>
		{/if}
	</div>
	{#if limusic.signingIn}
		<div class="border-t border-border/60 px-4 py-4 text-center">
			<p class="font-mono text-2xl font-semibold tracking-widest">{limusic.signingIn.code}</p>
		</div>
	{:else if linked && a}
		<p class="border-t border-border/60 px-4 py-3 text-xs text-muted-foreground">
			{a.can_review ? t('limusic.reviewer') : a.trusted ? t('limusic.trusted') : t('limusic.untrusted')}
		</p>
		{#if a.can_review}
			<div class="flex items-center gap-3 border-t border-border/60 px-4 py-3">
				<p class="min-w-0 flex-1 text-sm">
					{limusic.waiting && limusic.waiting.count > 0
						? t('review.settings_waiting', { count: waitingLabel(limusic.waiting.count, limusic.waiting.more) })
						: t('review.settings_none')}
				</p>
				<Button
					size="sm"
					variant="outline"
					onclick={() => {
						ui.settingsOpen = false;
						goto('/review');
					}}
				>
					{t('review.settings_open')}
				</Button>
			</div>
		{/if}
	{:else if unlinked}
		<div class="space-y-2 border-t border-border/60 px-4 py-3 text-xs text-muted-foreground">
			<p>{t('limusic.intro')}</p>
			{#if limusic.providers.length}
				<p>{t('limusic.link_benefits')}</p>
				<p>{t('limusic.link_shares')}</p>
			{/if}
		</div>
		{#if limusic.providers.length}
			<div class="flex flex-wrap gap-2 border-t border-border/60 px-4 py-3">
				{#each limusic.providers as p (p)}
					<Button size="sm" variant="outline" onclick={() => signIn(p)}>
						{t('limusic.link_with', { provider: providerName(p) })}
					</Button>
				{/each}
			</div>
		{/if}
	{/if}
</div>
