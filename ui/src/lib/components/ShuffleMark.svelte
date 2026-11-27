<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ShuffleIcon, SparkleIcon } from '@hugeicons/core-free-icons';
	// The shuffle button's icon. Dynamic shuffle adds a sparkle in the corner: the free set has no
	// shuffle-with-sparkle glyph. `class` sizes and colours the shuffle; the sparkle scales with it.
	let { dynamic = false, class: cls = '' }: { dynamic?: boolean; class?: string } = $props();
</script>

<span class="relative inline-flex">
	<HugeiconsIcon icon={ShuffleIcon} class={cls} />
	{#if dynamic}
		<HugeiconsIcon
			icon={SparkleIcon}
			strokeWidth={2.5}
			class="absolute -top-[30%] -right-[30%] size-[60%] text-primary"
		/>
	{/if}
</span>
