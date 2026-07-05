<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Search01Icon } from '@hugeicons/core-free-icons';
	import { Input } from '$lib/components/ui/input';
	import { Button } from '$lib/components/ui/button';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import * as api from '$lib/api';
	import type { SongItem } from '$lib/api';
	import { openAddToPlaylist } from '$lib/player.svelte';

	let query = $state('');
	let results = $state<SongItem[]>([]);
	let searching = $state(false);
	let error = $state<string | null>(null);

	async function runSearch() {
		if (!query.trim()) return;
		searching = true;
		error = null;
		try {
			results = await api.search(query);
		} catch (e) {
			error = String(e);
		} finally {
			searching = false;
		}
	}
</script>

<div class="flex h-full flex-col">
	<div class="border-b p-6">
		<h1 class="mb-4 font-heading text-2xl font-bold">Search</h1>
		<form
			class="flex max-w-xl gap-2"
			onsubmit={(e) => {
				e.preventDefault();
				runSearch();
			}}
		>
			<Input bind:value={query} placeholder="Search songs on YouTube Music…" />
			<Button type="submit" class="gap-2" disabled={searching}>
				<HugeiconsIcon icon={Search01Icon} class="h-4 w-4" />
				{searching ? 'Searching…' : 'Search'}
			</Button>
		</form>
		{#if error}<p class="mt-2 text-sm text-destructive">{error}</p>{/if}
	</div>
	<div class="min-h-0 flex-1 overflow-y-auto p-4">
		{#each results as item (item.video_id)}
			<TrackRow
				song={item}
				onplay={() => api.play(item)}
				onAdd={() => openAddToPlaylist(item.video_id)}
			/>
		{:else}
			<p class="p-4 text-sm text-muted-foreground">Search for a song to start.</p>
		{/each}
	</div>
</div>
