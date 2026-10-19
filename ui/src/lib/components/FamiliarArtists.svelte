<script lang="ts">
	// The artists you actually play, ranked by the plays the stats page counts (#497).
	//
	// What makes this section worth a slot is the one number no YouTube shelf has, your own play
	// count, so every artist carries it. It has been a flower of floating avatars, then a poster for
	// #1 beside rows whose backgrounds filled in proportion to their plays; those fills read as five
	// boxes of different widths (#319). Now it is six posters in rank order, the same frame the feed
	// uses for an artist (PortraitCard), with the rank and the count set on the photograph: the order
	// carries the ranking, the count carries the "yours".
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { UserLove02Icon, UserIcon, UserStar01Icon } from '@hugeicons/core-free-icons';
	import SectionHeading from './SectionHeading.svelte';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import PlaylistMenu from './PlaylistMenu.svelte';
	import * as api from '$lib/api';
	import type { ArtistPage, BrowseItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { putCached } from '$lib/pagecache';
	import { artistPage } from '$lib/browse';
	import { toast } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	// ponytail: one browse call per artist, because the subscriber count and the subscribe state
	// only exist on the artist page. Six of them, shared with the artist route's cache (same key),
	// fired once per mount. Cut the count before reaching for a batch endpoint that doesn't exist.
	const COUNT = 6; // one row of posters at full width, whole rows of two or three narrower
	const MIN = 3; // fewer familiar artists than this and the section isn't worth a slot
	const SLACK = 2; // extra ids fetched, to backfill the ones whose page comes back unparseable
	// ponytail: artists without a channel (local files, unlinked credits) are dropped after the
	// fetch, so a top 30 made almost entirely of them can leave the section short. Filter in Rust if
	// that ever happens.
	const TOP = 30;

	/**
	 * The play count is carried on the row rather than looked up by `channelId` at render time,
	 * because the two ids are not the same one: we browse by the id the song's credit links, and
	 * the page comes back carrying the *subscribe button's* channel (an artist's official channel,
	 * which is often a different `UC…`). Keying the count off `channelId` therefore read 0 for
	 * everyone except the artists whose page failed to parse and fell back to the browse id.
	 *
	 * `browseId` is that same recorded id, kept so opening a row browses what we browsed. The
	 * official channel answers with an artist page too, but it is a different page: no "From your
	 * library" shelf, because the library is keyed to the music channel. Issue #193.
	 */
	type Familiar = ArtistPage & { plays: number; browseId: string };

	let artists = $state<Familiar[]>([]);
	let loading = $state(true);
	/** Channel ids by plays, most first; empty until the history has been read. */
	let ids = $state<string[]>([]);
	/** Subscribe state per channel, optimistic — seeded from each artist page as it lands. */
	let subs = $state<Record<string, boolean>>({});
	let subBusy = $state<string | null>(null);
	/** 0 = sized thumb, 1 = the original URL, 2 = give up and draw the icon. Same ladder as MediaCard. */
	let attempt = $state<Record<string, number>>({});

	// A channel YouTube Music doesn't treat as an artist has a wide banner as `thumbnail` and its
	// round avatar apart; the avatar is the one that crops to a poster.
	const art = (a: ArtistPage) => a.avatar ?? a.thumbnail;
	const src = (a: ArtistPage) => ((attempt[a.channelId] ?? 0) === 0 ? thumb(art(a), 400) : art(a));
	const hasArt = (a: ArtistPage) => !!art(a) && (attempt[a.channelId] ?? 0) < 2;
	const imgFailed = (a: ArtistPage) => {
		const n = attempt[a.channelId] ?? 0;
		attempt = { ...attempt, [a.channelId]: n === 0 && thumb(art(a), 400) !== art(a) ? 1 : 2 };
	};

	onMount(async () => {
		const top = (await api.topArtists(TOP).catch(() => []))
			.flatMap(({ id, plays }) => (id?.startsWith('UC') ? [{ id, plays }] : []))
			.slice(0, COUNT + SLACK);
		ids = top.map((a) => a.id);
		if (ids.length < MIN) {
			loading = false;
			return;
		}
		// A page with no name never parsed (no header, so no art and no subscriber count either); it
		// would sit in the list as "Unknown Artist" over a placeholder icon. Drop it and let the
		// slack ids take the slot.
		const pages = (
			await Promise.all(
				top.map(async ({ id, plays }) => {
					const page = await artistPage(id); // null: one dead channel doesn't cost the section
					return page?.name ? { ...page, browseId: id, plays } : null;
				})
			)
		)
			.filter((p): p is Familiar => !!p)
			.slice(0, COUNT);
		// Set once, in play-count order: filling the list artist by artist would reflow the feed
		// under the reader as each request lands.
		artists = pages;
		subs = Object.fromEntries(pages.map((p) => [p.channelId, p.subscribed]));
		loading = false;
	});

	const asItem = (a: Familiar): BrowseItem => ({
		kind: 'artist',
		id: a.browseId,
		title: a.name ?? t('common.artist_singular'),
		subtitle: a.subscribers,
		thumbnail: art(a)
	});

	const open = (a: Familiar) => goto(`/artist/${encodeURIComponent(a.browseId)}`);
	const rank = (i: number) => String(i + 1).padStart(2, '0');

	async function toggleSub(a: Familiar) {
		if (subBusy) return;
		const next = !subs[a.channelId];
		subBusy = a.channelId;
		subs = { ...subs, [a.channelId]: next };
		try {
			await api.subscribe(a.channelId, next);
			putCached(`artist:${a.browseId}`, { ...a, subscribed: next }); // keep the cache truthful
			toast.success(next ? t('artist.subscribed') : t('artist.subscribe'));
		} catch (e) {
			subs = { ...subs, [a.channelId]: !next };
			toast.error(String(e));
		} finally {
			subBusy = null;
		}
	}
</script>

{#snippet subButton(a: Familiar)}
	<button
		class="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full bg-black/40 backdrop-blur-sm transition-colors hover:bg-black/60 {subs[
			a.channelId
		]
			? 'text-primary'
			: 'text-white/70 hover:text-white'}"
		class:animate-pulse={subBusy === a.channelId}
		aria-label={subs[a.channelId]
			? t('artist.unsubscribe_from', { name: a.name ?? '' })
			: t('artist.subscribe_to', { name: a.name ?? '' })}
		onclick={(e) => {
			e.stopPropagation();
			toggleSub(a);
		}}
	>
		<HugeiconsIcon icon={UserLove02Icon} class="h-5 w-5" />
	</button>
{/snippet}

{#snippet avatar(a: ArtistPage, iconClass: string)}
	{#if hasArt(a)}
		<img
			src={src(a)}
			alt=""
			class="h-full w-full object-cover object-[center_22%]"
			loading="lazy"
			draggable="false"
			onerror={() => imgFailed(a)}
		/>
	{:else}
		<div class="flex h-full w-full items-center justify-center text-muted-foreground/40">
			<HugeiconsIcon icon={UserIcon} class={iconClass} />
		</div>
	{/if}
{/snippet}

{#if loading ? ids.length >= MIN : artists.length >= MIN}
	<section>
		<SectionHeading title={t('home.familiar_artists')} icon={UserStar01Icon} />
		<div class="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-6">
			{#if loading}
				{#each Array(Math.min(ids.length, COUNT)) as _, i (i)}
					<Skeleton class="aspect-[4/5] w-full rounded-2xl" />
				{/each}
			{:else}
				{#each artists as a, i (a.channelId)}
					<div class="group relative aspect-[4/5] w-full" data-ctx>
						<div
							class="relative h-full w-full cursor-pointer overflow-hidden rounded-2xl bg-muted"
							role="button"
							tabindex="0"
							onclick={() => open(a)}
							onkeydown={(e) => {
								if (e.target !== e.currentTarget) return;
								if (e.key === 'Enter' || e.key === ' ') {
									e.preventDefault();
									open(a);
								}
							}}
							title={a.name ?? t('common.artist_singular')}
						>
							<div
								class="h-full w-full"
							>
								{@render avatar(a, 'h-10 w-10')}
							</div>
							<div
								class="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/85 via-black/20 to-transparent"
							></div>
							<div class="pointer-events-none absolute inset-x-0 bottom-0 p-3.5">
								<div class="font-heading text-xs font-semibold tracking-widest text-primary">
									{rank(i)}
								</div>
								<div
									class="mt-0.5 line-clamp-2 font-heading text-base font-bold leading-tight text-white"
								>
									{a.name ?? t('common.unknown_artist')}
								</div>
								<div class="mt-0.5 truncate text-xs text-white/65">
									{t('library.play_count', { count: a.plays })}
								</div>
							</div>
						</div>
						<!-- On hover or focus only: six posters each carrying two buttons at rest is a
						     dozen dark discs over the faces. -->
						<div
							class="absolute right-2 top-2 flex items-center gap-1 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100"
						>
							{@render subButton(a)}
							<PlaylistMenu
								item={asItem(a)}
								showPin={false}
								vertical
								iconClass="h-5 w-5"
								triggerClass="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full bg-black/40 text-white/70 backdrop-blur-sm transition hover:bg-black/60 hover:text-white"
							/>
						</div>
					</div>
				{/each}
			{/if}
		</div>
	</section>
{/if}
