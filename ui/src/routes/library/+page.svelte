<script lang="ts">
	import { onMount } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Add01Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import * as Dialog from '$lib/components/ui/dialog';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import { auth, toast, library, loadLibrary, createLibraryPlaylist } from '$lib/player.svelte';

	let dialogOpen = $state(false);
	let newTitle = $state('');
	let busy = $state(false);

	// Shared library state (see player.svelte) — kept in sync with the sidebar list. Refresh on visit.
	onMount(() => {
		if (auth.account?.signedIn) loadLibrary(true);
	});

	async function createNew() {
		const title = newTitle.trim();
		if (!title || busy) return;
		busy = true;
		try {
			await createLibraryPlaylist(title);
			toast(`Created "${title}"`);
			newTitle = '';
			dialogOpen = false;
		} catch (e) {
			toast(String(e));
		} finally {
			busy = false;
		}
	}
</script>

<div class="p-6">
	<div class="mb-6 flex items-center justify-between">
		<h1 class="font-heading text-2xl font-bold">Library</h1>
		{#if auth.account?.signedIn}
			<Button variant="outline" size="sm" class="gap-2" onclick={() => (dialogOpen = true)}>
				<HugeiconsIcon icon={Add01Icon} class="h-4 w-4" /> New playlist
			</Button>
		{/if}
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
					<Button type="button" variant="outline" onclick={() => (dialogOpen = false)}>
						Cancel
					</Button>
					<Button type="submit" disabled={busy || !newTitle.trim()}>
						{busy ? 'Creating…' : 'Create'}
					</Button>
				</Dialog.Footer>
			</form>
		</Dialog.Content>
	</Dialog.Root>

	{#if !auth.account?.signedIn}
		<p class="text-sm text-muted-foreground">Sign in to see your playlists and liked songs.</p>
	{:else if library.loading && !library.items.length}
		<p class="text-sm text-muted-foreground">Loading…</p>
	{:else if library.error}
		<p class="text-sm text-destructive">{library.error}</p>
	{:else}
		<div class="grid grid-cols-[repeat(auto-fill,10rem)] gap-4">
			{#each library.items as item (item.id)}
				<MediaCard {item} />
			{/each}
		</div>
	{/if}
</div>
