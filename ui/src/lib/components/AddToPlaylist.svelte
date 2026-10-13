<script lang="ts">
	import { fade, fly, scale } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Add01Icon,
		ArrowLeft01Icon,
		Cancel01Icon,
		ComputerIcon,
		Copy01Icon,
		SquareArrowRightDoubleIcon,
		Tick02Icon
	} from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import { ui, toast, addSongsToPlaylists, openNewPlaylist, savedIn } from '$lib/player.svelte';
	import { Button } from '$lib/components/ui/button';

	let playlists = $state<BrowseItem[]>([]);
	let loading = $state(false);
	let filter = $state('');
	let selectedIds = $state<string[]>([]);
	let box = $state<HTMLInputElement | null>(null);
	let step = $state<'pick' | 'dupes'>('pick');
	let skipBtn = $state<HTMLButtonElement | null>(null);
	// Which way the counter rolls: up as playlists are ticked, down as they're unticked.
	let dir = $state(1);
	// `autofocus` is unreliable on an element inserted after load (and mid-transition), so focus it
	// ourselves the frame it exists: the modal opens ready to type, the duplicates step ready to Enter.
	$effect(() => {
		box?.focus();
	});
	$effect(() => {
		skipBtn?.focus();
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
	const chosen = $derived(targets.filter((p) => selectedIds.includes(p.id)));
	const songs = $derived(ui.addSongs ?? []);
	// How many of the songs each chosen playlist already holds, per the savedIn index.
	const dupeRows = $derived(
		chosen
			.map((pl) => ({
				pl,
				n: songs.filter((s) => savedIn.map[s.video_id]?.includes(pl.id)).length
			}))
			.filter((r) => r.n)
	);
	const dupeTotal = $derived(dupeRows.reduce((sum, r) => sum + r.n, 0));
	// Playlists on this machine never hold a song twice, so "Add anyway" skips those too.
	const localDupes = $derived(
		dupeRows.reduce((sum, r) => sum + (api.isLocalPlaylist(r.pl.id) ? r.n : 0), 0)
	);
	const skipAdds = $derived(chosen.length * songs.length - dupeTotal);
	const anywayAdds = $derived(chosen.length * songs.length - localDupes);

	// Fetch the library playlists fresh each time the picker opens (cheap; picks up new playlists).
	// On Repeat and Liked Music are dropped: On Repeat is built from local play counts, and Liked
	// Music takes likes rather than playlist edits (YouTube 400s the add). The command boundary
	// refuses both too, but a target you can tap and can't use is the bug. When the account's half
	// can't be fetched, the playlists on this machine still can: they need no network.
	$effect(() => {
		if (ui.addSongs) {
			loading = true;
			filter = '';
			selectedIds = [];
			step = 'pick';
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

	function toggleSelect(id: string) {
		const on = selectedIds.includes(id);
		dir = on ? -1 : 1;
		selectedIds = on ? selectedIds.filter((i) => i !== id) : [...selectedIds, id];
	}

	function confirm() {
		if (ui.addPending || !chosen.length || !songs.length) return;
		if (dupeTotal) step = 'dupes';
		else add('skip');
	}

	function add(mode: 'skip' | 'anyway') {
		// Snapshot before close(): both are derived from ui.addSongs, which close() clears.
		const picked = chosen;
		const batch = songs;
		close();
		addSongsToPlaylists(picked, batch, mode);
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
		if (!ui.addSongs || e.key !== 'Escape') return;
		if (step === 'dupes') step = 'pick';
		else close();
	}}
/>

{#if ui.addSongs}
	<div
		transition:fade={{ duration: 150 }}
		class="fixed inset-0 z-[49] flex items-center justify-center bg-black/50 p-4"
		role="presentation"
		onclick={(e) => {
			if (e.target === e.currentTarget) close();
		}}
	>
		<div
			transition:scale={{ duration: 180, start: 0.96, easing: cubicOut }}
			class="flex max-h-[32rem] w-full max-w-sm flex-col rounded-xl border bg-card p-4 shadow-xl"
		>
			<div class="mb-3 flex items-center gap-2">
				{#if step === 'dupes'}
					<button
						class="-ml-1 flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
						onclick={() => (step = 'pick')}
						aria-label={t('common.back')}
					>
						<HugeiconsIcon icon={ArrowLeft01Icon} class="h-4 w-4" />
					</button>
				{/if}
				<div class="min-w-0 flex-1">
					<h2 class="font-heading text-base font-semibold">
						{step === 'pick' ? t('player.add_to_playlist') : t('add_playlist.dupes_title')}
					</h2>
					{#if step === 'dupes'}
						<p class="truncate text-xs text-muted-foreground">
							{songs.length === 1 ? songs[0].title : t('library.songs_count', { count: songs.length })}
						</p>
					{/if}
				</div>
				<button
					class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
					onclick={close}
					aria-label={t('a11y.close')}
				>
					<HugeiconsIcon icon={Cancel01Icon} class="h-4 w-4" />
				</button>
			</div>
			{#if step === 'pick'}
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
						onkeydown={(e) => {
							if (e.key === 'Enter' && matches[0]) {
								toggleSelect(matches[0].id);
							}
						}}
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
								onclick={() => toggleSelect(pl.id)}
								aria-pressed={selectedIds.includes(pl.id)}
							>
								<span
									aria-hidden="true"
									class="flex h-4 w-4 shrink-0 items-center justify-center rounded-sm border {selectedIds.includes(pl.id) ? 'bg-primary text-primary-foreground border-primary' : ''}"
								>
									{#if selectedIds.includes(pl.id)}
										<HugeiconsIcon icon={Tick02Icon} strokeWidth={2.5} class="h-3 w-3" />
									{/if}
								</span>
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
				<div class="mt-3 pt-2">
					<Button
						class="relative w-full"
						disabled={selectedIds.length === 0 || ui.addPending}
						onclick={confirm}
					>
						{selectedIds.length === 1 ? t('add_playlist.confirm_one') : t('add_playlist.confirm')}
						{#if selectedIds.length}
							<!-- Pinned to the edge so the label never shifts. An odometer: one small element, so
							     its transforms cost nothing (docs/UI-PERFORMANCE.md). -->
							<span
								transition:scale={{ duration: 150, start: 0.4, easing: cubicOut }}
								class="absolute top-1/2 right-2 grid h-5 -translate-y-1/2 min-w-5 place-items-center overflow-hidden rounded-full bg-primary-foreground px-1.5 text-xs font-semibold tabular-nums text-primary"
							>
								{#key selectedIds.length}
									<span
										class="[grid-area:1/1]"
										in:fly={{ y: 12 * dir, duration: 180, easing: cubicOut }}
										out:fly={{ y: -12 * dir, duration: 180, easing: cubicOut }}
									>
										{selectedIds.length}
									</span>
								{/key}
							</span>
						{/if}
					</Button>
				</div>
			{:else}
				<div in:fade={{ duration: 120 }} class="flex min-h-0 flex-1 flex-col">
					<div class="min-h-0 flex-1 overflow-y-auto">
						{#each dupeRows as { pl, n } (pl.id)}
							<div class="flex items-center gap-3 rounded-lg p-2">
								{#if pl.thumbnail}
									<img src={thumb(pl.thumbnail, 96)} alt="" class="h-10 w-10 rounded-md object-cover" />
								{:else}
									<div class="h-10 w-10 rounded-md bg-muted"></div>
								{/if}
								<div class="min-w-0 flex-1">
									<div class="truncate text-sm font-medium">{pl.title}</div>
									<div class="flex items-center gap-1 text-xs text-muted-foreground">
										{#if api.isLocalPlaylist(pl.id)}
											<HugeiconsIcon icon={ComputerIcon} class="h-3 w-3 shrink-0" />
										{/if}
										<span class="truncate">
											{songs.length === 1
												? t('add_playlist.dupes_there')
												: t('add_playlist.dupes_of', { count: n, total: songs.length })}
										</span>
									</div>
								</div>
								{#if songs.length > 1}
									<!-- How much of the batch this playlist already has. -->
									<div class="h-1.5 w-12 shrink-0 overflow-hidden rounded-full bg-muted">
										<div class="h-full rounded-full bg-primary" style:width="{(n / songs.length) * 100}%"></div>
									</div>
								{/if}
							</div>
						{/each}
					</div>
					<div class="mt-3 flex flex-col gap-2">
						<button
							bind:this={skipBtn}
							class="flex w-full items-center gap-3 rounded-xl bg-primary p-3 text-left text-primary-foreground transition-colors outline-none hover:bg-primary/85 focus-visible:ring-[3px] focus-visible:ring-ring/50"
							onclick={() => add('skip')}
						>
							<span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary-foreground/15">
								<HugeiconsIcon icon={SquareArrowRightDoubleIcon} class="h-4 w-4" />
							</span>
							<span class="min-w-0 flex-1">
								<span class="block text-sm font-semibold">{t('add_playlist.dupes_skip')}</span>
								<span class="block text-xs opacity-80">
									{skipAdds ? t('add_playlist.skip_hint') : t('add_playlist.skip_none')}
								</span>
							</span>
							{#if skipAdds}
								<span class="font-heading text-base font-semibold tabular-nums">
									{t('add_playlist.adds', { count: skipAdds })}
								</span>
							{/if}
						</button>
						{#if anywayAdds > skipAdds}
							<button
								class="flex w-full items-center gap-3 rounded-xl border p-3 text-left transition-colors outline-none hover:bg-muted focus-visible:ring-[3px] focus-visible:ring-ring/50"
								onclick={() => add('anyway')}
							>
								<span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-muted">
									<HugeiconsIcon icon={Copy01Icon} class="h-4 w-4" />
								</span>
								<span class="min-w-0 flex-1">
									<span class="block text-sm font-semibold">{t('add_playlist.dupes_add')}</span>
									<span class="block text-xs text-muted-foreground">
										{localDupes ? t('add_playlist.anyway_hint_local') : t('add_playlist.anyway_hint')}
									</span>
								</span>
								<span class="font-heading text-base font-semibold tabular-nums">
									{t('add_playlist.adds', { count: anywayAdds })}
								</span>
							</button>
						{/if}
					</div>
				</div>
			{/if}
		</div>
	</div>
{/if}
