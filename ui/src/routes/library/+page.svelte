<script module lang="ts">
	// Module scope, so returning to the library (back from an album you opened, or via the sidebar)
	// keeps the tab you were on instead of snapping to All.
	let lastTab = 'all';
</script>

<script lang="ts">
	import { onMount, untrack } from 'svelte';
	import { page } from '$app/state';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Add01Icon,
		CloudSyncIcon,
		CloudUploadIcon,
		DriveIcon,
		MusicNote01Icon,
		MusicNoteSquare02Icon,
		Playlist02Icon,
		SpotifyIcon,
		SquareStackIcon,
		UserCheck01Icon,
		UserSharingIcon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import * as Tabs from '$lib/components/ui/tabs';
	import * as Tooltip from '$lib/components/ui/tooltip';
	import LibrarySongs from '$lib/components/LibrarySongs.svelte';
	import LocalMusic from '$lib/components/LocalMusic.svelte';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import MediaCardSkeleton from '$lib/components/MediaCardSkeleton.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import type { BrowseItem } from '$lib/api';
	import {
		auth,
		personal,
		toast,
		library,
		loadLibrary,
		loadLibraryExtras,
		loadUploadAlbums,
		openNewPlaylist,
		syncSavedToYouTube
	} from '$lib/player.svelte';
	import { mergeSaved, unsynced } from '$lib/personal';
	import { reveal } from '$lib/reveal.svelte';
	import { t } from '$lib/i18n.svelte';
	import { openImport } from '$lib/import.svelte';
	import ExperimentalBadge from '$lib/components/ExperimentalBadge.svelte';

	// `?tab=local` so anything that sends you back here (an album whose files were deleted) lands
	// on the tab you came from instead of a sign-in prompt.
	let tab = $state(page.url.searchParams.get('tab') ?? lastTab);
	$effect(() => {
		lastTab = tab;
	});
	// Uploads splits three ways (all / songs / albums): YouTube Music takes uploaded albums too, and
	// they are a card grid rather than rows, so they can't just join the track list.
	let uploadTab = $state('all');
	// Artists splits like YouTube Music's: the artists behind your songs, or the ones you subscribe
	// to. Only the second can be taken out of the library (an unsubscribe); the first leaves with
	// its songs.
	let artistTab = $state('artists');

	// Everything here lives in the shared `library` store, so a revisit renders the cached grid
	// immediately and the forced refresh below swaps in fresh data behind it. What was saved on this
	// machine merges in per tab (`mergeSaved`), which is the whole library when signed out.
	const playlists = $derived(mergeSaved(personal, library.items, 'playlist'));
	const albums = $derived(mergeSaved(personal, library.albums, 'album'));
	const artists = $derived(mergeSaved(personal, library.artists, 'artist'));
	const subscriptions = $derived(mergeSaved(personal, library.subscriptions, 'artist'));
	const all = $derived([...playlists, ...albums, ...artists]);
	// One per tab rather than one shared instance reset on switch: an `$effect` reset lands
	// after the render it is meant to govern, so switching tabs would build the new tab's grid
	// against the old tab's count and immediately tear the excess back down. A tab keeping its
	// own depth also means coming back to one lands where you left it.
	const rvAll = reveal();
	const rvPlaylists = reveal();
	const rvAlbums = reveal();
	const rvArtists = reveal();
	const rvSubscriptions = reveal();
	const rvUploadsAll = reveal();
	const rvUploadAlbums = reveal();
	const loading = $derived((library.loading || library.extrasLoading) && !all.length);
	const error = $derived(library.error ?? library.extrasError);
	// Only the empty states differ: signed out there is no account library to be missing yet.
	const signedOut = $derived(!auth.account?.signedIn);
	// What the sync button has left to push. Synced rows stay in the local library (they are what
	// the user still has after signing out), so counting all of `personal.saved` would nag forever.
	const toSync = $derived(unsynced(personal));

	onMount(load);

	// Only when the tab is opened: most accounts have no uploads at all, so this stays off the
	// Library's own load. `untrack` because the loader writes the very state it reads to decide
	// whether to run, which would otherwise re-trigger this effect forever.
	$effect(() => {
		if (tab === 'uploads') untrack(() => loadUploadAlbums());
	});

	function load() {
		loadLibrary(true);
		loadLibraryExtras(true);
	}

	let syncing = $state(false);
	async function sync() {
		if (syncing) return;
		syncing = true;
		const n = toSync.length;
		try {
			const { synced, failed } = await syncSavedToYouTube();
			if (failed && synced) toast(t('toasts.synced_partial', { synced, total: n, failed }));
			else if (failed) toast.error(t('toasts.synced_none', { failed }));
			else toast.success(t('toasts.synced_all', { count: synced }));
		} catch (e) {
			toast.error(String(e));
		} finally {
			syncing = false;
		}
	}
</script>

{#snippet grid(items: BrowseItem[], empty: string, rv: ReturnType<typeof reveal>, nudge = false)}
	{#if items.length}
		<div class="card-grid content-in">
			{#each items.slice(0, rv.count(items.length)) as item (item.kind + item.id)}
				<MediaCard {item} />
			{/each}
		</div>
		<!-- Outside the grid, or it would be laid out as a cell. -->
		{#if rv.more(items.length)}<div {@attach rv.sentinel}></div>{/if}
	{:else}
		<p class="text-sm text-muted-foreground">{empty}</p>
		<!-- An empty library is most likely a new one, and a new one is often someone coming from
		     Spotify (#375). Gone as soon as there is anything here, so it never needs dismissing. -->
		{#if nudge}
		<button
			class="mt-4 flex max-w-md cursor-pointer items-center gap-4 rounded-2xl border p-4 text-left transition-colors hover:bg-accent/10"
			onclick={() => openImport()}
		>
			<HugeiconsIcon icon={SpotifyIcon} class="h-8 w-8 shrink-0 text-primary" />
			<span>
				<span class="flex items-center gap-2 text-sm font-medium">
					{t('import.nudge_title')}
					<ExperimentalBadge />
				</span>
				<span class="block text-xs text-muted-foreground">{t('import.nudge_desc')}</span>
			</span>
		</button>
		{/if}
	{/if}
{/snippet}

<div class="p-6">
	<div class="mb-6 flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
		<h1 class="min-w-0 font-heading text-2xl font-bold">{t('library.title')}</h1>
		<!-- Wraps onto its own line below sm rather than pushing the buttons off the page. -->
		<div class="flex items-center gap-2 max-sm:w-full max-sm:justify-end">
			<!-- Only with something to push: saves made before signing in, which live on this
			     machine until this button puts them on the account. -->
			{#if auth.account?.signedIn && toSync.length}
				<!-- A cloud glyph with a number on it says nothing about what pressing it does, and
				     that's a write to someone's YouTube account. Hence a real tooltip rather than the
				     `title` this app uses elsewhere: it has to be read before the click, not after a
				     second of hovering. `child` keeps our own Button as the trigger element. -->
				<Tooltip.Provider delayDuration={150}>
					<Tooltip.Root>
						<Tooltip.Trigger>
							{#snippet child({ props })}
								<Button
									{...props}
									variant="outline"
									size="icon-sm"
									onclick={sync}
									disabled={syncing}
									aria-label={t('a11y.sync_to_ytm', { count: toSync.length })}
								>
									<span class="relative">
										<HugeiconsIcon
											icon={CloudSyncIcon}
											class="h-4 w-4 {syncing ? 'animate-pulse' : ''}"
										/>
										<!-- ring-background so the count reads over the icon's stroke (as in
										     Titlebar). -->
										<span
											class="absolute -right-2 -top-1.5 min-w-3.5 rounded-full bg-accent px-[3px] text-[9px] font-semibold leading-[0.875rem] text-accent-foreground ring-[1.5px] ring-background"
										>
											{toSync.length}
										</span>
									</span>
								</Button>
							{/snippet}
						</Tooltip.Trigger>
						<Tooltip.Content side="bottom">
							{syncing
								? t('common.loading')
								: t('library.sync_idle_tooltip', { count: toSync.length })}
						</Tooltip.Content>
					</Tooltip.Root>
				</Tooltip.Provider>
			{/if}
			<Button
				variant="outline"
				size="sm"
				class="gap-2"
				title={t('import.button_tooltip')}
				onclick={() => openImport()}
			>
				<HugeiconsIcon icon={SpotifyIcon} class="h-4 w-4" /> {t('import.button')}
				<ExperimentalBadge />
			</Button>
			<!-- Signed out too: a playlist can live on this machine with no account (#251). -->
			<Button variant="outline" size="sm" class="gap-2" onclick={() => openNewPlaylist()}>
				<HugeiconsIcon icon={Add01Icon} class="h-4 w-4" /> {t('nav.new_playlist')}
			</Button>
		</div>
	</div>


	<!-- The tabs always render: Local music needs neither an account nor a connection. -->
	<Tabs.Root bind:value={tab}>
		<!-- `h-auto!`: the primitive pins the pill to h-9 through a data-attribute variant, and two
		     rows of tabs would spill out of it. -->
		<Tabs.List class="mb-4 h-auto! max-sm:w-full max-sm:flex-wrap max-sm:justify-start">
			<Tabs.Trigger value="all">
				<HugeiconsIcon icon={SquareStackIcon} class="h-4 w-4" /> {t('common.all')}
			</Tabs.Trigger>
			<Tabs.Trigger value="playlists">
				<HugeiconsIcon icon={Playlist02Icon} class="h-4 w-4" /> {t('library.playlists_tab')}
			</Tabs.Trigger>
			<Tabs.Trigger value="albums">
				<HugeiconsIcon icon={MusicNoteSquare02Icon} class="h-4 w-4" /> {t('library.albums_tab')}
			</Tabs.Trigger>
			<Tabs.Trigger value="artists">
				<HugeiconsIcon icon={UserSharingIcon} class="h-4 w-4" /> {t('library.artists_tab')}
			</Tabs.Trigger>
			<Tabs.Trigger value="songs">
				<HugeiconsIcon icon={MusicNote01Icon} class="h-4 w-4" /> {t('library.songs_tab')}
			</Tabs.Trigger>
			<Tabs.Trigger value="uploads">
				<HugeiconsIcon icon={CloudUploadIcon} class="h-4 w-4" /> {t('library.uploads_tab')}
			</Tabs.Trigger>
			<Tabs.Trigger value="local">
				<HugeiconsIcon icon={DriveIcon} class="h-4 w-4" /> {t('library.local_tab')}
			</Tabs.Trigger>
		</Tabs.List>
		<!-- Every branch below is gated on `tab`, because bits-ui never unmounts an inactive panel: it
		     renders every one and hides the inactive ones. Left alone, opening Library builds each card twice
		     (once for All, once for its own tab) and mounts the whole Local tab, disk scan included,
		     for a panel you cannot see. -->
		<!-- Songs, Uploads and Local stand apart: two are track lists rather than card grids, the
		     third needs neither an account nor a connection, and the states below fit none of them. -->
		<Tabs.Content value="songs">
			{#if tab === 'songs'}
				{#if signedOut}
					<p class="text-sm text-muted-foreground">{t('library.songs_signed_out')}</p>
				{:else}
					<LibrarySongs />
				{/if}
			{/if}
		</Tabs.Content>
		<Tabs.Content value="uploads">
			{#if tab === 'uploads'}
				{#if signedOut}
					<p class="text-sm text-muted-foreground">{t('library.uploads_signed_out')}</p>
				{:else}
					<!-- `line` rather than the pill row above it: two identical pill rows stacked read as
					     one control drawn twice. -->
					<Tabs.Root bind:value={uploadTab}>
						<Tabs.List variant="line" class="mb-4">
							<Tabs.Trigger value="all">
								<HugeiconsIcon icon={SquareStackIcon} class="h-4 w-4" /> {t('common.all')}
							</Tabs.Trigger>
							<Tabs.Trigger value="songs">
								<HugeiconsIcon icon={MusicNote01Icon} class="h-4 w-4" /> {t('common.songs')}
							</Tabs.Trigger>
							<Tabs.Trigger value="albums">
								<HugeiconsIcon icon={MusicNoteSquare02Icon} class="h-4 w-4" /> {t('common.albums')}
							</Tabs.Trigger>
						</Tabs.List>
						<Tabs.Content value="all">
							{#if uploadTab === 'all'}
								<LibrarySongs uploads limit={20} onSeeAll={() => (uploadTab = 'songs')} />
								{#if library.uploadAlbums.length}
									<h2 class="mb-3 mt-8 font-heading text-lg font-semibold">{t('common.albums')}</h2>
									{@render grid(library.uploadAlbums, '', rvUploadsAll)}
								{/if}
							{/if}
						</Tabs.Content>
						<Tabs.Content value="songs">
							{#if uploadTab === 'songs'}<LibrarySongs uploads />{/if}
						</Tabs.Content>
						<Tabs.Content value="albums">
							{#if uploadTab === 'albums'}
								{#if library.uploadAlbumsLoading && !library.uploadAlbums.length}
									<div class="card-grid">
										{#each Array(6) as _, i (i)}
											<MediaCardSkeleton />
										{/each}
									</div>
								{:else if library.uploadAlbumsError && !library.uploadAlbums.length}
									<ErrorState
										message={library.uploadAlbumsError}
										onRetry={() => loadUploadAlbums(true)}
									/>
								{:else}
									{@render grid(
										library.uploadAlbums,
										t('library.no_upload_albums'),
										rvUploadAlbums
									)}
								{/if}
							{/if}
						</Tabs.Content>
					</Tabs.Root>
				{/if}
			{/if}
		</Tabs.Content>
		<Tabs.Content value="local">{#if tab === 'local'}<LocalMusic />{/if}</Tabs.Content>
		{#if tab === 'local' || tab === 'songs' || tab === 'uploads'}
			<!-- nothing else: the grid states below have no bearing on these three -->
		{:else if loading}
			<div class="card-grid">
				{#each Array(12) as _, i (i)}
					<MediaCardSkeleton />
				{/each}
			</div>
		{:else if error && !all.length}
			<!-- Only when there is nothing to fall back on. Now that the grid is cached across visits, a
			     refresh that fails should leave the library you were looking at on screen. -->
			<ErrorState message={error} onRetry={load} />
		{:else}
			<Tabs.Content value="all">
				{#if tab === 'all'}
					{@render grid(
						all,
						signedOut ? t('library.empty_signed_out') : t('library.empty'),
						rvAll,
						true
					)}
				{/if}
			</Tabs.Content>
			<Tabs.Content value="playlists">
				{#if tab === 'playlists'}
					{@render grid(
						playlists,
						t('library.no_saved_playlists'),
						rvPlaylists,
						true
					)}
				{/if}
			</Tabs.Content>
			<Tabs.Content value="albums">
				{#if tab === 'albums'}
					{@render grid(
						albums,
						t('library.no_saved_albums'),
						rvAlbums
					)}
				{/if}
			</Tabs.Content>
			<Tabs.Content value="artists">
				{#if tab === 'artists' && signedOut}
					{@render grid(artists, t('library.no_saved_artists'), rvArtists)}
				{:else if tab === 'artists'}
					<!-- `line`, same as Uploads: a second pill row would read as the first drawn twice. -->
					<Tabs.Root bind:value={artistTab}>
						<Tabs.List variant="line" class="mb-4">
							<Tabs.Trigger value="artists">
								<HugeiconsIcon icon={UserSharingIcon} class="h-4 w-4" /> {t('library.artists_tab')}
							</Tabs.Trigger>
							<Tabs.Trigger value="subscriptions">
								<HugeiconsIcon icon={UserCheck01Icon} class="h-4 w-4" /> {t('library.subscriptions_tab')}
							</Tabs.Trigger>
						</Tabs.List>
						<Tabs.Content value="artists">
							{#if artistTab === 'artists'}
								<!-- Unmerged: what was saved here is a subscription, and lists under that. -->
								{@render grid(library.artists, t('library.no_artists'), rvArtists)}
							{/if}
						</Tabs.Content>
						<Tabs.Content value="subscriptions">
							{#if artistTab === 'subscriptions'}
								{@render grid(subscriptions, t('library.no_subscriptions'), rvSubscriptions)}
							{/if}
						</Tabs.Content>
					</Tabs.Root>
				{/if}
			</Tabs.Content>
		{/if}
	</Tabs.Root>
</div>
