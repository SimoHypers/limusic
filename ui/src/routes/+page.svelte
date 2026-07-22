<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Search01Icon, UserMultiple02Icon } from '@hugeicons/core-free-icons';
	import { Input } from '$lib/components/ui/input';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import { Button } from '$lib/components/ui/button';
	import MediaCardSkeleton from '$lib/components/MediaCardSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import QuickPicks from '$lib/components/QuickPicks.svelte';
	import Shelf from '$lib/components/Shelf.svelte';
	import * as api from '$lib/api';
	import type { BrowseItem, HomeChip, HomePage } from '$lib/api';
	import { auth, personal, ui, toast } from '$lib/player.svelte';
	import { interleave, recentItems, topArtists } from '$lib/personal';
	import { lt } from '$lib/lt.svelte';
	import { getCached, putCached } from '$lib/pagecache';

	// Fixed at mount — a greeting that flips mid-session is uncanny.
	const hour = new Date().getHours();
	const daypart =
		hour < 5 ? 'Good night' : hour < 12 ? 'Good morning' : hour < 18 ? 'Good afternoon' : 'Good evening';

	let home = $state<HomePage | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let searchQuery = $state('');
	// The mood chips + which one is active. Kept out of `home` so the row survives a filter switch's
	// loading state (every home response carries the same chips anyway). Limusic is music-only.
	let chips = $state<HomeChip[]>([]);
	let selected = $state<string | null>(null);
	let loadingMore = $state(false);
	const recent = $derived(recentItems(personal));

	function goSearch() {
		if (!searchQuery.trim()) return;
		goto(`/search?${new URLSearchParams({ q: searchQuery }).toString()}`);
	}

	function showMore(section: { title: string; moreBrowseId?: string; moreParams?: string }) {
		const q = new URLSearchParams({ id: section.moreBrowseId!, title: section.title });
		if (section.moreParams) q.set('params', section.moreParams);
		goto(`/list?${q.toString()}`);
	}

	async function load(params: string | null = selected) {
		selected = params;
		const key = params ? `home:${params}` : 'home';
		const hit = getCached<HomePage>(key);
		if (hit) {
			home = hit;
			loading = false;
			cater(hit, params);
		} else {
			loading = true;
		}
		error = null;
		try {
			const fresh = await api.getHome(params ?? undefined);
			// A stale response from a chip the user already clicked away from must not win.
			if (selected !== params) return;
			home = fresh;
			putCached(key, fresh);
			cater(fresh, params);
		} catch (e) {
			if (!hit) error = String(e);
		} finally {
			loading = false;
		}
	}

	async function loadMore() {
		const token = home?.continuation;
		if (!token || loadingMore) return;
		loadingMore = true;
		const params = selected; // guard against chip switches mid-flight
		try {
			const more = await api.getHomeMore(token);
			if (selected !== params || home?.continuation !== token) return; // stale
			home = {
				...home!,
				sections: [...home!.sections, ...more.sections],
				continuation: more.continuation
			};
		} catch (e) {
			toast('Could not load more');
		} finally {
			loadingMore = false;
		}
	}

	/**
	 * YouTube's "From the community" shelf is already account-personalized, but it isn't tied to what
	 * the user actually plays *in Limusic*. Swap its items for community playlists searched from
	 * their top artists, keeping the shelf's title and position. With no listening signal yet — or if
	 * the searches fail — YouTube's own items are left exactly as they came. Best-effort: this can
	 * never fail the page.
	 */
	async function cater(page: HomePage, params: string | null) {
		if (params) return; // a mood-filtered feed is the chip's, not the user's
		const idx = page.sections.findIndex((s) => /community/i.test(s.title));
		if (idx < 0) return;
		const artists = topArtists(personal, 3);
		if (!artists.length) return;
		const key = `community:${artists.join('|')}`;
		let items = getCached<BrowseItem[]>(key);
		if (!items) {
			const lists = await Promise.all(
				artists.map((a) => api.searchCards(a, 'playlists').catch(() => [] as BrowseItem[]))
			);
			items = interleave(lists, 20);
			if (!items.length) return;
			putCached(key, items);
		}
		// Same race guard as load(): a chip switch or a fresh response may have landed meanwhile.
		if (selected !== params || home !== page) return;
		home = { ...page, sections: page.sections.map((s, i) => (i === idx ? { ...s, items } : s)) };
	}

	// Chips only refresh when a response actually carries them (never blank the row mid-switch).
	$effect(() => {
		if (home?.chips?.length) chips = home.chips.filter((c) => c.title !== 'Podcasts');
	});

	onMount(() => load(null));
</script>

<div class="p-6">
	<div class="mb-6 flex items-center justify-between gap-4">
		<h1 class="font-heading text-2xl font-bold">
			{daypart}{auth.account?.name ? `, ${auth.account.name.split(' ')[0]}` : ''}
		</h1>
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
	<!-- Mood chips stay pinned directly under the header — they filter the whole feed, so they read as
	     page-level controls and must not sit below content they act on. -->
	{#if chips.length}
		<div class="mb-6 flex gap-2 overflow-x-auto pb-2">
			{#each chips as chip (chip.params)}
				<button
					onclick={() => load(selected === chip.params ? null : chip.params)}
					class="shrink-0 cursor-pointer rounded-lg px-3 py-1.5 text-sm font-medium transition-colors {selected ===
					chip.params
						? 'bg-foreground text-background'
						: 'bg-muted text-foreground hover:bg-muted/70'}"
				>
					{chip.title}
				</button>
			{/each}
		</div>
	{/if}
	<!-- Quick Picks is the user's own grid, not part of the filterable feed, so it steps aside while a
	     mood filter is active. -->
	{#if !selected}
		<QuickPicks />
		{#if recent.length}
			<div class="mb-8">
				<Shelf title="Jump back in" items={recent} />
			</div>
		{/if}
	{/if}
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
		<div class="content-in flex flex-col gap-8">
			{#each home.sections as section, i (i + ':' + section.title)}
				<Shelf
					title={section.title}
					items={section.items}
					onMore={section.moreBrowseId ? () => showMore(section) : undefined}
				/>
			{/each}
			{#if home.continuation}
				<div class="p-3 text-center">
					<Button variant="outline" size="sm" onclick={loadMore} disabled={loadingMore}>
						{loadingMore ? 'Loading…' : 'Show more'}
					</Button>
				</div>
			{/if}
		</div>
	{:else}
		<p class="text-sm text-muted-foreground">
			Nothing here yet.
			{auth.account?.signedIn ? '' : 'Sign in to see your personalized home feed.'}
		</p>
	{/if}
</div>
