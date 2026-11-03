<script lang="ts">
	// "Share": the YTM link for a song/album/playlist/artist, with the artwork and title so you can
	// see what you're about to send. Opened from any ⋯ menu via `openShare`, mounted once in the
	// layout like AddToPlaylist.
	import { untrack } from 'svelte';
	import { fade, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Cancel01Icon, Copy01Icon, Tick02Icon, Alert02Icon } from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import type { BrowseItem, PlaylistPage } from '$lib/api';
	import { copyText } from '$lib/clipboard';
	import { getCached, invalidateCached, putCached } from '$lib/pagecache';
	import { Switch } from '$lib/components/ui/switch';
	import { Checkbox } from '$lib/components/ui/checkbox';
	import { thumb } from '$lib/thumb';
	import { formatClock, parseClock } from '$lib/timecode';
	import { playback, ui, toast } from '$lib/player.svelte';
	import { windowCorners } from '$lib/win.svelte';
	import { t } from '$lib/i18n.svelte';

	// Playlist browseIds carry a `VL` prefix that the watch/playlist URLs don't take.
	function shareUrl(item: BrowseItem, start?: number | null): string {
		const id = item.id.replace(/^VL/, '');
		if (item.kind === 'song') {
			const at = start && start > 0 ? `&t=${Math.floor(start)}` : '';
			return `https://music.youtube.com/watch?v=${id}${at}`;
		}
		if (item.kind === 'artist') return `https://music.youtube.com/channel/${id}`;
		// Albums only ever reach us as an `MPRE…` browseId, so link the browse page rather than the
		// `OLAK5uy_…` playlist URL YouTube's own share sheet hands out. Both resolve to the album.
		if (item.kind === 'album') return `https://music.youtube.com/browse/${id}`;
		return `https://music.youtube.com/playlist?list=${id}`;
	}

	// "Start at": a link to the song that is playing right now can open it partway in. Only offered
	// for that song, because it is the only one whose position we know. The box is filled with the
	// position at the moment the dialog opens (not live: a moving field is hard to edit).
	let canStartAt = $state(false);
	let atTime = $state(false);
	let startText = $state('');
	// The shared song's length as of opening the dialog. The player's own `duration` is the shared
	// song's only while it is still the one playing: the track can change under an open dialog.
	let openLength = $state(0);
	const trackLength = $derived(
		ui.share?.id === playback.now?.videoId ? playback.duration || openLength : openLength
	);
	const parsedStart = $derived(canStartAt && atTime ? parseClock(startText) : null);
	// A switched-on box that doesn't parse, or points past the end of the track, is flagged with a
	// message; the link just has no `t` until it is fixed. An unknown duration (0) can't be checked.
	const startError = $derived.by(() => {
		if (!canStartAt || !atTime) return '';
		if (parsedStart === null) return t('dialogs.share.start_invalid');
		if (trackLength > 0 && parsedStart > trackLength) {
			return t('dialogs.share.start_too_long', { length: formatClock(trackLength) });
		}
		return '';
	});
	const startSecs = $derived(startError ? null : parsedStart);

	const url = $derived(ui.share ? shareUrl(ui.share, startSecs) : '');
	let copied = $state(false);

	// A private playlist's link 404s for everyone else, so the modal has to say so before the link
	// is sent. Only playlists carry a privacy setting, and only YouTube's edit header reports it,
	// which means only playlists the signed-in user owns ever resolve to anything here.
	let privacy = $state<string | undefined>(undefined);
	let owned = $state(false);
	// Keeps the toggle on screen after it has been flipped public, so the flip can be undone.
	let wasPrivate = $state(false);
	// Liked Music reports as owned but has no editable privacy (same carve-out as the playlist page).
	const canToggle = $derived(owned && ui.share?.id.replace(/^VL/, '') !== 'LM');

	$effect(() => {
		const item = ui.share;
		privacy = undefined;
		owned = false;
		wasPrivate = false;
		atTime = false;
		// Player state is read untracked: this runs when the dialog opens (or retargets), not on
		// every position tick, and not when the track changes under it, which would drop the row
		// and whatever was typed into it.
		const playing = untrack(() => playback.now?.videoId);
		canStartAt = !!item && item.kind === 'song' && item.id === playing;
		startText = canStartAt ? formatClock(untrack(() => playback.position)) : '';
		openLength = canStartAt ? untrack(() => playback.duration) : 0;
		if (!item || item.kind !== 'playlist') return;
		const key = `playlist:${item.id}`;
		const apply = (p: PlaylistPage) => {
			// The modal may have been closed or retargeted while the fetch was in flight.
			if (ui.share?.id !== item.id) return;
			privacy = p.privacy;
			owned = p.owned;
			wasPrivate = p.privacy === 'PRIVATE';
		};
		const hit = getCached<PlaylistPage>(key);
		if (hit) return apply(hit);
		// No lighter endpoint exists: privacy rides along with the track list. The result is cached
		// under the same key the playlist page reads, so the fetch is not wasted.
		api
			.getPlaylist(item.id)
			.then((p) => {
				putCached(key, p);
				apply(p);
			})
			// A playlist we cannot read (signed out, someone else's) gets no warning: it is not
			// private, or it would not have been visible in the first place.
			.catch(() => {});
	});

	async function setPublic(next: boolean) {
		const item = ui.share;
		if (!item) return;
		const before = privacy;
		privacy = next ? 'PUBLIC' : 'PRIVATE';
		try {
			await api.editPlaylistDetails(item.id, { public: next });
			// ponytail: the playlist page keeps its own copy of `privacy` for the edit dialog, so drop
			// the cached page rather than plumbing a patch through. Worst case its Public switch reads
			// stale until the next visit, and a Save that doesn't touch it sends nothing.
			invalidateCached(`playlist:${item.id}`);
		} catch (e) {
			privacy = before;
			toast.error(String(e));
		}
	}

	function close() {
		ui.share = null;
		copied = false;
	}

	function copy() {
		copyText(url).then(
			() => {
				copied = true;
				setTimeout(() => (copied = false), 1500);
			},
			() => toast.error(t('toasts.could_not_copy_link'))
		);
	}
</script>

<svelte:window
	onkeydown={(e) => {
		if (ui.share && e.key === 'Escape') close();
	}}
/>

{#if ui.share}
	<div
		transition:fade={{ duration: 150 }}
		class="fixed inset-0 z-[49] flex items-center justify-center bg-black/50 p-4 {windowCorners()}"
		role="presentation"
		onclick={(e) => {
			if (e.target === e.currentTarget) close();
		}}
	>
		<div
			transition:scale={{ duration: 180, start: 0.96, easing: cubicOut }}
			class="w-full max-w-md rounded-xl border bg-card p-4 shadow-xl"
		>
			<div class="mb-4 flex items-center justify-between">
				<h2 class="text-base font-semibold">{t('dialogs.share.title')}</h2>
				<button
					class="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
					onclick={close}
					aria-label={t('common.close')}
				>
					<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
				</button>
			</div>

			<div class="flex items-center gap-4">
				{#if ui.share.thumbnail}
					<img
						src={thumb(ui.share.thumbnail, 400)}
						alt=""
						class="h-20 w-20 shrink-0 object-cover {ui.share.kind === 'artist'
							? 'rounded-full'
							: 'rounded-lg'}"
					/>
				{:else}
					<div class="h-20 w-20 shrink-0 rounded-lg bg-muted"></div>
				{/if}
				<div class="min-w-0">
					<div class="truncate font-medium">{ui.share.title}</div>
					{#if ui.share.subtitle}
						<div class="truncate text-sm text-muted-foreground">{ui.share.subtitle}</div>
					{/if}
				</div>
			</div>

			<div class="mt-4 flex items-center gap-2 rounded-lg border bg-muted/40 py-1 pl-3 pr-1">
				<input
					class="min-w-0 flex-1 bg-transparent py-1 text-sm text-muted-foreground outline-none"
					value={url}
					readonly
					onfocus={(e) => e.currentTarget.select()}
				/>
				<button
					class="flex h-8 shrink-0 cursor-pointer items-center gap-1.5 rounded-md px-2.5 text-sm transition-colors hover:bg-accent/10"
					onclick={copy}
					aria-label={t('dialogs.share.copy_link')}
				>
					<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
					<HugeiconsIcon icon={Copy01Icon} altIcon={Tick02Icon} showAlt={copied} class="h-4 w-4" />
					{copied ? t('common.done') : t('dialogs.share.copy_link')}
				</button>
			</div>

			{#if canStartAt}
				<!-- YouTube's own share sheet: a checkbox, then the time as part of the sentence, an
				     underlined field rather than a boxed one. Typing a time ticks it. -->
				<div class="mt-3 flex items-center gap-2.5 px-1 text-sm">
					<label class="flex cursor-pointer items-center gap-2.5 select-none">
						<Checkbox bind:checked={atTime} />
						{t('dialogs.share.start_at')}
					</label>
					<input
						class="w-16 border-b border-border bg-transparent px-0.5 py-0.5 tabular-nums outline-none focus:border-primary aria-invalid:border-destructive {atTime
							? ''
							: 'text-muted-foreground'}"
						inputmode="numeric"
						placeholder="0:00"
						bind:value={startText}
						oninput={() => (atTime = true)}
						aria-label={t('a11y.share_start_time')}
						aria-invalid={!!startError}
					/>
				</div>
				{#if startError}
					<div class="mt-2 flex items-start gap-2 px-1 text-xs text-destructive" role="alert">
						<HugeiconsIcon icon={Alert02Icon} class="mt-px h-4 w-4 shrink-0" />
						<p>{startError}</p>
					</div>
				{/if}
			{/if}

			{#if privacy === 'PRIVATE'}
				<div class="mt-3 flex items-start gap-2 text-xs text-amber-600 dark:text-amber-500">
					<HugeiconsIcon icon={Alert02Icon} class="mt-px h-4 w-4 shrink-0" />
					<p>{t('dialogs.share.private_note')}</p>
				</div>
			{/if}

			{#if wasPrivate && canToggle}
				<div class="mt-3 flex items-center justify-between gap-4 rounded-lg border px-3 py-2.5">
					<div class="min-w-0">
						<div class="text-sm font-medium">{t('common.public')}</div>
						<p class="text-xs text-muted-foreground">
							{privacy === 'PUBLIC'
								? t('dialogs.share.public_link_on')
								: t('dialogs.share.public_link_off')}
						</p>
					</div>
					<Switch
						checked={privacy === 'PUBLIC'}
						onCheckedChange={setPublic}
						aria-label={t('a11y.public_playlist')}
					/>
				</div>
			{/if}
		</div>
	</div>
{/if}
