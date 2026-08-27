<script lang="ts">
	// Ctrl+K search, without leaving the page you're on. Runs the same debounced `search_all` preview
	// the search field runs (`searchPreview`, same page-cache key), so the two show the same rows and
	// a query previewed here doesn't get searched again when you open the full results.
	//
	// shouldFilter={false}: the rows come back already ranked by YouTube, and re-scoring them against
	// the raw query locally would hide results whose title doesn't contain what you typed.
	// vimBindings={false}: those bind ctrl+k to "move up", which is the key that opens this.
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Search01Icon, MusicNote01Icon, UserIcon } from '@hugeicons/core-free-icons';
	import * as Command from '$lib/components/ui/command/index.js';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import ExplicitIcon from './ExplicitIcon.svelte';
	import ItemMenu from './ItemMenu.svelte';
	import type { BrowseItem } from '$lib/api';
	import { openItem, searchPreview } from '$lib/browse';
	import { ui } from '$lib/player.svelte';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';

	const KIND: Record<string, string> = $derived({
		song: t('common.song_singular'),
		album: t('common.album_singular'),
		artist: t('common.artist_singular'),
		playlist: t('common.playlist_singular')
	});

	let query = $state('');
	let items = $state<BrowseItem[]>([]);
	let loading = $state(false);
	let loadedFor = ''; // query `items` belongs to, so a stale response can't land
	// The row a right-click menu belongs to: whatever the pointer last entered. One menu for the
	// whole dialog, because `data-ctx` sits on the dialog itself (see below) and only one row can be
	// under the pointer.
	let ctxItem = $state<BrowseItem | null>(null);

	// The menu's popup lives on <body>, which the dialog counts as an interaction outside itself and
	// would close on, unmounting the menu mid-click. `data-menu` marks the popup and its backdrop, so
	// clicking one is treated as still being inside. Everything else outside still dismisses.
	const inMenu = (e: { detail: { originalEvent: Event } }) => {
		const t = e.detail.originalEvent.target;
		return t instanceof Element && !!t.closest('[data-menu]');
	};

	// Opening is itself a keystroke, so nothing is fetched until the typing pauses. `loading` is set
	// on the keystroke rather than when the timer fires: otherwise the empty list reads as "no
	// results" for the whole debounce, on every query.
	$effect(() => {
		const q = query.trim();
		if (q.length < 2) {
			items = [];
			loading = false;
			loadedFor = '';
			return;
		}
		if (q === loadedFor) return;
		items = [];
		loading = true;
		const timer = setTimeout(() => load(q), 300);
		return () => clearTimeout(timer);
	});

	// Closing clears the field, which the effect above turns into an empty list: reopening starts
	// fresh instead of on the last search's rows.
	$effect(() => {
		if (!ui.paletteOpen) query = '';
	});

	async function load(q: string) {
		loadedFor = q;
		try {
			const next = await searchPreview(q);
			if (loadedFor === q) items = next;
		} catch {
			if (loadedFor === q) items = [];
		} finally {
			if (loadedFor === q) loading = false;
		}
	}

	function choose(item: BrowseItem) {
		ui.paletteOpen = false;
		openItem(item); // a song plays, everything else opens its page
	}

	function allResults() {
		const q = query.trim();
		if (!q) return;
		ui.paletteOpen = false;
		goto(`/search?q=${encodeURIComponent(q)}`);
	}
</script>

<Command.Dialog
	bind:open={ui.paletteOpen}
	shouldFilter={false}
	vimBindings={false}
	loop
	title={t('common.search')}
	description={t('common.command_description')}
	class="sm:max-w-xl"
	contentProps={{
		// data-ctx: right-clicking a row opens that item's menu at the pointer (see `ctxHost`). The
		// input keeps WebKit's own menu (`wantsNative`).
		'data-ctx': '',
		onInteractOutside: (e: { detail: { originalEvent: Event }; preventDefault: () => void }) => {
			if (inMenu(e)) e.preventDefault();
		},
		onFocusOutside: (e: { detail: { originalEvent: Event }; preventDefault: () => void }) => {
			if (inMenu(e)) e.preventDefault();
		}
	}}
>
	<Command.Input bind:value={query} placeholder={t('common.search_placeholder')} />
	<Command.List class="max-h-[22rem]">
		{#if loading}
			{#each Array(4) as _, i (i)}
				<div class="flex items-center gap-3 px-3 py-2">
					<Skeleton class="h-10 w-10 shrink-0 rounded-md" />
					<div class="min-w-0 flex-1">
						<Skeleton class="h-3 w-40 rounded" />
						<Skeleton class="mt-2 h-2.5 w-24 rounded" />
					</div>
				</div>
			{/each}
		{:else if !items.length}
			<div class="px-4 py-6 text-center text-sm text-muted-foreground">
				{query.trim().length < 2 ? t('common.type_to_search') : t('common.nothing_quick')}
			</div>
		{:else}
			<Command.Group heading={t('common.results')}>
				{#each items as item (item.id)}
					<Command.Item
						value={item.id}
						onSelect={() => choose(item)}
						onmouseenter={() => (ctxItem = item)}
						class="gap-3 px-2 py-1.5"
					>
						{#if item.thumbnail}
							<!-- 400, the same size the cards ask for: the CDN doesn't serve every rewritten
							     size, that one is verified, and the row lands on an image the grid already
							     fetched. -->
							<img
								src={thumb(item.thumbnail, 400)}
								alt=""
								class="h-10 w-10 shrink-0 object-cover {item.kind === 'artist'
									? 'rounded-full'
									: 'rounded-md'}"
							/>
						{:else}
							<div
								class="flex h-10 w-10 shrink-0 items-center justify-center bg-muted text-muted-foreground/50 {item.kind ===
								'artist'
									? 'rounded-full'
									: 'rounded-md'}"
							>
								<HugeiconsIcon
									icon={item.kind === 'artist' ? UserIcon : MusicNote01Icon}
									class="h-5 w-5"
								/>
							</div>
						{/if}
						<div class="min-w-0 flex-1">
							<div class="truncate text-sm">{item.title}</div>
							<div class="flex items-center gap-1 text-xs text-muted-foreground">
								{#if item.explicit}
									<ExplicitIcon class="h-3 w-3 shrink-0" />
								{/if}
								<span class="truncate">
									{KIND[item.kind]}{item.subtitle ? ` • ${item.subtitle}` : ''}
								</span>
							</div>
						</div>
					</Command.Item>
				{/each}
			</Command.Group>
		{/if}

		{#if query.trim().length >= 2}
			<Command.Group>
				<Command.Item value="__all__" onSelect={allResults} class="gap-2 text-muted-foreground">
					<HugeiconsIcon icon={Search01Icon} class="h-3.5 w-3.5" />
					<span class="truncate">All results for “{query.trim()}”</span>
				</Command.Item>
			</Command.Group>
		{/if}
	</Command.List>
	<!-- No visible trigger: a palette row is too small for a hover-only ⋯, and this only ever opens
	     from a right-click. -->
	{#if ctxItem}
		<ItemMenu item={ctxItem} triggerClass="hidden" />
	{/if}
</Command.Dialog>
