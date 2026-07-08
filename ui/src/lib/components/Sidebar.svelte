<script lang="ts">
	import { page } from '$app/state';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Home01Icon,
		Search01Icon,
		LibraryIcon,
		Settings01Icon,
		Sun01Icon,
		Moon02Icon,
		UserCircleIcon,
		Logout01Icon,
		Add01Icon
	} from '@hugeicons/core-free-icons';
	import { toggleMode } from 'mode-watcher';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import * as Dialog from '$lib/components/ui/dialog';
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { auth, library, ui, createLibraryPlaylist, toast } from '$lib/player.svelte';

	const nav = [
		{ href: '/', label: 'Home', icon: Home01Icon },
		{ href: '/search', label: 'Search', icon: Search01Icon },
		{ href: '/library', label: 'Library', icon: LibraryIcon }
	];
	const isActive = (href: string) =>
		href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(href);

	const playlistHref = (item: BrowseItem) =>
		item.kind === 'album'
			? `/album/${encodeURIComponent(item.id)}`
			: item.kind === 'artist'
				? `/artist/${encodeURIComponent(item.id)}`
				: `/playlist/${encodeURIComponent(item.id)}`;

	// New-playlist dialog (mirrors the Library page).
	let dialogOpen = $state(false);
	let newTitle = $state('');
	let creating = $state(false);
	async function createNew() {
		const title = newTitle.trim();
		if (!title || creating) return;
		creating = true;
		try {
			await createLibraryPlaylist(title);
			toast(`Created "${title}"`);
			newTitle = '';
			dialogOpen = false;
		} catch (e) {
			toast(String(e));
		} finally {
			creating = false;
		}
	}

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

<aside
	class="flex h-full w-16 shrink-0 flex-col border-r bg-sidebar p-3 text-sidebar-foreground lg:w-60"
>
	<div class="flex items-center justify-center px-2 py-2 lg:justify-between">
		<span class="hidden font-heading text-lg font-bold tracking-tight lg:block">Limusic</span>
		<Button variant="ghost" size="icon-sm" onclick={toggleMode} aria-label="Toggle theme">
			<HugeiconsIcon icon={Sun01Icon} class="h-4 w-4 dark:hidden" />
			<HugeiconsIcon icon={Moon02Icon} class="hidden h-4 w-4 dark:block" />
		</Button>
	</div>

	<nav class="mt-2 flex flex-col gap-1">
		{#each nav as n (n.href)}
			<a
				href={n.href}
				title={n.label}
				class="flex items-center justify-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors lg:justify-start {isActive(
					n.href
				)
					? 'bg-sidebar-accent text-sidebar-accent-foreground'
					: 'text-sidebar-foreground/70 hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'}"
			>
				<HugeiconsIcon icon={n.icon} class="h-5 w-5 shrink-0" />
				<span class="hidden lg:inline">{n.label}</span>
			</a>
		{/each}
		<button
			onclick={() => (ui.settingsOpen = true)}
			title="Settings"
			class="flex items-center justify-center gap-3 rounded-lg px-3 py-2 text-sm font-medium text-sidebar-foreground/70 transition-colors hover:bg-sidebar-accent/50 hover:text-sidebar-foreground lg:justify-start"
		>
			<HugeiconsIcon icon={Settings01Icon} class="h-5 w-5 shrink-0" />
			<span class="hidden lg:inline">Settings</span>
		</button>
	</nav>

	<!-- Playlists (signed in). Hidden on the icon rail — needs labels; matches YTM's collapsed rail.
	     flex-1 lets the list fill the space and scroll, pinning the account to the bottom. -->
	{#if auth.account?.signedIn}
		<div class="mt-3 hidden min-h-0 flex-1 flex-col border-t pt-3 lg:flex">
			<Button
				variant="outline"
				size="sm"
				class="mb-2 w-full gap-2"
				onclick={() => (dialogOpen = true)}
			>
				<HugeiconsIcon icon={Add01Icon} class="h-4 w-4" /> New playlist
			</Button>
			<div class="min-h-0 flex-1 overflow-y-auto">
				{#each library.items as pl (pl.id)}
					<a
						href={playlistHref(pl)}
						title={pl.title}
						class="block rounded-lg px-3 py-1.5 transition-colors hover:bg-sidebar-accent/50"
					>
						<div class="truncate text-sm font-medium">{pl.title}</div>
						{#if pl.subtitle}
							<div class="truncate text-xs text-muted-foreground">{pl.subtitle}</div>
						{/if}
					</a>
				{:else}
					{#if library.loading}
						<p class="px-3 py-1.5 text-xs text-muted-foreground">Loading…</p>
					{/if}
				{/each}
			</div>
		</div>

		<Dialog.Root bind:open={dialogOpen}>
			<Dialog.Content class="sm:max-w-md">
				<Dialog.Header>
					<Dialog.Title>New playlist</Dialog.Title>
					<Dialog.Description>Give your playlist a name to get started.</Dialog.Description>
				</Dialog.Header>
				<form
					class="flex flex-col gap-4"
					onsubmit={(e) => {
						e.preventDefault();
						createNew();
					}}
				>
					<Input bind:value={newTitle} placeholder="Playlist name" autofocus />
					<Dialog.Footer>
						<Button type="button" variant="outline" onclick={() => (dialogOpen = false)}>Cancel</Button>
						<Button type="submit" disabled={creating || !newTitle.trim()}>
							{creating ? 'Creating…' : 'Create'}
						</Button>
					</Dialog.Footer>
				</form>
			</Dialog.Content>
		</Dialog.Root>
	{/if}

	<!-- Account (context/15) -->
	<div class="relative mt-auto">
		{#if showAccount}
			<!-- z-50: the popup is w-64 and overflows into <main> on the icon rail; without a
			     stacking layer <main> (a later sibling) paints its track list over the popup. -->
			<div
				class="absolute bottom-full left-0 z-50 mb-2 w-64 rounded-xl border bg-popover p-4 text-popover-foreground shadow-lg"
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
			title={auth.account?.signedIn ? (auth.account.name ?? 'Account') : 'Sign in'}
			class="flex w-full items-center justify-center gap-3 rounded-lg px-3 py-2 text-sm transition-colors hover:bg-sidebar-accent/50 lg:justify-start"
		>
			{#if auth.account?.signedIn && auth.account.thumbnail}
				<!-- max-width:none defeats Tailwind Preflight's `img{max-width:100%}`, which on the
				     narrow icon rail clamps width to the tiny button content-box while height stays
				     fixed → a vertical oval. Inline so it's immune to Preflight and stale dev CSS. -->
				<img
					src={auth.account.thumbnail}
					alt=""
					style="width:2.25rem;height:2.25rem;max-width:none"
					class="shrink-0 rounded-full object-cover ring-1 ring-border"
				/>
			{:else}
				<HugeiconsIcon icon={UserCircleIcon} class="h-9 w-9 shrink-0 text-muted-foreground" />
			{/if}
			<span class="hidden min-w-0 flex-1 truncate text-left font-medium lg:block">
				{auth.account?.signedIn ? (auth.account.name ?? 'Account') : 'Sign in'}
			</span>
		</button>
	</div>
</aside>
