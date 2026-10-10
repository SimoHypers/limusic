<script lang="ts">
	import { MediaQuery, SvelteSet } from 'svelte/reactivity';
	import { fade, slide } from 'svelte/transition';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { PlusSignIcon, RefreshIcon, SparklesIcon, Tick02Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import TrackRow from './TrackRow.svelte';
	import TrackRowSkeleton from './TrackRowSkeleton.svelte';
	import * as api from '$lib/api';
	import type { SongItem } from '$lib/api';
	import { openAddToPlaylist, playback, toast } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	// Suggestions under a playlist you own (#395): YouTube Music's own shelf, or the playlist's
	// radio where YouTube has none (past 100 tracks). Inline at the foot of the list rather than in
	// a dialog: a pick lands in the rows right above it, so you watch the playlist grow as you go,
	// and nothing covers what you are building.
	let {
		playlistId,
		token,
		current,
		onadd
	}: {
		playlistId: string;
		/** `PlaylistPage.suggestions`. Read when the shelf first scrolls near, not at mount. */
		token?: string;
		/** The playlist's rows, so a suggestion it already holds is skipped. */
		current: () => SongItem[];
		/** Puts the song in the playlist. Answers whether it stuck. */
		onadd: (song: SongItem) => Promise<boolean>;
	} = $props();

	const reducedMotion = new MediaQuery('(prefers-reduced-motion: reduce)');
	const ms = (n: number) => (reducedMotion.current ? 0 : n);
	// How long a picked row shows "Added" before it folds away: long enough to read, short enough
	// that picking down the list never waits on it.
	const LINGER_MS = 450;
	// Rows on the shelf at once. YouTube's own shelf comes in sevens; a radio window is ~50.
	const SHOW = 10;

	// Raw: every write reassigns, and the rows go to TrackRow and `onadd` as plain objects.
	let items = $state.raw<SongItem[]>([]);
	// Fetched and not shown yet, so Refresh deals out the rest of a radio window before it asks
	// YouTube for another.
	let pool: SongItem[] = [];
	// Everything fetched so far: radio windows overlap, and a Refresh should never repeat a row.
	const seen = new Set<string>();
	// Where the next fetch picks up: the last batch's `refresh`, or `token` before the first.
	let cursor: string | undefined;
	let loading = $state(false);
	let failed = $state(false);
	let fetched = $state(false);
	// Bumped per batch, so a fresh batch fades in as a whole instead of row by row out of the old.
	let batch = $state(0);
	// Rows showing "Added" while they linger.
	const picked = new SvelteSet<string>();

	async function refill() {
		// A window can be all rows already seen or already in the playlist. A couple more tries
		// reach fresh ones; the cap keeps a radio that has run dry from asking forever.
		for (let tries = 0; tries < 3 && !pool.length; tries++) {
			const got = await api.getPlaylistSuggestions(playlistId, cursor ?? token);
			const have = new Set(current().map((s) => s.video_id));
			pool = got.items.filter((s) => !seen.has(s.video_id) && !have.has(s.video_id));
			for (const s of got.items) seen.add(s.video_id);
			const stuck = !got.refresh || got.refresh === cursor;
			cursor = got.refresh ?? cursor;
			if (stuck) break;
		}
	}

	async function load() {
		if (loading) return;
		loading = true;
		failed = false;
		try {
			if (!pool.length) await refill();
			items = pool.slice(0, SHOW);
			pool = pool.slice(SHOW);
			picked.clear();
			batch++;
		} catch {
			failed = true;
			// With rows on screen the failure has nowhere else to show: they just stay.
			if (items.length) toast.error(t('library.suggestions_failed'));
		} finally {
			loading = false;
			fetched = true;
		}
	}

	// One request per visit, and only for someone who scrolls down to it: most opens of a playlist
	// are to play it.
	function near(node: HTMLElement) {
		const io = new IntersectionObserver(
			([e]) => {
				if (!e.isIntersecting) return;
				io.disconnect();
				load();
			},
			{ rootMargin: '400px 0px' }
		);
		io.observe(node);
		return () => io.disconnect();
	}

	async function add(song: SongItem) {
		if (picked.has(song.video_id)) return;
		picked.add(song.video_id);
		const at = items.indexOf(song);
		const leave = setTimeout(() => {
			items = items.filter((s) => s !== song);
			// Picked the lot: the next batch follows on its own rather than leaving an empty shelf.
			if (!items.length) load();
		}, ms(LINGER_MS));
		if (await onadd(song)) return;
		// Refused: back where it was, with its Add button back.
		clearTimeout(leave);
		picked.delete(song.video_id);
		if (!items.includes(song)) items = [...items.slice(0, at), song, ...items.slice(at)];
	}

	// A batch plays as a queue of its own, so you can listen down the shelf and pick as you go.
	function play(i: number) {
		api.playPlaylist(items, i, undefined, t('library.suggestions')).catch((e) => toast.error(String(e)));
	}
</script>

<div {@attach near}></div>
{#if !fetched || loading || failed || items.length}
	<section class="mt-8 rounded-2xl bg-muted/40 p-2 pt-4" aria-busy={loading}>
		<div class="mb-2 flex items-center justify-between gap-4 px-2">
			<div class="flex min-w-0 items-center gap-3">
				<span
					class="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-primary/15 text-primary"
				>
					<HugeiconsIcon icon={SparklesIcon} class="h-4 w-4" />
				</span>
				<div class="min-w-0">
					<h2 class="truncate font-heading text-xl font-bold tracking-tight">
						{t('library.suggestions')}
					</h2>
					<p class="truncate text-xs text-muted-foreground">{t('library.suggestions_hint')}</p>
				</div>
			</div>
			<Button
				variant="ghost"
				size="sm"
				class="shrink-0 gap-2 text-muted-foreground"
				onclick={load}
				disabled={loading}
			>
				<!-- Spins only while a batch is in the air: nothing loops at rest. -->
				<HugeiconsIcon icon={RefreshIcon} class="h-4 w-4 {loading ? 'animate-spin' : ''}" />
				{t('common.refresh')}
			</Button>
		</div>

		{#if items.length}
			<!-- The old batch stays put, dimmed, until the new one is here to replace it. -->
			<div class="transition-opacity {loading ? 'opacity-50' : ''}">
				{#key batch}
					{#each items as song, i (song.video_id)}
						{@const added = picked.has(song.video_id)}
						<div
							in:fade|global={{ duration: ms(220), delay: ms(i * 35) }}
							out:slide={{ duration: ms(180) }}
						>
							<TrackRow
								{song}
								active={song.video_id === playback.now?.videoId}
								onplay={() => play(i)}
								onAdd={() => openAddToPlaylist(song)}
							>
								{#snippet trailing()}
									<!-- Always shown, unlike the row's other controls: adding is what the shelf
									     is for. No transition, like everything else on a row (UI-PERFORMANCE). -->
									<button
										class="ml-1 flex h-8 shrink-0 cursor-pointer items-center gap-1.5 rounded-full border px-3 text-xs font-medium {added
											? 'border-primary bg-primary text-primary-foreground'
											: 'hover:border-primary hover:bg-primary hover:text-primary-foreground'}"
										aria-label={t('library.suggestion_add', { title: song.title })}
										aria-disabled={added}
										onclick={(e) => {
											e.stopPropagation();
											add(song);
										}}
									>
										<!-- altIcon/showAlt, not a ternary: `icon` is read once at mount. -->
										<HugeiconsIcon
											icon={PlusSignIcon}
											altIcon={Tick02Icon}
											showAlt={added}
											class="h-3.5 w-3.5"
										/>
										{added ? t('library.suggestion_added') : t('common.add')}
									</button>
								{/snippet}
							</TrackRow>
						</div>
					{/each}
				{/key}
			</div>
		{:else if loading || !fetched}
			{#each Array(5) as _, i (i)}
				<TrackRowSkeleton />
			{/each}
		{:else}
			<div class="flex items-center gap-3 px-2 pb-2">
				<p class="text-sm text-muted-foreground">{t('library.suggestions_failed')}</p>
				<Button variant="outline" size="sm" onclick={load}>{t('common.try_again')}</Button>
			</div>
		{/if}
	</section>
{/if}
