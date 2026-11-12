<script lang="ts">
	// Contributing to Limusic Lyrics (see `limusic.rs`). One card that says who you are here:
	// nobody yet, anonymous, or a linked account with its name, avatar and points. Linking Discord
	// or GitHub is optional, so it is shown as what it gives and the one thing it reads. While a
	// link waits for the browser, the card is the code its confirm page shows.
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ChampionIcon, FlashIcon, GithubIcon, Link01Icon, SecurityCheckIcon, UserIcon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import DiscordIcon from './DiscordIcon.svelte';
	import { ui } from '$lib/player.svelte';
	import { currentLocale, t } from '$lib/i18n.svelte';
	import { cancelSignIn, limusic, refreshAccount, rename, signIn, signOut } from '$lib/limusic.svelte';
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
	const points = $derived(a ? new Intl.NumberFormat(currentLocale.id).format(a.score) : '');
	const SEC = 'border-t border-border/60 px-4 py-3.5';
	const STAT = 'truncate text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase';
</script>

{#snippet brand(p: string, cls: string)}
	{#if p === 'discord'}
		<DiscordIcon class={cls} />
	{:else if p === 'github'}
		<HugeiconsIcon icon={GithubIcon} class={cls} />
	{:else}
		<HugeiconsIcon icon={Link01Icon} class={cls} />
	{/if}
{/snippet}

<div data-settings-card class="overflow-hidden rounded-xl border bg-card">
	{#if limusic.signingIn}
		<!-- Waiting for the browser: the code is the card. -->
		<div class="flex items-center gap-3.5 px-4 py-4">
			<div class="flex size-[52px] shrink-0 items-center justify-center rounded-full border-2 border-dashed border-muted-foreground text-muted-foreground">
				{@render brand(limusic.signingIn.provider, 'size-6')}
			</div>
			<div class="min-w-0 flex-1">
				<p class="font-heading text-[17px] leading-tight font-bold">{t('limusic.waiting')}</p>
				<p class="text-[13px] text-muted-foreground">{t('limusic.code_hint')}</p>
			</div>
		</div>
		<div class="{SEC} text-center">
			<div class="flex justify-center gap-1.5" role="group" aria-label={limusic.signingIn.code}>
				{#each limusic.signingIn.code as ch, i (i)}
					{#if /[\s-]/.test(ch)}
						<span class="w-2.5"></span>
					{:else}
						<kbd class="flex h-[42px] w-[34px] items-center justify-center rounded-[9px] border border-b-[3px] bg-muted text-xl font-semibold" aria-hidden="true">{ch}</kbd>
					{/if}
				{/each}
			</div>
			<div class="mt-3 flex items-center justify-center gap-2 text-[13px] text-muted-foreground">
				{t('limusic.waiting_for', { provider: providerName(limusic.signingIn.provider) })}
				<Button variant="ghost" size="sm" onclick={cancelSignIn}>{t('common.cancel')}</Button>
			</div>
		</div>
	{:else}
		<!-- Who you are here. -->
		<div class="flex flex-wrap items-center gap-x-3.5 gap-y-2 px-4 py-4">
			<div
				class="flex size-[52px] shrink-0 items-center justify-center overflow-hidden rounded-full {linked
					? 'bg-muted ring-1 ring-border'
					: 'border-2 border-dashed border-muted-foreground text-muted-foreground'}"
			>
				{#if linked && a?.avatar_url}
					<img src={a.avatar_url} alt="" class="size-full object-cover" draggable="false" />
				{:else if linked && limusic.handle}
					<span class="text-lg font-semibold text-muted-foreground uppercase">{limusic.handle[0]}</span>
				{:else}
					<HugeiconsIcon icon={UserIcon} class="size-6" />
				{/if}
			</div>
			{#if linked && renaming}
				<form
					class="flex min-w-48 flex-1 flex-wrap items-center gap-2"
					onsubmit={(e) => {
						e.preventDefault();
						saveRename();
					}}
				>
					<div class="min-w-32 flex-1">
						<!-- svelte-ignore a11y_autofocus -->
						<input
							bind:value={draft}
							autofocus
							maxlength="20"
							aria-label={t('limusic.rename')}
							class="w-full border-b border-primary bg-transparent py-px font-heading text-[17px] font-bold outline-none"
						/>
						<p class="mt-0.5 text-[13px] {error ? 'text-destructive' : 'text-muted-foreground'}">
							{error ? t(error) : t('limusic.rename_hint')}
						</p>
					</div>
					<Button type="button" variant="ghost" size="sm" onclick={() => (renaming = false)}>{t('common.cancel')}</Button>
					<Button type="submit" size="sm" disabled={busy}>{t('common.save')}</Button>
				</form>
			{:else}
				<div class="min-w-40 flex-1">
					<p class="truncate font-heading text-[17px] leading-tight font-bold">
						{linked ? limusic.handle : limusic.signedIn ? t('limusic.anonymous_title') : t('limusic.none_title')}
					</p>
					<p class="text-[13px] text-muted-foreground">
						{linked ? t('limusic.linked_hint') : limusic.signedIn ? t('limusic.anonymous_hint') : t('limusic.none_hint')}
					</p>
				</div>
				{#if linked}
					<div class="flex items-center">
						<Button variant="ghost" size="sm" class="text-muted-foreground" onclick={startRename}>{t('limusic.rename')}</Button>
						<Button variant="ghost" size="sm" class="text-muted-foreground" onclick={signOut}>{t('limusic.sign_out')}</Button>
					</div>
				{/if}
			{/if}
		</div>

		{#if limusic.signedIn && a}
			<div class="grid divide-x divide-border/60 border-t border-border/60 {a.can_review ? 'grid-cols-3' : 'grid-cols-2'}">
				<div class="min-w-0 px-4 py-2.5">
					<div class={STAT}>{t('limusic.stat_points')}</div>
					<div class="truncate text-[15px] font-semibold tabular-nums">{points}</div>
				</div>
				<div class="min-w-0 px-4 py-2.5">
					<div class={STAT}>{t('limusic.stat_lyrics')}</div>
					<div class="truncate text-[15px] font-semibold">
						{linked && a.trusted ? t('limusic.stat_live') : t('limusic.stat_reviewed')}
					</div>
				</div>
				{#if a.can_review}
					<div class="min-w-0 px-4 py-2.5">
						<div class={STAT}>{t('limusic.stat_role')}</div>
						<div class="truncate text-[15px] font-semibold">{t('limusic.role_reviewer')}</div>
					</div>
				{/if}
			</div>
		{/if}

		{#if linked && a?.can_review}
			<div class="{SEC} flex flex-wrap items-center gap-3">
				<p class="min-w-40 flex-1 text-sm">
					{limusic.waiting && limusic.waiting.count > 0
						? t('review.settings_waiting', { count: waitingLabel(limusic.waiting.count, limusic.waiting.more) })
						: t('review.settings_none')}
				</p>
				<Button
					size="sm"
					variant={limusic.waiting && limusic.waiting.count > 0 ? 'default' : 'outline'}
					onclick={() => {
						ui.settingsOpen = false;
						goto('/review');
					}}
				>
					{t('review.settings_open')}
				</Button>
			</div>
		{/if}

		{#if !linked}
			{#if limusic.providers.length}
				<!-- Linking: what it gives, the one thing it reads, and that it is optional. -->
				<div class={SEC}>
					<p class="text-sm font-semibold">{t('limusic.link_title')}</p>
					<ul class="mt-2.5 mb-3.5 space-y-2 text-[13px] text-muted-foreground">
						<li class="flex gap-2.5"><HugeiconsIcon icon={ChampionIcon} class="mt-0.5 size-4 shrink-0 text-foreground" />{t('limusic.link_name')}</li>
						<li class="flex gap-2.5"><HugeiconsIcon icon={FlashIcon} class="mt-0.5 size-4 shrink-0 text-foreground" />{t('limusic.link_trust')}</li>
						<li class="flex gap-2.5"><HugeiconsIcon icon={SecurityCheckIcon} class="mt-0.5 size-4 shrink-0 text-foreground" />{t('limusic.link_shares')}</li>
					</ul>
					<div class="flex flex-wrap items-center gap-2">
						{#each limusic.providers as p (p)}
							<Button variant="outline" disabled={limusic.launching} onclick={() => signIn(p)}>
								{@render brand(p, 'size-4')}
								{t('limusic.link_with', { provider: providerName(p) })}
							</Button>
						{/each}
						<span class="ml-1 text-[12.5px] text-muted-foreground">{t('limusic.optional')}</span>
					</div>
				</div>
			{/if}
		{/if}
	{/if}
</div>
