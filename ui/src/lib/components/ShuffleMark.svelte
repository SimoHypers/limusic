<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ShuffleIcon } from '@hugeicons/core-free-icons';
	import { DynamicShuffleIcon } from '$lib/shuffleIcons';
	// The shuffle button's icon. HugeiconsIcon draws `icon` once at mount and only re-renders on
	// `showAlt`, so dynamic is the alt icon rather than a swapped `icon`.
	let { dynamic = false, class: cls = '' }: { dynamic?: boolean; class?: string } = $props();
</script>

<HugeiconsIcon icon={ShuffleIcon} altIcon={DynamicShuffleIcon} showAlt={dynamic} class={cls} />
