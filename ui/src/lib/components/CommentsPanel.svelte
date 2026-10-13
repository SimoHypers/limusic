<script lang="ts">
	import { fade, fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import CommentsView from './CommentsView.svelte';
	import { t } from '$lib/i18n.svelte';

	let { onClose, queueOpen = false }: { onClose: () => void; queueOpen?: boolean } = $props();
</script>

<!-- Same slot and overlay pattern as LyricsPanel: always over the content, with a dismiss scrim
     below lg. Lyrics and comments share the slot (opening one closes the other, see +layout), so
     when the queue is open too this steps left of it at lg+, exactly where the lyrics do. -->
<button
	class="absolute inset-0 z-20 cursor-default bg-black/40 lg:hidden"
	onclick={onClose}
	aria-label={t('a11y.close_comments')}
	transition:fade={{ duration: 150 }}
></button>
<aside
	transition:fly={{ x: 32, duration: 220, easing: cubicOut }}
	class="absolute inset-y-0 right-0 z-30 flex h-full w-80 max-w-[80vw] flex-col border-l bg-card/95 shadow-2xl {queueOpen
		? 'lg:right-80'
		: ''}"
>
	<h2 class="border-b px-4 py-3 font-heading text-sm font-semibold">{t('comments.title')}</h2>
	<CommentsView />
</aside>
