<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		ShuffleIcon,
		Radio02Icon,
		Add01Icon,
		Tick02Icon,
		MoreVerticalIcon,
		DashboardSquare02Icon,
		Share08Icon,
		BookmarkAdd02Icon,
		BookmarkCheck02Icon,
		ArrowRight01Icon
	} from '@hugeicons/core-free-icons';
	import MediaCardSkeleton from '$lib/components/MediaCardSkeleton.svelte';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import TrackRowSkeleton from '$lib/components/TrackRowSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import Shelf from '$lib/components/Shelf.svelte';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import * as api from '$lib/api';
	import type { ArtistPage, BrowseItem, PlaylistPage } from '$lib/api';
	import {
		addPick,
		openShare,
		auth,
		isSaved,
		playback,
		openAddToPlaylist,
		playFrom,
		startRadio,
		toast,
		noteLibrary,
		toggleSaved
	} from '$lib/player.svelte';
	import { getCached, putCached } from '$lib/pagecache';
	import { anchorMenu, fitMenu, NO_ANCHOR } from '$lib/menu';
	import { t } from '$lib/i18n.svelte';

	let artist = $state<ArtistPage | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let expanded = $state(false);
	let subscribed = $state(false);
	let subBusy = $state(false);
	let shuffleBusy = $state(false);

	const id = $derived(page.params.id ?? '');
	const nowId = $derived(playback.now?.videoId);
	// Saved to the library on this machine (the signed-out counterpart of subscribing).
	const savedHere = $derived(isSaved(id));

	async function load(cid: string) {
		const key = `artist:${cid}`;
		const hit = getCached<ArtistPage>(key);
		if (hit) {
			artist = hit;
			subscribed = hit.subscribed;
			loading = false;
		} else {
			loading = true;
			artist = null;
		}
		error = null;
		expanded = false;
		try {
			const fresh = await api.getArtist(cid);
			if (cid !== id) return; // superseded by navigation — drop the stale response
			artist = fresh;
			subscribed = fresh.subscribed;
			putCached(key, fresh);
		} catch (e) {
			if (cid !== id) return;
			if (!hit) error = String(e);
		} finally {
			if (cid === id) loading = false;
		}
	}

	$effect(() => {
		if (id) load(id);
	});

	// ⋯ options menu, positioned `fixed` at the button so it isn't clipped (matches the album page).
	let menuOpen = $state(false);
	let anchor = $state(NO_ANCHOR);

	function openMenu(e: MouseEvent) {
		anchor = anchorMenu(e);
		menuOpen = true;
	}

	// This artist as a card, for the sidebar's last-played sort and the Shortcuts grid.
	const asItem = (): BrowseItem => ({
		kind: 'artist',
		id,
		title: artist?.name ?? t('common.artist_singular'),
		subtitle: artist?.subscribers,
		thumbnail: artist?.thumbnail
	});

	// Shuffles the whole top-songs playlist (what "See all" opens), not just the 5 rows on this
	// page. The backend walks the continuation in the background, so playback starts immediately
	// on the first ~100. Artists without that playlist fall back to the visible rows.
	async function shuffle() {
		if (!artist || shuffleBusy) return;
		const pid = artist.topSongsId;
		if (pid) {
			const cid = id;
			shuffleBusy = true;
			try {
				const key = `playlist:${pid}`;
				const pl = getCached<PlaylistPage>(key) ?? (await api.getPlaylist(pid));
				if (cid !== id) return; // navigated away mid-fetch
				putCached(key, pl);
				if (pl.items.length) {
					playFrom(asItem(), pl.items, null, pid, true, pl.continuation);
					return;
				}
			} catch (e) {
				toast.error(String(e));
				return;
			} finally {
				shuffleBusy = false;
			}
		}
		if (artist.topSongs.length) playFrom(asItem(), artist.topSongs, null, undefined, true);
	}

	async function toggleSub() {
		if (!artist || subBusy) return;
		// Everything below the await names the artist this click was about, not whoever the page is
		// showing by the time YouTube answers (same guard as `shuffle`).
		const who = artist;
		const cid = id;
		const item = asItem();
		const next = !subscribed;
		subBusy = true;
		subscribed = next; // optimistic
		try {
			await api.subscribe(who.channelId, next);
			putCached(`artist:${cid}`, { ...who, subscribed: next }); // keep the cache truthful
			noteLibrary(item, next); // so every card's menu agrees the artist is in the library
			toast.success(next ? `Subscribed to ${who.name ?? ''}` : `Unsubscribed`);
		} catch (e) {
			if (cid === id) subscribed = !next; // revert, unless we've since left that artist
			toast.error(String(e));
		} finally {
			subBusy = false;
		}
	}

	function showMore(section: { title: string; moreBrowseId?: string; moreParams?: string }) {
		const q = new URLSearchParams({ id: section.moreBrowseId!, title: section.title });
		if (section.moreParams) q.set('params', section.moreParams);
		goto(`/list?${q.toString()}`);
	}
</script>

{#if loading}
	<div class="relative flex min-h-[45vh] flex-col justify-end overflow-hidden border-b">
		<Skeleton class="absolute inset-0 h-full w-full rounded-none" />
		<div class="relative space-y-4 p-8">
			<Skeleton class="h-12 w-1/2 rounded-lg" />
			<Skeleton class="h-4 w-40 rounded" />
			<div class="flex gap-3">
				<Skeleton class="h-11 w-28 rounded-full" />
				<Skeleton class="h-11 w-32 rounded-full" />
			</div>
		</div>
	</div>
	<div class="flex flex-col gap-8 p-6">
		<section>
			<Skeleton class="mb-3 h-6 w-32 rounded" />
			{#each Array(5) as _, i (i)}
				<TrackRowSkeleton />
			{/each}
		</section>
		<section>
			<Skeleton class="mb-3 h-6 w-40 rounded" />
			<div class="flex gap-2 overflow-hidden pb-2">
				{#each Array(6) as _, i (i)}
					<div class="w-40 shrink-0"><MediaCardSkeleton /></div>
				{/each}
			</div>
		</section>
	</div>
{:else if error}
	<div class="p-6"><ErrorState message={error} onRetry={() => load(id)} /></div>
{:else if artist}
	<!-- Hero -->
	<div class="content-in relative flex min-h-[45vh] flex-col justify-end overflow-hidden">
		{#if artist.thumbnail}
			<img src={artist.thumbnail} alt="" class="absolute inset-0 h-full w-full object-cover" />
		{/if}
		<div
			class="absolute inset-0 bg-gradient-to-t from-background via-background/60 to-background/10"
		></div>
		<div class="relative max-w-3xl p-8">
			<h1 class="font-heading text-5xl font-bold tracking-tight drop-shadow-lg">{artist.name}</h1>
			{#if artist.subscribers || artist.monthlyListeners}
				<p class="mt-2 text-sm text-muted-foreground">
					{#if artist.subscribers}{artist.subscribers}{/if}
					{#if artist.subscribers && artist.monthlyListeners}<br />{/if}
					{#if artist.monthlyListeners}{artist.monthlyListeners}{/if}
				</p>
			{/if}
			{#if artist.description}
				<p class="mt-3 max-w-2xl text-sm text-foreground/80 {expanded ? '' : 'line-clamp-2'}">
					{artist.description}
				</p>
				<button
					class="mt-1 text-xs font-semibold uppercase text-muted-foreground hover:text-foreground"
					onclick={() => (expanded = !expanded)}
				>
					{expanded ? t('common.less') : t('common.more')}
				</button>
			{/if}
			<div class="mt-5 flex items-center gap-3">
				<button
					class="flex items-center gap-2 rounded-full bg-foreground px-5 py-2.5 text-sm font-semibold text-background transition hover:opacity-90 disabled:opacity-50"
					onclick={shuffle}
					disabled={!artist.topSongs.length || shuffleBusy}
				>
					<HugeiconsIcon icon={ShuffleIcon} class="h-4 w-4" /> {t('common.shuffle')}
				</button>
				<!-- The deep counterpart to Shuffle above: that one is the finite top-songs playlist,
				     this one asks YouTube for the artist's own endless mix. -->
				<button
					class="flex cursor-pointer items-center gap-2 rounded-full border px-5 py-2.5 text-sm font-semibold transition hover:bg-accent/10"
					onclick={() => startRadio('artist', id, artist?.name)}
				>
					<HugeiconsIcon icon={Radio02Icon} class="h-4 w-4" /> {t('common.radio')}
				</button>
				<!-- Subscribing is a YouTube write action. Signed out, the same slot saves the artist to
				     the local library instead of offering a button that can only fail. -->
				{#if auth.account?.signedIn}
					<button
						class="flex items-center gap-2 rounded-full border px-5 py-2.5 text-sm font-semibold transition hover:bg-accent/10 disabled:opacity-60 {subscribed
							? 'border-primary text-primary'
							: ''}"
						onclick={toggleSub}
						disabled={subBusy}
					>
						<HugeiconsIcon icon={Add01Icon} altIcon={Tick02Icon} showAlt={subscribed} class="h-4 w-4" />
						{subscribed ? t('artist.subscribed') : t('artist.subscribe')}
					</button>
				{:else}
					<button
						class="flex cursor-pointer items-center gap-2 rounded-full border px-5 py-2.5 text-sm font-semibold transition hover:bg-accent/10 {savedHere
							? 'border-primary text-primary'
							: ''}"
						onclick={() =>
							toast.success(
								toggleSaved(asItem()) ? t('library.in_library') : t('common.remove')
							)}
					>
						<HugeiconsIcon
							icon={BookmarkAdd02Icon}
							altIcon={BookmarkCheck02Icon}
							showAlt={savedHere}
							class="h-4 w-4"
						/>
						{savedHere ? t('library.in_library') : t('library.save_to_library')}
					</button>
				{/if}
				<button
					class="flex h-10 w-10 cursor-pointer items-center justify-center rounded-full border text-muted-foreground transition hover:bg-accent/10 hover:text-foreground"
					onclick={openMenu}
					aria-label={t('common.more')}
				>
					<HugeiconsIcon icon={MoreVerticalIcon} class="h-5 w-5" />
				</button>
			</div>
		</div>
	</div>

	<div class="content-in flex flex-col gap-8 p-6">
		{#if artist.topSongs.length}
			<section>
				<!-- Same header shape as Shelf: title and "See all" both navigate. -->
				<div class="mb-3 flex items-baseline justify-between gap-3">
					{#if artist.topSongsId}
						<button
							class="min-w-0 cursor-pointer text-left hover:underline"
							onclick={() => goto(`/playlist/${artist!.topSongsId}`)}
							title={t('common.see_all')}
						>
							<h2 class="truncate font-heading text-xl font-bold">{t('artist.top_songs')}</h2>
						</button>
						<button
							class="flex shrink-0 cursor-pointer items-center gap-0.5 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
							onclick={() => goto(`/playlist/${artist!.topSongsId}`)}
						>
							{t('common.see_all')}
							<HugeiconsIcon icon={ArrowRight01Icon} class="h-3.5 w-3.5" />
						</button>
					{:else}
						<h2 class="truncate font-heading text-xl font-bold">{t('artist.top_songs')}</h2>
					{/if}
				</div>
				{#each artist.topSongs as song, i (song.video_id + i)}
					<TrackRow
						{song}
						showPlayCount
						active={song.video_id === nowId}
						onplay={() => playFrom(asItem(), artist!.topSongs, i)}
						onAdd={() => openAddToPlaylist(song)}
					/>
				{/each}
			</section>
		{/if}

		{#each artist.sections as section, i (i + ':' + section.title)}
			<Shelf
				title={section.title}
				items={section.items}
				headingClass="font-heading text-xl font-bold"
				onMore={section.moreBrowseId ? () => showMore(section) : undefined}
			/>
		{/each}
	</div>
{/if}

{#if menuOpen}
	<button
		class="fixed inset-0 z-40 cursor-default"
		onclick={() => (menuOpen = false)}
		oncontextmenu={(e) => {
			e.preventDefault();
			menuOpen = false;
		}}
		aria-label={t('common.close')}
	></button>
	<div
		class="fixed z-50 min-w-52 animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style={anchor.style}
		{@attach fitMenu(anchor)}
	>
		<button
			class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={() => {
				menuOpen = false;
				addPick(asItem());
			}}
		>
			<HugeiconsIcon icon={DashboardSquare02Icon} class="h-4 w-4" /> {t('home.add_shortcut')}
		</button>
		<button
			class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={() => {
				menuOpen = false;
				openShare(asItem());
			}}
		>
			<HugeiconsIcon icon={Share08Icon} class="h-4 w-4" /> {t('player.share')}
		</button>
	</div>
{/if}
