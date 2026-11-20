<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { scale } from 'svelte/transition';
	import { flip } from 'svelte/animate';
	import { cubicOut } from 'svelte/easing';
	import { MediaQuery } from 'svelte/reactivity';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Home01Icon,
		Search01Icon,
		LibraryIcon,
		Settings01Icon,
		Sun01Icon,
		Moon02Icon,
		Add01Icon,
		PinIcon,
		ListRestartIcon,
		ComputerIcon,
		SquareArrowLeft01Icon,
		SquareArrowRight01Icon,
		SpotifyIcon,
		Chart02Icon,
		VolumeHighIcon,
		PlayIcon,
		PauseIcon,
		DiscordIcon
	} from '@hugeicons/core-free-icons';
	import LogoMark from './LogoMark.svelte';
	import LimusicLoader from './LimusicLoader.svelte';
	import { playItem } from '$lib/browse';
	import { toggleMode } from 'mode-watcher';
	import { Button } from '$lib/components/ui/button';
	import { DISCORD_URL, ON_REPEAT_ID, isLocalPlaylist, openExternal, togglePause, type BrowseItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import PlaylistMenu from './PlaylistMenu.svelte';
	import {
		library,
		personal,
		playback,
		ui,
		openNewPlaylist,
		toggleSidebar,
		toast
	} from '$lib/player.svelte';
	import { mergeSaved, orderLibrary } from '$lib/personal';
	import { t } from '$lib/i18n.svelte';
	import { imp } from '$lib/import.svelte';

	const nav = $derived([
		{ href: '/', label: t('nav.home'), icon: Home01Icon },
		{ href: '/search', label: t('nav.search'), icon: Search01Icon },
		{ href: '/library', label: t('nav.library'), icon: LibraryIcon },
		{ href: '/stats', label: t('nav.stats'), icon: Chart02Icon }
	]);
	const isActive = (href: string) =>
		href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(href);

	// Pinned first (in pin order), then everything else by last played. Derived here rather than in
	// the shared `library` store so the Library page keeps YouTube's own ordering. Playlists saved
	// on this machine sit in the same list: signed out they are the only ones there.
	const playlists = $derived(
		orderLibrary(mergeSaved(personal, library.items, 'playlist'), personal)
	);
	// The playlist the queue is playing from (#646), like Spotify's library: its title in the accent
	// colour, and a speaker while it plays. Not once autoplay has moved on to a radio past its end.
	const playingFrom = $derived.by(() => {
		const q = playback.queue;
		const current = q.items[q.currentIndex];
		return current && !current.autoplay ? q.sourceId : null;
	});
	// The cover plays the playlist, like Spotify's library, or pauses and resumes it once it is the
	// one playing. `starting`: the playlist being fetched before it can play.
	let starting = $state<string | null>(null);
	async function playRow(item: BrowseItem) {
		if (starting) return;
		if (playingFrom === item.id) return togglePause();
		starting = item.id;
		try {
			await playItem(item);
		} finally {
			starting = null;
		}
	}
	// A playlist that starts playing moves to the top (rows go by last played): slid there, not
	// cut. The same 200 ms as the queue's and Shortcuts' reorders.
	const reducedMotion = new MediaQuery('(prefers-reduced-motion: reduce)');
	const slide = $derived(reducedMotion.current ? 0 : 200);
	// How many of the leading rows are pinned — a rule under the last one explains the split.
	const pinnedCount = $derived(playlists.filter((p) => personal.pins.includes(p.id)).length);

	// YTM's library subtitle is "Owner • 20 tracks" and the rail is too narrow for both, so keep the
	// count and drop the rest. Subtitles without a number (albums: "Album • Artist") stay whole.
	const rowSubtitle = (s?: string) =>
		s
			?.split('•')
			.map((p) => p.trim())
			.filter((p) => /\d/.test(p))
			.at(-1) ?? s;

	const playlistHref = (item: BrowseItem) =>
		item.kind === 'album'
			? `/album/${encodeURIComponent(item.id)}`
			: item.kind === 'artist'
				? `/artist/${encodeURIComponent(item.id)}`
				: `/playlist/${encodeURIComponent(item.id)}`;

	// Account lives in the titlebar now — see AccountMenu.svelte.

	// Manual collapse is a large-screen preference: below lg the rail is already collapsed by the
	// breakpoint, so the button is hidden there and `wide()` has nothing to drop. Every expanded
	// style is an `lg:` class, so collapsing is just not emitting them. The flag lives in `ui`
	// because the overlays that offset by the sidebar's width read it too.
	const collapsed = $derived(ui.sidebarCollapsed);

	// A Spotify import working in the background (#375): what it is doing, and how far along.
	const importing = $derived.by(() => {
		const s = imp.snapshot;
		if (!s) return null;
		const of = (done: number, total: number) => (total ? done / total : 0);
		if (s.update) return { label: t('import.pill_updating'), progress: null };
		switch (s.phase) {
			case 'matching':
				return {
					label: t('import.pill_matching', { percent: Math.round(of(s.done, s.total) * 100) }),
					progress: of(s.done, s.total)
				};
			case 'review':
				return { label: t('import.pill_review'), progress: null };
			case 'creating':
				return { label: t('import.pill_creating'), progress: of(s.step[0], s.step[1]) };
			case 'done':
				return { label: t('import.pill_done'), progress: null };
			default:
				return { label: t('import.pill_failed'), progress: null };
		}
	});
	// An update has no dialog to go back to, so its pill goes to the playlist instead.
	function openImporting() {
		const id = imp.snapshot?.update;
		if (id) goto(`/playlist/${encodeURIComponent(id)}`);
		else imp.open = true;
	}
	const wide = (cls: string) => (collapsed ? '' : cls);
</script>

<aside
	data-nav-rail
	class="flex h-full w-16 shrink-0 flex-col border-r bg-sidebar p-3 text-sidebar-foreground {wide(
		'lg:w-60'
	)}"
>
	<div class="flex items-center justify-center px-2 py-2 {wide('lg:justify-between')}">
		<span class="hidden font-heading text-lg font-bold tracking-tight {wide('lg:block')}">Limusic</span>
		<!-- Column when collapsed: the two buttons don't fit side by side in the 64px rail. -->
		<div class="flex items-center gap-1 {collapsed ? 'flex-col' : ''}">
			<Button
				variant="ghost"
				size="icon-sm"
				class="hidden hover:text-primary lg:inline-flex"
				onclick={toggleSidebar}
				aria-label={collapsed ? t('a11y.expand_sidebar') : t('a11y.collapse_sidebar')}
			>
				<!-- altIcon/showAlt, not a ternary: `icon` is read once at mount. -->
				<HugeiconsIcon
					icon={SquareArrowLeft01Icon}
					altIcon={SquareArrowRight01Icon}
					showAlt={collapsed}
					strokeWidth={2}
					class="h-4 w-4"
				/>
			</Button>
			<Button
				variant="ghost"
				size="icon-sm"
				class="hover:text-primary"
				onclick={toggleMode}
				aria-label={t('a11y.toggle_theme')}
			>
				<HugeiconsIcon icon={Sun01Icon} strokeWidth={2} class="h-4 w-4 dark:hidden" />
				<HugeiconsIcon icon={Moon02Icon} strokeWidth={2} class="hidden h-4 w-4 dark:block" />
			</Button>
			<Button
				variant="ghost"
				size="icon-sm"
				class="hover:text-primary"
				onclick={() => openExternal(DISCORD_URL).catch((e) => toast.error(t('toasts.browser_failed', { error: String(e) })))}
				title={t('nav.discord')}
				aria-label={t('nav.discord')}
			>
				<HugeiconsIcon icon={DiscordIcon} strokeWidth={2} class="h-4 w-4" />
			</Button>
		</div>
	</div>

	<nav class="mt-2 flex flex-col gap-1">
		{#each nav as n (n.href)}
			<a
				href={n.href}
				onclick={n.href === '/search' ? async (e) => {
					if (!isActive('/search') || e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;
					e.preventDefault();
					// Run navigation hooks to dismiss the player before focusing the search field.
					await goto(page.url, { keepFocus: true, replaceState: true });
					const input = document.querySelector<HTMLInputElement>('#search-form input');
					input?.focus();
					input?.select();
				} : undefined}
				title={n.label}
				data-nav-item
				data-nav-active={isActive(n.href) ? '' : undefined}
				class="group relative flex items-center justify-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors {wide(
					'lg:justify-start'
				)} {isActive(n.href)
					? 'bg-primary/10 text-primary'
					: 'text-sidebar-foreground/70 hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'}"
			>
				{#if isActive(n.href)}
					<span
						data-nav-indicator
						transition:scale={{ duration: 200, start: 0.4 }}
						class="absolute left-0 top-1/2 h-5 w-1 -translate-y-1/2 rounded-r-full bg-primary"
					></span>
				{/if}
				<HugeiconsIcon
					icon={n.icon}
					class="h-5 w-5 shrink-0"
				/>
				<span class="hidden {wide('lg:inline')}">{n.label}</span>
			</a>
		{/each}
		<button
			onclick={() => (ui.settingsOpen = true)}
			title={t('nav.settings')}
			class="group flex items-center justify-center gap-3 rounded-lg px-3 py-2 text-sm font-medium text-sidebar-foreground/70 transition-colors hover:bg-sidebar-accent/50 hover:text-sidebar-foreground {wide(
				'lg:justify-start'
			)}"
		>
			<HugeiconsIcon
				icon={Settings01Icon}
				class="h-5 w-5 shrink-0"
			/>
			<span class="hidden {wide('lg:inline')}">{t('nav.settings')}</span>
		</button>
	</nav>

	{#if importing}
		<button
			onclick={openImporting}
			title={importing.label}
			class="mt-2 flex flex-col gap-1.5 rounded-lg bg-primary/10 px-3 py-2 text-left text-xs font-medium text-primary transition-colors hover:bg-primary/15"
		>
			<span class="flex items-center justify-center gap-3 {wide('lg:justify-start')}">
				<HugeiconsIcon icon={SpotifyIcon} class="h-5 w-5 shrink-0" />
				<span class="hidden truncate {wide('lg:inline')}">{importing.label}</span>
			</span>
			{#if importing.progress !== null}
				<span class="block h-1 w-full overflow-hidden rounded-full bg-primary/15">
					<span
						class="block h-full rounded-full bg-primary transition-[width] duration-300"
						style="width: {Math.round(importing.progress * 100)}%"
					></span>
				</span>
			{/if}
		</button>
	{/if}

	<!-- Playlists. Hidden on the icon rail (needs labels; matches YTM's collapsed rail). flex-1 lets
	     the list fill the space and scroll. Always there, signed out included: a playlist can be
	     made on this machine without an account (#251). -->
	<div data-nav-playlists class="mt-3 hidden min-h-0 flex-1 flex-col border-t pt-3 {wide('lg:flex')}">
		<Button variant="outline" size="sm" class="mb-2 w-full gap-2" onclick={() => openNewPlaylist()}>
			<HugeiconsIcon icon={Add01Icon} class="h-4 w-4" /> {t('nav.new_playlist')}
		</Button>
		<!-- `relative z-0` gives the list its own stacking context, which keeps its scrollbar on
		     screen. Without it, the ⋯ button fading in on a hovered row blanks the thumb on WebKitGTK
		     for as long as the fade runs (#420, same bug as #343). -->
		<div class="min-h-0 flex-1 overflow-y-auto relative z-0">
			{#each playlists as pl, i (pl.id)}
				{@const current = playingFrom === pl.id}
				<!-- One element per row: `animate:flip` takes the only child of a keyed each. -->
				<div animate:flip={{ duration: slide, easing: cubicOut }}>
					<!-- The ⋯ and the cover's play button are siblings of the link, not children: a
					     <button> inside an <a> is invalid HTML. pr-9 keeps the title clear of the ⋯ that
					     overlays the row on hover. The link takes its hover from the row so the highlight
					     stays while the pointer is on either button. -->
					<div class="group/row relative" data-ctx>
						<a
							href={playlistHref(pl)}
							title={pl.title}
							data-nav-item
							class="flex items-center gap-2.5 rounded-lg py-1.5 pl-2 pr-9 transition-colors group-hover/row:bg-sidebar-accent/50"
						>
							<div
								class="relative h-10 w-10 shrink-0 overflow-hidden bg-muted {pl.kind === 'artist'
									? 'rounded-full'
									: 'rounded-md'}"
							>
								{#if pl.thumbnail && pl.id !== ON_REPEAT_ID}
									<img
										src={thumb(pl.thumbnail, 96)}
										alt=""
										class="h-full w-full object-cover"
										loading="lazy"
									/>
								{:else}
									<!-- On Repeat has no artwork by nature: icon tile, same as its card. -->
									<div
										class="flex h-full w-full items-center justify-center {pl.id === ON_REPEAT_ID
											? 'bg-primary/10 text-primary'
											: 'text-muted-foreground/20'}"
									>
										{#if pl.id === ON_REPEAT_ID}
											<HugeiconsIcon icon={ListRestartIcon} class="h-5 w-5" />
										{:else}
											<LogoMark class="size-3/4" />
										{/if}
									</div>
								{/if}
							</div>
							{#if personal.pins.includes(pl.id)}
								<span
									class="absolute left-9 top-0.5 z-10 flex h-4 w-4 items-center justify-center rounded-full bg-primary text-primary-foreground shadow"
								>
									<HugeiconsIcon icon={PinIcon} class="h-2.5 w-2.5" />
								</span>
							{/if}
							<div class="min-w-0 flex-1">
								<div class="truncate text-[13px] font-medium {current ? 'text-primary' : ''}">
									{pl.title}
								</div>
								{#if current}
									<span class="sr-only">{t('player.now_playing')}</span>
								{/if}
								{#if pl.subtitle}
									<!-- The rail keeps only the count, so a playlist on this machine says
									     where it lives with an icon instead of the words. -->
									<div class="flex items-center gap-1 text-xs text-muted-foreground">
										{#if isLocalPlaylist(pl.id)}
											<HugeiconsIcon icon={ComputerIcon} class="h-3 w-3 shrink-0" />
										{/if}
										<span class="truncate">{rowSubtitle(pl.subtitle)}</span>
									</div>
								{/if}
							</div>
						</a>
						{#if pl.kind !== 'artist'}
							<!-- Over the cover, the same dark wash and white mark as a Shortcuts tile (a wash
							     over artwork, not a theme surface). Shown with visibility, no fade: rows in a
							     scrolling list carry no transition (docs/UI-PERFORMANCE.md). -->
							<button
								class="absolute inset-y-0 left-2 my-auto flex h-10 w-10 cursor-pointer items-center justify-center rounded-md bg-black/50 text-white hover:bg-black/60 {starting ===
								pl.id
									? 'visible'
									: 'invisible group-hover/row:visible group-focus-within/row:visible'}"
								disabled={starting === pl.id}
								aria-label={current && !playback.paused
									? t('player.pause')
									: t('a11y.play_item', { title: pl.title })}
								onclick={() => playRow(pl)}
							>
								{#if starting === pl.id}
									<LimusicLoader class="size-6 text-white" />
								{:else}
									<!-- altIcon/showAlt, not a ternary: `icon` is read once at mount. -->
									<HugeiconsIcon
										icon={PlayIcon}
										altIcon={PauseIcon}
										showAlt={current && !playback.paused}
										class="h-5 w-5"
									/>
								{/if}
							</button>
						{/if}
						{#if current && !playback.paused}
							<!-- Where the ⋯ appears on hover, and gives way to it. Still, like the playing
							     row's mark in a track list: a looping animation keeps the compositor busy. -->
							<span
								class="pointer-events-none absolute inset-y-0 right-1 flex w-7 items-center justify-center text-primary group-hover/row:invisible"
							>
								<HugeiconsIcon icon={VolumeHighIcon} class="h-4 w-4" />
							</span>
						{/if}
						<PlaylistMenu item={pl} />
					</div>
					{#if pinnedCount && i === pinnedCount - 1}
						<div class="mx-3 my-1.5 h-px bg-border"></div>
					{/if}
				</div>
			{:else}
				{#if library.loading}
					<p class="px-3 py-1.5 text-xs text-muted-foreground">{t('common.loading')}</p>
				{/if}
			{/each}
		</div>
	</div>
</aside>
