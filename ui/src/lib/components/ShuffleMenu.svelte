<script lang="ts">
	import type { Snippet } from 'svelte';
	import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/svelte';
	import { ShuffleIcon, Tick02Icon } from '@hugeicons/core-free-icons';
	import { DynamicShuffleIcon } from '$lib/shuffleIcons';
	import { anchorMenu, fitMenu, NO_ANCHOR, toBody } from '$lib/menu';
	import { t } from '$lib/i18n.svelte';

	type Mode = 'on' | 'dynamic';

	// A playlist or album page's shuffle button. Where dynamic shuffle can draw on the page's radio
	// it opens a menu to pick the mode; where it can't, a one-item menu would be a click for
	// nothing, so the button shuffles straight away as it always did.
	let {
		dynamic,
		active = null,
		onpick,
		label,
		class: cls = '',
		disabled = false,
		children
	}: {
		/** Dynamic shuffle is on offer for this page. */
		dynamic: boolean;
		/** The mode the queue plays this page in, when it's the one playing: checked in the menu. */
		active?: 'off' | Mode | null;
		onpick: (mode: Mode) => void;
		label: string;
		class?: string;
		disabled?: boolean;
		children: Snippet;
	} = $props();

	let open = $state(false);
	let anchor = $state(NO_ANCHOR);
	let trigger = $state<HTMLButtonElement>();

	function click(e: MouseEvent) {
		if (!dynamic) return onpick('on');
		anchor = anchorMenu(e);
		open = true;
	}

	function close(refocus = false) {
		open = false;
		if (refocus) trigger?.focus();
	}

	// Focus goes back to the button first: a pick that changes nothing on screen (the mode that's
	// already on) would otherwise leave it on a menu item that no longer exists.
	function pick(mode: Mode) {
		close(true);
		onpick(mode);
	}

	// Keyboard: the first item takes focus on open, the arrows move between the two, Esc closes.
	// The items are out of the tab order, so Tab leaves the menu: it closes, and focus carries on
	// from the button in whichever direction Tab was going (the default action, from there).
	const focusFirst = (el: HTMLElement) => {
		el.querySelector<HTMLElement>('[role="menuitemradio"]')?.focus();
	};
	function onKey(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.preventDefault();
			close(true);
		} else if (e.key === 'Tab') {
			close(true);
		} else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			e.preventDefault();
			const items = [...(e.currentTarget as HTMLElement).querySelectorAll<HTMLElement>('[role="menuitemradio"]')];
			const at = items.indexOf(document.activeElement as HTMLElement);
			items[(at + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length]?.focus();
		}
	}

	const options: { mode: Mode; icon: IconSvgElement; title: string; hint: string }[] = $derived([
		{ mode: 'on', icon: ShuffleIcon, title: t('player.shuffle'), hint: t('player.shuffle_hint') },
		{
			mode: 'dynamic',
			icon: DynamicShuffleIcon,
			title: t('player.dynamic_shuffle'),
			hint: t('player.dynamic_shuffle_hint')
		}
	]);
</script>

<button
	bind:this={trigger}
	type="button"
	class={cls}
	onclick={click}
	{disabled}
	aria-label={label}
	title={label}
	aria-haspopup={dynamic ? 'menu' : undefined}
	aria-expanded={dynamic ? open : undefined}
>
	{@render children()}
</button>

{#if open}
	<!-- Moved to <body>, like the page's ⋯ menu: the header clips its overflow. -->
	<button
		class="fixed inset-0 z-40 cursor-default"
		onclick={() => close(true)}
		oncontextmenu={(e) => {
			e.preventDefault();
			close(true);
		}}
		aria-label={t('a11y.close_menu')}
		tabindex="-1"
		{@attach toBody}
	></button>
	<div
		role="menu"
		tabindex="-1"
		aria-label={label}
		class="fixed z-50 w-64 animate-in rounded-lg border bg-popover p-1 text-popover-foreground shadow-xl duration-150 fade-in-0 zoom-in-95"
		style={anchor.style}
		onkeydown={onKey}
		{@attach toBody}
		{@attach fitMenu(anchor)}
		{@attach focusFirst}
	>
		{#each options as o (o.mode)}
			{@const checked = active === o.mode}
			<button
				role="menuitemradio"
				tabindex="-1"
				aria-checked={checked}
				class="flex w-full cursor-pointer items-start gap-3 rounded-md px-2 py-2 text-left outline-none hover:bg-accent/10 focus-visible:bg-accent/10"
				onclick={() => pick(o.mode)}
			>
				<HugeiconsIcon icon={o.icon} class="mt-0.5 h-4 w-4 shrink-0 {checked ? 'text-primary' : ''}" />
				<span class="min-w-0 flex-1">
					<span class="block text-sm {checked ? 'font-medium text-primary' : ''}">{o.title}</span>
					<span class="block text-xs text-muted-foreground">{o.hint}</span>
				</span>
				{#if checked}
					<HugeiconsIcon icon={Tick02Icon} class="mt-0.5 h-4 w-4 shrink-0 text-primary" />
				{/if}
			</button>
		{/each}
	</div>
{/if}
