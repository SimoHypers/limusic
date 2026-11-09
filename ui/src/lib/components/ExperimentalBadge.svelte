<script lang="ts">
	// The "Experimental" mark on features still being tuned (the Spotify import #375, karaoke #466),
	// styled like the badge on Settings ▸ Themes. `tooltip` says what is rough about this one.
	import { t } from '$lib/i18n.svelte';

	let { tooltip }: { tooltip?: string } = $props();
</script>

<span
	class="shrink-0 rounded-full bg-primary/12 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-primary"
	title={tooltip ?? t('import.experimental_tooltip')}
>
	{t('import.experimental')}
</span>
