<script lang="ts">
	// "Now playing ↑/↓": the way back to the playing row once a list has scrolled it off screen.
	// The queue and the playlist page both float it over their scroller. Place it after the
	// scroller in the DOM, inside a positioned box: a transition painted before a scroller can
	// blank its scrollbar on WebKitGTK (docs/UI-PERFORMANCE.md). Opacity only.
	import { fade } from 'svelte/transition';
	import { MediaQuery } from 'svelte/reactivity';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowDown01Icon, ArrowUp01Icon } from '@hugeicons/core-free-icons';
	import { t } from '$lib/i18n.svelte';

	let { above, onclick }: { above: boolean; onclick: () => void } = $props();

	const reducedMotion = new MediaQuery('(prefers-reduced-motion: reduce)');
</script>

<button
	class="absolute bottom-3 left-1/2 z-10 flex -translate-x-1/2 cursor-pointer items-center gap-1.5 rounded-full border bg-popover px-3 py-1.5 text-xs font-medium text-popover-foreground shadow-lg hover:bg-accent"
	{onclick}
	transition:fade={{ duration: reducedMotion.current ? 0 : 150 }}
>
	<!-- altIcon/showAlt: `icon` is frozen at mount. -->
	<HugeiconsIcon icon={ArrowDown01Icon} altIcon={ArrowUp01Icon} showAlt={above} class="size-3.5" />
	{t('player.now_playing')}
</button>
