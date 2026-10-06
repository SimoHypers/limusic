<script lang="ts">
	// The ⋯ options menu shared by TrackRow (inline trigger) and MediaCard (overlay trigger).
	// Right-clicking anywhere in the surrounding `[data-ctx]` element opens the same menu at the
	// pointer (see `ctxHost`), which is what a track row's whole surface is for.
	// The queue actions + like are universal; go-to-artist/album/playlist show when the song carries
	// them. The popup is `fixed`, anchored at the trigger and moved to <body> (`toBody`), so no
	// scroll container clips it and no contained ancestor becomes its containing block.
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		MoreHorizontalIcon,
		MoreVerticalIcon,
		PlayListAddIcon,
		PlayListRemoveIcon,
		BookmarkMinus02Icon,
		BookPlusIcon,
		ArrowUpNarrowWideIcon,
		ArrowDownWideNarrowIcon,
		Radio02Icon,
		ThumbsUpIcon,
		ThumbsDownIcon,
		UserListIcon,
		UserBlock01Icon,
		Vynil02Icon,
		DashboardSquare02Icon,
		Share08Icon,
		PreferenceVerticalIcon,
		PencilEdit02Icon,
		Download01Icon,
		Delete02Icon,
		RefreshIcon,
		Cancel01Icon as CancelDownloadIcon
	} from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import type { SongItem } from '$lib/api';
	import { anchorMenu, ctxHost, fitMenu, NO_ANCHOR, toBody } from '$lib/menu';
	import { removableFromPlaylist } from '$lib/queue';
	import {
		addPick,
		blockArtist,
		bumpLibraryTrackCount,
		enqueue,
		inSongLibrary,
		songLibraryToken,
		openShare,
		notePlaylistRemove,
		noteUnsavedFrom,
		personal,
		playback,
		prefs,
		ratingOf,
		removePick,
		savedIn,
		toast,
		startRadio,
		toggleRating,
		toggleSongLibrary,
		ui
	} from '$lib/player.svelte';
	import { lastfm } from '$lib/lastfm.svelte';
	import { lt } from '$lib/lt.svelte';
	import { t } from '$lib/i18n.svelte';
	import { invalidateCachedPrefix } from '$lib/pagecache';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import { downloads, dlBusy, actOnDownload, downloadSong } from '$lib/downloads.svelte';
	import { canDownload, downloadFor } from '$lib/downloads';
	import TempoPitchDialog from './TempoPitchDialog.svelte';

	let {
		song,
		triggerClass = '',
		onAdd,
		onRemove,
		removeLabel = t('player.remove_from_playlist'),
		playlistId,
		queueIndex,
		linksOnly = false,
		inLibraryList = false
	}: {
		song: SongItem;
		/** Classes for the ⋯ trigger button (positioning differs per host: inline vs overlay). */
		triggerClass?: string;
		/** Adds an "Add to playlist" menu item. */
		onAdd?: () => void;
		/** Adds a remove menu item (label via `removeLabel`). */
		onRemove?: () => void;
		removeLabel?: string;
		/**
		 * The playlist this row is *playing from* (the queue's `sourceId`), which gets the row its
		 * own "Remove from this playlist" on top of whatever `onRemove` the host already spends on
		 * its own list. Only the player surfaces pass it: everywhere else the list on screen is the
		 * playlist, and `onRemove` is already that.
		 */
		playlistId?: string | null;
		/** This row's index in the backend queue, where the row *is* a queue row (the queue panel,
		    the player bar). Lets the playlist removal take the row out of the queue that came from
		    that playlist, rather than leaving a song the playlist no longer has queued up. */
		queueIndex?: number;
		/** Player-bar variant: ⋮ trigger, and only artist/album/shortcuts (queue and like already
		    have their own buttons there). */
		linksOnly?: boolean;
		/**
		 * The row is being shown *in* Library ▸ Songs, where "Save to library" makes no sense: the
		 * only direction is out, and that is the host's `onRemove` (it owns the list the row has to
		 * disappear from). A song sitting in that list because its album is saved gets neither.
		 */
		inLibraryList?: boolean;
	} = $props();

	// Already on the home grid: the menu offers the way out rather than a second copy.
	const isPick = $derived(personal.picks.some((p) => p.id === song.video_id));

	let menuOpen = $state(false);
	// Player-bar only: tempo/pitch belong to playback, not to a row you happen to be pointing at.
	let advancedOpen = $state(false);
	let anchor = $state(NO_ANCHOR);

	// Click on the ⋯ opens under the button; right-click on the host row opens at the pointer.
	function openMenu(e: MouseEvent) {
		e.preventDefault(); // a right-click must not also raise WebKit's own menu
		e.stopPropagation();
		anchor = anchorMenu(e, { align: 'right' });
		menuOpen = true;
	}
	// stopPropagation everywhere: the trigger sits inside a clickable row (TrackRow's whole row is a
	// play target), so its click must not reach the row's onplay (e.g. replacing the queue with the
	// playlist). The popup itself now lives at <body> and no longer bubbles into the row, but these
	// stay: they cost nothing and the trigger still needs them.
	function run(e: MouseEvent, action?: () => void) {
		e.stopPropagation();
		menuOpen = false;
		action?.();
	}
	// Right-clicking off the menu dismisses it, same as a left click: the backdrop swallows the
	// event, so the row underneath never sees it.
	function close(e: MouseEvent) {
		e.preventDefault();
		e.stopPropagation();
		menuOpen = false;
	}

	const rated = $derived(ratingOf(song));
	// Library ▸ Songs, which is a different list from Liked Music and a different write. Only rows
	// YouTube sent a menu with carry the tokens for it, so the row is absent on the ones this app
	// builds itself (local files, On Repeat, a mirrored guest queue) rather than dead.
	const inLib = $derived(inSongLibrary(song));
	const libraryToken = $derived(songLibraryToken(song));
	// A local file has no YouTube identity: liking it or sharing it is not a thing, so those items
	// don't show. Queue, shortcuts, go-to-album and saving it to a playlist on this machine work.
	const isLocal = $derived(api.isLocalId(song.video_id));
	// "Remove from this playlist", for a row playing out of a playlist (issue #270). What the three
	// conditions are and why is in `removableFromPlaylist` (queue.ts), where they are checkable.
	const removable = $derived(removableFromPlaylist(song, playlistId, savedIn.map));

	// Offline download: the shared store is the one source, so a state set on the Downloads page
	// (or by another window's event) is what this menu shows. A local file has nothing to fetch.
	const dl = $derived(canDownload(song) ? downloadFor(downloads.items, song.video_id) : undefined);
	const dlPending = $derived(!!dl && !!dlBusy[dl.id]);
	// Removing a download deletes the saved file, so it confirms first; the dialog lives below,
	// outside the popup (which is unmounted the moment the menu closes).
	let confirmingDlRemove = $state(false);

	function askRemoveDownload() {
		confirmingDlRemove = true;
	}

	async function removeDownload() {
		if (dl && (await actOnDownload(dl.id, 'remove'))) confirmingDlRemove = false;
	}

	// "Play next" on a track that is already coming up in the queue moves it into the Play next block
	// rather than queueing a second copy. A row the user queued, the backend moves by itself
	// (`insert_queued`). Any other row comes out first and goes back in as a real Play next: dropped
	// at `current + 1` unmarked, it would cut the Play next run in two, and later Play nexts scan that
	// run from the front (`guest_insert_index`). A guest owns no queue, so theirs stays a suggestion.
	async function playNext() {
		const q = playback.queue;
		// Checked again by id: the index is from when the menu opened, and an autoplay trim since
		// then shifts every row.
		const row = queueIndex !== undefined ? q.items[queueIndex] : undefined;
		const upcoming =
			row?.video_id === song.video_id && queueIndex! > q.currentIndex && lt.role !== 'guest';
		if (upcoming && !row.queued && !row.queued_end) {
			try {
				await api.removeFromQueue(queueIndex!);
			} catch (e) {
				toast.error(String(e));
				return;
			}
		}
		enqueue([song], true);
	}

	async function removeFromPlaylist() {
		if (!playlistId || !song.set_video_id) return;
		const setVideoId = song.set_video_id;
		try {
			await api.removeFromPlaylist(playlistId, song.video_id, setVideoId);
		} catch (e) {
			toast.error(String(e));
			return; // nothing below happens on a failed write: the row is still in the playlist
		}
		bumpLibraryTrackCount(playlistId, -1);
		noteUnsavedFrom(playlistId, song.video_id);
		// An open page for this playlist drops the row now; the cache drop covers every other one,
		// which has no rendered list to patch.
		notePlaylistRemove(playlistId, setVideoId);
		// Every order the page cached this playlist in, not just the bare key: a sort the user
		// picked earlier still holds the row, and the playlist page serves that hit as it is.
		invalidateCachedPrefix(`playlist:${playlistId}`);
		toast.success(t('toasts.removed_from_playlist'));
		// The song is out of the playlist, so it does not stay in the queue that playlist filled.
		// The playing row can't just be dropped (`remove_from_queue` refuses the current index, and
		// the user would keep listening to a track they just threw out), so skip past it first:
		// that leaves it behind the pointer, where removing it shifts `current` back onto the song
		// now playing.
		if (queueIndex === undefined) return;
		// Resolved again rather than reused: the index this menu opened on is a position, and a
		// queue edit during the request above (a guest add, another removal) moves every row
		// behind it. The setVideoId identifies the row itself.
		const at = playback.queue.items.findIndex((r) => r.set_video_id === setVideoId);
		if (at < 0) return; // already gone from the queue
		if (at === playback.queue.currentIndex) await api.nextTrack();
		await api.removeFromQueue(at);
	}
</script>

<!-- type="button": SearchSuggest keeps a hidden trigger inside the search <form>, and a default
     submit button there is what Enter clicks, opening this menu at the top left (#334). -->
<button
	type="button"
	class="{triggerClass} {menuOpen ? 'opacity-100' : ''}"
	onclick={openMenu}
	aria-label={t('a11y.track_options')}
	{@attach ctxHost(openMenu)}
>
	<!-- icon swap via altIcon/showAlt — `icon` is frozen at mount -->
	<HugeiconsIcon
		icon={MoreHorizontalIcon}
		altIcon={MoreVerticalIcon}
		showAlt={linksOnly}
		class="h-4 w-4"
	/>
</button>

{#if menuOpen}
	<!-- Above the modal layer (z-50), not below it: opened from a row inside a dialog, a backdrop
	     underneath left the dialog's own rows live under the menu. Only the popup itself is above.
	     pointer-events-auto on both: an open bits-ui dialog sets `pointer-events: none` on <body>,
	     which is where these two are portalled, so without it the menu is visible but every click
	     falls through it onto the dialog's own rows. -->
	<button
		data-menu
		class="pointer-events-auto fixed inset-0 z-[60] cursor-default"
		onclick={close}
		oncontextmenu={close}
		aria-label={t('a11y.close_menu')}
		{@attach toBody}
	></button>
	<div
		data-menu
		class="pointer-events-auto fixed z-[70] min-w-44 animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style={anchor.style}
		{@attach toBody}
		{@attach fitMenu(anchor)}
	>
		{#if !linksOnly}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, playNext)}
			>
				<HugeiconsIcon icon={ArrowUpNarrowWideIcon} class="h-4 w-4" /> {t('player.play_next')}
			</button>
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => enqueue([song], false))}
			>
				<HugeiconsIcon icon={ArrowDownWideNarrowIcon} class="h-4 w-4" /> {t('player.add_to_queue')}
			</button>
		{/if}
		<!-- Radio is the one action worth having in the player bar too (`linksOnly`): it's how you
		     say "keep going with more like this" about the song that's playing. -->
		{#if !isLocal}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => startRadio('song', song.video_id, song.title))}
			>
				<HugeiconsIcon icon={Radio02Icon} class="h-4 w-4" /> {t('player.start_radio')}
			</button>
		{/if}
		<!-- In the player bar (`linksOnly`) like has its own button, which drops below lg to leave the
		     title room, so the menu carries it at that width instead. Dislike has a button of its own only in the
		     mini player, so here it stays visible at every width. -->
		{#if !isLocal}
			<button
				class="w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10 {linksOnly
					? 'flex lg:hidden'
					: 'flex'}"
				onclick={(e) => run(e, () => toggleRating(song, 'like'))}
			>
				<HugeiconsIcon
					icon={ThumbsUpIcon}
					class="h-4 w-4 {rated === 'like' ? 'fill-current text-primary' : ''}"
				/>
				{rated === 'like' ? t('player.remove_from_liked') : t('player.save_to_liked')}
			</button>
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => toggleRating(song, 'dislike'))}
			>
				<HugeiconsIcon
					icon={ThumbsDownIcon}
					class="h-4 w-4 {rated === 'dislike' ? 'fill-current text-foreground' : ''}"
				/>
				{rated === 'dislike' ? t('player.remove_dislike') : t('common.dislike')}
			</button>
		{/if}
		{#if libraryToken && !inLibraryList}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => toggleSongLibrary(song))}
			>
				<!-- altIcon/showAlt, not a ternary: `icon` is read once at mount. -->
				<HugeiconsIcon
					icon={BookPlusIcon}
					altIcon={BookmarkMinus02Icon}
					showAlt={inLib}
					class="h-4 w-4"
				/>
				{inLib ? t('library.remove_from_library') : t('library.save_to_library')}
			</button>
		{/if}
		{#if !isLocal}
			{#if !dl}
				<button
					class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
					onclick={(e) => run(e, () => downloadSong(song))}
				>
					<HugeiconsIcon icon={Download01Icon} class="h-4 w-4" /> {t('downloads.action_download')}
				</button>
			{:else if dl.state === 'done'}
				<button
					class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-destructive hover:bg-destructive/10 disabled:opacity-50"
					disabled={dlPending}
					onclick={(e) => run(e, askRemoveDownload)}
				>
					<HugeiconsIcon icon={Delete02Icon} class="h-4 w-4" /> {t('downloads.action_remove')}
				</button>
			{:else if dl.state === 'error' || dl.state === 'cancelled'}
				<button
					class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10 disabled:opacity-50"
					disabled={dlPending}
					onclick={(e) => run(e, () => actOnDownload(dl.id, 'retry'))}
				>
					<HugeiconsIcon icon={RefreshIcon} class="h-4 w-4" /> {t('downloads.action_retry')}
				</button>
			{:else}
				<button
					class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-destructive hover:bg-destructive/10 disabled:opacity-50"
					disabled={dlPending}
					onclick={(e) => run(e, () => actOnDownload(dl.id, 'cancel'))}
				>
					<HugeiconsIcon icon={CancelDownloadIcon} class="h-4 w-4" /> {t('downloads.action_cancel')}
				</button>
			{/if}
		{/if}
		{#if song.artist_id}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => goto(`/artist/${encodeURIComponent(song.artist_id!)}`))}
			>
				<HugeiconsIcon icon={UserListIcon} class="h-4 w-4" /> {t('player.go_to_artist')}
			</button>
		{/if}
		<!-- Not gated on `artist_id`: a row whose byline links nothing is exactly the case this
		     exists for, and the name is a key in its own right. -->
		{#if !isLocal && song.artists?.trim()}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) =>
					run(e, () => {
						// The first run when the byline has any, id and text together: pairing run[0]'s
						// text with `artist_id` would block the wrong channel on "Unlinked & Linked".
						const first = song.artist_runs?.[0];
						return first
							? blockArtist(first.id, first.text)
							: blockArtist(song.artist_id, song.artists);
					})}
			>
				<HugeiconsIcon icon={UserBlock01Icon} class="h-4 w-4" /> {t('player.block_artist')}
			</button>
		{/if}
		<!-- Local files carry no album_id (local.rs). Checked here too: a queue restored from before
		     that changed still has one on its rows, and it would open a page this menu shouldn't offer. -->
		{#if song.album_id && !isLocal}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => goto(`/album/${encodeURIComponent(song.album_id!)}`))}
			>
				<HugeiconsIcon icon={Vynil02Icon} class="h-4 w-4" /> {t('player.go_to_album')}
			</button>
		{/if}
		<button
			class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={(e) =>
				run(e, () =>
					isPick
						? removePick(song.video_id)
						: addPick({
								kind: 'song',
								id: song.video_id,
								title: song.title,
								subtitle: song.artists,
								thumbnail: song.thumbnail
							})
				)}
		>
			<HugeiconsIcon icon={DashboardSquare02Icon} class="h-4 w-4" />
			{isPick ? t('home.remove_shortcut') : t('home.add_shortcut')}
		</button>
		{#if !isLocal}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) =>
					run(e, () =>
						openShare({
							kind: 'song',
							id: song.video_id,
							title: song.title,
							subtitle: song.artists,
							thumbnail: song.thumbnail
						})
					)}
			>
				<HugeiconsIcon icon={Share08Icon} class="h-4 w-4" /> {t('player.share')}
			</button>
		{/if}
		<!-- #404: fix how this track scrobbles, for good. Opens the Scrobbling tab on it, where the
		     preview shows what Last.fm gets before and after. Only for someone actually scrobbling:
		     connected, and the tab's switch on. -->
		{#if lastfm.connected && prefs.scrobbling}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) =>
					run(e, () => {
						ui.scrobbleTrack = song;
						ui.settingsFocus = 'scrobbling';
						ui.settingsOpen = true;
					})}
			>
				<HugeiconsIcon icon={PencilEdit02Icon} class="h-4 w-4" /> {t('player.edit_scrobble')}
			</button>
		{/if}
		{#if linksOnly}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => (advancedOpen = true))}
			>
				<HugeiconsIcon icon={PreferenceVerticalIcon} class="h-4 w-4" /> {t('dialogs.tempo_pitch.title')}
			</button>
		{/if}
		<!-- Always here, including the player bar at full width where the + button is right there:
		     people look for this in the menu and miss the icon. -->
		{#if onAdd}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, onAdd)}
			>
				<HugeiconsIcon icon={PlayListAddIcon} class="h-4 w-4" /> {t('player.save_to_playlist')}
			</button>
		{/if}
		{#if onRemove}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-destructive hover:bg-destructive/10"
				onclick={(e) => run(e, onRemove)}
			>
				<HugeiconsIcon icon={PlayListRemoveIcon} class="h-4 w-4" /> {removeLabel}
			</button>
		{/if}
		{#if removable}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-destructive hover:bg-destructive/10"
				onclick={(e) => run(e, removeFromPlaylist)}
			>
				<HugeiconsIcon icon={PlayListRemoveIcon} class="h-4 w-4" />
				{t('player.remove_from_this_playlist')}
			</button>
		{/if}
	</div>
{/if}

{#if linksOnly}
	<TempoPitchDialog bind:open={advancedOpen} />
{/if}

<!-- Removing a download deletes the file from disk: two-step, like the Downloads page. -->
<AlertDialog.Root bind:open={confirmingDlRemove}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Title>{t('downloads.remove_confirm_title')}</AlertDialog.Title>
			<AlertDialog.Description>
				{t('downloads.remove_confirm_desc', { title: song.title })}
			</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel>{t('common.cancel')}</AlertDialog.Cancel>
			<AlertDialog.Action variant="destructive" disabled={dlPending} onclick={removeDownload}>
				{t('downloads.action_remove')}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
