<script lang="ts">
	import { onMount, untrack } from 'svelte';
	import { MediaQuery, SvelteSet } from 'svelte/reactivity';
	import { fade, scale, slide } from 'svelte/transition';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		ArrowDown01Icon,
		ArrowUp01Icon,
		LayerAddIcon,
		PlusSignIcon,
		Refresh01Icon,
		Tick02Icon,
		ViewIcon,
		ViewOffSlashIcon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import TrackRow from './TrackRow.svelte';
	import TrackRowSkeleton from './TrackRowSkeleton.svelte';
	import * as api from '$lib/api';
	import type { SongItem } from '$lib/api';
	import { openAddToPlaylist, playback, toast } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import { thumb } from '$lib/thumb';

	// Suggestions under a playlist you own (#395): YouTube Music's own shelf, or the playlist's
	// radio where YouTube has none (past 100 tracks). Inline at the foot of the list rather than in
	// a dialog: a pick lands in the rows right above it, so you watch the playlist grow as you go,
	// and nothing covers what you are building.
	//
	// On a long playlist the foot of the list is a long way down (#457), so the shelf also folds into
	// a dock at the page's corner: a pill that opens into a tray holding the same rows. It steps
	// aside whenever the shelf itself is on screen, so there is only ever one of them.
	let {
		playlistId,
		token,
		current,
		onadd,
		inline
	}: {
		playlistId: string;
		/** `PlaylistPage.suggestions`. Read once, at mount. */
		token?: string;
		/** The playlist's rows, so a suggestion it already holds is skipped. */
		current: () => SongItem[];
		/** Puts the song in the playlist. Answers whether it stuck. */
		onadd: (song: SongItem) => Promise<boolean>;
		/** Every page of the playlist is in, so the shelf has a foot of the list to sit at. Until
		 * then only the dock shows. */
		inline: boolean;
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
	// Something to show, or about to be: a radio that came back empty takes the shelf and the dock
	// away together.
	const live = $derived(!fetched || loading || failed || items.length > 0);
	let open = $state(false);
	// Taken as on screen when the shelf is there from the start, until its observer answers: on a
	// short playlist it is, and the pill would flash up for a frame and fade out over it.
	let shelfInView = $state(untrack(() => inline));

	// Hidden (#508) folds the shelf to its header at the foot of the list and takes the dock away.
	// One switch for every playlist, kept until shown again, and nothing is fetched while it is on.
	const HIDDEN_KEY = 'playlist_suggestions_hidden';
	let hidden = $state(storedHidden());
	function storedHidden() {
		try {
			return localStorage.getItem(HIDDEN_KEY) === '1';
		} catch {
			return false;
		}
	}
	function setHidden(v: boolean, docked = false) {
		hidden = v;
		open = false;
		// Shown again from the foot of the list, so the shelf opens on screen: without this the pill
		// flashes up for the frame before the observer answers.
		shelfInView = inline;
		try {
			localStorage.setItem(HIDDEN_KEY, v ? '1' : '0');
		} catch {
			// Not stored: it still holds for this visit.
		}
		// From the dock the whole thing vanishes, so say where it went.
		if (v && docked) toast.info(t('library.suggestions_hidden'));
		if (!v && !fetched) load();
	}
	const dockable = $derived(live && !hidden && !shelfInView);

	async function refill() {
		// A window can be all rows already seen or already in the playlist. A couple more tries
		// reach fresh ones; the cap keeps a radio that has run dry from asking forever.
		for (let tries = 0; tries < 3 && !pool.length && !hidden; tries++) {
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
		// Hidden: the reload a last pick schedules stops here, refill's retries at its loop.
		if (loading || hidden) return;
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

	// One request per visit, at open: the dock shows the batch's covers, so it needs the batch
	// before anyone scrolls or clicks. The shelf at the foot of the list then keeps that batch.
	onMount(() => {
		if (!hidden) load();
	});

	// Where the shelf is on screen the dock has nothing to add, and an open tray would be a second
	// copy of the rows right next to the first.
	function watchShelf(node: HTMLElement) {
		const io = new IntersectionObserver(([e]) => {
			shelfInView = e.isIntersecting;
			if (shelfInView) open = false;
		});
		io.observe(node);
		return () => {
			io.disconnect();
			shelfInView = false;
		};
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

{#snippet header(docked: boolean)}
	<div class="{hidden ? '' : 'mb-2'} flex items-center justify-between gap-4 px-2">
		<div class="flex min-w-0 items-center gap-3">
			<span
				class="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-primary/15 text-primary"
			>
				<HugeiconsIcon icon={LayerAddIcon} strokeWidth={2} class="h-4 w-4" />
			</span>
			<div class="min-w-0">
				<h2 class="truncate font-heading text-xl font-bold tracking-tight">
					{t('library.suggestions')}
				</h2>
				<p class="truncate text-xs text-muted-foreground">{t('library.suggestions_hint')}</p>
			</div>
		</div>
		<div class="flex shrink-0 items-center gap-1">
			{#if hidden}
				<Button
					variant="ghost"
					size="sm"
					class="gap-2 text-muted-foreground"
					onclick={() => setHidden(false)}
				>
					<HugeiconsIcon icon={ViewIcon} class="h-4 w-4" />
					{t('library.suggestions_show')}
				</Button>
			{:else}
				<Button
					variant="ghost"
					size="sm"
					class="gap-2 text-muted-foreground"
					onclick={load}
					disabled={loading}
				>
					<!-- Spins only while a batch is in the air: nothing loops at rest. -->
					<HugeiconsIcon icon={Refresh01Icon} class="h-4 w-4 {loading ? 'animate-spin' : ''}" />
					{t('common.refresh')}
				</Button>
				<Button
					variant="ghost"
					size="sm"
					class="gap-2 text-muted-foreground"
					onclick={() => setHidden(true, docked)}
				>
					<HugeiconsIcon icon={ViewOffSlashIcon} class="h-4 w-4" />
					{t('library.suggestions_hide')}
				</Button>
			{/if}
			{#if docked}
				<Button
					variant="ghost"
					size="icon"
					class="text-muted-foreground"
					aria-label={t('a11y.close')}
					onclick={() => (open = false)}
				>
					<HugeiconsIcon icon={ArrowDown01Icon} class="h-4 w-4" />
				</Button>
			{/if}
		</div>
	</div>
{/snippet}

{#snippet body(docked: boolean)}
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
						<!-- hideRating in the tray: like the queue panel, it is too narrow for the thumbs
						     and the Add button both. -->
						<TrackRow
							{song}
							active={song.video_id === playback.now?.videoId}
							hideRating={docked}
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
									<!-- Trimmed to cap height so it centres on the glyphs, not on a line box
									     that carries descender space below them: untrimmed, the word sat ~1.5px
									     above the plus. Fonts change per theme, so no fixed nudge. -->
									<span class="[text-box:trim-both_cap_alphabetic]">
										{added ? t('library.suggestion_added') : t('common.add')}
									</span>
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
{/snippet}

{#if inline}
	<div class="px-4 pb-6">
		{#if hidden}
			<section class="mt-8 rounded-2xl bg-muted/40 p-2 py-3">
				{@render header(false)}
			</section>
		{:else if live}
			<section class="mt-8 rounded-2xl bg-muted/40 p-2 pt-4" aria-busy={loading} {@attach watchShelf}>
				{@render header(false)}
				{@render body(false)}
			</section>
		{/if}
	</div>
{/if}

<!-- The dock. Fixed, placed like home's back-to-top: it clears the player bar when there is one,
     and z-10 keeps it under the queue and lyrics overlays. Not absolute against the page: rendered
     inside the scroller, WebKitGTK drew an absolute one off the corner (half under the player bar)
     until the window was resized. Plain fade and one scale on a single element: nothing repeats. -->
{#if dockable && open}
	<div
		class="fixed right-6 z-10 flex max-h-[min(36rem,calc(100vh-12rem))] w-[min(30rem,calc(100vw-3rem))] origin-bottom-right {playback.now ? 'bottom-24' : 'bottom-6'} flex-col rounded-2xl border bg-card p-2 pt-3 shadow-xl outline-none"
		role="dialog"
		aria-label={t('library.suggestions')}
		aria-busy={loading}
		tabindex="-1"
		onkeydown={(e) => e.key === 'Escape' && (open = false)}
		{@attach (node) => node.focus()}
		transition:scale={{ start: 0.95, duration: ms(160) }}
	>
		<!-- Focused on open: the pill that opened it is gone, and the tray sits after the whole list
		     in tab order, so a keyboard would otherwise start from the top of the page. -->
		{@render header(true)}
		<div class="min-h-0 overflow-y-auto">
			{@render body(true)}
		</div>
	</div>
{:else if dockable}
	<!-- The shelf's own badge until the batch is in, then the next picks' covers: it reads as the
	     same shelf, pocketed. -->
	<button
		class="fixed right-6 z-10 flex h-11 {playback.now ? 'bottom-24' : 'bottom-6'} cursor-pointer items-center gap-2.5 rounded-full border bg-card pl-1.5 pr-4 text-sm font-medium shadow-lg transition-colors hover:bg-muted"
		aria-expanded="false"
		onclick={() => (open = true)}
		transition:fade={{ duration: ms(150) }}
	>
		{#if items.length}
			<span class="flex -space-x-3">
				<!-- |global so they also deal in when the batch lands and when a pick brings the next
				     cover up, not only when the pill itself appears. -->
				{#each items.slice(0, 3) as song, i (song.video_id)}
					<img
						src={thumb(song.thumbnail, 96)}
						alt=""
						class="h-8 w-8 rounded-full bg-muted object-cover ring-2 ring-card"
						in:scale|global={{ start: 0.6, duration: ms(220), delay: ms(i * 50) }}
					/>
				{/each}
			</span>
		{:else}
			<span
				class="flex h-8 w-8 items-center justify-center rounded-full bg-primary/15 text-primary"
			>
				<HugeiconsIcon icon={LayerAddIcon} strokeWidth={2} class="h-4 w-4" />
			</span>
		{/if}
		{t('library.suggestions')}
		<HugeiconsIcon icon={ArrowUp01Icon} class="h-4 w-4 text-muted-foreground" />
	</button>
{/if}
