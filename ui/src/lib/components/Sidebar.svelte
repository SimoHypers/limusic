<script lang="ts">
	import { page } from '$app/state';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Home01Icon,
		Search01Icon,
		LibraryIcon,
		Sun01Icon,
		Moon02Icon,
		UserCircleIcon,
		Logout01Icon
	} from '@hugeicons/core-free-icons';
	import { toggleMode } from 'mode-watcher';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import * as api from '$lib/api';
	import { auth } from '$lib/player.svelte';

	const nav = [
		{ href: '/', label: 'Home', icon: Home01Icon },
		{ href: '/search', label: 'Search', icon: Search01Icon },
		{ href: '/library', label: 'Library', icon: LibraryIcon }
	];
	const isActive = (href: string) =>
		href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(href);

	let showAccount = $state(false);
	let cookieInput = $state('');
	let authError = $state<string | null>(null);
	let signingIn = $state(false);

	async function submitCookie() {
		if (!cookieInput.trim()) return;
		signingIn = true;
		authError = null;
		try {
			auth.account = await api.setCookie(cookieInput);
			cookieInput = '';
			showAccount = false;
		} catch (e) {
			authError = String(e);
		} finally {
			signingIn = false;
		}
	}

	async function doSignOut() {
		await api.signOut();
		auth.account = await api.getAccount();
		showAccount = false;
	}

	function signInGoogle() {
		api.loginWebview(); // native sign-in window takes over; result arrives via auth-changed
		showAccount = false;
	}
</script>

<aside class="flex h-full w-60 shrink-0 flex-col border-r bg-sidebar p-3 text-sidebar-foreground">
	<div class="flex items-center justify-between px-2 py-2">
		<span class="font-heading text-lg font-bold tracking-tight">Limusic</span>
		<Button variant="ghost" size="icon-sm" onclick={toggleMode} aria-label="Toggle theme">
			<HugeiconsIcon icon={Sun01Icon} class="h-4 w-4 dark:hidden" />
			<HugeiconsIcon icon={Moon02Icon} class="hidden h-4 w-4 dark:block" />
		</Button>
	</div>

	<nav class="mt-2 flex flex-col gap-1">
		{#each nav as n (n.href)}
			<a
				href={n.href}
				class="flex items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors {isActive(
					n.href
				)
					? 'bg-sidebar-accent text-sidebar-accent-foreground'
					: 'text-sidebar-foreground/70 hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'}"
			>
				<HugeiconsIcon icon={n.icon} class="h-5 w-5" />
				{n.label}
			</a>
		{/each}
	</nav>

	<!-- Account (context/15) -->
	<div class="relative mt-auto">
		{#if showAccount}
			<div
				class="absolute bottom-full left-0 mb-2 w-full rounded-xl border bg-popover p-4 text-popover-foreground shadow-lg"
			>
				{#if auth.account?.signedIn}
					<div class="flex items-center gap-3">
						{#if auth.account.thumbnail}
							<img src={auth.account.thumbnail} alt="" class="h-10 w-10 rounded-full object-cover" />
						{/if}
						<div class="min-w-0">
							<div class="truncate text-sm font-medium">{auth.account.name ?? 'Signed in'}</div>
							{#if auth.account.handle}
								<div class="truncate text-xs text-muted-foreground">{auth.account.handle}</div>
							{/if}
						</div>
					</div>
					<Button variant="outline" size="sm" class="mt-3 w-full gap-2" onclick={doSignOut}>
						<HugeiconsIcon icon={Logout01Icon} class="h-4 w-4" />
						Sign out
					</Button>
				{:else}
					<p class="text-sm font-medium">Sign in</p>
					<Button class="mt-3 w-full" onclick={signInGoogle}>Sign in with Google</Button>
					<div class="my-3 flex items-center gap-2 text-xs text-muted-foreground">
						<span class="h-px flex-1 bg-border"></span> or paste a cookie
						<span class="h-px flex-1 bg-border"></span>
					</div>
					<p class="text-xs text-muted-foreground">
						music.youtube.com → DevTools → Network → any request → copy the
						<span class="font-mono">Cookie</span> header.
					</p>
					<form
						class="mt-2 flex flex-col gap-2"
						onsubmit={(e) => {
							e.preventDefault();
							submitCookie();
						}}
					>
						<Input bind:value={cookieInput} placeholder="VISITOR_INFO1_LIVE=…; SAPISID=…; …" />
						<Button type="submit" variant="outline" disabled={signingIn}>
							{signingIn ? 'Signing in…' : 'Use cookie'}
						</Button>
					</form>
					{#if authError}<p class="mt-2 text-xs text-destructive">{authError}</p>{/if}
				{/if}
			</div>
		{/if}
		<button
			onclick={() => (showAccount = !showAccount)}
			class="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm transition-colors hover:bg-sidebar-accent/50"
		>
			{#if auth.account?.signedIn && auth.account.thumbnail}
				<img src={auth.account.thumbnail} alt="" class="h-8 w-8 rounded-full object-cover" />
			{:else}
				<HugeiconsIcon icon={UserCircleIcon} class="h-8 w-8 text-muted-foreground" />
			{/if}
			<span class="min-w-0 flex-1 truncate text-left font-medium">
				{auth.account?.signedIn ? (auth.account.name ?? 'Account') : 'Sign in'}
			</span>
		</button>
	</div>
</aside>
