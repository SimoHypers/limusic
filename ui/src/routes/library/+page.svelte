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
		SquareStackIcon,
		UserSharingIcon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { Input } from '$lib/components/ui/input';
	import * as Dialog from '$lib/components/ui/dialog';
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
		createLibraryPlaylist,
		syncSavedToYouTube
	} from '$lib/player.svelte';
	import { mergeSaved, unsynced } from '$lib/personal';
	import { reveal } from '$lib/reveal.svelte';
	import { t } from '$lib/i18n.svelte';

	let dialogOpen = $state(false);
	let newTitle = $state('');
	let busy = $state(false);
	// `?tab=local` so anything that sends you back here (an album whose files were deleted) lands
	// on the tab you came from instead of a sign-in prompt.
	let tab = $state(page.url.searchParams.get('tab') ?? lastTab);
	$effect(() => {
		lastTab = tab;
	});
	// Uploads splits three ways (all / songs / albums): YouTube Music takes uploaded albums too, and
	// they are a card grid rather than rows, so they can't just join the track list.
	let uploadTab = $state('all');

	// Everything here lives in the shared `library` store, so a revisit renders the cached grid
	// immediately and the forced refresh below swaps in fresh data behind it. What was saved on this
	// machine merges in per tab (`mergeSaved`), which is the whole library when signed out.
	const playlists = $derived(mergeSaved(personal, library.items, 'playlist'));
	const albums = $derived(mergeSaved(personal, library.albums, 'album'));
	const artists = $derived(mergeSaved(personal, library.artists, 'artist'));
	const all = $derived([...playlists, ...albums, ...artists]);
	// One per tab rather than one shared instance reset on switch: an `$effect` reset lands
	// after the render it is meant to govern, so switching tabs would build the new tab's grid
	// against the old tab's count and immediately tear the excess back down. A tab keeping its
	// own depth also means coming back to one lands where you left it.
	const rvAll = reveal();
	const rvPlaylists = reveal();
	const rvAlbums = reveal();
	const rvArtists = reveal();
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

	async function createNew() {
		const title = newTitle.trim();
		if (!title || busy) return;
		busy = true;
		try {
			await createLibraryPlaylist(title);
			toast.success(t('toasts.playlist_created', { title }));
			newTitle = '';
			dialogOpen = false;
		} catch (e) {
			toast.error(String(e));
		} finally {
			busy = false;
		}
	}
</script>

{#snippet grid(items: BrowseItem[], empty: string, rv: ReturnType<typeof reveal>)}
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
	{/if}
{/snippet}

<div class="p-6">
	<div class="mb-6 flex items-center justify-between">
		<h1 class="font-heading text-2xl font-bold">{t('library.title')}</h1>
		{#if auth.account?.signedIn}
			<div class="flex items-center gap-2">
				<!-- Only with something to push: saves made before signing in, which live on this
				     machine until this button puts them on the account. -->
				{#if toSync.length}
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
				<Button variant="outline" size="sm" class="gap-2" onclick={() => (dialogOpen = true)}>
					<HugeiconsIcon icon={Add01Icon} class="h-4 w-4" /> {t('nav.new_playlist')}
				</Button>
			</div>
		{/if}
	</div>

	<Dialog.Root bind:open={dialogOpen}>
		<Dialog.Content class="sm:max-w-md">
			<Dialog.Header>
				<Dialog.Title>{t('dialogs.edit_playlist.new_title')}</Dialog.Title>
				<Dialog.Description>{t('dialogs.edit_playlist.new_desc')}</Dialog.Description>
			</Dialog.Header>
			<form
				class="flex flex-col gap-4"
				onsubmit={(e) => {
					e.preventDefault();
					createNew();
				}}
			>
				<Input bind:value={newTitle} placeholder={t('dialogs.edit_playlist.name_placeholder')} autofocus />
				<Dialog.Footer>
					<Button type="button" variant="outline" onclick={() => (dialogOpen = false)}>
						{t('common.cancel')}
					</Button>
					<Button type="submit" disabled={busy || !newTitle.trim()}>
						{busy ? t('common.loading') : t('common.create')}
					</Button>
				</Dialog.Footer>
			</form>
		</Dialog.Content>
	</Dialog.Root>

	<!-- The tabs always render: Local music needs neither an account nor a connection. -->
	<Tabs.Root bind:value={tab}>
		<Tabs.List class="mb-4">
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
					<p class="text-sm text-muted-foreground">
						Sign in to see the songs saved in your YouTube Music library. Music on this machine is
						in the Local tab.
					</p>
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
						signedOut
							? 'Nothing saved yet. Open a playlist or album and hit Save to library, or sign in for the one on your account.'
							: 'Your library is empty.',
						rvAll
					)}
				{/if}
			</Tabs.Content>
			<Tabs.Content value="playlists">
				{#if tab === 'playlists'}
					{@render grid(
						playlists,
						'No playlists yet. Open one and hit Save to library to keep it here.',
						rvPlaylists
					)}
				{/if}
			</Tabs.Content>
			<Tabs.Content value="albums">
				{#if tab === 'albums'}
					{@render grid(
						albums,
						'No saved albums yet. Open an album and hit Save to library.',
						rvAlbums
					)}
				{/if}
			</Tabs.Content>
			<Tabs.Content value="artists">
				{#if tab === 'artists'}
					{@render grid(
						artists,
						signedOut
							? 'No artists yet. Save one from its page to keep it here.'
							: 'No artists yet. They show up once you save their songs or albums.',
						rvArtists
					)}
				{/if}
			</Tabs.Content>
		{/if}
	</Tabs.Root>
</div>
