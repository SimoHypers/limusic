<script lang="ts">
	// Ctrl+K search, without leaving the page you're on. Runs the same debounced suggestions request
	// the search field runs (SearchSuggest), so the two show the same rows. The matching rows come
	// before the query completions because bits-ui highlights the first row after every update, and
	// that keeps Enter on the best match rather than on a completion of what you typed.
	//
	// shouldFilter={false}: the rows come back already ranked by YouTube, and re-scoring them against
	// the raw query locally would hide results whose title doesn't contain what you typed.
	// vimBindings={false}: those bind ctrl+k to "move up", which is the key that opens this.
	import { goto } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Search01Icon,
		HistoryIcon,
		MusicNote01Icon,
		UserIcon,
		FileImportIcon
	} from '@hugeicons/core-free-icons';
	import * as Command from '$lib/components/ui/command/index.js';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import ExplicitIcon from './ExplicitIcon.svelte';
	import ItemMenu from './ItemMenu.svelte';
	import { searchSuggestions, type BrowseItem, type SearchSuggestions } from '$lib/api';
	import { openItem, rowMeta } from '$lib/browse';
	import { ui } from '$lib/player.svelte';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';

	let query = $state('');
	let items = $state<BrowseItem[]>([]);
	let queries = $state<SearchSuggestions['queries']>([]);
	let loading = $state(false);
	let loadedFor = ''; // query `items` belongs to, so a stale response can't land
	// The row a right-click menu belongs to: whatever the pointer last entered. One menu for the
	// whole dialog, because `data-ctx` sits on the dialog itself (see below) and only one row can be
	// under the pointer.
	let ctxItem = $state<BrowseItem | null>(null);

	// The menu's popup lives on <body>, which the dialog counts as an interaction outside itself and
	// would close on, unmounting the menu mid-click. `data-menu` marks the popup and its backdrop, so
	// clicking one is treated as still being inside. Everything else outside still dismisses.
	const inMenu = (e: Event) => {
		const t = e.target;
		return t instanceof Element && !!t.closest('[data-menu]');
	};

	// Opening is itself a keystroke, so nothing is fetched until the typing pauses. `loading` is set
	// on the keystroke rather than when the timer fires: otherwise the empty list reads as "no
	// results" for the whole debounce, on every query.
	$effect(() => {
		const q = query.trim();
		if (q.length < 2) {
			[items, queries] = [[], []];
			loading = false;
			loadedFor = '';
			return;
		}
		if (q === loadedFor) return;
		[items, queries] = [[], []];
		loading = true;
		const timer = setTimeout(() => load(q), 300);
		return () => clearTimeout(timer);
	});

	// Closing clears the field, which the effect above turns into an empty list: reopening starts
	// fresh instead of on the last search's rows.
	$effect(() => {
		if (!ui.paletteOpen) query = '';
	});

	// A modal opened off a row (right-click ▸ Add to playlist, Share) is a plain fixed layer in the
	// layout at z-50, while this dialog portals to <body>: the z ties and DOM order decides, so the
	// modal opens behind the palette. Raising its z wouldn't be enough either, since the dialog's
	// focus trap pulls focus straight back out of the modal's filter field. So the palette steps
	// aside rather than fighting for the layer. Issue #218.
	$effect(() => {
		if (ui.addSongs || ui.share) ui.paletteOpen = false;
	});

	async function load(q: string) {
		loadedFor = q;
		try {
			const next = await searchSuggestions(q);
			if (loadedFor === q) ({ items, queries } = next);
		} catch {
			if (loadedFor === q) [items, queries] = [[], []];
		} finally {
			if (loadedFor === q) loading = false;
		}
	}

	function choose(item: BrowseItem) {
		ui.paletteOpen = false;
		openItem(item); // a song plays, everything else opens its page
	}

	function search(q: string) {
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
		// Closing hands focus back to whatever held it before Ctrl+K, which would take it off the
		// modal that just replaced the palette (see the effect above).
		onCloseAutoFocus: (e: Event) => {
			if (ui.addSongs || ui.share) e.preventDefault();
		},
		onInteractOutside: (e: PointerEvent) => {
			if (inMenu(e)) e.preventDefault();
		},
		onFocusOutside: (e: FocusEvent) => {
			if (inMenu(e)) e.preventDefault();
		}
	}}
>
	<Command.Input bind:value={query} placeholder={t('common.search_placeholder')} />
	<Command.List class="max-h-[22rem]">
		{#if !query.trim() || 'import playlist'.includes(query.toLowerCase().trim()) || 'import'.includes(query.toLowerCase().trim())}
			<Command.Group>
				<Command.Item
					value="import_playlist"
					onSelect={() => {
						ui.paletteOpen = false;
						goto('/import');
					}}
					class="gap-3 px-2 py-1.5"
				>
					<HugeiconsIcon icon={FileImportIcon} class="h-4 w-4 shrink-0 text-muted-foreground" />
					<span class="truncate text-sm">{t('import.title')}</span>
				</Command.Item>
			</Command.Group>
		{/if}
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
		{:else if !items.length && !queries.length}
			<div class="px-4 py-6 text-center text-sm text-muted-foreground">
				{query.trim().length < 2 ? t('common.type_to_search') : t('common.nothing_quick')}
			</div>
		{:else}
			{#if items.length}
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
									<span class="truncate">{rowMeta(item)}</span>
								</div>
							</div>
						</Command.Item>
					{/each}
				</Command.Group>
			{/if}
			{#if queries.length}
				<Command.Group>
					{#each queries as q (q.text)}
						<!-- Clears the right-click target: a completion has no item menu, and leaving the last
						     hovered row's would open it over a row it isn't about. -->
						<Command.Item
							value={`q:${q.text}`}
							onSelect={() => search(q.text)}
							onmouseenter={() => (ctxItem = null)}
							class="gap-3 px-2 py-2"
						>
							<!-- altIcon/showAlt, not a ternary: HugeiconsIcon freezes `icon` at mount. -->
							<HugeiconsIcon
								icon={Search01Icon}
								altIcon={HistoryIcon}
								showAlt={q.history}
								class="h-4 w-4 shrink-0 text-muted-foreground"
							/>
							<span class="truncate text-sm">{q.text}</span>
						</Command.Item>
					{/each}
				</Command.Group>
			{/if}
		{/if}

		{#if query.trim().length >= 2}
			<Command.Group>
				<Command.Item
					value="__all__"
					onSelect={() => search(query.trim())}
					class="gap-2 text-muted-foreground"
				>
					<HugeiconsIcon icon={Search01Icon} class="h-3.5 w-3.5" />
					<span class="truncate">{t('common.all_results_for', { query: query.trim() })}</span>
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
