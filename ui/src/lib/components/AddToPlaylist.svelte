<script lang="ts">
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { ui, toast } from '$lib/player.svelte';

	let playlists = $state<BrowseItem[]>([]);
	let loading = $state(false);

	// Fetch the library playlists fresh each time the picker opens (cheap; picks up new playlists).
	$effect(() => {
		if (ui.addVideoId) {
			loading = true;
			api
				.getLibrary()
				.then((p) => (playlists = p))
				.catch((e) => toast(String(e)))
				.finally(() => (loading = false));
		}
	});

	function close() {
		ui.addVideoId = null;
	}

	async function pick(pl: BrowseItem) {
		const videoId = ui.addVideoId;
		close();
		if (!videoId) return;
		try {
			await api.addToPlaylist(pl.id, videoId);
			toast(`Added to ${pl.title}`);
		} catch (e) {
			toast(String(e));
		}
	}
</script>

<svelte:window
	onkeydown={(e) => {
		if (ui.addVideoId && e.key === 'Escape') close();
	}}
/>

{#if ui.addVideoId}
	<div class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
		<div class="w-full max-w-sm rounded-xl border bg-card p-4 shadow-xl">
			<div class="mb-3 flex items-center justify-between">
				<h2 class="font-heading text-base font-semibold">Add to playlist</h2>
				<button class="text-muted-foreground hover:text-foreground" onclick={close} aria-label="Close">
					✕
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
