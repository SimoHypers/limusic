<script lang="ts">
	// Recently played playlists/albums/artists, as a list you scan rather than a row you page
	// through. Recents are re-entry points — you already know what they are, you just want the one
	// you want — so a title you can read beats a wall of identical squares.
	//
	// Bare rows on purpose, against the surfaced tiles of Shortcuts directly above: the things you
	// chose are elevated, the ones the app noticed for you are not. The caller drops anything that
	// is already a shortcut before it gets here — otherwise this is the same eight items again in a
	// different shape, which is exactly what it was.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		PlayIcon,
		MusicNote01Icon,
		UserIcon,
		ListRestartIcon,
		ArrowTurnBackwardIcon
	} from '@hugeicons/core-free-icons';
	import SectionHeading from './SectionHeading.svelte';
	import { ON_REPEAT_ID } from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { thumb } from '$lib/thumb';
	import { setDragItem } from '$lib/dnd';
	import { openItem, playItem } from '$lib/browse';
	import { formatSubtitle, t } from '$lib/i18n.svelte';

	let { items }: { items: BrowseItem[] } = $props();

	// Which row's fetch-then-play is in flight (playlists/albums have to be loaded before they can
	// play). One at a time: a second click while the first resolves is a mis-click, not a request.
	let busy = $state<string | null>(null);
	// Google's CDN 404s some rewritten sizes; a dead thumb degrades to the placeholder icon rather
	// than the browser's broken-image glyph. Keyed by id — a handful of rows, so a plain object.
	let failed = $state<Record<string, boolean>>({});

	async function play(item: BrowseItem) {
		if (busy) return;
		busy = item.id;
		try {
			await playItem(item);
		} finally {
			busy = null;
		}
	}
</script>

<section>
	<SectionHeading title={t('home.jump_back_in')} icon={ArrowTurnBackwardIcon} />
	<!-- CSS columns, same idiom as Forgotten favourites: fills top-to-bottom, balances the last
	     column itself, and needs no row count per breakpoint. -->
	<div class="columns-1 gap-x-6 md:columns-2 xl:columns-3">
		{#each items as item (item.id)}
			{@const round = item.kind === 'artist'}
			<div
				class="group/row flex break-inside-avoid cursor-pointer items-center gap-3 rounded-lg p-1.5 text-left transition-colors hover:bg-accent/10"
				role="button"
				tabindex="0"
				draggable="true"
				ondragstart={(e) => setDragItem(e, item)}
				onclick={() => openItem(item)}
				onkeydown={(e) => {
					if (e.target !== e.currentTarget) return;
					if (e.key === 'Enter' || e.key === ' ') {
						e.preventDefault();
						openItem(item);
					}
				}}
				title={item.subtitle ? `${item.title} — ${item.subtitle}` : item.title}
			>
				<div
					class="relative h-10 w-10 shrink-0 overflow-hidden bg-muted {round
						? 'rounded-full'
						: 'rounded-md'}"
				>
					{#if item.thumbnail && !failed[item.id]}
						<!-- 400 for a 40px slot: it's the size every card on the page already asked for, so
						     it comes straight out of the webview's cache. -->
						<img
							src={thumb(item.thumbnail, 400)}
							alt=""
							class="h-full w-full object-cover"
							loading="lazy"
							draggable="false"
							onerror={() => (failed = { ...failed, [item.id]: true })}
						/>
					{:else}
						{@const onRepeat = item.id === ON_REPEAT_ID}
						<div
							class="flex h-full w-full items-center justify-center {onRepeat
								? 'bg-primary/10 text-primary'
								: 'text-muted-foreground/50'}"
						>
							<!-- altIcon/showAlt, not a third ternary: `icon` is read once at mount. -->
							<HugeiconsIcon
								icon={round ? UserIcon : MusicNote01Icon}
								altIcon={ListRestartIcon}
								showAlt={onRepeat}
								class="h-4 w-4"
							/>
						</div>
					{/if}
				</div>
				<div class="min-w-0 flex-1">
					<div class="truncate text-sm font-medium">{item.title}</div>
					<!-- Subtitle when there is one (a creator or an artist tells you more than the kind
					     does); the kind is the fallback so the second line never collapses. -->
					<div class="truncate text-xs capitalize text-muted-foreground">
						{formatSubtitle(item.subtitle) || item.kind}
					</div>
				</div>
				{#if !round}
					<button
						class="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full bg-primary text-primary-foreground opacity-0 shadow-md transition-opacity hover:brightness-110 focus-visible:opacity-100 group-hover/row:opacity-100"
						class:animate-pulse={busy === item.id}
						disabled={busy === item.id}
						aria-label="Play {item.title}"
						onclick={(e) => {
							e.stopPropagation();
							play(item);
						}}
					>
						<HugeiconsIcon icon={PlayIcon} class="h-3.5 w-3.5" />
					</button>
				{/if}
			</div>
		{/each}
	</div>
</section>
