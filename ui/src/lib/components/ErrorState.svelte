<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { RefreshIcon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';

	let { message, onRetry }: { message: string; onRetry: () => void } = $props();
</script>

<div class="flex flex-col items-start gap-3">
	<p class="text-sm text-destructive">{message}</p>
	<Button variant="outline" size="sm" class="gap-2" onclick={onRetry}>
		<HugeiconsIcon icon={RefreshIcon} class="h-4 w-4" /> Try again
	</Button>
</div>
