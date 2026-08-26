<script lang="ts">
	import { fade, fly } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { beforeNavigate } from '$app/navigation';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { Maximize01Icon, Minimize01Icon } from '@hugeicons/core-free-icons';
	import LyricsView from './LyricsView.svelte';
	import { ui } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	let { onClose, queueOpen = false }: { onClose: () => void; queueOpen?: boolean } = $props();

	let expanded = $state(false);

	// Expanded, the panel covers the page — so navigating anywhere means the user wants to see that
	// page, not the lyrics. The docked panel sits beside the content, so it stays put.
	// beforeNavigate (not a pathname effect) so clicking the tab you're already on also closes it.
	beforeNavigate(() => {
		if (expanded) onClose();
	});
</script>

<!-- Same overlay pattern as QueuePanel: always over the content, with a dismiss scrim below lg. When
     the queue is open too, this one steps left of it at lg+; narrower than that they stack. -->
<button
	class="absolute inset-0 z-20 cursor-default bg-black/40 lg:hidden"
	onclick={onClose}
	aria-label={t('a11y.close_lyrics')}
	transition:fade={{ duration: 150 }}
></button>
<aside
	transition:fly={{ x: 32, duration: 220, easing: cubicOut }}
	class={expanded
		? // ponytail: left offsets mirror Sidebar's w-16/lg:w-60 (and its manual collapse), right
			// offset mirrors QueuePanel's w-80 — keep in sync if those change.
			`absolute inset-y-0 left-16 right-0 z-30 flex h-full flex-col border-l bg-card shadow-2xl ${ui.sidebarCollapsed ? '' : 'lg:left-60'} ${queueOpen ? 'lg:right-80' : ''}`
		: `absolute inset-y-0 right-0 z-30 flex h-full w-80 max-w-[80vw] flex-col border-l bg-card shadow-2xl ${queueOpen ? 'lg:right-80' : ''}`}
>
	<div class="flex items-center justify-between border-b px-4 py-3">
		<h2 class="font-heading text-sm font-semibold">{t('lyrics.title')}</h2>
		<button
			onclick={() => (expanded = !expanded)}
			class="cursor-pointer text-muted-foreground transition-colors hover:text-foreground"
			aria-label={expanded ? t('a11y.shrink_lyrics') : t('a11y.expand_lyrics')}
		>
			<!-- icon swap via altIcon/showAlt: `icon` is frozen at mount -->
			<HugeiconsIcon
				icon={Maximize01Icon}
				altIcon={Minimize01Icon}
				showAlt={expanded}
				class="h-4 w-4"
			/>
		</button>
	</div>
	<LyricsView {expanded} />
</aside>
