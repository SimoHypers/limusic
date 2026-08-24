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
		ArrowUpNarrowWideIcon,
		ArrowDownWideNarrowIcon,
		Radio02Icon,
		ThumbsUpIcon,
		ThumbsDownIcon,
		UserListIcon,
		Vynil02Icon,
		DashboardSquare02Icon,
		Share08Icon,
		PreferenceVerticalIcon
	} from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import type { SongItem } from '$lib/api';
	import { anchorMenu, ctxHost, fitMenu, NO_ANCHOR, toBody } from '$lib/menu';
	import {
		addPick,
		enqueue,
		openShare,
		personal,
		ratingOf,
		removePick,
		startRadio,
		toggleRating
	} from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import TempoPitchDialog from './TempoPitchDialog.svelte';

	let {
		song,
		triggerClass = '',
		onAdd,
		onRemove,
		removeLabel = t('player.remove_from_playlist'),
		linksOnly = false
	}: {
		song: SongItem;
		/** Classes for the ⋯ trigger button (positioning differs per host: inline vs overlay). */
		triggerClass?: string;
		/** Adds an "Add to playlist" menu item. */
		onAdd?: () => void;
		/** Adds a remove menu item (label via `removeLabel`). */
		onRemove?: () => void;
		removeLabel?: string;
		/** Player-bar variant: ⋮ trigger, and only artist/album/shortcuts (queue and like already
		    have their own buttons there). */
		linksOnly?: boolean;
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
	// A local file has no YouTube identity: liking it or putting it in a YTM playlist is not a
	// thing, so those items don't show. Queue, shortcuts and go-to-album work normally.
	const isLocal = $derived(api.isLocalId(song.video_id));
</script>

<button
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
	<button
		class="fixed inset-0 z-40 cursor-default"
		onclick={close}
		oncontextmenu={close}
		aria-label={t('a11y.close_menu')}
		{@attach toBody}
	></button>
	<div
		class="fixed z-50 min-w-44 animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style={anchor.style}
		{@attach toBody}
		{@attach fitMenu(anchor)}
	>
		{#if !linksOnly}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => enqueue([song], true))}
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
		<!-- In the player bar (`linksOnly`) like and add-to-playlist have their own buttons, but those
		     drop below lg to leave the title room, so the menu carries them at that width instead.
		     Dislike has no button of its own anywhere, so it stays visible at every width. -->
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
		{#if song.artist_id}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => goto(`/artist/${encodeURIComponent(song.artist_id!)}`))}
			>
				<HugeiconsIcon icon={UserListIcon} class="h-4 w-4" /> {t('player.go_to_artist')}
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
		{#if linksOnly}
			<button
				class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => (advancedOpen = true))}
			>
				<HugeiconsIcon icon={PreferenceVerticalIcon} class="h-4 w-4" /> {t('dialogs.tempo_pitch.title')}
			</button>
		{/if}
		{#if onAdd && !isLocal}
			<button
				class="w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10 {linksOnly
					? 'flex lg:hidden'
					: 'flex'}"
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
	</div>
{/if}

{#if linksOnly}
	<TempoPitchDialog bind:open={advancedOpen} />
{/if}
