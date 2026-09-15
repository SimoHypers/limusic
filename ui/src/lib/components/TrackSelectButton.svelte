<script lang="ts">
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { CursorMagicSelection04Icon } from '@hugeicons/core-free-icons';
	import { t } from '$lib/i18n.svelte';
	import type { TrackSelection } from '$lib/selection.svelte';

	// Lives in each list's own header row, so `class` covers the pages whose controls aren't the
	// shadcn ghost button (the album's round bordered row).
	let {
		selection,
		class: className =
			'flex h-9 w-9 cursor-pointer items-center justify-center rounded-full transition hover:bg-muted hover:text-foreground'
	}: { selection: TrackSelection; class?: string } = $props();
</script>

<button
	class="{className} {selection.active ? 'text-primary' : 'text-muted-foreground'}"
	aria-pressed={selection.active}
	title={selection.active ? t('selection.exit') : t('selection.enter')}
	aria-label={selection.active ? t('selection.exit') : t('selection.enter')}
	onclick={() => (selection.active ? selection.exit() : selection.enter())}
>
	<HugeiconsIcon icon={CursorMagicSelection04Icon} class="h-5 w-5" />
</button>
