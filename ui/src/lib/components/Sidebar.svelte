<script lang="ts">
	import { page } from '$app/state';
	import { scale } from 'svelte/transition';
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
		MusicNote01Icon,
		ListRestartIcon,
		ComputerIcon,
		SquareArrowLeft01Icon,
		SquareArrowRight01Icon
	} from '@hugeicons/core-free-icons';
	import { toggleMode } from 'mode-watcher';
	import { Button } from '$lib/components/ui/button';
	import { ON_REPEAT_ID, isLocalPlaylist, type BrowseItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import PlaylistMenu from './PlaylistMenu.svelte';
	import { library, personal, ui, openNewPlaylist, toggleSidebar } from '$lib/player.svelte';
	import { mergeSaved, orderLibrary } from '$lib/personal';
	import { t } from '$lib/i18n.svelte';

	const nav = $derived([
		{ href: '/', label: t('nav.home'), icon: Home01Icon },
		{ href: '/search', label: t('nav.search'), icon: Search01Icon },
		{ href: '/library', label: t('nav.library'), icon: LibraryIcon }
	]);
	const isActive = (href: string) =>
		href === '/' ? page.url.pathname === '/' : page.url.pathname.startsWith(href);

	// Pinned first (in pin order), then everything else by last played. Derived here rather than in
	// the shared `library` store so the Library page keeps YouTube's own ordering. Playlists saved
	// on this machine sit in the same list: signed out they are the only ones there.
	const playlists = $derived(
		orderLibrary(mergeSaved(personal, library.items, 'playlist'), personal)
	);
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
	const wide = (cls: string) => (collapsed ? '' : cls);
</script>

<aside
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
		</div>
	</div>

	<nav class="mt-2 flex flex-col gap-1">
		{#each nav as n (n.href)}
			<a
				href={n.href}
				title={n.label}
				class="group relative flex items-center justify-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors {wide(
					'lg:justify-start'
				)} {isActive(n.href)
					? 'bg-primary/10 text-primary'
					: 'text-sidebar-foreground/70 hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'}"
			>
				{#if isActive(n.href)}
					<span
						transition:scale={{ duration: 200, start: 0.4 }}
						class="absolute left-0 top-1/2 h-5 w-1 -translate-y-1/2 rounded-r-full bg-primary"
					></span>
				{/if}
				<HugeiconsIcon
					icon={n.icon}
					class="h-5 w-5 shrink-0 transition-transform duration-200 group-hover:scale-110"
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
				class="h-5 w-5 shrink-0 transition-transform duration-200 group-hover:scale-110"
			/>
			<span class="hidden {wide('lg:inline')}">{t('nav.settings')}</span>
		</button>
	</nav>

	<!-- Playlists. Hidden on the icon rail (needs labels; matches YTM's collapsed rail). flex-1 lets
	     the list fill the space and scroll. Always there, signed out included: a playlist can be
	     made on this machine without an account (#251). -->
	<div class="mt-3 hidden min-h-0 flex-1 flex-col border-t pt-3 {wide('lg:flex')}">
		<Button variant="outline" size="sm" class="mb-2 w-full gap-2" onclick={() => openNewPlaylist()}>
			<HugeiconsIcon icon={Add01Icon} class="h-4 w-4" /> {t('nav.new_playlist')}
		</Button>
		<div class="min-h-0 flex-1 overflow-y-auto">
			{#each playlists as pl, i (pl.id)}
				<!-- The ⋯ is a sibling of the link, not a child: a <button> inside an <a> is invalid
				     HTML. pr-9 keeps the title clear of the button that overlays the row on hover. -->
				<div class="group/row relative" data-ctx>
					<a
						href={playlistHref(pl)}
						title={pl.title}
						class="flex items-center gap-2.5 rounded-lg py-1.5 pl-2 pr-9 transition-colors hover:bg-sidebar-accent/50"
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
										: 'text-muted-foreground/50'}"
								>
									<!-- altIcon/showAlt, not a ternary: `icon` is read once at mount. -->
									<HugeiconsIcon
										icon={MusicNote01Icon}
										altIcon={ListRestartIcon}
										showAlt={pl.id === ON_REPEAT_ID}
										class={pl.id === ON_REPEAT_ID ? 'h-5 w-5' : 'h-4 w-4'}
									/>
								</div>
							{/if}
						</div>
						{#if personal.pins.includes(pl.id)}
							<span
								class="absolute left-9 top-0.5 flex h-4 w-4 items-center justify-center rounded-full bg-primary text-primary-foreground shadow"
							>
								<HugeiconsIcon icon={PinIcon} class="h-2.5 w-2.5" />
							</span>
						{/if}
						<div class="min-w-0 flex-1">
							<div class="truncate text-[13px] font-medium">{pl.title}</div>
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
					<PlaylistMenu item={pl} />
				</div>
				{#if pinnedCount && i === pinnedCount - 1}
					<div class="mx-3 my-1.5 h-px bg-border"></div>
				{/if}
			{:else}
				{#if library.loading}
					<p class="px-3 py-1.5 text-xs text-muted-foreground">{t('common.loading')}</p>
				{/if}
			{/each}
		</div>
	</div>
</aside>
