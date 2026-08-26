<script lang="ts">
	// Account control for the titlebar (context/15) — moved out of the sidebar so sign-in lives in the
	// top bar. Its own component because Titlebar.svelte already uses a single shared mx/my/menuOpen
	// for the Last.fm menu; a second menu in that file would fight over them.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { UserCircleIcon, Logout01Icon, ArrowDown01Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import * as api from '$lib/api';
	import { auth, openChannelPicker } from '$lib/player.svelte';
	import { thumb } from '$lib/thumb';
	import { anchorMenu, fitMenu, NO_ANCHOR } from '$lib/menu';
	import { t } from '$lib/i18n.svelte';

	let menuOpen = $state(false);
	let anchor = $state(NO_ANCHOR);

	// Right-anchored under the trigger, like the Last.fm menu next to it.
	function openMenu(e: MouseEvent) {
		anchor = anchorMenu(e, { align: 'right' });
		menuOpen = !menuOpen;
	}

	// Sign-in/out state arrives via the `auth-changed` event (player.svelte.ts), which also reloads
	// the library and remounts the page — nothing to assign here.
	async function doSignOut() {
		menuOpen = false;
		await api.signOut();
	}

	function signInGoogle() {
		api.loginWebview(); // native sign-in window takes over; result arrives via auth-changed
		menuOpen = false;
	}

	function switchChannel() {
		menuOpen = false;
		openChannelPicker();
	}
</script>

<button
	onclick={openMenu}
	title={auth.account?.signedIn ? (auth.account.name ?? t('nav.account')) : t('nav.sign_in')}
	aria-expanded={menuOpen}
	class="flex h-full cursor-pointer items-center gap-2 px-2.5 text-xs transition-colors hover:bg-muted aria-expanded:bg-muted"
>
	{#if auth.account?.signedIn && auth.account.thumbnail}
		<!-- max-width:none defeats Tailwind Preflight's `img{max-width:100%}`, which in a tight box
		     clamps width to the content-box while height stays fixed → a vertical oval. Inline so it's
		     immune to Preflight and to stale dev CSS. -->
		<img
			src={thumb(auth.account.thumbnail, 64)}
			alt=""
			style="width:1.25rem;height:1.25rem;max-width:none"
			class="shrink-0 rounded-full object-cover ring-1 ring-border"
		/>
	{:else}
		<HugeiconsIcon icon={UserCircleIcon} class="h-5 w-5 shrink-0 text-muted-foreground" />
	{/if}
	<span class="hidden max-w-28 truncate font-medium lg:block">
		{auth.account?.signedIn ? (auth.account.name ?? t('nav.account')) : t('nav.sign_in')}
	</span>
	<HugeiconsIcon
		icon={ArrowDown01Icon}
		class="hidden h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform duration-200 lg:block {menuOpen
			? 'rotate-180'
			: ''}"
	/>
</button>

{#if menuOpen}
	<button
		class="fixed inset-0 z-40 cursor-default"
		onclick={() => (menuOpen = false)}
		aria-label={t('common.close')}
	></button>
	<div
		class="fixed z-50 w-72 animate-in rounded-xl border bg-popover p-4 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style={anchor.style}
		{@attach fitMenu(anchor)}
	>
		{#if auth.account?.signedIn}
			<div class="mb-3">
				<div class="truncate text-sm font-medium">{auth.account.name ?? t('nav.account')}</div>
				{#if auth.account.handle || auth.account.email}
					<div class="truncate text-xs text-muted-foreground">
						{auth.account.handle ?? auth.account.email}
					</div>
				{/if}
			</div>
			<!-- Always offered, never gated on a stored "you have one channel": that answer comes from
			     a single accounts_list call at sign-in, and when it fails the switcher used to vanish
			     for good. The picker fetches the list live and shows its own error. -->
			<Button variant="outline" size="sm" class="mb-2 w-full gap-2" onclick={switchChannel}>
				<HugeiconsIcon icon={UserCircleIcon} class="h-4 w-4" />
				{t('nav.switch_channel')}
			</Button>
			<Button variant="outline" size="sm" class="w-full gap-2" onclick={doSignOut}>
				<HugeiconsIcon icon={Logout01Icon} class="h-4 w-4" />
				{t('nav.sign_out')}
			</Button>
		{:else}
			<p class="text-sm font-medium">{t('nav.sign_in')}</p>
			<p class="mt-1 text-xs text-muted-foreground">
				{t('nav.sign_in_hint')}
			</p>
			<Button class="mt-3 w-full" onclick={signInGoogle}>{t('common.sign_in_google')}</Button>
		{/if}
	</div>
{/if}
