<script lang="ts">
	import { onMount } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Add01Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { auth, toast } from '$lib/player.svelte';

	let items = $state<BrowseItem[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let creating = $state(false);
	let newTitle = $state('');
	let busy = $state(false);

	async function load() {
		loading = true;
		error = null;
		try {
			items = await api.getLibrary();
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}
	onMount(load);

	async function createNew() {
		const title = newTitle.trim();
		if (!title || busy) return;
		busy = true;
		try {
			const id = await api.createPlaylist(title);
			// YouTube's library browse is eventually-consistent and won't include a brand-new
			// playlist for a few seconds, so show it immediately instead of refetching.
			const browseId = id.startsWith('VL') ? id : `VL${id}`;
			items = [{ kind: 'playlist', id: browseId, title }, ...items];
			toast(`Created "${title}"`);
			newTitle = '';
			creating = false;
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
			<Button variant="outline" size="sm" class="gap-2" onclick={() => (creating = !creating)}>
				<HugeiconsIcon icon={Add01Icon} class="h-4 w-4" /> New playlist
			</Button>
		{/if}
	</div>

	{#if creating}
		<form
			class="mb-6 flex max-w-md gap-2"
			onsubmit={(e) => {
				e.preventDefault();
				createNew();
			}}
		>
			<Input bind:value={newTitle} placeholder="Playlist name" />
			<Button type="submit" disabled={busy || !newTitle.trim()}>
				{busy ? 'Creating…' : 'Create'}
			</Button>
		</form>
	{/if}

	{#if !auth.account?.signedIn}
		<p class="text-sm text-muted-foreground">Sign in to see your playlists and liked songs.</p>
	{:else if loading}
		<p class="text-sm text-muted-foreground">Loading…</p>
	{:else if error}
		<p class="text-sm text-destructive">{error}</p>
	{:else}
		<div class="grid grid-cols-[repeat(auto-fill,minmax(10rem,1fr))] gap-2">
			{#each items as item (item.id)}
				<MediaCard {item} />
			{/each}
		</div>
	{/if}
</div>
