<script lang="ts">
	// Stands where the seek bar would for a broadcast on air (#424): it has no length, and the app
	// does not seek in one. 16 px tall, the seek row's height, so nothing around it moves.
	import { t } from '$lib/i18n.svelte';

	// `onArt`: drawn over the cover (the mini player), where the theme's red on a tint of itself
	// loses to whatever the artwork is. White on a dark veil reads on any cover.
	let { onArt = false }: { onArt?: boolean } = $props();
</script>

<span
	class="inline-flex h-4 items-center gap-1.5 rounded-full px-2 text-[0.6875rem] font-semibold uppercase leading-none tracking-wider {onArt
		? 'bg-black/35 text-white'
		: 'bg-destructive/15 text-destructive'}"
>
	<span class="size-1.5 rounded-full bg-destructive" aria-hidden="true"></span>
	{t('player.live')}
</span>
