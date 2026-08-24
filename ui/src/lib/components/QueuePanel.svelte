<script lang="ts">
	import { fade, fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import QueueList from './QueueList.svelte';
	import { t } from '$lib/i18n.svelte';

	let { onClose }: { onClose: () => void } = $props();
</script>

<!-- The panel always floats over the content (see the `relative` wrapper in +layout) rather than
     squeezing it into a column: two docked panels left the page too narrow to read, and a page you
     can't use behind a panel you opened on purpose is the better trade. Below lg a scrim dismisses
     it; at lg+ the content stays visible underneath and the player bar's button closes it. -->
<button
	class="absolute inset-0 z-20 cursor-default bg-black/40 lg:hidden"
	onclick={onClose}
	aria-label={t('a11y.close_queue')}
	transition:fade={{ duration: 150 }}
></button>
<aside
	transition:fly={{ x: 32, duration: 220, easing: cubicOut }}
	class="absolute inset-y-0 right-0 z-30 flex h-full w-80 max-w-[80vw] flex-col border-l bg-card shadow-2xl"
>
	<h2 class="border-b px-4 py-3 font-heading text-sm font-semibold">{t('queue.title')}</h2>
	<QueueList />
</aside>
