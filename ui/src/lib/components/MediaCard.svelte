<script lang="ts">
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { PlayIcon, Add01Icon, Tick02Icon } from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { toast } from '$lib/player.svelte';

	let { item }: { item: BrowseItem } = $props();

	const isArtist = $derived(item.kind === 'artist');
	const clickable = $derived(!isArtist);
	const round = $derived(isArtist);

	let subscribed = $state(false);
	let subBusy = $state(false);

	function activate() {
		if (item.kind === 'song') {
			api.play({
				video_id: item.id,
				title: item.title,
				artists: item.subtitle ?? '',
				thumbnail: item.thumbnail
			});
		} else if (item.kind === 'playlist' || item.kind === 'album') {
			goto(`/playlist/${encodeURIComponent(item.id)}`);
		}
	}

	async function toggleSub() {
		if (subBusy) return;
		const next = !subscribed;
		subBusy = true;
		subscribed = next; // optimistic
		try {
			await api.subscribe(item.id, next);
			toast(next ? `Subscribed to ${item.title}` : `Unsubscribed from ${item.title}`);
		} catch (e) {
			subscribed = !next; // revert
			toast(String(e));
		} finally {
			subBusy = false;
		}
	}
</script>

<div class="group flex w-full flex-col gap-2">
	<button
		class="flex flex-col gap-2 rounded-xl p-2 text-left transition-colors hover:bg-accent/10 disabled:pointer-events-none"
		onclick={activate}
		disabled={!clickable}
	>
		<div
			class="relative aspect-square w-full overflow-hidden bg-muted {round
				? 'rounded-full'
				: 'rounded-lg'}"
		>
			{#if item.thumbnail}
				<img src={item.thumbnail} alt="" class="h-full w-full object-cover" loading="lazy" />
			{/if}
			{#if clickable}
				<div
					class="absolute bottom-2 right-2 flex h-9 w-9 items-center justify-center rounded-full bg-primary text-primary-foreground opacity-0 shadow-lg transition-opacity group-hover:opacity-100"
				>
					<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" />
				</div>
			{/if}
		</div>
		<div class="min-w-0 {round ? 'text-center' : ''}">
			<div class="truncate text-sm font-medium">{item.title}</div>
			{#if item.subtitle}
				<div class="truncate text-xs text-muted-foreground">{item.subtitle}</div>
			{/if}
		</div>
	</button>

	{#if isArtist}
		<button
			class="mx-auto -mt-1 flex items-center gap-1 rounded-full border px-3 py-1 text-xs font-medium transition hover:bg-accent/10 disabled:opacity-60 {subscribed
				? 'text-primary'
				: ''}"
			onclick={toggleSub}
			disabled={subBusy}
		>
			<HugeiconsIcon icon={subscribed ? Tick02Icon : Add01Icon} class="h-3.5 w-3.5" />
			{subscribed ? 'Subscribed' : 'Subscribe'}
		</button>
	{/if}
</div>
