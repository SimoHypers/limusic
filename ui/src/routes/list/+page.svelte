<script lang="ts">
	import { page } from '$app/state';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import MediaCardSkeleton from '$lib/components/MediaCardSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { getCached, putCached } from '$lib/pagecache';

	let items = $state<BrowseItem[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	const id = $derived(page.url.searchParams.get('id') ?? '');
	const params = $derived(page.url.searchParams.get('params') ?? undefined);
	const title = $derived(page.url.searchParams.get('title') ?? 'More');

	async function load(browseId: string, p?: string) {
		const key = `list:${browseId}:${p ?? ''}`;
		const hit = getCached<BrowseItem[]>(key);
		if (hit) {
			items = hit;
			loading = false;
		} else {
			loading = true;
			items = [];
		}
		error = null;
		try {
			const fresh = await api.getBrowseGrid(browseId, p);
			if (browseId !== id || p !== params) return; // superseded by navigation
			items = fresh;
			putCached(key, fresh);
		} catch (e) {
			if (browseId !== id || p !== params) return;
			if (!hit) error = String(e);
		} finally {
			if (browseId === id && p === params) loading = false;
		}
	}

	$effect(() => {
		if (id) load(id, params);
	});
</script>

<div class="p-6">
	<h1 class="mb-6 font-heading text-2xl font-bold">{title}</h1>
	{#if loading}
		<div class="grid grid-cols-[repeat(auto-fill,10rem)] gap-4">
			{#each Array(12) as _, i (i)}
				<MediaCardSkeleton />
			{/each}
		</div>
	{:else if error}
		<ErrorState message={error} onRetry={() => load(id, params)} />
	{:else if items.length}
		<div class="grid grid-cols-[repeat(auto-fill,10rem)] gap-4">
			{#each items as item (item.id + item.title)}
				<MediaCard {item} />
			{/each}
		</div>
	{:else}
		<p class="text-sm text-muted-foreground">Nothing here.</p>
	{/if}
</div>
