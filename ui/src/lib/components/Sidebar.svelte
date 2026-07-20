<script lang="ts">
	import { page } from '$app/state';
	import { scale } from 'svelte/transition';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Home01Icon,
		Search01Icon,
		LibraryIcon,
		Settings01Icon,
		Sun01Icon,
		Moon02Icon,
		Add01Icon,
		PinIcon
	} from '@hugeicons/core-free-icons';
	import { toggleMode } from 'mode-watcher';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import * as Dialog from '$lib/components/ui/dialog';
	import type { BrowseItem } from '$lib/api';
	import PlaylistMenu from './PlaylistMenu.svelte';
	import { auth, library, personal, ui, createLibraryPlaylist, toast } from '$lib/player.svelte';
	import { orderLibrary } from '$lib/personal';

	const nav = [
		{ href: '/', label: 'Home', icon: Home01Icon },
		{ href: '/search', label: 'Search', icon: Search01Icon },
		{ href: '/library', label: 'Library', icon: LibraryIcon }
	];
	const isActive = (href: string) =>
		href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(href);

	// Pinned first (in pin order), then everything else by last played. Derived here rather than in
	// the shared `library` store so the Library page keeps YouTube's own ordering.
	const playlists = $derived(orderLibrary(library.items, personal));
	// How many of the leading rows are pinned — a rule under the last one explains the split.
	const pinnedCount = $derived(playlists.filter((p) => personal.pins.includes(p.id)).length);

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

	// Account lives in the titlebar now — see AccountMenu.svelte.
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
				class="group relative flex items-center justify-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors lg:justify-start {isActive(
					n.href
				)
					? 'bg-primary/10 text-primary'
					: 'text-sidebar-foreground/70 hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'}"
			>
				{#if isActive(n.href)}
					<span
						transition:scale={{ duration: 200, start: 0.4 }}
						class="absolute left-0 top-1/2 h-5 w-1 -translate-y-1/2 rounded-r-full bg-primary"
					></span>
				{/if}
				<HugeiconsIcon
					icon={n.icon}
					class="h-5 w-5 shrink-0 transition-transform duration-200 group-hover:scale-110"
				/>
				<span class="hidden lg:inline">{n.label}</span>
			</a>
		{/each}
		<button
			onclick={() => (ui.settingsOpen = true)}
			title="Settings"
			class="group flex items-center justify-center gap-3 rounded-lg px-3 py-2 text-sm font-medium text-sidebar-foreground/70 transition-colors hover:bg-sidebar-accent/50 hover:text-sidebar-foreground lg:justify-start"
		>
			<HugeiconsIcon
				icon={Settings01Icon}
				class="h-5 w-5 shrink-0 transition-transform duration-200 group-hover:scale-110"
			/>
			<span class="hidden lg:inline">Settings</span>
		</button>
	</nav>

	<!-- Playlists (signed in). Hidden on the icon rail — needs labels; matches YTM's collapsed rail.
	     flex-1 lets the list fill the space and scroll. -->
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
				{#each playlists as pl, i (pl.id)}
					<!-- The ⋯ is a sibling of the link, not a child: a <button> inside an <a> is invalid
					     HTML. pr-9 keeps the title clear of the button that overlays the row on hover. -->
					<div class="group/row relative">
						<a
							href={playlistHref(pl)}
							title={pl.title}
							class="block rounded-lg py-1.5 pl-3 pr-9 transition-colors hover:bg-sidebar-accent/50"
						>
							<div class="flex items-center gap-1.5">
								{#if personal.pins.includes(pl.id)}
									<HugeiconsIcon icon={PinIcon} class="h-3 w-3 shrink-0 text-primary" />
								{/if}
								<span class="truncate text-sm font-medium">{pl.title}</span>
							</div>
							{#if pl.subtitle}
								<div class="truncate text-xs text-muted-foreground">{pl.subtitle}</div>
							{/if}
						</a>
						<PlaylistMenu item={pl} />
					</div>
					{#if pinnedCount && i === pinnedCount - 1}
						<div class="mx-3 my-1.5 h-px bg-border"></div>
					{/if}
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

</aside>
