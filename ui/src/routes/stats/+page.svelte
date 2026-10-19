<script lang="ts" module>
	import type { Range } from '$lib/stats';
	// Kept for the session: whoever checks the month comes back to the month.
	let lastRange: Range = 'month';
</script>

<script lang="ts">
	// Listening stats (#396): how much, what and when, from the plays this device has counted. One
	// local query per visit, range switch or new song (`listening_stats` in Rust); the only network
	// is the top artists' photos, through the same cache as the artist pages.
	import { untrack } from 'svelte';
	import { goto } from '$app/navigation';
	import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/svelte';
	import {
		ArrowDownRight01Icon,
		ArrowUpRight01Icon,
		Chart02Icon,
		Clock01Icon,
		Delete02Icon,
		FireIcon,
		MusicNote01Icon,
		PlayCircleIcon,
		UserIcon,
		UserMultipleIcon,
		UserStar01Icon,
		Vynil03Icon
	} from '@hugeicons/core-free-icons';
	import * as AlertDialog from '$lib/components/ui/alert-dialog';
	import * as Tabs from '$lib/components/ui/tabs';
	import { Button } from '$lib/components/ui/button';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import SectionHeading from '$lib/components/SectionHeading.svelte';
	import TrackRow from '$lib/components/TrackRow.svelte';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import ErrorState from '$lib/components/ErrorState.svelte';
	import ListeningChart from '$lib/components/ListeningChart.svelte';
	import ListeningHours from '$lib/components/ListeningHours.svelte';
	import ListeningDayparts from '$lib/components/ListeningDayparts.svelte';
	import * as api from '$lib/api';
	import type { ListeningStats, TopArtist } from '$lib/api';
	import { artistPage, hrefFor } from '$lib/browse';
	import { thumb } from '$lib/thumb';
	import { openAddToPlaylist, openPlayer, playback, toast } from '$lib/player.svelte';
	import {
		RANGES,
		bars as toBars,
		bigDuration,
		busiest,
		change,
		listened,
		rangeStart,
		tagOf,
		type Bar
	} from '$lib/stats';
	import { currentLocale, t } from '$lib/i18n.svelte';

	// A touch heavier than HugeIcons' 1.5 on every icon on this page: the thin default read faint
	// next to the big numbers and the charts.
	const STROKE = 2;

	let range = $state<Range>(lastRange);
	let stats = $state.raw<ListeningStats | null>(null);
	// The range `stats` answers. The tabs move on the click, the numbers when the answer lands, and
	// everything drawn from `stats` reads this one so the two never mix.
	let shown = $state<Range>(lastRange);
	let error = $state<string | null>(null);

	let seq = 0;
	async function load(r: Range) {
		const mine = ++seq;
		try {
			const s = await api.listeningStats(rangeStart(r));
			if (mine !== seq) return; // a newer range or song asked since
			stats = s;
			shown = r;
			error = null;
		} catch (e) {
			if (mine === seq) error = String(e);
		}
	}

	// A song's time heard is settled when the next one starts, so each new song refreshes the page
	// as well as each range switch. The old numbers stay up meanwhile: a local query is quicker than
	// a skeleton would be.
	$effect(() => {
		playback.now?.videoId;
		const r = range;
		lastRange = r;
		untrack(() => load(r));
	});

	const tag = $derived(tagOf(currentLocale.id));
	const num = $derived(new Intl.NumberFormat(tag));
	const percent = $derived(new Intl.NumberFormat(tag, { style: 'percent' }));
	const signed = $derived(
		new Intl.NumberFormat(tag, { style: 'percent', signDisplay: 'exceptZero' })
	);
	const fmt = (opts: Intl.DateTimeFormatOptions) => new Intl.DateTimeFormat(tag, opts);

	const chart = $derived(stats ? toBars(stats.days, shown) : []);
	const monthly = $derived(chart[0]?.key.length === 7);
	const average = $derived(chart.length ? (stats?.secs ?? 0) / chart.length : 0);
	const peak = $derived(
		chart.reduce<Bar | undefined>((top, b) => (b.secs > (top?.secs ?? 0) ? b : top), undefined)
	);
	const delta = $derived(stats ? change(stats.secs, stats.prevSecs) : null);
	const when = $derived(stats ? busiest(stats.hours) : null);
	const total = $derived(bigDuration(stats?.secs ?? 0, tag));
	// The rows carry our play count where an album page's carry YouTube's.
	const songs = $derived(
		stats?.topSongs.map((s) => ({ ...s.song, play_count: num.format(s.plays) })) ?? []
	);
	const nowId = $derived(playback.now?.videoId);
	const cover = $derived(stats?.topSongs[0]?.song.thumbnail ?? stats?.topArtists[0]?.thumbnail);

	const summary = $derived(
		shown === 'all'
			? t('stats.listened.all', {
					date: fmt({ dateStyle: 'long' }).format(new Date((stats?.firstAt ?? 0) * 1000))
				})
			: t(`stats.listened.${shown}`)
	);

	// A decorative backdrop degrades to nothing rather than a broken-image glyph (HomeHero).
	let artFailed = $state(false);
	$effect(() => {
		cover; // re-arm when the artwork changes
		artFailed = false;
	});

	// Artist photos only live on their pages. Eight at most, asked once each per visit.
	let photos = $state<Record<string, string>>({});
	let failed = $state<Record<string, boolean>>({});
	const asked = new Set<string>();
	$effect(() => {
		for (const { id } of stats?.topArtists ?? []) {
			if (!id?.startsWith('UC') || asked.has(id)) continue;
			asked.add(id);
			artistPage(id).then((p) => {
				const src = p?.avatar ?? p?.thumbnail;
				if (src) photos = { ...photos, [id]: src };
			});
		}
	});

	function play(i: number) {
		openPlayer();
		api.playPlaylist(songs, i, undefined, t('stats.top_songs'));
	}

	const openArtist = (a: TopArtist) =>
		a.id && goto(hrefFor({ kind: 'artist', id: a.id, title: a.name }));

	let confirming = $state(false);
	let clearing = $state(false);
	async function clearHistory() {
		clearing = true;
		try {
			await api.clearListeningHistory();
			confirming = false;
			toast.success(t('stats.cleared'));
			await load(range);
		} catch (e) {
			toast.error(String(e));
		} finally {
			clearing = false;
		}
	}
</script>

{#snippet tile(icon: IconSvgElement, label: string, value: string, hint?: string)}
	<div class="rounded-2xl bg-foreground/[0.04] p-4">
		<div class="flex items-center gap-2 text-xs font-medium text-muted-foreground">
			<HugeiconsIcon strokeWidth={STROKE} {icon} class="h-4 w-4 shrink-0 text-primary/70" />
			<span class="truncate">{label}</span>
		</div>
		<div class="mt-1.5 flex min-w-0 items-baseline gap-2">
			<span class="font-heading text-2xl font-bold tabular-nums">{value}</span>
			{#if hint}<span class="truncate text-xs text-muted-foreground">{hint}</span>{/if}
		</div>
	</div>
{/snippet}

{#snippet fact(label: string, value: string, detail?: string)}
	<div class="flex flex-col gap-0.5">
		<span class="text-xs text-muted-foreground">{label}</span>
		<span class="text-sm font-semibold">
			{value}{#if detail}<span class="font-normal text-muted-foreground"> · {detail}</span>{/if}
		</span>
	</div>
{/snippet}

{#snippet avatar(a: TopArtist)}
	<!-- Their photo, else the cover of their most-heard song, else a silhouette. -->
	{@const src = [a.id ? photos[a.id] : undefined, a.thumbnail].find((s) => s && !failed[s])}
	{#if src}
		<img
			src={thumb(src, 400)}
			alt=""
			class="h-full w-full object-cover object-[center_22%]"
			loading="lazy"
			draggable="false"
			onerror={() => (failed = { ...failed, [src]: true })}
		/>
	{:else}
		<div class="flex h-full w-full items-center justify-center text-muted-foreground/50">
			<HugeiconsIcon strokeWidth={STROKE} icon={UserIcon} class="h-1/3 w-1/3" />
		</div>
	{/if}
{/snippet}

<!-- isolate: the header's artwork sits at -z-10 and must stay inside this page (see HomeHero). -->
<div class="relative isolate">
	<header class="relative">
		<div
			class="art-wash pointer-events-none absolute inset-x-0 top-0 -z-10 h-[28rem] overflow-hidden"
			aria-hidden="true"
		>
			{#if cover && !artFailed}
				<!-- 96px: blur-2xl throws away every detail bigger than a few pixels anyway. -->
				<img
					src={thumb(cover, 96)}
					alt=""
					class="absolute inset-0 h-full w-full scale-110 object-cover opacity-60 blur-2xl"
					onerror={() => (artFailed = true)}
				/>
			{:else}
				<div
					class="absolute inset-0 opacity-[0.2]"
					style="background:radial-gradient(90% 75% at 10% 0%, var(--primary) 0%, transparent 70%)"
				></div>
			{/if}
			<div
				class="absolute inset-0 bg-gradient-to-b from-background/25 via-background/70 to-background"
			></div>
			<div
				class="absolute inset-0 bg-gradient-to-r from-background/60 via-background/15 to-transparent"
			></div>
		</div>

		<div class="flex flex-wrap items-center justify-between gap-4 px-6 pt-10">
			<h1 class="font-heading text-2xl font-bold tracking-tight">{t('stats.title')}</h1>
			{#if stats?.firstAt}
				<Tabs.Root value={range} onValueChange={(v) => (range = v as Range)}>
					<Tabs.List>
						{#each RANGES as r (r)}
							<Tabs.Trigger value={r} class="px-3">{t(`stats.ranges.${r}`)}</Tabs.Trigger>
						{/each}
					</Tabs.List>
				</Tabs.Root>
			{/if}
		</div>

		{#if stats?.firstAt}
			<div class="content-in px-6 pt-6">
				<p class="font-heading text-6xl font-bold leading-none tracking-tight tabular-nums sm:text-7xl">
					{#each total as part, i (i)}
						<span>{part.n}</span><span
							class="ml-1 mr-4 text-3xl font-semibold text-muted-foreground last:mr-0 sm:text-4xl"
							>{part.unit}</span
						>
					{/each}
				</p>
				<p class="mt-3 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground">
					<span>{summary}</span>
					{#if delta !== null && shown !== 'all'}
						<!-- Less listening isn't bad news, so a drop is quiet rather than red. -->
						<span
							class="inline-flex items-center gap-0.5 rounded-full px-2 py-0.5 text-xs font-semibold tabular-nums {delta >=
							0
								? 'bg-primary/10 text-primary'
								: 'bg-foreground/[0.06] text-foreground/70'}"
						>
							<!-- altIcon/showAlt, not a ternary: `icon` is read once at mount. -->
							<HugeiconsIcon
								icon={ArrowUpRight01Icon}
								strokeWidth={STROKE}
								altIcon={ArrowDownRight01Icon}
								showAlt={delta < 0}
								class="h-3.5 w-3.5"
							/>
							{signed.format(delta / 100)}
						</span>
						<span>{t(`stats.compared.${shown}`)}</span>
					{/if}
				</p>
			</div>
			<div class="content-in grid grid-cols-2 gap-3 px-6 pt-8 lg:grid-cols-4">
				{@render tile(PlayCircleIcon, t('stats.plays'), num.format(stats.plays))}
				{@render tile(
					MusicNote01Icon,
					t('common.songs'),
					num.format(stats.songs),
					shown !== 'all' && stats.newSongs
						? t('stats.new_songs', { count: num.format(stats.newSongs) })
						: undefined
				)}
				{@render tile(UserMultipleIcon, t('common.artists'), num.format(stats.artists))}
				{@render tile(
					FireIcon,
					t('stats.streak'),
					num.format(stats.streak),
					stats.bestStreak > stats.streak
						? t('stats.best_streak', { count: num.format(stats.bestStreak) })
						: undefined
				)}
			</div>
		{:else if !stats && !error}
			<div class="px-6 pt-6" aria-hidden="true">
				<Skeleton class="h-16 w-72 rounded-lg" />
				<Skeleton class="mt-4 h-4 w-56 rounded" />
				<div class="mt-8 grid grid-cols-2 gap-3 lg:grid-cols-4">
					{#each Array(4) as _, i (i)}<Skeleton class="h-[5.25rem] rounded-2xl" />{/each}
				</div>
				<Skeleton class="mt-12 h-56 w-full rounded-2xl" />
			</div>
		{/if}
	</header>

	{#if error && !stats}
		<div class="px-6 pt-6"><ErrorState message={error} onRetry={() => load(range)} /></div>
	{:else if stats && !stats.firstAt}
		<!-- Nothing counted yet: say what this page becomes, rather than a wall of zeros. -->
		<div class="content-in flex flex-col items-center gap-3 px-6 py-24 text-center">
			<div class="flex h-16 w-16 items-center justify-center rounded-2xl bg-primary/10 text-primary">
				<HugeiconsIcon strokeWidth={STROKE} icon={Chart02Icon} class="h-8 w-8" />
			</div>
			<h2 class="font-heading text-xl font-bold tracking-tight">{t('stats.empty_title')}</h2>
			<p class="max-w-sm text-sm text-muted-foreground">{t('stats.empty')}</p>
			<p class="max-w-sm text-xs text-muted-foreground/70">{t('stats.on_device')}</p>
		</div>
	{:else if stats}
		<div class="content-in flex flex-col gap-12 px-6 pb-10 pt-12">
			<section>
				<SectionHeading strokeWidth={STROKE} title={t('stats.activity')} icon={Chart02Icon} />
				<div class="mb-4 flex flex-wrap gap-x-10 gap-y-3">
					{@render fact(
						t(monthly ? 'stats.monthly_average' : 'stats.daily_average'),
						listened(average, tag)
					)}
					{#if peak}
						{@render fact(
							t(monthly ? 'stats.busiest_month' : 'stats.busiest_day'),
							fmt(
								monthly
									? { month: 'long', year: 'numeric' }
									: { weekday: 'short', month: 'short', day: 'numeric' }
							).format(peak.date),
							listened(peak.secs, tag)
						)}
					{/if}
				</div>
				<ListeningChart bars={chart} {monthly} peak={peak?.key} />
				{#if !stats.plays}
					<p class="mt-4 text-sm text-muted-foreground">{t('stats.nothing_in_range')}</p>
				{/if}
			</section>

			{#if stats.plays}
				<div class="grid gap-12 xl:grid-cols-2">
					{#if stats.topArtists.length}
						{@const top = stats.topArtists[0]}
						<section class="min-w-0">
							<SectionHeading strokeWidth={STROKE} title={t('stats.top_artists')} icon={UserStar01Icon} />
							<button
								class="flex w-full items-center gap-5 rounded-2xl bg-foreground/[0.04] p-4 text-left transition-colors enabled:cursor-pointer enabled:hover:bg-foreground/[0.07]"
								disabled={!top.id}
								onclick={() => openArtist(top)}
							>
								<div class="h-28 w-28 shrink-0 overflow-hidden rounded-full bg-muted shadow-lg">
									{@render avatar(top)}
								</div>
								<div class="min-w-0">
									<div class="text-xs font-semibold tabular-nums text-primary">01</div>
									<div class="truncate font-heading text-2xl font-bold tracking-tight">{top.name}</div>
									<div class="mt-1 text-sm text-muted-foreground">
										{listened(top.secs, tag)} · {t('library.play_count', {
											count: num.format(top.plays)
										})}
									</div>
									{#if stats.secs}
										<div class="mt-0.5 text-sm text-muted-foreground">
											{t('stats.share', { percent: percent.format(top.secs / stats.secs) })}
										</div>
									{/if}
								</div>
							</button>
							<!-- No transition on the rows: a hover fade on repeated rows is what costs frames
							     (docs/UI-PERFORMANCE.md). -->
							<ol class="mt-2">
								{#each stats.topArtists.slice(1) as a, i (a.name)}
									<li>
										<button
											class="flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left enabled:cursor-pointer enabled:hover:bg-foreground/5"
											disabled={!a.id}
											onclick={() => openArtist(a)}
										>
											<span class="w-5 shrink-0 text-center text-xs tabular-nums text-muted-foreground">
												{String(i + 2).padStart(2, '0')}
											</span>
											<div class="h-10 w-10 shrink-0 overflow-hidden rounded-full bg-muted">
												{@render avatar(a)}
											</div>
											<div class="min-w-0 flex-1">
												<div class="truncate text-sm font-medium">{a.name}</div>
												<!-- Measured against #1, so the bars read as how close each one came. -->
												<div class="mt-1.5 h-1 overflow-hidden rounded-full bg-foreground/[0.06]">
													<div
														class="h-full rounded-full bg-primary/60"
														style="width:{top.secs ? (a.secs / top.secs) * 100 : 0}%"
													></div>
												</div>
											</div>
											<!-- A fixed width, so every bar's track ends at the same place whatever
											     the time beside it says. -->
											<span class="w-20 shrink-0 text-right text-xs tabular-nums text-muted-foreground">
												{listened(a.secs, tag)}
											</span>
										</button>
									</li>
								{/each}
							</ol>
						</section>
					{/if}
					<section class="min-w-0">
						<SectionHeading strokeWidth={STROKE} title={t('stats.top_songs')} icon={MusicNote01Icon} />
						{#each songs as song, i (song.video_id)}
							<TrackRow
								{song}
								index={i}
								showPlayCount
								active={song.video_id === nowId}
								onplay={() => play(i)}
								onAdd={() => openAddToPlaylist(song)}
							/>
						{/each}
					</section>
				</div>

				{#if stats.topAlbums.length}
					<section>
						<SectionHeading strokeWidth={STROKE} title={t('stats.top_albums')} icon={Vynil03Icon} />
						<div class="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-6">
							{#each stats.topAlbums as al (al.id)}
								<MediaCard
									item={{
										kind: 'album',
										id: al.id,
										title: al.title,
										subtitle: [al.artist, listened(al.secs, tag)].filter(Boolean).join(' · '),
										thumbnail: al.thumbnail ?? undefined
									}}
								/>
							{/each}
						</div>
					</section>
				{/if}

				<section>
					<SectionHeading strokeWidth={STROKE} title={t('stats.when')} icon={Clock01Icon} />
					<div class="grid gap-8 lg:grid-cols-[16rem_minmax(0,1fr)]">
						<ListeningDayparts hours={stats.hours} />
						<div class="min-w-0">
							{#if when}
								<div class="mb-4 flex flex-wrap gap-x-10 gap-y-3">
									{@render fact(
										t('stats.busiest_weekday'),
										fmt({ weekday: 'long' }).format(new Date(2026, 0, 4 + when.day))
									)}
									{@render fact(
										t('stats.busiest_hour'),
										fmt({ hour: 'numeric' }).format(new Date(2026, 0, 4, when.hour))
									)}
								</div>
							{/if}
							<ListeningHours hours={stats.hours} />
						</div>
					</div>
				</section>
			{/if}

			<footer class="flex flex-wrap items-center justify-between gap-3 pt-4">
				<p class="text-xs text-muted-foreground">{t('stats.on_device')}</p>
				<Button
					variant="ghost"
					size="sm"
					class="gap-1.5 text-muted-foreground hover:text-destructive"
					onclick={() => (confirming = true)}
				>
					<HugeiconsIcon strokeWidth={STROKE} icon={Delete02Icon} class="h-4 w-4" />
					{t('stats.clear')}
				</Button>
			</footer>
		</div>
	{/if}
</div>

<AlertDialog.Root bind:open={() => confirming, (v) => !clearing && (confirming = v)}>
	<AlertDialog.Content>
		<AlertDialog.Header>
			<AlertDialog.Media class="bg-destructive/10 text-destructive">
				<HugeiconsIcon strokeWidth={STROKE} icon={Delete02Icon} />
			</AlertDialog.Media>
			<AlertDialog.Title>{t('stats.clear_title')}</AlertDialog.Title>
			<AlertDialog.Description>{t('stats.clear_desc')}</AlertDialog.Description>
		</AlertDialog.Header>
		<AlertDialog.Footer>
			<AlertDialog.Cancel disabled={clearing}>{t('common.cancel')}</AlertDialog.Cancel>
			<AlertDialog.Action variant="destructive" onclick={clearHistory} disabled={clearing}>
				{clearing ? t('common.loading') : t('stats.clear_confirm')}
			</AlertDialog.Action>
		</AlertDialog.Footer>
	</AlertDialog.Content>
</AlertDialog.Root>
