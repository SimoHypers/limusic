<script lang="ts">
	// The player bar's download trigger. The choices (format, quality, remembering it) live in the
	// shared DownloadDialog, which the track menus open too.
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Download01Icon } from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import { openDownload } from '$lib/downloads.svelte';
	import { playback } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	/** The video id of the track to download. */
	let { videoId }: { videoId: string } = $props();

	function open() {
		openDownload({
			videoId,
			title: playback.now?.title ?? '',
			artists: playback.now?.artists ?? ''
		});
	}
</script>

<Button
	variant="ghost"
	size="icon-sm"
	aria-label={t('player.download_track')}
	title={t('player.download_track')}
	onclick={open}
>
	<HugeiconsIcon icon={Download01Icon} class="h-4 w-4 text-muted-foreground" />
</Button>
