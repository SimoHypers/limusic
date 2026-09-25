<script lang="ts">
	import { fade, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Add01Icon, Cancel01Icon, ComputerIcon } from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import { ui, toast, addSongsToPlaylist, openNewPlaylist } from '$lib/player.svelte';

	let playlists = $state<BrowseItem[]>([]);
	let loading = $state(false);
	let filter = $state('');
	let box = $state<HTMLInputElement | null>(null);
	// `autofocus` is unreliable on an element inserted after load (and mid-transition), so focus it
	// ourselves the frame it exists: the modal opens ready to type.
	$effect(() => {
		box?.focus();
	});
	// A file on disk has no YouTube identity, so only a playlist on this machine can hold one.
	const hasLocalFiles = $derived(!!ui.addSongs?.some((s) => api.isLocalId(s.video_id)));
	const targets = $derived(
		hasLocalFiles ? playlists.filter((p) => api.isLocalPlaylist(p.id)) : playlists
	);
	// ponytail: plain substring, not fuzzy. A library is tens of playlists, and "rap" finding
	// "Rap Caviar" is what issue #100 actually asked for.
	const matches = $derived(
		targets.filter((p) => p.title.toLowerCase().includes(filter.trim().toLowerCase()))
	);

	// Fetch the library playlists fresh each time the picker opens (cheap; picks up new playlists).
	// On Repeat and Liked Music are dropped: On Repeat is built from local play counts, and Liked
	// Music takes likes rather than playlist edits (YouTube 400s the add). The command boundary
	// refuses both too, but a target you can tap and can't use is the bug. When the account's half
	// can't be fetched, the playlists on this machine still can: they need no network.
	$effect(() => {
		if (ui.addSongs) {
			loading = true;
			filter = '';
			api
				.getLibrary()
				.catch((e) => {
					toast.error(String(e));
					return api.getLocalPlaylists();
				})
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

	function pick(pl: BrowseItem) {
		if (ui.addPending) return;
		const songs = ui.addSongs;
		close();
		if (songs?.length) addSongsToPlaylist(pl, songs);
	}

	// Hands the songs to the create dialog, which adds them once the playlist exists.
	function createNew() {
		const songs = ui.addSongs ?? [];
		close();
		openNewPlaylist(songs);
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
		role="presentation"
		onclick={(e) => {
			if (e.target === e.currentTarget) close();
		}}
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
			<button
				class="mb-1 flex w-full items-center gap-3 rounded-lg p-2 text-left hover:bg-accent/10"
				onclick={createNew}
			>
				<div class="flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
					<HugeiconsIcon icon={Add01Icon} class="h-5 w-5" />
				</div>
				<span class="text-sm font-medium">{t('nav.new_playlist')}</span>
			</button>
			{#if hasLocalFiles}
				<p class="mb-2 px-2 text-xs text-muted-foreground">{t('dialogs.new_playlist.local_files')}</p>
			{/if}
			{#if targets.length > 1}
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
								<!-- thumb(): a playlist on this machine can wear a local file's art, a path. -->
								<img src={thumb(pl.thumbnail, 96)} alt="" class="h-10 w-10 rounded-md object-cover" />
							{:else}
								<div class="h-10 w-10 rounded-md bg-muted"></div>
							{/if}
							<div class="min-w-0">
								<div class="truncate text-sm font-medium">{pl.title}</div>
								{#if pl.subtitle}
									<div class="flex items-center gap-1 truncate text-xs text-muted-foreground">
										{#if api.isLocalPlaylist(pl.id)}
											<HugeiconsIcon icon={ComputerIcon} class="h-3 w-3 shrink-0" />
										{/if}
										<span class="truncate">{pl.subtitle}</span>
									</div>
								{/if}
							</div>
						</button>
					{/each}
				</div>
			{:else if filter.trim()}
				<!-- With no playlists at all there is nothing to say: the New playlist row is the way on. -->
				<p class="p-2 text-sm text-muted-foreground">{t('common.no_matches')}</p>
			{/if}
		</div>
	</div>
{/if}
