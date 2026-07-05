<script lang="ts">
	import { onMount } from 'svelte';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import * as api from '$lib/api';
	import type { HomePage } from '$lib/api';
	import { auth } from '$lib/player.svelte';

	let home = $state<HomePage | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);

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
	<h1 class="mb-6 font-heading text-2xl font-bold">Home</h1>
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
