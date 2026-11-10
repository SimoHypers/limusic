<script lang="ts">
	// The search page's top result, drawn as the answer rather than as one card among many: the
	// artwork large, the title at display size, a play button that does the right thing for the
	// kind, and the few rows YouTube pairs with it (an artist's hits, a song's other takes).
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { MusicNote01Icon, PlayIcon, UserIcon } from '@hugeicons/core-free-icons';
	import type { BrowseItem } from '$lib/api';
	import { asSong, openItem, playItem } from '$lib/browse';
	import { openAddToPlaylist, playSong, playback, startRadio } from '$lib/player.svelte';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';
	import ExplicitIcon from './ExplicitIcon.svelte';
	import ItemMenu from './ItemMenu.svelte';
	import TrackRow from './TrackRow.svelte';

	let { item, related = [] }: { item: BrowseItem; related?: BrowseItem[] } = $props();

	const round = $derived(item.kind === 'artist');
	// A song's subtitle is its artists alone (it becomes the player bar's line once played), so on
	// its own it doesn't say "song". Every other kind arrives as YouTube wrote it: "Album • …".
	const subtitle = $derived(
		item.kind === 'song'
			? [t('common.song_singular'), item.subtitle].filter(Boolean).join(' • ')
			: item.subtitle
	);
	const rows = $derived(
		related
			.filter((r) => r.kind === 'song')
			.slice(0, 3)
			.map(asSong)
	);

	// MediaCard's retry chain: the sized URL, then the original once, then an icon tile.
	let attempt = $state(0);
	let washOk = $state(true);
	$effect(() => {
		item.thumbnail; // re-arm when a new search lands a different top result
		attempt = 0;
		washOk = true;
	});
	const sized = $derived(thumb(item.thumbnail, 400));
	const src = $derived(attempt === 0 ? sized : item.thumbnail);

	let busy = $state(false);
	async function play() {
		// An artist has nothing to fetch and play in order; its radio is what "play" means here.
		if (item.kind === 'artist') return startRadio('artist', item.id, item.title);
		if (busy) return;
		busy = true;
		try {
			await playItem(item);
		} finally {
			busy = false;
		}
	}
</script>

<div class="relative isolate overflow-hidden rounded-2xl border bg-card" data-ctx>
	<!-- Lit by its own artwork. 96px because blur-2xl throws away everything finer anyway (the same
	     reasoning as HomeHero), and a failed size just leaves the plain card. -->
	{#if item.thumbnail && washOk}
		<!-- The wash and its fade share one layer (`art-wash` on the wrapper): see HomeHero. -->
		<div class="art-wash pointer-events-none absolute inset-0 -z-10 overflow-hidden" aria-hidden="true">
			<img
				src={thumb(item.thumbnail, 96)}
				alt=""
				class="absolute inset-0 h-full w-full scale-125 object-cover opacity-60 blur-2xl"
				onerror={() => (washOk = false)}
			/>
			<div class="absolute inset-0 bg-gradient-to-b from-card/30 via-card/75 to-card"></div>
		</div>
	{/if}
	<!-- Below sm the card becomes two lines: the cover and the two buttons share the top one, the
	     name runs the full width under them. In a single line the buttons alone ate the space the
	     name needed, and a top result that shows one letter of the song is not an answer. -->
	<div class="relative flex items-center gap-4 p-5 max-sm:flex-col max-sm:items-stretch max-sm:gap-3">
		<div
			role="button"
			tabindex="0"
			class="flex min-w-0 flex-1 cursor-pointer items-center gap-5 max-sm:flex-col max-sm:items-start max-sm:gap-3"
			onclick={() => openItem(item)}
			onkeydown={(e) => {
				if (e.target !== e.currentTarget) return;
				if (e.key === 'Enter' || e.key === ' ') {
					e.preventDefault();
					openItem(item);
				}
			}}
			title={item.subtitle ? `${item.title} • ${item.subtitle}` : item.title}
		>
			<div
				class="flex h-28 w-28 shrink-0 items-center justify-center overflow-hidden bg-muted text-muted-foreground/50 shadow-xl max-sm:h-20 max-sm:w-20 {round
					? 'rounded-full'
					: 'rounded-xl'}"
			>
				{#if item.thumbnail && attempt < 2}
					<img
						{src}
						alt=""
						class="h-full w-full object-cover"
						draggable="false"
						onerror={() => (attempt = attempt === 0 && sized !== item.thumbnail ? 1 : 2)}
					/>
				{:else}
					<HugeiconsIcon icon={round ? UserIcon : MusicNote01Icon} class="h-9 w-9" />
				{/if}
			</div>
			<div class="min-w-0">
				<div
					class="line-clamp-2 font-heading font-bold leading-tight tracking-tight {item.title
						.length > 28
						? 'text-2xl'
						: 'text-3xl'} max-sm:text-2xl"
				>
					{item.title}
				</div>
				{#if subtitle || item.explicit}
					<div class="mt-2 flex items-center gap-1.5 text-sm text-muted-foreground">
						{#if item.explicit}<ExplicitIcon class="h-3.5 w-3.5 shrink-0" />{/if}
						<span class="truncate">{subtitle}</span>
					</div>
				{/if}
			</div>
		</div>
		<!-- Below sm this block leaves the flow and pins itself to the card's top-right corner, so the
		     cover and the name get the full width under it. -->
		<div class="flex shrink-0 items-center gap-4 max-sm:absolute max-sm:right-5 max-sm:top-5 max-sm:gap-2">
			<button
				class="flex h-12 w-12 shrink-0 cursor-pointer items-center justify-center rounded-full bg-primary text-primary-foreground shadow-lg transition-transform hover:scale-105 active:scale-95 max-sm:h-10 max-sm:w-10"
				class:animate-pulse={busy}
				disabled={busy}
				aria-label={item.kind === 'artist' ? t('player.start_radio') : t('player.play')}
				title={item.kind === 'artist' ? t('player.start_radio') : t('player.play')}
				onclick={play}
			>
				<HugeiconsIcon icon={PlayIcon} class="h-5 w-5" />
			</button>
			<ItemMenu
				{item}
				triggerClass="flex h-9 w-9 shrink-0 cursor-pointer items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground"
			/>
		</div>
	</div>
	{#if rows.length}
		<div class="border-t border-border/60 px-2 py-1.5">
			{#each rows as song, i (song.video_id + ':' + i)}
				<TrackRow
					{song}
					compact
					active={playback.now?.videoId === song.video_id}
					onplay={() => playSong(song)}
					onAdd={() => openAddToPlaylist(song)}
				/>
			{/each}
		</div>
	{/if}
</div>
