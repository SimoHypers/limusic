<script lang="ts">
	// One line at the top edge of the player bar while another linked device is the player: what it
	// plays, and Continue here to take it over (src-tauri/src/devicelink.rs).
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { SmartPhone01Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import * as api from '$lib/api';
	import { link } from '$lib/link.svelte';
	import { toast } from '$lib/player.svelte';
	import { thumb } from '$lib/thumb';
	import { t } from '$lib/i18n.svelte';

	let busy = $state(false);

	async function continueHere() {
		busy = true;
		try {
			await api.linkContinueHere();
		} catch (e) {
			toast.error(String(e));
		} finally {
			busy = false;
		}
	}
</script>

{#if link.player && link.track}
	<div class="flex items-center gap-3 border-t bg-card px-3 py-1.5 sm:px-4">
		{#if link.track.thumbnail}
			<img
				src={thumb(link.track.thumbnail, 60)}
				alt=""
				class="h-7 w-7 shrink-0 rounded object-cover"
			/>
		{:else}
			<HugeiconsIcon icon={SmartPhone01Icon} class="size-5 shrink-0 text-muted-foreground" />
		{/if}
		<p class="min-w-0 flex-1 truncate text-xs">
			<span class="font-medium">{link.track.title}</span>
			<span class="text-muted-foreground">
				· {link.isPlaying
					? t('player.link_playing_on', { name: link.player.name })
					: t('player.link_paused_on', { name: link.player.name })}
			</span>
		</p>
		<Button size="sm" variant="secondary" class="h-7" disabled={busy || !link.track.id} onclick={continueHere}>
			{t('player.link_continue_here')}
		</Button>
	</div>
{/if}
