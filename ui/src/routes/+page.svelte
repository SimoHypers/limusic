<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Search01Icon } from '@hugeicons/core-free-icons';
	import { Input } from '$lib/components/ui/input';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import * as api from '$lib/api';
	import type { HomePage } from '$lib/api';
	import { auth } from '$lib/player.svelte';

	let home = $state<HomePage | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let searchQuery = $state('');

	function goSearch() {
		if (!searchQuery.trim()) return;
		goto(`/search?${new URLSearchParams({ q: searchQuery }).toString()}`);
	}

	async function load() {
		loading = true;
		error = null;
		try {
			home = await api.getHome();
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}
	onMount(load);
</script>

<div class="p-6">
	<div class="mb-6 flex items-center justify-between gap-4">
		<h1 class="font-heading text-2xl font-bold">Home</h1>
		<form class="relative w-full max-w-xs" onsubmit={(e) => { e.preventDefault(); goSearch(); }}>
			<HugeiconsIcon
				icon={Search01Icon}
				class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"
			/>
			<Input bind:value={searchQuery} placeholder="Search" class="rounded-full pl-9" />
		</form>
	</div>
	{#if loading}
		<p class="text-sm text-muted-foreground">Loading…</p>
	{:else if error}
		<p class="text-sm text-destructive">{error}</p>
	{:else if home && home.sections.length}
		<div class="flex flex-col gap-8">
			{#each home.sections as section (section.title)}
				<section>
					<h2 class="mb-3 font-heading text-lg font-semibold">{section.title}</h2>
					<div class="flex gap-2 overflow-x-auto pb-2">
						{#each section.items as item (item.id + item.title)}
							<div class="w-40 shrink-0">
								<MediaCard {item} />
							</div>
						{/each}
					</div>
				</section>
			{/each}
		</div>
	{:else}
		<p class="text-sm text-muted-foreground">
			Nothing here yet.
			{auth.account?.signedIn ? '' : 'Sign in to see your personalized home feed.'}
		</p>
	{/if}
</div>
