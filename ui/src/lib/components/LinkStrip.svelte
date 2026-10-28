<script lang="ts">
	// Under the player bar while it is the remote for another linked device, as Spotify marks it:
	// the bar above shows and controls what plays there (src-tauri/src/devicelink.rs).
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { SmartPhone01Icon } from '@hugeicons/core-free-icons';
	import { link } from '$lib/link.svelte';
	import { t } from '$lib/i18n.svelte';
</script>

{#if link.remote && link.player}
	<div
		class="flex items-center justify-center gap-1.5 bg-primary px-3 py-1 text-xs font-medium text-primary-foreground"
	>
		<HugeiconsIcon icon={SmartPhone01Icon} class="size-3.5" />
		{link.isPlaying
			? t('player.link_playing_on', { name: link.player.name })
			: t('player.link_paused_on', { name: link.player.name })}
	</div>
{/if}
