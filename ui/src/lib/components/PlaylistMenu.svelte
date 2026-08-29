<script lang="ts">
	// The ⋯ menu on a sidebar library row, a card, or an artist row. Positioned `fixed` and moved to
	// <body> like TrackMenu: the playlist list is a scroll container, so an absolute popup would be
	// clipped by it. Right-clicking the surrounding `[data-ctx]` element opens it at the pointer.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		MoreHorizontalIcon,
		MoreVerticalIcon,
		PinIcon,
		PinOffIcon,
		Radio02Icon,
		ArrowUpNarrowWideIcon,
		ArrowDownWideNarrowIcon,
		BookmarkCheck02Icon,
		BookmarkMinus02Icon,
		BookPlusIcon,
		DashboardSquare02Icon,
		Share08Icon
	} from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { enqueueItem } from '$lib/browse';
	import { anchorMenu, ctxHost, fitMenu, NO_ANCHOR, toBody } from '$lib/menu';
	import { t } from '$lib/i18n.svelte';
	import {
		addPick,
		addToLibrary,
		auth,
		inLibrary,
		isSaved,
		ownedByUser,
		removeFromLibrary,
		isSynced,
		loadLibraryExtras,
		openShare,
		personal,
		removePick,
		startRadio,
		toast,
		togglePin,
		toggleSaved
	} from '$lib/player.svelte';

	let {
		item,
		showPin = true,
		vertical = false,
		iconClass = 'h-4 w-4',
		// Visibility lives here too: most triggers only appear on hover, but a row that has nothing
		// else to reveal on hover shows its ⋯ all the time.
		triggerClass = 'absolute right-1 top-1/2 flex h-7 w-7 -translate-y-1/2 cursor-pointer items-center justify-center rounded-md text-muted-foreground opacity-0 transition hover:bg-sidebar-accent hover:text-foreground focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring group-hover/row:opacity-100'
	}: {
		item: BrowseItem;
		showPin?: boolean;
		vertical?: boolean;
		iconClass?: string;
		triggerClass?: string;
	} = $props();

	const pinned = $derived(personal.pins.includes(item.id));
	// Already on the home grid: the menu offers the way out rather than a second copy.
	const isPick = $derived(personal.picks.some((p) => p.id === item.id));
	// A synced row is on the account too, and dropping only the local copy would leave the card on
	// screen with a "removed" toast under it. Signed out, the local copy is the whole library again.
	const savedHere = $derived(
		isSaved(item.id) && !(auth.account?.signedIn && isSynced(item.id))
	);
	// Saved here or on the account. `savedHere` is the narrower one: only a local row can be taken
	// back out from a menu, since undoing YouTube's own copy belongs on the item's page.
	const inLib = $derived(inLibrary(item));
	// Your own playlists, uploads, Liked Music and On Repeat are in the library by being what they
	// are: the row says so and does nothing. Everything else got saved, so it can be unsaved.
	const owned = $derived(ownedByUser(item));

	// Radio and Share both need a YouTube item behind them: local folders and the locally-built
	// On Repeat have none.
	const onYouTube = $derived(!api.isLocalId(item.id) && item.id !== api.ON_REPEAT_ID);
	// An artist isn't a track list — there's nothing unambiguous to queue. Songs, albums and
	// playlists (local ones included) all are.
	const canQueue = $derived(item.kind === 'song' || item.kind === 'album' || item.kind === 'playlist');

	// The tracks have to be fetched before anything can be queued, so the menu stays open and the
	// row shows it's working. Guards a second click from queueing the album twice.
	let queueing = $state(false);
	async function queue(next: boolean) {
		if (queueing) return;
		queueing = true;
		try {
			await enqueueItem(item, next);
			menuOpen = false;
		} finally {
			queueing = false;
		}
	}

	let saving = $state(false);
	async function save() {
		if (saving) return;
		saving = true;
		try {
			const result = await addToLibrary(item);
			toast.success(
				result === 'already' ? t('toasts.already_in_library') : t('library.saved_to_library')
			);
			menuOpen = false;
		} catch (e) {
			toast.error(String(e));
		} finally {
			saving = false;
		}
	}

	async function unsave() {
		if (saving) return;
		saving = true;
		try {
			await removeFromLibrary(item);
			toast.success(t('toasts.removed_from_library'));
			menuOpen = false;
		} catch (e) {
			toast.error(String(e));
		} finally {
			saving = false;
		}
	}

	let menuOpen = $state(false);
	let anchor = $state(NO_ANCHOR);

	// Click on the ⋯ opens under the button; right-click on the host card or row opens at the pointer.
	function openMenu(e: MouseEvent) {
		e.preventDefault(); // a right-click must not also raise WebKit's own menu
		e.stopPropagation();
		// Saved albums and artists are only fetched by the Library page, and without them every card
		// outside it would offer "Save to library" for something the account already holds. Cached
		// after the first menu, so this is one pair of requests per session.
		if (auth.account?.signedIn && (item.kind === 'album' || item.kind === 'artist'))
			loadLibraryExtras();
		anchor = anchorMenu(e, { align: 'right' });
		menuOpen = true;
	}
	// stopPropagation everywhere: the trigger sits over a clickable host (a card's whole surface is a
	// play/navigate target), so its click must not reach the host's handler. The popup itself now
	// lives at <body> and no longer bubbles into the host, but these stay: the trigger needs them.
	function run(e: MouseEvent, action?: () => void) {
		e.stopPropagation();
		menuOpen = false;
		action?.();
	}
	// Right-clicking off the menu dismisses it, same as a left click.
	function close(e: MouseEvent) {
		e.preventDefault();
		e.stopPropagation();
		menuOpen = false;
	}
</script>

<button
	class="{triggerClass} {menuOpen ? 'opacity-100' : ''}"
	onclick={openMenu}
	aria-label={t('a11y.playlist_options')}
	{@attach ctxHost(openMenu)}
>
	<!-- icon swap via altIcon/showAlt — `icon` is frozen at mount -->
	<HugeiconsIcon
		icon={MoreHorizontalIcon}
		altIcon={MoreVerticalIcon}
		showAlt={vertical}
		class={iconClass}
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
		class="pointer-events-auto fixed z-[70] min-w-48 animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style={anchor.style}
		{@attach toBody}
		{@attach fitMenu(anchor)}
	>
		{#if showPin}
			<button
				class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => togglePin(item.id))}
			>
				<HugeiconsIcon icon={pinned ? PinOffIcon : PinIcon} class="h-4 w-4" />
				{pinned ? t("player.unpin") : t("player.pin_to_top")}
			</button>
		{/if}
		{#if canQueue}
			<button
				class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10 disabled:opacity-50"
				disabled={queueing}
				onclick={(e) => {
					e.stopPropagation();
					queue(true);
				}}
			>
				<HugeiconsIcon icon={ArrowUpNarrowWideIcon} class="h-4 w-4" /> {t('player.play_next')}
			</button>
			<button
				class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10 disabled:opacity-50"
				disabled={queueing}
				onclick={(e) => {
					e.stopPropagation();
					queue(false);
				}}
			>
				<HugeiconsIcon icon={ArrowDownWideNarrowIcon} class="h-4 w-4" /> {t("player.add_to_queue")}
			</button>
		{/if}
		{#if onYouTube}
			<button
				class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => startRadio(item.kind as 'artist' | 'album' | 'playlist', item.id, item.title))}
			>
				<HugeiconsIcon icon={Radio02Icon} class="h-4 w-4" /> {t('player.start_radio')}
			</button>
		{/if}
		<button
			class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
			onclick={(e) => run(e, () => (isPick ? removePick(item.id) : addPick(item)))}
		>
			<HugeiconsIcon icon={DashboardSquare02Icon} class="h-4 w-4" />
			{isPick ? t('home.remove_shortcut') : t('player.add_to_shortcuts')}
		</button>
		{#if onYouTube}
			<button
				class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) => run(e, () => openShare(item))}
			>
				<HugeiconsIcon icon={Share08Icon} class="h-4 w-4" /> {t("player.share")}
			</button>
		{/if}
		<!-- One row for library membership: put it in, take the local copy back out, or just say it
		     is already there (YouTube's own copy is unsaved from the item's page, which knows which
		     write to send). Local folders and On Repeat have no library to be in. -->
		{#if onYouTube && !inLib}
			<button
				class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10 disabled:opacity-50"
				disabled={saving}
				onclick={(e) => {
					e.stopPropagation();
					save();
				}}
			>
				<HugeiconsIcon icon={BookPlusIcon} class="h-4 w-4" /> {t('library.save_to_library')}
			</button>
		{:else if onYouTube && !savedHere && !owned}
			<button
				class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10 disabled:opacity-50"
				disabled={saving}
				onclick={(e) => {
					e.stopPropagation();
					unsave();
				}}
			>
				<HugeiconsIcon icon={BookmarkMinus02Icon} class="h-4 w-4" />
				{t('library.remove_from_library')}
			</button>
		{:else if onYouTube}
			<div class="flex items-center gap-2 rounded-md px-2 py-1.5 text-sm text-muted-foreground">
				<HugeiconsIcon icon={BookmarkCheck02Icon} class="h-4 w-4" /> {t('library.in_library')}
			</div>
		{/if}
		<!-- Only for cards saved on this machine: YouTube's own library rows are unsaved from their
		     page, where the button knows which write action to send. -->
		{#if savedHere}
			<button
				class="flex w-full cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent/10"
				onclick={(e) =>
					run(e, () => {
						toggleSaved(item);
						toast.success(t('toasts.removed_from_library'));
					})}
			>
				<HugeiconsIcon icon={BookmarkMinus02Icon} class="h-4 w-4" /> {t('library.remove_from_library')}
			</button>
		{/if}
	</div>
{/if}
