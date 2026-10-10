<script lang="ts">
	import { Button } from '$lib/components/ui/button';
	import { Textarea } from '$lib/components/ui/textarea';
	import { t } from '$lib/i18n.svelte';

	// One text box with a send button, for the comment box, a reply and an edit. It owns no state
	// beyond the text: the panel decides what sending means, and a failed send leaves the text here.
	let {
		value = $bindable(''),
		placeholder,
		label,
		submitLabel,
		pending = false,
		uncertain = false,
		focus = false,
		onsubmit,
		oncancel,
		onreload
	}: {
		value?: string;
		placeholder: string;
		/** The box's accessible name: a placeholder is not one. */
		label: string;
		submitLabel: string;
		pending?: boolean;
		/** The last send may have gone through and its answer was lost. */
		uncertain?: boolean;
		/** Take the keyboard focus when it appears (an inline reply or edit). */
		focus?: boolean;
		onsubmit: () => void;
		oncancel?: () => void;
		onreload?: () => void;
	} = $props();

	let ref = $state<HTMLTextAreaElement | null>(null);
	$effect(() => {
		if (focus && ref) ref.focus();
	});

	const empty = $derived(value.trim().length === 0);

	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
			e.preventDefault();
			if (!empty && !pending) onsubmit();
		} else if (e.key === 'Escape' && oncancel && !pending) {
			e.preventDefault();
			e.stopPropagation();
			oncancel();
		}
	}
</script>

<div class="flex flex-col gap-2">
	<Textarea
		bind:ref
		bind:value
		{placeholder}
		aria-label={label}
		rows={3}
		disabled={pending}
		class="min-h-16 text-sm"
		{onkeydown}
	/>
	{#if uncertain}
		<p role="status" class="text-xs text-muted-foreground">{t('comments.write_uncertain')}</p>
	{/if}
	<div class="flex items-center justify-end gap-2">
		{#if uncertain && onreload}
			<Button variant="secondary" size="sm" disabled={pending} onclick={onreload}>
				{t('comments.reload')}
			</Button>
		{/if}
		{#if oncancel}
			<Button variant="ghost" size="sm" disabled={pending} onclick={oncancel}>
				{t('common.cancel')}
			</Button>
		{/if}
		<Button size="sm" disabled={empty || pending} onclick={onsubmit}>
			{pending ? t('comments.sending') : submitLabel}
		</Button>
	</div>
</div>
