<script lang="ts">
	// Account control for the titlebar (context/15) — moved out of the sidebar so sign-in lives in the
	// top bar. Its own component because Titlebar.svelte already uses a single shared mx/my/menuOpen
	// for the Last.fm menu; a second menu in that file would fight over them.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { UserCircleIcon, Logout01Icon, ArrowDown01Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import * as api from '$lib/api';
	import { auth } from '$lib/player.svelte';
	import { thumb } from '$lib/thumb';

	let menuOpen = $state(false);
	let mx = $state(0);
	let my = $state(0);
	let cookieInput = $state('');
	let authError = $state<string | null>(null);
	let signingIn = $state(false);

	// Right-anchored under the trigger, like the Last.fm menu next to it.
	function openMenu(e: MouseEvent) {
		const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
		mx = window.innerWidth - r.right;
		my = r.bottom + 6;
		menuOpen = !menuOpen;
	}

	// Sign-in/out state arrives via the `auth-changed` event (player.svelte.ts), which also reloads
	// the library and remounts the page — nothing to assign here.
	async function submitCookie() {
		if (!cookieInput.trim()) return;
		signingIn = true;
		authError = null;
		try {
			await api.setCookie(cookieInput);
			cookieInput = '';
			menuOpen = false;
		} catch (e) {
			authError = String(e);
		} finally {
			signingIn = false;
		}
	}

	async function doSignOut() {
		menuOpen = false;
		await api.signOut();
	}

	function signInGoogle() {
		api.loginWebview(); // native sign-in window takes over; result arrives via auth-changed
		menuOpen = false;
	}
</script>

<button
	onclick={openMenu}
	title={auth.account?.signedIn ? (auth.account.name ?? 'Account') : 'Sign in'}
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
		{auth.account?.signedIn ? (auth.account.name ?? 'Account') : 'Sign in'}
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
		aria-label="Close menu"
	></button>
	<div
		class="fixed z-50 w-72 origin-top-right animate-in rounded-xl border bg-popover p-4 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style="right:{mx}px; top:{my}px;"
	>
		{#if auth.account?.signedIn}
			<div class="mb-3">
				<div class="truncate text-sm font-medium">{auth.account.name ?? 'Account'}</div>
				{#if auth.account.handle}
					<div class="truncate text-xs text-muted-foreground">{auth.account.handle}</div>
				{/if}
			</div>
			<Button variant="outline" size="sm" class="w-full gap-2" onclick={doSignOut}>
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
