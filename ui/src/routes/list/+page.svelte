<script lang="ts">
	import { page } from '$app/state';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';

	let items = $state<BrowseItem[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	const id = $derived(page.url.searchParams.get('id') ?? '');
	const params = $derived(page.url.searchParams.get('params') ?? undefined);
	const title = $derived(page.url.searchParams.get('title') ?? 'More');

	async function load(browseId: string, p?: string) {
		loading = true;
		error = null;
		items = [];
		try {
			items = await api.getBrowseGrid(browseId, p);
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		if (id) load(id, params);
	});
</script>

<div class="p-6">
	<h1 class="mb-6 font-heading text-2xl font-bold">{title}</h1>
	{#if loading}
		<p class="text-sm text-muted-foreground">Loading…</p>
	{:else if error}
		<p class="text-sm text-destructive">{error}</p>
	{:else if items.length}
		<div class="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-2">
			{#each items as item (item.id + item.title)}
				<MediaCard {item} />
			{/each}
		</div>
	{:else}
		<p class="text-sm text-muted-foreground">Nothing here.</p>
	{/if}
</div>
