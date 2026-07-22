<script lang="ts">
	import { fade, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Cancel01Icon } from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { ui, toast } from '$lib/player.svelte';

	let playlists = $state<BrowseItem[]>([]);
	let loading = $state(false);

	// Fetch the library playlists fresh each time the picker opens (cheap; picks up new playlists).
	$effect(() => {
		if (ui.addVideoIds) {
			loading = true;
			api
				.getLibrary()
				.then((p) => (playlists = p))
				.catch((e) => toast(String(e)))
				.finally(() => (loading = false));
		}
	});

	function close() {
		ui.addVideoIds = null;
	}

	async function pick(pl: BrowseItem) {
		const ids = ui.addVideoIds;
		close();
		if (!ids?.length) return;
		try {
			// Sequential — a whole album is a handful of requests; don't hammer the API in parallel.
			for (const videoId of ids) await api.addToPlaylist(pl.id, videoId);
			toast(ids.length > 1 ? `Added ${ids.length} songs to ${pl.title}` : `Added to ${pl.title}`);
		} catch (e) {
			toast(String(e));
		}
	}
</script>

<svelte:window
	onkeydown={(e) => {
		if (ui.addVideoIds && e.key === 'Escape') close();
	}}
/>

{#if ui.addVideoIds}
	<div
		transition:fade={{ duration: 150 }}
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
	>
		<div
			transition:scale={{ duration: 180, start: 0.96, easing: cubicOut }}
			class="w-full max-w-sm rounded-xl border bg-card p-4 shadow-xl"
		>
			<div class="mb-3 flex items-center justify-between">
				<h2 class="font-heading text-base font-semibold">Add to playlist</h2>
				<button
					class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
					onclick={close}
					aria-label="Close"
				>
					<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
				</button>
			</div>
			{#if loading}
				<p class="p-2 text-sm text-muted-foreground">Loading…</p>
			{:else if playlists.length}
				<div class="max-h-80 overflow-y-auto">
					{#each playlists as pl (pl.id)}
						<button
							class="flex w-full items-center gap-3 rounded-lg p-2 text-left hover:bg-accent/10"
							onclick={() => pick(pl)}
						>
							{#if pl.thumbnail}
								<img src={pl.thumbnail} alt="" class="h-10 w-10 rounded-md object-cover" />
							{:else}
								<div class="h-10 w-10 rounded-md bg-muted"></div>
							{/if}
							<div class="min-w-0">
								<div class="truncate text-sm font-medium">{pl.title}</div>
								{#if pl.subtitle}
									<div class="truncate text-xs text-muted-foreground">{pl.subtitle}</div>
								{/if}
							</div>
						</button>
					{/each}
				</div>
			{:else}
				<p class="p-2 text-sm text-muted-foreground">
					No playlists yet — create one in your Library.
				</p>
			{/if}
		</div>
	</div>
{/if}
