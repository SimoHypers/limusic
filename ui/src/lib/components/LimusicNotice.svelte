<script lang="ts">
	// One message from Limusic Lyrics or about a draft: what happened, one sentence, and the button
	// that helps. The editor, its send dialog and the review page all say things this way.
	import type { Snippet } from 'svelte';
	import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/svelte';

	let {
		icon,
		tone = 'info',
		title,
		body,
		lead,
		actions,
		class: cls = ''
	}: {
		icon?: IconSvgElement;
		tone?: 'info' | 'error' | 'success';
		title: string;
		body?: string;
		/** In place of the icon: a cover, say. */
		lead?: Snippet;
		actions?: Snippet;
		class?: string;
	} = $props();
</script>

<div
	role="status"
	class="flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border bg-card px-3.5 py-3 {tone === 'error'
		? 'border-destructive/35'
		: ''} {cls}"
>
	{#if lead}
		{@render lead()}
	{:else if icon}
		<span
			class="flex size-8 shrink-0 items-center justify-center rounded-[9px] {tone === 'error'
				? 'bg-destructive/15 text-destructive'
				: tone === 'success'
					? 'bg-primary text-primary-foreground'
					: 'bg-muted text-foreground'}"
		>
			<HugeiconsIcon {icon} class="size-4" />
		</span>
	{/if}
	<div class="min-w-40 flex-1">
		<p class="text-sm font-semibold">{title}</p>
		{#if body}<p class="text-[13px] leading-relaxed text-muted-foreground">{body}</p>{/if}
	</div>
	{#if actions}
		<div class="flex flex-wrap items-center gap-2">{@render actions()}</div>
	{/if}
</div>
