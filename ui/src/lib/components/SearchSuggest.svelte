<script lang="ts">
	// The search field plus its typeahead preview: type, wait 500ms, get a handful of real results
	// under the input. Runs the same `search_all` the search page runs and writes the same page-cache
	// key, so submitting a previewed query paints from cache instead of searching twice.
	//
	// Must live inside a <form>: Enter with nothing highlighted, and the "All results" row, fall
	// through to that form's onsubmit, which is where each caller decides what a full search means
	// (run it in place, or navigate to /search).
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Search01Icon, MusicNote01Icon, UserIcon } from '@hugeicons/core-free-icons';
	import { Input } from '$lib/components/ui/input';
	import { Skeleton } from '$lib/components/ui/skeleton';
	import ExplicitIcon from './ExplicitIcon.svelte';
	import type { BrowseItem } from '$lib/api';
	import { openItem, searchPreview } from '$lib/browse';
	import { MOD } from '$lib/shortcuts';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';

	let {
		value = $bindable(''),
		placeholder = 'Search',
		inputClass = '',
		/** Panel geometry. Default matches the field; a narrow field wants its own width. */
		panelClass = 'left-0 right-0',
		onpick
	}: {
		value?: string;
		placeholder?: string;
		inputClass?: string;
		panelClass?: string;
		/** Fired after a row is taken (played or navigated) — for callers that dismiss themselves. */
		onpick?: () => void;
	} = $props();

	let open = $state(false);
	let items = $state<BrowseItem[]>([]);
	let loading = $state(false);
	let active = $state(-1); // keyboard-highlighted row, -1 = none (Enter submits the form)
	let loadedFor = ''; // query `items` belongs to, so a stale response can't land
	let debounce: ReturnType<typeof setTimeout> | undefined;

	const KIND: Record<string, string> = $derived({
		song: t('common.song_singular'),
		album: t('common.album_singular'),
		artist: t('common.artist_singular'),
		playlist: t('common.playlist_singular')
	});

	async function load(q: string) {
		loadedFor = q;
		active = -1;
		loading = true;
		try {
			const next = await searchPreview(q);
			if (loadedFor === q) items = next;
		} catch {
			if (loadedFor === q) items = [];
		} finally {
			if (loadedFor === q) loading = false;
		}
	}

	// Reads the element, not `value`: the binding lands on this same event and the order of the two
	// listeners is not ours to assume.
	function onType(e: Event & { currentTarget: HTMLInputElement }) {
		clearTimeout(debounce);
		const q = e.currentTarget.value.trim();
		if (q.length < 2) {
			close();
			return;
		}
		open = true;
		if (q !== loadedFor) {
			// Loading starts now, not when the timer fires: otherwise the empty panel reads as
			// "no results" for the whole debounce, on every query.
			items = [];
			loading = true;
		}
		debounce = setTimeout(() => load(q), 500);
	}

	function close() {
		clearTimeout(debounce);
		open = false;
		loading = false;
		active = -1;
	}

	function choose(item: BrowseItem) {
		close();
		openItem(item); // a song plays, everything else opens its page
		onpick?.();
	}

	function onKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape' && open) {
			e.preventDefault();
			close();
		} else if (e.key === 'Enter') {
			// Only a highlighted row is ours; a bare Enter is the caller's form submit.
			if (active >= 0 && items[active]) {
				e.preventDefault();
				choose(items[active]);
			} else {
				close();
			}
		} else if ((e.key === 'ArrowDown' || e.key === 'ArrowUp') && items.length) {
			e.preventDefault();
			open = true;
			const n = items.length;
			active = e.key === 'ArrowDown' ? (active + 1) % n : (active <= 0 ? n : active) - 1;
		}
	}
</script>

<!-- Rows preventDefault on mousedown, so focus never leaves the input while one is being clicked:
     anything that reaches focusout is a real move away from the field. -->
<div
	class="relative w-full min-w-0"
	onfocusout={(e) => {
		if (!e.currentTarget.contains(e.relatedTarget as Node | null)) close();
	}}
>
	<Input
		bind:value
		{placeholder}
		class="pr-16 {inputClass}"
		autocomplete="off"
		role="combobox"
		aria-expanded={open}
		aria-controls="search-suggest"
		oninput={onType}
		onkeydown={onKeydown}
		onfocus={() => {
			if (items.length && value.trim() === loadedFor) open = true;
		}}
	/>
	<!-- Advertises the palette, which searches the same thing from anywhere in the app
	     (shortcuts.ts). Out of the way once there is a query to read, and never a click target:
	     the field behind it is the target. -->
	{#if !value}
		<kbd
			class="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 rounded border bg-muted px-1.5 py-0.5 font-mono text-[0.625rem] font-medium tracking-wide text-muted-foreground"
		>
			{MOD}K
		</kbd>
	{/if}
	{#if open}
		<div
			id="search-suggest"
			role="listbox"
			aria-label={t('a11y.search_preview')}
			class="absolute top-full z-50 mt-2 overflow-hidden rounded-xl border bg-popover text-popover-foreground shadow-xl animate-in fade-in-0 zoom-in-95 duration-150 {panelClass}"
		>
			{#if loading && !items.length}
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
				<div class="px-4 py-3 text-sm text-muted-foreground">{t('common.nothing_quick')}</div>
			{:else}
				{#each items as item, i (item.id)}
					{@const hero = i === 0}
					<button
						type="button"
						role="option"
						aria-selected={i === active}
						class="flex w-full cursor-pointer items-center gap-3 px-3 text-left transition-colors {i ===
						active
							? 'bg-accent/60'
							: 'hover:bg-accent/40'} {hero ? 'border-b py-2.5' : 'py-1.5'}"
						onmousedown={(e) => e.preventDefault()}
						onmouseenter={() => (active = i)}
						onclick={() => choose(item)}
					>
						{#if item.thumbnail}
							<!-- 400, the same size the cards ask for: the CDN doesn't serve every rewritten size,
							     that one is verified, and the row lands on an image the grid already fetched. -->
							<img
								src={thumb(item.thumbnail, 400)}
								alt=""
								class="shrink-0 object-cover {item.kind === 'artist'
									? 'rounded-full'
									: 'rounded-md'} {hero ? 'h-12 w-12' : 'h-10 w-10'}"
							/>
						{:else}
							<div
								class="flex shrink-0 items-center justify-center bg-muted text-muted-foreground/50 {item.kind ===
								'artist'
									? 'rounded-full'
									: 'rounded-md'} {hero ? 'h-12 w-12' : 'h-10 w-10'}"
							>
								<HugeiconsIcon
									icon={item.kind === 'artist' ? UserIcon : MusicNote01Icon}
									class="h-5 w-5"
								/>
							</div>
						{/if}
						<div class="min-w-0 flex-1">
							<div class="truncate {hero ? 'font-semibold' : 'text-sm'}">{item.title}</div>
							<div class="flex items-center gap-1 text-xs text-muted-foreground">
								{#if item.explicit}
									<ExplicitIcon class="h-3 w-3 shrink-0" />
								{/if}
								<span class="truncate">
									{KIND[item.kind]}{item.subtitle ? ` • ${item.subtitle}` : ''}
								</span>
							</div>
						</div>
						{#if hero}
							<span
								class="shrink-0 rounded-full bg-primary/10 px-2 py-0.5 text-[0.625rem] font-semibold uppercase tracking-wide text-primary"
							>
								Top result
							</span>
						{/if}
					</button>
				{/each}
			{/if}
			<!-- submit, so the enclosing form decides what "all results" does. -->
			<button
				type="submit"
				class="flex w-full cursor-pointer items-center gap-2 border-t bg-muted/30 px-3 py-2 text-left text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
				onmousedown={(e) => e.preventDefault()}
				onmouseenter={() => (active = -1)}
				onclick={close}
			>
				<HugeiconsIcon icon={Search01Icon} class="h-3.5 w-3.5" />
				All results for “{value.trim()}”
			</button>
		</div>
	{/if}
</div>
