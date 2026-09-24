<script lang="ts">
	import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/svelte';
	import {
		FavouriteIcon,
		MusicNote01Icon,
		PlayIcon,
		PlayListAddIcon,
		ThumbsDownIcon,
		ThumbsUpIcon
	} from '@hugeicons/core-free-icons';
	import * as api from '$lib/api';
	import type { SongItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { lt } from '$lib/lt.svelte';
	import { anySaved, isLiked, ratingOf, savedPlaylists, toggleRating } from '$lib/player.svelte';
	import SavedInPlaylists from './SavedInPlaylists.svelte';
	import TrackMenu from './TrackMenu.svelte';
	import ArtistLine from './ArtistLine.svelte';
	import ExplicitIcon from './ExplicitIcon.svelte';
	import { t } from '$lib/i18n.svelte';
	import type { TrackSelection } from '$lib/selection.svelte';
	import { Checkbox } from './ui/checkbox';

	let {
		song,
		index,
		active = false,
		hideThumb = false,
		compact = false,
		showPlayCount = false,
		hideRating = false,
		onplay,
		onAdd,
		onRemove,
		removeLabel = t('player.remove_from_playlist'),
		playlistId,
		queueIndex,
		inLibraryList = false,
		selection,
		selectionKey,
		lazy = false
	}: {
		song: SongItem;
		/** Position badge when set (playlist/queue); omitted for flat search results. */
		index?: number;
		active?: boolean;
		/** Hide the leading thumbnail (album track lists show a number, not a cover). */
		hideThumb?: boolean;
		/**
		 * Grid variant (home's Forgotten favourites): the duration joins the artist line instead of
		 * claiming its own column, and a like heart sits next to the ⋯ — narrow columns have no room
		 * for a separate duration column, and hearting is the whole point of that shelf.
		 */
		compact?: boolean;
		/**
		 * Opt-in, because `play_count` rides along on the song object wherever it goes after an album
		 * page (queue, previously played) and a narrow panel has no width to spare for it.
		 */
		showPlayCount?: boolean;
		/**
		 * The narrow queue-panel variant: drops the inline thumbs and the explicit mark. Two buttons
		 * plus the duration leave nothing for the title and artists at that width, and the queue is
		 * not where you decide what to listen to. The ⋯ menu carries like and dislike either way.
		 */
		hideRating?: boolean;
		onplay: () => void;
		/** The row lives in Library ▸ Songs: passed through so its menu drops "Save to library". */
		inLibraryList?: boolean;
		/** Adds an "Add to playlist" menu item. */
		onAdd?: () => void;
		/** Adds a remove menu item (label via `removeLabel`). */
		onRemove?: () => void;
		removeLabel?: string;
		/** The playlist this row is playing from — adds "Remove from this playlist" to its menu. */
		playlistId?: string | null;
		/** This row's index in the backend queue, where it is one (the queue panel). */
		queueIndex?: number;
		/** Optional list-owned selection; the key identifies this occurrence, not the song. */
		selection?: TrackSelection;
		selectionKey?: string;
		/**
		 * For a list that mounts every row it has and grows without bound (Library ▸ Songs, Local
		 * music, History, all three paginated by an IntersectionObserver): lets the browser skip the
		 * rows that are off screen. Off by default because it is not free — see the comment on the
		 * row below.
		 */
		lazy?: boolean;
	} = $props();
	const selectionDescriptionId = $props.id();

	// In a session as guest, clicking a song adds it to the shared queue instead of playing it —
	// reflect that in the hover icon + label so the row doesn't lie.
	const guestAdd = $derived(lt.role === 'guest');
	// Only in select mode: at rest the row is a plain click-to-play row, with no checkbox and no
	// Space/click rebinding.
	const selectable = $derived(!!selection?.active && selectionKey !== undefined);
	const selected = $derived(selection?.has(selectionKey) ?? false);

	function select(range = false) {
		if (selection && selectionKey !== undefined) selection.toggle(selectionKey, range);
	}

	function clickRow(e: MouseEvent) {
		// In select mode the row selects; Enter (onKey) is what still plays it.
		if (selectable) {
			e.preventDefault();
			select(e.shiftKey);
			return;
		}
		onplay();
	}

	// Digits and colons, nothing else. A queue saved before the parser stopped reading a name with a
	// colon in it ("Cast of EPIC: The Musical") as a length still holds those strings, and printing
	// one here squeezes the title and artists down to nothing.
	const duration = $derived(/^[\d:]+$/.test(song.duration ?? '') ? song.duration : undefined);

	const rated = $derived(ratingOf(song));
	// A local file has no YouTube identity, so there is nothing to rate (the same guard the ⋯ menu
	// applies to its like item). The compact variant has no room: it keeps its single heart.
	const showRating = $derived(!compact && !hideRating && !api.isLocalId(song.video_id));

	// Your own playlists holding this song, for the "saved" mark. Gated on `showRating` because it
	// answers the same three questions: a local file is in no YTM playlist, and the compact and
	// queue variants have no width left for another mark.
	const inPlaylists = $derived(showRating ? savedPlaylists(song.video_id) : []);

	// The whole row is a play target (role="button"), so mirror native button keyboard activation.
	// Only when the key lands on the row itself — keydowns bubble up from nested interactive
	// elements (⋯ menu, artist link), and hijacking those would play the row instead.
	function onKey(e: KeyboardEvent) {
		if (e.target !== e.currentTarget) {
			if (e.key === ' ') e.stopPropagation();
			return;
		}
		if (selection && selectable) {
			if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
				e.preventDefault();
				e.stopPropagation();
				selection.selectAll();
				return;
			}
			if (e.key === 'Escape' || e.key === ' ') {
				e.preventDefault();
				e.stopPropagation();
				if (!e.repeat) {
					if (e.key === 'Escape') selection.exit();
					else select(e.shiftKey);
				}
				return;
			}
		}
		if (e.key === 'Enter' || e.key === ' ') {
			e.preventDefault();
			onplay();
		}
	}
</script>

<!-- Both rating buttons, so they can't drift apart. `icon` is a constant per call site, not a
     reactive ternary, which is the only way HugeiconsIcon takes it (it freezes at mount). -->
{#snippet rateButton(icon: IconSvgElement, want: 'like' | 'dislike', label: string)}
	<button
		class="cursor-pointer rounded-md p-1.5 text-muted-foreground hover:bg-accent/20 hover:text-foreground"
		aria-label={rated === want ? t('player.remove_rating') : label}
		aria-pressed={rated === want}
		onclick={(e) => {
			e.stopPropagation();
			toggleRating(song, want);
		}}
	>
		<!-- Liked wears the accent; disliked fills plain, since the accent reads as approval. -->
		<HugeiconsIcon
			{icon}
			class="h-4 w-4 {rated === want
				? `fill-current ${want === 'like' ? 'text-primary' : 'text-foreground'}`
				: ''}"
		/>
	</button>
{/snippet}

<!-- content-visibility, and only where `lazy` asks for it: a liked-songs list runs to thousands of
     rows and the browser keeps every one in style, layout and paint. 3.5rem is a row (8px padding,
     40px thumbnail, 8px); `auto` swaps in the measured size after first paint.

     It is not the free win it looks like, which is why it is opt-in now (issue #311). A row locking
     or unlocking changes the paint artifact, so on a list where rows cross the boundary on every
     frame the browser re-composites the whole region rather than sliding a layer. Measured in
     Chromium at 6x CPU throttling (`ui/perf/scroll.mjs`), frames over 20 ms across a wheel gesture:
     the queue panel pays 28% for it and 7% without, while Library ▸ Songs at 400 rows pays 8% with
     it and 29% without. The queue and the playlist page window their rows themselves
     (`rows.svelte.ts`), so they have nothing left for it to skip; the three paginated lists do.

     No `transition-colors`, and the hover-only controls below hide with `invisible`, not `opacity-0`
     (issue #311). Scrolling drags the whole list under a stationary pointer, so every row that goes
     past takes :hover and drops it again — with a transition that is a 150 ms repaint per row with
     several always in flight, and `opacity` below 1 puts an effect node in the paint property tree
     for every row, which Chromium re-walks on every frame it composites. Measured in Chromium at 4x
     CPU throttling (`ui/perf/scroll.mjs`), frames over 20 ms across a wheel gesture: queue panel
     50% -> 6%, Library > Songs 14% -> 1%. Hover still lands, it just lands at once. -->
<div
	role="button"
	tabindex="0"
	data-ctx
	onclick={clickRow}
	onkeydown={onKey}
	data-selection-key={selectionKey}
	data-selected={selectable ? selected : undefined}
	aria-describedby={selectable ? selectionDescriptionId : undefined}
	aria-label={selectable ? t(guestAdd ? 'selection.track_guest' : 'selection.track', { title: song.title }) : guestAdd ? `Add ${song.title} to the session queue` : `Play ${song.title}`}
	class="group flex w-full cursor-pointer items-center gap-3 rounded-lg p-2 hover:bg-accent/10 {selected
		? 'bg-primary/15'
		: active
		? 'bg-accent/10'
		: ''} {lazy && !compact ? '[content-visibility:auto] [contain-intrinsic-size:auto_3.5rem]' : ''}"
>
	{#if selectable}
		<span id={selectionDescriptionId} class="sr-only">
			{t(selected ? 'selection.selected' : 'selection.not_selected')}
		</span>
		<Checkbox
			checked={selected}
			aria-label={t('selection.select_track', { title: song.title })}
			class="cursor-pointer"
			onclick={(e) => {
				// The list owns checked state, including Shift ranges whose endpoint stays selected.
				e.preventDefault();
				e.stopPropagation();
				select(e.shiftKey);
			}}
			onkeydown={(e) => {
				if (e.key === ' ') {
					e.preventDefault(); e.stopPropagation();
					if (!e.repeat) select(e.shiftKey);
				}
				if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); selection!.exit(); }
				if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
					e.preventDefault(); e.stopPropagation(); selection!.selectAll();
				}
			}}
		/>
	{/if}
	<div class="flex min-w-0 flex-1 items-center gap-3">
		<div class="flex min-w-0 shrink-0 items-center gap-3">
			{#if index !== undefined}
				<span
					class="relative w-5 shrink-0 text-center text-xs {active
						? 'text-primary'
						: 'text-muted-foreground'}"
				>
					<span class={selectable ? '' : 'group-hover:invisible'}>{index + 1}</span>
					<HugeiconsIcon
						icon={guestAdd ? PlayListAddIcon : PlayIcon}
						class="invisible absolute inset-0 m-auto h-3.5 w-3.5 {selectable ? '' : 'group-hover:visible'}"
					/>
				</span>
			{/if}
			{#if !hideThumb}
				{#if song.thumbnail}
					<img src={thumb(song.thumbnail, 96)} alt="" class="h-10 w-10 shrink-0 rounded-md object-cover" loading="lazy" />
				{:else}
					<!-- An untagged file has no artwork of its own. A music note keeps the row aligned
					     with its neighbours and says so plainly. -->
					<div
						class="flex h-10 w-10 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground/50"
					>
						<HugeiconsIcon icon={MusicNote01Icon} class="h-4 w-4" />
					</div>
				{/if}
			{/if}
		</div>
		<div class="min-w-0 flex-1">
			<div class="flex min-w-0 items-center gap-2">
				<span class="min-w-0 truncate text-sm font-medium {active ? 'text-primary' : ''}">
					{song.title}
				</span>
				{#if song.queued_by}
					<span
						class="shrink-0 rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium text-primary"
					>
						{song.queued_by}
					</span>
				{/if}
			</div>
			<div class="flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
				<ArtistLine runs={song.artist_runs} text={song.artists} />
				{#if compact && duration}
					<!-- No leading dot with nothing before it: a search row can come back artist-less
					     (YouTube drops the name when the query is the artist), leaving the length alone
					     on the line. -->
					<span class="shrink-0">{song.artists.trim() ? '· ' : ''}{duration}</span>
				{/if}
			</div>
		</div>
	</div>

	<!-- Album rows only. Wide rows are mostly empty between the title and the duration, so it takes a
	     centred column of its own there; narrow ones sit it next to the duration at its natural width
	     rather than dropping it, since it never needs more than "1,234 plays" worth of room. -->
	{#if song.play_count && showPlayCount && !compact}
		<div class="flex shrink-0 items-center justify-center text-xs text-muted-foreground lg:flex-1">
			<span class="truncate">{t('library.play_count', { count: song.play_count })}</span>
		</div>
	{/if}

	<div class="flex shrink-0 items-center {compact ? 'gap-0.5' : 'gap-2'}">
		<!-- Collaborative playlists only: who put this track in the list. Every row on such a page
		     carries one, so it can't leave a hole on its neighbours. -->
		{#if song.added_by_avatar && !compact}
			<img
				src={song.added_by_avatar}
				alt=""
				title={t('library.added_by', { user: song.added_by ?? '' })}
				class="h-5 w-5 shrink-0 rounded-full object-cover"
				loading="lazy"
			/>
		{/if}
		<!-- Both marks are always on, unlike the thumbs beside them: they are properties of the song,
		     not actions, so hiding either until the pointer arrives would hide half of what it's for.
		     Each also keeps its slot when it has nothing to show, or a track without an explicit tag
		     would pull its checkmark into the tag's place and the column would zig-zag down the list.
		     The checkmark's slot only exists once something is indexed: signed out, or before the
		     first crawl, reserving it would be a hole on every row that can never fill. -->
		{#if showRating && anySaved()}
			<span class="flex h-7 w-7 shrink-0 items-center justify-center">
				{#if inPlaylists.length}
					<SavedInPlaylists playlists={inPlaylists} />
				{/if}
			</span>
		{/if}
		{#if !hideRating}
			<span class="flex h-3.5 w-3.5 shrink-0 items-center">
				{#if song.explicit}
					<ExplicitIcon class="h-3.5 w-3.5 text-muted-foreground" />
				{/if}
			</span>
		{/if}
		{#if showRating}
			<!-- Hover-revealed, except on a row that carries a rating: at rest that filled thumb is the
			     only place the state shows at all. Hidden rather than removed, so the duration and the ⋯
			     don't shift sideways when the pointer arrives. `group-focus-within`, not the button's own
			     `focus-within`: a hidden element can't take focus, so the row (which is focusable) is
			     what has to reveal it before Tab can reach it. -->
			<div
				class="flex items-center gap-0.5 {rated === 'indifferent'
					? 'invisible group-focus-within:visible group-hover:visible'
					: ''}"
			>
				{@render rateButton(ThumbsUpIcon, 'like', t('common.like'))}
				{@render rateButton(ThumbsDownIcon, 'dislike', t('common.dislike'))}
			</div>
		{/if}
		{#if duration && !compact}
			<span class="shrink-0 text-xs tabular-nums text-muted-foreground">{duration}</span>
		{/if}
		{#if compact}
			<!-- Persistent, not hover-only: a filled heart is state the row has to keep showing. -->
			<button
				class="cursor-pointer rounded-md p-1.5 text-muted-foreground hover:bg-accent/20 hover:text-foreground"
				aria-label={isLiked(song) ? t('player.remove_from_liked') : t('player.save_to_liked')}
				aria-pressed={isLiked(song)}
				onclick={(e) => {
					e.stopPropagation();
					toggleRating(song, 'like');
				}}
			>
				<HugeiconsIcon
					icon={FavouriteIcon}
					class="h-4 w-4 {isLiked(song) ? 'fill-current text-primary' : ''}"
				/>
			</button>
		{/if}
		<TrackMenu
			{song}
			{onAdd}
			{onRemove}
			{removeLabel}
			{playlistId}
			{queueIndex}
			{inLibraryList}
			triggerClass="cursor-pointer rounded-md p-1.5 text-muted-foreground hover:bg-accent/20 hover:text-foreground {compact
				? ''
				: 'invisible group-focus-within:visible group-hover:visible'}"
		/>
	</div>
</div>
