<script lang="ts">
	// The Library page's Local tab: music that lives on this machine. Works signed out and offline,
	// because nothing here goes near YouTube (Rust `local.rs`). Albums open the normal album page
	// and songs play through the normal queue, so everything past this component is shared.
	import { onMount } from 'svelte';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { open } from '@tauri-apps/plugin-dialog';
	import {
		Add01Icon,
		Delete02Icon,
		DriveIcon,
		PlayIcon,
		RefreshIcon,
		ShuffleIcon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import * as Tabs from '$lib/components/ui/tabs';
	import MediaCard from './MediaCard.svelte';
	import MediaCardSkeleton from './MediaCardSkeleton.svelte';
	import ErrorState from './ErrorState.svelte';
	import TrackFilter from './TrackFilter.svelte';
	import TrackRow from './TrackRow.svelte';
	import * as api from '$lib/api';
	import { indexCards, indexSongs, match } from '$lib/localsearch';
	import { t } from '$lib/i18n.svelte';
	import {
		addLocalFolder,
		local,
		openPlayer,
		playback,
		removeLocalFolder,
		scanLocal,
		toast
	} from '$lib/player.svelte';

	// Files come and go while the app runs, so the tab rescans when you open it (a no-op scan is a
	// stat per file). The startup scan is what keeps deleted music out of the home grid.
	onMount(() => {
		scanLocal();
	});

	let view = $state('albums');
	// A local collection can be thousands of files, and WebKitGTK does not enjoy thousands of rows.
	// Render a page at a time — Play all and Shuffle still take every song in the list.
	const PAGE = 100;
	let shown = $state(PAGE);

	// Filtering the collection. It is all in memory already, so this is a scan and not a request:
	// no debounce, no loading state, the lists narrow as you type. The one cost worth dodging is
	// Svelte's — `local` is `$state`, so reading `song.title` for thousands of songs on every
	// keystroke goes through as many proxy traps — so `localsearch.ts` flattens the text into plain
	// strings once per library change and each keystroke scans those. Reading `ix` at all is behind
	// the empty-query check below, so a library nobody searches never pays for the pass.
	let query = $state('');
	const ix = $derived({
		songs: indexSongs(local.songs),
		albums: indexCards(local.albums),
		artists: indexCards(local.artists)
	});
	const q = $derived(query.trim());
	const songs = $derived(q ? match(ix.songs, q) : local.songs);
	const albums = $derived(q ? match(ix.albums, q) : local.albums);
	const artists = $derived(q ? match(ix.artists, q) : local.artists);

	$effect(() => {
		q; // a narrower list starts from the first page again
		shown = PAGE;
	});

	// Same shape as the playlist page and home: one page per approach to the bottom. Nothing is
	// fetched here (the whole library is already in memory), so this only grows how much of it is
	// rendered — no loading state, nothing that can fail.
	function sentinel(node: HTMLElement) {
		const io = new IntersectionObserver(([e]) => e.isIntersecting && (shown += PAGE), {
			rootMargin: '600px 0px'
		});
		io.observe(node);
		return () => io.disconnect();
	}
	const nowId = $derived(playback.now?.videoId);
	// The song list as one queue — what the Play/Shuffle buttons above it do, and what the queue
	// panel calls it. Not a `playFrom`: there's no page behind "the music on this disk", so it has
	// no business landing in recents or the sidebar's last-played order.
	const SOURCE = 'Local music';

	async function pickFolder() {
		const picked = await open({ directory: true, multiple: false, title: t('local.pick_folder_dialog') });
		if (typeof picked === 'string') await addLocalFolder(picked);
	}

	// The filtered list, not the whole library: Play all plays what the list shows.
	function playAll(shuffle: boolean) {
		if (!songs.length) return;
		openPlayer();
		api.playPlaylist(songs, null, undefined, SOURCE, shuffle);
	}

	async function forget(path: string) {
		await removeLocalFolder(path);
		toast.success(t('toasts.folder_removed'));
	}
</script>

<div class="flex flex-col gap-5">
	<!-- Folders -->
	<div class="rounded-xl border bg-card/40 p-4">
		<div class="mb-3 flex items-center justify-between gap-3">
			<div class="min-w-0">
				<div class="flex items-center gap-2 font-medium">
					<HugeiconsIcon icon={DriveIcon} class="h-4 w-4" /> Folders
				</div>
				<p class="mt-0.5 text-xs text-muted-foreground">
					Music in these folders plays without an internet connection.
				</p>
			</div>
			<div class="flex shrink-0 gap-2">
				<Button
					variant="ghost"
					size="sm"
					class="gap-2"
					disabled={local.loading || !local.folders.length}
					onclick={() => scanLocal()}
				>
					<HugeiconsIcon icon={RefreshIcon} class="h-4 w-4" />
					{local.loading ? t('local.scanning') : t('local.rescan')}
				</Button>
				<Button variant="outline" size="sm" class="gap-2" onclick={pickFolder}>
					<HugeiconsIcon icon={Add01Icon} class="h-4 w-4" /> {t('local.add_folder')}
				</Button>
			</div>
		</div>
		{#if local.folders.length}
			<ul class="flex flex-col gap-1">
				{#each local.folders as folder (folder)}
					<li class="flex items-center justify-between gap-3 rounded-lg px-2 py-1.5 hover:bg-accent/10">
						<span class="truncate font-mono text-xs text-muted-foreground" title={folder}>
							{folder}
						</span>
						<button
							class="flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition hover:bg-destructive/10 hover:text-destructive"
							aria-label={t('a11y.remove_folder')}
							onclick={() => forget(folder)}
						>
							<HugeiconsIcon icon={Delete02Icon} class="h-4 w-4" />
						</button>
					</li>
				{/each}
			</ul>
		{:else}
			<p class="text-sm text-muted-foreground">
				No folders yet. Add the one your music sits in and it shows up here.
			</p>
		{/if}
	</div>

	{#if local.error}
		<ErrorState message={local.error} onRetry={() => scanLocal()} />
	{:else if local.loading && !local.scanned}
		<div class="card-grid">
			{#each Array(6) as _, i (i)}
				<MediaCardSkeleton />
			{/each}
		</div>
	{:else if local.songs.length}
		<Tabs.Root bind:value={view}>
			<!-- The counts follow the filter, so the tabs say where the matches are. -->
			<div class="mb-4 flex flex-wrap items-center justify-between gap-3">
				<Tabs.List>
					<Tabs.Trigger value="albums">{t('local.albums_count', { count: albums.length })}</Tabs.Trigger>
					<Tabs.Trigger value="artists">{t('local.artists_count', { count: artists.length })}</Tabs.Trigger>
					<Tabs.Trigger value="songs">{t('local.songs_count', { count: songs.length })}</Tabs.Trigger>
				</Tabs.List>
				<TrackFilter bind:value={query} placeholder={t('common.search_your_music')} />
			</div>
			{#if q && !songs.length && !albums.length && !artists.length}
				<p class="text-sm text-muted-foreground">{t('local.nothing_matches_device', { query: q })}</p>
			{/if}
			<!-- Gated on `view`: bits-ui hides an inactive panel rather than unmounting it, so all three
			     views of the same collection would be built on every visit. -->
			<Tabs.Content value="albums">
				{#if view === 'albums'}
					<div class="card-grid content-in">
						{#each albums as album (album.id)}
							<MediaCard item={album} />
						{/each}
					</div>
				{/if}
			</Tabs.Content>
			<Tabs.Content value="artists">
				{#if view === 'artists'}
					<div class="card-grid content-in">
						{#each artists as artist (artist.id)}
							<MediaCard item={artist} />
						{/each}
					</div>
				{/if}
			</Tabs.Content>
			<Tabs.Content value="songs">
				{#if view === 'songs'}
					<div class="mb-3 flex gap-2">
						<Button
							size="sm"
							class="gap-2 rounded-full"
							disabled={!songs.length}
							onclick={() => playAll(false)}
						>
							<HugeiconsIcon icon={PlayIcon} class="h-4 w-4" /> Play all
						</Button>
						<Button
							size="sm"
							variant="outline"
							class="gap-2 rounded-full"
							disabled={!songs.length}
							onclick={() => playAll(true)}
						>
							<HugeiconsIcon icon={ShuffleIcon} class="h-4 w-4" /> Shuffle
						</Button>
					</div>
					<div class="content-in">
						{#each songs.slice(0, shown) as song, i (song.video_id)}
							<TrackRow
								{song}
								index={i}
								active={song.video_id === nowId}
								onplay={() => {
									openPlayer();
									api.playPlaylist(songs, i, undefined, SOURCE);
								}}
							/>
						{/each}
					</div>
					{#if songs.length > shown}
						<div {@attach sentinel}></div>
					{/if}
				{/if}
			</Tabs.Content>
		</Tabs.Root>
	{:else if local.folders.length}
		<p class="text-sm text-muted-foreground">
			Nothing playable found in those folders yet. Looking for mp3, flac, m4a, aac, ogg, opus,
			wav, wma, aiff, ape, wv and mka files.
		</p>
	{/if}
</div>
