<script lang="ts">
	import './layout.css';
	import favicon from '$lib/assets/favicon.svg';
	import { ModeWatcher } from 'mode-watcher';
	import { onMount } from 'svelte';
	import Sidebar from '$lib/components/Sidebar.svelte';
	import PlayerBar from '$lib/components/PlayerBar.svelte';
	import QueuePanel from '$lib/components/QueuePanel.svelte';
	import AddToPlaylist from '$lib/components/AddToPlaylist.svelte';
	import { initApp, playback, ui } from '$lib/player.svelte';

	let { children } = $props();
	let showQueue = $state(false);

	// Wire the Tauri event bridge once for the whole app; teardown on destroy.
	onMount(() => initApp());
</script>

<svelte:head><link rel="icon" href={favicon} /></svelte:head>
<ModeWatcher />

<div class="flex h-screen flex-col overflow-hidden bg-background text-foreground">
	<div class="flex min-h-0 flex-1">
		<Sidebar />
		<main class="min-w-0 flex-1 overflow-y-auto">
			{@render children()}
		</main>
		{#if showQueue}<QueuePanel />{/if}
	</div>
	<PlayerBar onToggleQueue={() => (showQueue = !showQueue)} queueOpen={showQueue} />
</div>

<AddToPlaylist />

{#if ui.toast}
	<div
		class="fixed bottom-24 left-1/2 z-50 -translate-x-1/2 rounded-lg border bg-card px-4 py-2 text-sm shadow-lg"
	>
		{ui.toast}
	</div>
{/if}

{#if playback.error}
	<div
		class="fixed bottom-24 left-1/2 z-50 flex -translate-x-1/2 items-center gap-3 rounded-lg border border-destructive/40 bg-card px-4 py-2 text-sm text-destructive shadow-lg"
	>
		<span>{playback.error}</span>
		<button
			class="text-muted-foreground hover:text-foreground"
			aria-label="Dismiss"
			onclick={() => (playback.error = null)}>✕</button
		>
	</div>
{/if}
