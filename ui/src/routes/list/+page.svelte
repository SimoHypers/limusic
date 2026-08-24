<script lang="ts">
	import { page } from '$app/state';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import MediaCardSkeleton from '$lib/components/MediaCardSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { getCached, putCached } from '$lib/pagecache';
	import { reveal } from '$lib/reveal.svelte';
	import { t } from '$lib/i18n.svelte';

	let items = $state<BrowseItem[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	// A browse grid is however many things YouTube sends; reveal it in chunks past a couple of
	// hundred rather than mounting the lot when the page opens.
	const rv = reveal();

	const id = $derived(page.url.searchParams.get('id') ?? '');
	const params = $derived(page.url.searchParams.get('params') ?? undefined);
	const title = $derived(page.url.searchParams.get('title') ?? 'More');

	async function load(browseId: string, p?: string) {
		const key = `list:${browseId}:${p ?? ''}`;
		const hit = getCached<BrowseItem[]>(key);
		if (hit) {
			items = hit;
			rv.reset();
			loading = false;
		} else {
			loading = true;
			items = [];
			rv.reset();
		}
		error = null;
		try {
			const fresh = await api.getBrowseGrid(browseId, p);
			if (browseId !== id || p !== params) return; // superseded by navigation
			items = fresh;
			rv.reset();
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
		<div class="card-grid">
			{#each Array(12) as _, i (i)}
				<MediaCardSkeleton />
			{/each}
		</div>
	{:else if error}
		<ErrorState message={error} onRetry={() => load(id, params)} />
	{:else if items.length}
		<div class="card-grid content-in">
			{#each items.slice(0, rv.count(items.length)) as item (item.id + item.title)}
				<MediaCard {item} />
			{/each}
		</div>
		<!-- Outside the grid, or it would be laid out as a cell. -->
		{#if rv.more(items.length)}<div {@attach rv.sentinel}></div>{/if}
	{:else}
		<p class="text-sm text-muted-foreground">{t('common.nothing_here')}</p>
	{/if}
</div>
