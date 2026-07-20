<script lang="ts">
	// The home grid the user curates. Nothing is ever auto-added — it holds exactly what was put in
	// it, up to MAX_PICKS. Logic in $lib/personal.ts.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Cancel01Icon } from '@hugeicons/core-free-icons';
	import MediaCard from './MediaCard.svelte';
	import { personal, removePick } from '$lib/player.svelte';
	import { MAX_PICKS, orderedPicks } from '$lib/personal';

	const picks = $derived(orderedPicks(personal));
</script>

<!-- Nothing added yet → the section isn't there at all. It appears the moment the user adds a tile
     and disappears again if they clear it out. -->
{#if picks.length}
	<section class="mb-8">
		<div class="mb-3 flex items-baseline gap-2">
			<h2 class="font-heading text-lg font-semibold">Quick Picks</h2>
			<span class="text-xs text-muted-foreground">{picks.length}/{MAX_PICKS}</span>
		</div>

		<!-- auto-fill across the full width: small tiles, as many per row as fit, no dead gutter on
		     wide windows. Deliberately denser than the 10rem cards in the shelves below. -->
		<div class="grid grid-cols-[repeat(auto-fill,minmax(5.5rem,1fr))] gap-2">
			{#each picks as item (item.id)}
				<!-- group/pick, not `group`: MediaCard already owns a plain `group` for its hover play
				     button, and an unnamed nested group would fire both from either hover. -->
				<div class="group/pick relative">
					<MediaCard {item} compact />
					<!-- Top-left: MediaCard puts the song ⋯ top-right and the play button bottom-right. -->
					<button
						onclick={() => removePick(item.id)}
						title="Remove from Quick Picks"
						aria-label="Remove from Quick Picks"
						class="absolute left-1.5 top-1.5 z-10 flex h-6 w-6 cursor-pointer items-center justify-center rounded-full bg-background/80 text-foreground opacity-0 shadow-md backdrop-blur-sm transition hover:bg-background focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring group-hover/pick:opacity-100"
					>
						<HugeiconsIcon icon={Cancel01Icon} class="h-3 w-3" />
					</button>
				</div>
			{/each}
		</div>
	</section>
{/if}
