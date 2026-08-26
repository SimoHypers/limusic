<script lang="ts">
	import { fade, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Cancel01Icon } from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { t } from '$lib/i18n.svelte';
	import {
		ui,
		toast,
		bumpLibraryTrackCount,
		notePlaylistAdd,
		noteSavedIn
	} from '$lib/player.svelte';

	let playlists = $state<BrowseItem[]>([]);
	let loading = $state(false);
	let filter = $state('');
	let box = $state<HTMLInputElement | null>(null);
	// `autofocus` is unreliable on an element inserted after load (and mid-transition), so focus it
	// ourselves the frame it exists: the modal opens ready to type.
	$effect(() => {
		box?.focus();
	});
	// ponytail: plain substring, not fuzzy. A library is tens of playlists, and "rap" finding
	// "Rap Caviar" is what issue #100 actually asked for.
	const matches = $derived(
		playlists.filter((p) => p.title.toLowerCase().includes(filter.trim().toLowerCase()))
	);

	// Fetch the library playlists fresh each time the picker opens (cheap; picks up new playlists).
	// On Repeat and Liked Music are dropped: On Repeat is built from local play counts, and Liked
	// Music takes likes rather than playlist edits (YouTube 400s the add). The command boundary
	// refuses both too, but a target you can tap and can't use is the bug.
	$effect(() => {
		if (ui.addSongs) {
			loading = true;
			filter = '';
			api
				.getLibrary()
				.then(
					(p) =>
						(playlists = p.filter(
							(i) => i.id !== api.ON_REPEAT_ID && i.id !== api.LIKED_MUSIC_ID
						))
				)
				.catch((e) => toast.error(String(e)))
				.finally(() => (loading = false));
		}
	});

	function close() {
		ui.addSongs = null;
	}

	async function pick(pl: BrowseItem) {
		const songs = ui.addSongs;
		close();
		if (!songs?.length) return;
		try {
			// Sequential — a whole album is a handful of requests; don't hammer the API in parallel.
			// YouTube refuses a track the playlist already holds, so only the ones it accepted get
			// counted and drawn: an optimistic row for a refused add is a row that can never be
			// removed (no setVideoId behind it) until the app restarts.
			const added: typeof songs = [];
			for (const song of songs) {
				if (await api.addToPlaylist(pl.id, song.video_id)) added.push(song);
			}
			const dupes = songs.length - added.length;
			// Every song, not just the accepted ones: a refusal means the playlist already holds it,
			// so its "saved" mark is right either way.
			noteSavedIn(pl.id, songs.map((s) => s.video_id));
			if (added.length) {
				bumpLibraryTrackCount(pl.id, added.length);
				notePlaylistAdd(pl.id, added);
			}
			if (!added.length) {
				toast(
					dupes > 1
						? t('toasts.already_in_all', { count: dupes, playlist: pl.title })
						: t('toasts.already_in', { playlist: pl.title })
				);
			} else if (dupes) {
				toast.success(
					t('toasts.added_to_playlist_dupes', { count: added.length, playlist: pl.title, dupes })
				);
			} else {
				toast.success(
					added.length > 1
						? t('toasts.added_songs', { count: added.length, playlist: pl.title })
						: t('toasts.added_one', { playlist: pl.title })
				);
			}
		} catch (e) {
			toast.error(String(e));
		}
	}
</script>

<svelte:window
	onkeydown={(e) => {
		if (ui.addSongs && e.key === 'Escape') close();
	}}
/>

{#if ui.addSongs}
	<div
		transition:fade={{ duration: 150 }}
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
	>
		<div
			transition:scale={{ duration: 180, start: 0.96, easing: cubicOut }}
			class="flex max-h-[32rem] w-full max-w-sm flex-col rounded-xl border bg-card p-4 shadow-xl"
		>
			<div class="mb-3 flex items-center justify-between">
				<h2 class="font-heading text-base font-semibold">{t('player.add_to_playlist')}</h2>
				<button
					class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
					onclick={close}
					aria-label={t('a11y.close')}
				>
					<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
				</button>
			</div>
			{#if playlists.length > 1}
				<input
					bind:this={box}
					bind:value={filter}
					placeholder={t('library.search_playlists')}
					onkeydown={(e) => e.key === 'Enter' && matches[0] && pick(matches[0])}
					class="mb-2 w-full rounded-lg border bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
				/>
			{/if}
			{#if loading}
				<p class="p-2 text-sm text-muted-foreground">{t('common.loading')}</p>
			{:else if matches.length}
				<div class="min-h-0 flex-1 overflow-y-auto">
					{#each matches as pl (pl.id)}
						<button
							class="flex w-full items-center gap-3 rounded-lg p-2 text-left hover:bg-accent/10"
							onclick={() => pick(pl)}
						>
							{#if pl.thumbnail}
								<img src={pl.thumbnail} alt="" class="h-10 w-10 rounded-md object-cover" />
							{:else}
								<div class="h-10 w-10 rounded-md bg-muted"></div>
							{/if}
							<div class="min-w-0">
								<div class="truncate text-sm font-medium">{pl.title}</div>
								{#if pl.subtitle}
									<div class="truncate text-xs text-muted-foreground">{pl.subtitle}</div>
								{/if}
							</div>
						</button>
					{/each}
				</div>
			{:else}
				<p class="p-2 text-sm text-muted-foreground">
					{filter.trim() ? t('common.no_matches') : t('library.no_playlists_create')}
				</p>
			{/if}
		</div>
	</div>
{/if}
