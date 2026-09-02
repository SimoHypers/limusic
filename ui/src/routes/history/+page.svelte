<script lang="ts">
	// YouTube Music's play history (`FEmusic_history`), in its own day buckets.
	//
	// Deliberately not YouTube Music's flat table: the day is the thing you actually navigate by, so
	// each bucket keeps a sticky heading with its own count and the rows sit under it as ordinary
	// TrackRows. One queue is built from every bucket at once, so Play all runs the whole history.
	import { onMount } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { HistoryIcon, PlayIcon, ShuffleIcon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import TrackFilter, { filterTracks } from '$lib/components/TrackFilter.svelte';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import TrackRowSkeleton from '$lib/components/TrackRowSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import * as api from '$lib/api';
	import type { HistoryGroup, SongItem } from '$lib/api';
	import { getCached, putCached } from '$lib/pagecache';
	import { thumb } from '$lib/thumb';
	import { auth, openAddToPlaylist, openPlayer, playback } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	const KEY = 'history';

	// `$state.raw` for the same reason the playlist page uses it: a deep proxy puts every read of
	// every row through a trap, and a year of listening is a lot of rows.
	let groups = $state.raw<HistoryGroup[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let query = $state('');

	const flat = $derived(groups.flatMap((g) => g.items));
	const filtering = $derived(!!query.trim());
	const matched = $derived(
		groups
			.map((g) => ({ title: g.title, items: filterTracks(g.items, query) }))
			.filter((g) => g.items.length)
	);
	const matchCount = $derived(matched.reduce((n, g) => n + g.items.length, 0));
	const nowId = $derived(playback.now?.videoId);

	// Rendered a slice at a time (WebKitGTK and hundreds of rows do not get along), counted across
	// buckets so the budget is rows and not days.
	// ponytail: a running count, not the windowing in `rows.ts` — this page scrolls with `main`.
	const PAGE = 80;
	let shown = $state(PAGE);
	$effect(() => {
		query; // a narrower list starts from the first page again
		shown = PAGE;
	});
	const visible = $derived.by(() => {
		let left = shown;
		const out: { title: string; items: SongItem[] }[] = [];
		for (const g of matched) {
			if (left <= 0) break;
			out.push({ title: g.title, items: g.items.slice(0, left) });
			left -= g.items.length;
		}
		return out;
	});

	// Distinct covers for the header stack: an afternoon spent on one album would otherwise draw the
	// same sleeve five times.
	const covers = $derived([...new Set(flat.slice(0, 40).flatMap((s) => (s.thumbnail ? [s.thumbnail] : [])))]);

	// A rewritten thumbnail size Google's CDN doesn't serve 404s, and a decorative backdrop has to
	// degrade to nothing rather than a broken-image glyph (same guard as HomeHero).
	let artFailed = $state(false);
	$effect(() => {
		covers[0]; // re-arm when the artwork changes
		artFailed = false;
	});

	onMount(() => {
		const cached = getCached<HistoryGroup[]>(KEY);
		if (cached) {
			groups = cached;
			loading = false;
			return;
		}
		load();
	});

	async function load() {
		loading = true;
		error = null;
		try {
			groups = await api.getHistory();
			putCached(KEY, groups);
		} catch (e) {
			error = String(e);
		} finally {
			loading = false;
		}
	}

	function sentinel(node: HTMLElement) {
		const io = new IntersectionObserver(([e]) => e.isIntersecting && (shown += PAGE), {
			rootMargin: '600px 0px'
		});
		io.observe(node);
		return () => io.disconnect();
	}

	// The whole history in order, never the filtered view: a filter finds a song, it doesn't decide
	// what plays after it. Not a `playFrom` either — there is no page behind "what you listened to",
	// so it has no business landing in recents.
	function play(start: number | null, shuffle = false) {
		if (!flat.length) return;
		openPlayer();
		api.playPlaylist(flat, start, undefined, t('history.title'), shuffle);
	}
</script>

<div class="p-6">
	{#if loading}
		<div class="mb-4 h-36 animate-pulse rounded-2xl border bg-card/40"></div>
		{#each Array(8) as _, i (i)}
			<TrackRowSkeleton />
		{/each}
	{:else if error}
		<ErrorState message={error} onRetry={load} />
	{:else}
		<!-- The same rounded band the Library ▸ Songs tab wears, tinted by the last thing played. -->
		<div class="relative mb-6 overflow-hidden rounded-2xl border">
			{#if covers[0] && !artFailed}
				<!-- 96px: blur-2xl throws away every detail bigger than a few pixels anyway (HomeHero). -->
				<img
					src={thumb(covers[0], 96)}
					alt=""
					class="pointer-events-none absolute inset-0 h-full w-full art-wash scale-110 object-cover opacity-60 blur-2xl"
					onerror={() => (artFailed = true)}
				/>
			{/if}
			<div
				class="absolute inset-0 bg-gradient-to-r from-background via-background/80 to-background/40"
			></div>
			<div class="relative flex flex-wrap items-center gap-4 p-4">
				{#if covers.length}
					<!-- An overlapping strip rather than a single cover: history has no artwork of its own,
					     and the last few sleeves say what the page holds better than any icon. -->
					<div class="flex shrink-0 items-center pl-1">
						{#each covers.slice(0, 5) as cover, i (cover)}
							<img
								src={thumb(cover, 400)}
								alt=""
								style="z-index:{5 - i}"
								class="relative -ml-5 h-20 w-20 rounded-xl object-cover shadow-lg ring-2 ring-background first:ml-0"
							/>
						{/each}
					</div>
				{:else}
					<div
						class="flex h-20 w-20 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary"
					>
						<HugeiconsIcon icon={HistoryIcon} class="h-8 w-8" />
					</div>
				{/if}
				<div class="min-w-0 flex-1">
					<h1 class="font-heading text-2xl font-bold tracking-tight">{t('history.title')}</h1>
					<p class="mt-0.5 text-sm text-muted-foreground">
						{filtering
							? t('history.matching', { count: matchCount.toLocaleString() })
							: t('history.subtitle')}
					</p>
					<div class="mt-3 flex flex-wrap items-center gap-2">
						<Button class="gap-2 rounded-full" disabled={!flat.length} onclick={() => play(null, true)}>
							<HugeiconsIcon icon={ShuffleIcon} class="h-4 w-4" /> {t('common.shuffle_all')}
						</Button>
						<Button
							variant="outline"
							class="gap-2 rounded-full"
							disabled={!flat.length}
							onclick={() => play(0)}
						>
							<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" /> {t('common.play_all')}
						</Button>
					</div>
				</div>
				{#if flat.length}
					<TrackFilter bind:value={query} placeholder={t('history.search')} />
				{/if}
			</div>
		</div>

		{#if visible.length}
			<div class="content-in">
				{#each visible as group (group.title)}
					<!-- Sticky per bucket, so the day you are reading stays named while you scroll it. The
					     blur is what keeps the rows legible as they pass underneath. -->
					<h2
						class="sticky top-0 z-10 mb-1 flex items-baseline gap-3 bg-background/85 py-2 backdrop-blur"
					>
						<span class="font-heading text-lg font-bold tracking-tight">{group.title}</span>
						<span class="h-px flex-1 bg-border"></span>
						<span class="text-xs text-muted-foreground">
							{t('history.songs_count', { count: group.items.length.toLocaleString() })}
						</span>
					</h2>
					<!-- Keyed on the id *and* the position: the same song can be played twice in a day. -->
					{#each group.items as song, i (song.video_id + i)}
						<TrackRow
							{song}
							active={song.video_id === nowId}
							onplay={() => play(flat.indexOf(song))}
							onAdd={() => openAddToPlaylist(song)}
						/>
					{/each}
				{/each}
			</div>
			{#if shown < matchCount}<div {@attach sentinel}></div>{/if}
		{:else}
			<p class="text-sm text-muted-foreground">
				{filtering
					? t('common.no_matches')
					: auth.account?.signedIn
						? t('history.empty')
						: t('history.signed_out')}
			</p>
		{/if}
	{/if}
</div>
