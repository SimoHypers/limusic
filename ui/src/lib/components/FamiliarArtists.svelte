<script lang="ts">
	// The artists you actually play, from local play counts — read as a short list on the left, and
	// drawn as a cluster of faces on the right. The cluster is the same handful of artists again in
	// picture form: it's what makes the section recognisable at a glance, which a third column of
	// text rows would not be.
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { UserLove02Icon, UserIcon, UserStar01Icon } from '@hugeicons/core-free-icons';
	import SectionHeading from './SectionHeading.svelte';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import PlaylistMenu from './PlaylistMenu.svelte';
	import * as api from '$lib/api';
	import type { ArtistPage, BrowseItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { getCached, putCached } from '$lib/pagecache';
	import { topArtistIds } from '$lib/personal';
	import { personal, toast } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	// ponytail: one browse call per artist, because the subscriber count and the subscribe state
	// only exist on the artist page. Seven of them, shared with the artist route's cache (same key),
	// fired once per mount. Cut the count before reaching for a batch endpoint that doesn't exist.
	const COUNT = 7;
	const LISTED = 5; // rows on the left; the rest live in the cluster only
	const MIN = 3; // fewer familiar artists than this and the section isn't worth a slot

	// A flower: one face in the middle, six on a ring around it at 60° steps starting from the top.
	// The diameters wobble a few px so it reads as a cluster of people, not a dial.
	//
	// The ring is an ellipse, not a circle: its height is fixed (the section can't grow taller than
	// the list beside it) while its width follows the column, so a wide screen spreads the faces out
	// instead of leaving a fixed 280px flower marooned in the middle of it.
	const BOX = 300; // the box's height, and the ring's vertical diameter budget
	const RY = 112;
	const RING = [
		{ a: -90, s: 72 },
		{ a: -30, s: 70 },
		{ a: 30, s: 68 },
		{ a: 90, s: 72 },
		{ a: 150, s: 70 },
		{ a: 210, s: 68 }
	];
	/** Measured column width — the ring's horizontal radius is derived from it. */
	let boxWidth = $state(0);
	// Floor keeps the faces off the centre one on a narrow column; the ceiling stops an ultrawide
	// window from stretching the flower into a flat line of dots.
	const rx = $derived(Math.min(Math.max(boxWidth / 2 - 46, RY), 170));
	const SLOTS = $derived([
		{ x: boxWidth / 2, y: BOX / 2, s: 96 },
		...RING.map(({ a, s }) => ({
			x: boxWidth / 2 + rx * Math.cos((a * Math.PI) / 180),
			y: BOX / 2 + RY * Math.sin((a * Math.PI) / 180),
			s
		}))
	]);

	let artists = $state<ArtistPage[]>([]);
	let loading = $state(true);
	/** Subscribe state per channel, optimistic — seeded from each artist page as it lands. */
	let subs = $state<Record<string, boolean>>({});
	let subBusy = $state<string | null>(null);
	let failed = $state<Record<string, boolean>>({});

	const ids = topArtistIds(personal, COUNT);
	const listed = $derived(artists.slice(0, LISTED));
	const cluster = $derived(artists.slice(0, SLOTS.length));

	async function fetchArtist(id: string): Promise<ArtistPage | null> {
		const key = `artist:${id}`;
		const hit = getCached<ArtistPage>(key);
		if (hit) return hit;
		try {
			const page = await api.getArtist(id);
			putCached(key, page);
			return page;
		} catch {
			return null; // one dead channel doesn't cost the section
		}
	}

	onMount(async () => {
		if (ids.length < MIN) {
			loading = false;
			return;
		}
		const pages = (await Promise.all(ids.map(fetchArtist))).filter((p): p is ArtistPage => !!p);
		// Set once, in play-count order: filling the list artist by artist would reflow the feed
		// under the reader as each request lands.
		artists = pages;
		subs = Object.fromEntries(pages.map((p) => [p.channelId, p.subscribed]));
		loading = false;
	});

	const asItem = (a: ArtistPage): BrowseItem => ({
		kind: 'artist',
		id: a.channelId,
		title: a.name ?? t('common.artist_singular'),
		subtitle: a.subscribers,
		thumbnail: a.thumbnail
	});

	const open = (a: ArtistPage) => goto(`/artist/${encodeURIComponent(a.channelId)}`);

	async function toggleSub(a: ArtistPage) {
		if (subBusy) return;
		const next = !subs[a.channelId];
		subBusy = a.channelId;
		subs = { ...subs, [a.channelId]: next };
		try {
			await api.subscribe(a.channelId, next);
			putCached(`artist:${a.channelId}`, { ...a, subscribed: next }); // keep the cache truthful
			toast.success(next ? t('artist.subscribed') : t('artist.subscribe'));
		} catch (e) {
			subs = { ...subs, [a.channelId]: !next };
			toast.error(String(e));
		} finally {
			subBusy = null;
		}
	}
</script>

{#if loading ? ids.length >= MIN : artists.length >= MIN}
	<section>
		<SectionHeading title={t('home.familiar_artists')} icon={UserStar01Icon} />
		<div class="grid items-center gap-6 md:grid-cols-2 md:gap-10">
			<div class="flex flex-col gap-1">
				{#if loading}
					{#each Array(Math.min(ids.length, LISTED)) as _, i (i)}
						<div class="flex items-center gap-3 p-1.5" aria-hidden="true">
							<Skeleton class="h-12 w-12 shrink-0 rounded-full" />
							<div class="flex min-w-0 flex-1 flex-col gap-1.5">
								<Skeleton class="h-3.5 w-32 rounded" />
								<Skeleton class="h-3 w-20 rounded" />
							</div>
						</div>
					{/each}
				{:else}
					{#each listed as a (a.channelId)}
						<div
							class="group/row flex cursor-pointer items-center gap-3 rounded-lg p-1.5 text-left transition-colors hover:bg-accent/10"
							role="button"
							tabindex="0"
							data-ctx
							onclick={() => open(a)}
							onkeydown={(e) => {
								if (e.target !== e.currentTarget) return;
								if (e.key === 'Enter' || e.key === ' ') {
									e.preventDefault();
									open(a);
								}
							}}
						>
							<div class="h-12 w-12 shrink-0 overflow-hidden rounded-full bg-muted">
								{#if a.thumbnail && !failed[a.channelId]}
									<img
										src={thumb(a.thumbnail, 400)}
										alt=""
										class="h-full w-full object-cover"
										loading="lazy"
										draggable="false"
										onerror={() => (failed = { ...failed, [a.channelId]: true })}
									/>
								{:else}
									<div
										class="flex h-full w-full items-center justify-center text-muted-foreground/50"
									>
										<HugeiconsIcon icon={UserIcon} class="h-5 w-5" />
									</div>
								{/if}
							</div>
							<div class="min-w-0 flex-1">
								<div class="truncate text-sm font-medium">{a.name ?? t('common.unknown_artist')}</div>
								<div class="truncate text-xs text-muted-foreground">
									{a.subscribers ?? t('common.artists')}
								</div>
							</div>
							<button
								class="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full transition-colors hover:bg-accent/10 {subs[
									a.channelId
								]
									? 'text-primary'
									: 'text-muted-foreground hover:text-foreground'}"
								class:animate-pulse={subBusy === a.channelId}
								aria-label={subs[a.channelId]
									? `Unsubscribe from ${a.name ?? ''}`
									: `Subscribe to ${a.name ?? ''}`}
								onclick={(e) => {
									e.stopPropagation();
									toggleSub(a);
								}}
							>
								<HugeiconsIcon icon={UserLove02Icon} class="h-5 w-5" />
							</button>
							<PlaylistMenu
								item={asItem(a)}
								showPin={false}
								vertical
								iconClass="h-5 w-5"
								triggerClass="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition hover:bg-accent/10 hover:text-foreground"
							/>
						</div>
					{/each}
				{/if}
			</div>
			<!-- The cluster is decoration that happens to be clickable: at narrow widths it would be a
			     pile of overlapping faces, so it just goes away. -->
			<div
				class="relative hidden w-full md:block"
				style="height:{BOX}px;"
				bind:clientWidth={boxWidth}
			>
				{#each (loading ? SLOTS.slice(0, COUNT) : cluster.map((_, i) => SLOTS[i])) as slot, i (i)}
					{@const a = cluster[i]}
					<div
						class="absolute -translate-x-1/2 -translate-y-1/2"
						style="left:{slot.x}px; top:{slot.y}px; width:{slot.s}px; height:{slot.s}px;"
					>
						{#if loading || !a}
							<Skeleton class="h-full w-full rounded-full" />
						{:else}
							<button
								class="h-full w-full cursor-pointer overflow-hidden rounded-full bg-muted transition-transform duration-200 ease-out hover:scale-105"
								title={a.name ?? t('common.artist_singular')}
								aria-label={a.name ?? t('common.artist_singular')}
								onclick={() => open(a)}
							>
								{#if a.thumbnail && !failed[a.channelId]}
									<img
										src={thumb(a.thumbnail, 400)}
										alt=""
										class="h-full w-full object-cover"
										loading="lazy"
										draggable="false"
										onerror={() => (failed = { ...failed, [a.channelId]: true })}
									/>
								{:else}
									<div
										class="flex h-full w-full items-center justify-center text-muted-foreground/50"
									>
										<HugeiconsIcon icon={UserIcon} class="h-7 w-7" />
									</div>
								{/if}
							</button>
						{/if}
					</div>
				{/each}
			</div>
		</div>
	</section>
{/if}
