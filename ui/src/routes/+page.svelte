<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Search01Icon, UserMultiple02Icon } from '@hugeicons/core-free-icons';
	import { Input } from '$lib/components/ui/input';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import MediaCardSkeleton from '$lib/components/MediaCardSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import * as api from '$lib/api';
	import type { HomePage } from '$lib/api';
	import { auth, ui } from '$lib/player.svelte';
	import { lt } from '$lib/lt.svelte';
	import { getCached, putCached } from '$lib/pagecache';

	let home = $state<HomePage | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let searchQuery = $state('');

	function goSearch() {
		if (!searchQuery.trim()) return;
		goto(`/search?${new URLSearchParams({ q: searchQuery }).toString()}`);
	}

	async function load() {
		const hit = getCached<HomePage>('home');
		if (hit) {
			home = hit;
			loading = false;
		} else {
			loading = true;
		}
		error = null;
		try {
			const fresh = await api.getHome();
			home = fresh;
			putCached('home', fresh);
		} catch (e) {
			if (!hit) error = String(e);
		} finally {
			loading = false;
		}
	}
	onMount(load);
</script>

<div class="p-6">
	<div class="mb-6 flex items-center justify-between gap-4">
		<h1 class="font-heading text-2xl font-bold">Home</h1>
		<div class="flex items-center gap-2">
			<button
				onclick={() => (ui.ltOpen = true)}
				title="Listen Together"
				aria-label="Listen Together"
				class="relative flex h-9 w-9 shrink-0 items-center justify-center rounded-full border transition-colors {lt.role !==
				'none'
					? 'border-primary text-primary hover:bg-primary/10'
					: 'border-border text-muted-foreground hover:bg-muted hover:text-foreground'}"
			>
				<HugeiconsIcon icon={UserMultiple02Icon} class="h-5 w-5" />
				{#if lt.role !== 'none'}
					<span
						class="absolute -right-0.5 -top-0.5 h-2.5 w-2.5 rounded-full bg-primary ring-2 ring-background"
					></span>
				{/if}
			</button>
			<form class="relative w-full max-w-xs" onsubmit={(e) => { e.preventDefault(); goSearch(); }}>
				<HugeiconsIcon
					icon={Search01Icon}
					class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground"
				/>
				<Input bind:value={searchQuery} placeholder="Search" class="rounded-full pl-9" />
			</form>
		</div>
	</div>
	{#if loading}
		<div class="flex flex-col gap-8">
			{#each Array(3) as _, s (s)}
				<section>
					<Skeleton class="mb-3 h-5 w-40 rounded" />
					<div class="flex gap-2 overflow-hidden pb-2">
						{#each Array(6) as _, i (i)}
							<div class="w-40 shrink-0"><MediaCardSkeleton /></div>
						{/each}
					</div>
				</section>
			{/each}
		</div>
	{:else if error}
		<ErrorState message={error} onRetry={load} />
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
