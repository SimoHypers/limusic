<script lang="ts">
	import { slide } from 'svelte/transition';
	import { cubicOut } from 'svelte/easing';
	import { MediaQuery } from 'svelte/reactivity';
	import { Button } from '$lib/components/ui/button';
	import { Textarea } from '$lib/components/ui/textarea';
	import { Avatar, AvatarFallback, AvatarImage } from '$lib/components/ui/avatar';
	import { auth } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import { thumb } from '$lib/thumb';

	// One text box with a send button, for the comment box, a reply and an edit. It owns no state
	// beyond the text: the panel decides what sending means, and a failed send leaves the text here.
	let {
		value = $bindable(''),
		placeholder,
		label,
		submitLabel,
		pending = false,
		focus = false,
		avatar,
		collapsible = false,
		onsubmit,
		oncancel
	}: {
		value?: string;
		placeholder: string;
		/** The box's accessible name: a placeholder is not one. */
		label: string;
		submitLabel: string;
		pending?: boolean;
		/** Take the keyboard focus when it appears (an inline reply or edit). */
		focus?: boolean;
		/** The signed-in account's picture beside the box, so it's clear who is writing. Sized like
		 *  the rows the comment will join; none for an edit, which sits beside its own avatar. */
		avatar?: 'size-6' | 'size-8';
		/** A one-line box with no buttons until it is focused, and again once it is left empty: the
		 *  panel's top box, which would otherwise hold its full height over the list. */
		collapsible?: boolean;
		onsubmit: () => void;
		oncancel?: () => void;
	} = $props();

	let ref = $state<HTMLTextAreaElement | null>(null);
	$effect(() => {
		if (focus && ref) ref.focus();
	});

	const empty = $derived(value.trim().length === 0);

	// Text or a send in flight keeps it open, so the send button is never pulled out from under a
	// click that blurs the box.
	let focused = $state(false);
	const open = $derived(!collapsible || focused || !empty || pending);
	// The global reduced-motion rule (layout.css) covers CSS transitions, not Svelte's.
	const reducedMotion = new MediaQuery('(prefers-reduced-motion: reduce)');

	function submit() {
		// The box is disabled while sending, which drops focus without always firing blur: once the
		// send clears the text, it closes.
		focused = false;
		onsubmit();
	}

	function onkeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
			e.preventDefault();
			if (!empty && !pending) submit();
		} else if (e.key === 'Escape' && oncancel && !pending) {
			e.preventDefault();
			e.stopPropagation();
			oncancel();
		}
	}
</script>

<div class="flex gap-3">
	{#if avatar}
		{@const account = auth.account}
		<!-- after:hidden matches CommentRow's avatars. -->
		<Avatar class="{avatar} after:hidden" title={account?.name ?? undefined}>
			{#if account?.thumbnail}<AvatarImage src={thumb(account.thumbnail, 64)} alt="" />{/if}
			<AvatarFallback class="text-xs">{account?.name?.slice(0, 1).toUpperCase()}</AvatarFallback>
		</Avatar>
	{/if}
	<div class="flex min-w-0 flex-1 flex-col">
		<!-- The box grows by its min-height and padding (field-sizing-content sizes it from those);
		     `rows` only matters in an engine without field-sizing. One element, once per focus. -->
		<Textarea
			bind:ref
			bind:value
			{placeholder}
			aria-label={label}
			rows={open ? 3 : 1}
			disabled={pending}
			class="text-sm transition-[min-height,padding,color,background-color,border-color,box-shadow] duration-200 ease-out {open
				? 'min-h-16'
				: 'min-h-9 py-2'}"
			onfocus={() => (focused = true)}
			onblur={() => (focused = false)}
			{onkeydown}
		/>
		{#if open}
			<div
				class="flex items-center justify-end gap-2 pt-2"
				transition:slide={{ duration: reducedMotion.current ? 0 : 200, easing: cubicOut }}
			>
				{#if oncancel}
					<Button variant="ghost" size="sm" disabled={pending} onclick={oncancel}>
						{t('common.cancel')}
					</Button>
				{/if}
				<Button size="sm" disabled={empty || pending} onclick={submit}>
					{pending ? t('comments.sending') : submitLabel}
				</Button>
			</div>
		{/if}
	</div>
</div>
