<script lang="ts">
	// Home's Live section (#424): broadcasts on air from official artist channels. Nothing is drawn
	// until they arrive, or when none are on air, so the slot never holds an empty heading.
	import { onMount } from 'svelte';
	import Shelf from './Shelf.svelte';
	import * as api from '$lib/api';
	import type { BrowseItem } from '$lib/api';
	import { personal } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	let items = $state<BrowseItem[]>([]);

	onMount(() => {
		api.getLiveNow()
			.then((found) => (items = found.map((i) => ({ ...i, live: true }))))
			.catch(() => {}); // a missing section is the whole failure: home itself loaded
	});
</script>

{#if items.length}
	<!-- `rich` off: a pile of song items would otherwise become track rows, and a broadcast needs
	     its picture. Not queued as a set, so autoplay doesn't line up the other streams. -->
	<Shelf
		title={t('home.live_now')}
		{items}
		rich={false}
		queueAll={false}
		size={personal.home.cards}
	/>
{/if}
