<script lang="ts">
	import { Skeleton } from '$lib/components/ui/skeleton';

	// Mirrors TrackRow's layout for loading track lists.
	let { hideThumb = false }: { hideThumb?: boolean } = $props();
</script>

<div class="flex w-full items-center gap-3 p-2">
	{#if !hideThumb}
		<Skeleton class="h-10 w-10 shrink-0 rounded-md" />
	{/if}
	<div class="min-w-0 flex-1 space-y-1.5">
		<Skeleton class="h-3.5 w-1/3 rounded" />
		<Skeleton class="h-3 w-1/5 rounded" />
	</div>
	<Skeleton class="h-3 w-8 shrink-0 rounded" />
</div>
