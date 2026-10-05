<script lang="ts">
	// The shortcuts list, and the place they are edited: click a row's keys, press the combo you
	// want, done. It documents the zoom keys too (they are bound here and handled by
	// shortcuts.ts/zoom.svelte.ts) — from the outside they are the same feature, and a list that
	// only covers half of them is worse than none.
	import * as Dialog from '$lib/components/ui/dialog';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import { ArrowTurnBackwardIcon } from '@hugeicons/core-free-icons';
	import { ui } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';
	import { KEYBIND_ACTIONS } from '$lib/keybinddefs';
	import { keybinds } from '$lib/keybinds.svelte';
	import { comboFromEvent } from '$lib/keycombo';

	// $derived, not a plain const: the list is rebuilt when the language changes under it.
	const groups = $derived([
		{
			id: 'playback',
			title: t('dialogs.shortcuts.group_playback'),
			actions: KEYBIND_ACTIONS.filter((a) => a.group === 'playback')
		},
		{
			id: 'general',
			title: t('dialogs.shortcuts.group_general'),
			actions: KEYBIND_ACTIONS.filter((a) => a.group === 'general')
		}
	]);
	const anyChanged = $derived(KEYBIND_ACTIONS.some((a) => !keybinds.isDefault(a.id)));

	function record(e: KeyboardEvent) {
		if (!keybinds.recording) return;
		// Swallow the key entirely, so the dialog's own Escape handling and the app's shortcut
		// listener never see it while a bind is being recorded.
		e.preventDefault();
		e.stopPropagation();
		if (e.key === 'Escape' && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
			keybinds.recording = null;
			return;
		}
		const combo = comboFromEvent(e);
		if (!combo) return;
		const id = keybinds.recording;
		keybinds.recording = null;
		keybinds.set(id, combo);
	}

	// Capture phase: the recorder has to win before anything else reacts to the key.
	$effect(() => {
		if (!keybinds.recording) return;
		window.addEventListener('keydown', record, { capture: true });
		return () => window.removeEventListener('keydown', record, { capture: true });
	});

	// Closing the list (or unmounting) abandons a recording rather than leaving it armed.
	$effect(() => {
		if (!ui.shortcutsOpen) keybinds.recording = null;
	});
</script>

<Dialog.Root bind:open={ui.shortcutsOpen}>
	<Dialog.Content class="sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>{t('dialogs.shortcuts.title')}</Dialog.Title>
			<Dialog.Description>
				{t('dialogs.shortcuts.reopen_hint', { key: keybinds.label('show_list') })}
			</Dialog.Description>
		</Dialog.Header>
		<p class="mb-3 text-xs text-muted-foreground">{t('dialogs.shortcuts.edit_hint')}</p>
		<!-- Two columns that flow, so adding a row never means rebalancing the layout by hand. -->
		<div class="gap-x-10 sm:columns-2">
			{#each groups as group (group.id)}
				<section class="mb-6 break-inside-avoid">
					<h3 class="mb-2 text-base font-semibold">{group.title}</h3>
					<dl>
						{#each group.actions as a (a.id)}
							{@const recording = keybinds.recording === a.id}
							<div class="grid grid-cols-2 items-center gap-4 border-b py-2 last:border-0">
								<dt class="text-sm text-muted-foreground">{t(a.titleKey)}</dt>
								<dd class="flex min-w-0 items-center gap-1">
									<button
										type="button"
										class="-mx-2 flex min-w-0 cursor-pointer items-center rounded-md px-2 py-1 text-left transition-colors hover:bg-muted {recording
											? 'bg-primary/10 ring-1 ring-primary'
											: ''}"
										onclick={() => (keybinds.recording = a.id)}
										aria-label={t('dialogs.shortcuts.rebind', { action: t(a.titleKey) })}
									>
										<span
											class="truncate font-mono text-xs font-medium {recording
												? 'text-primary'
												: ''}"
										>
											{recording ? t('dialogs.shortcuts.recording') : keybinds.label(a.id)}
										</span>
									</button>
									{#if !keybinds.isDefault(a.id)}
										<button
											type="button"
											class="flex h-6 w-6 shrink-0 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
											onclick={() => keybinds.reset(a.id)}
											aria-label={t('dialogs.shortcuts.reset_action')}
										>
											<HugeiconsIcon icon={ArrowTurnBackwardIcon} class="h-3.5 w-3.5" />
										</button>
									{/if}
								</dd>
							</div>
						{/each}
					</dl>
				</section>
			{/each}
		</div>
		<div class="flex justify-end">
			<button
				type="button"
				class="cursor-pointer rounded-md px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:cursor-default disabled:opacity-50"
				disabled={!anyChanged}
				onclick={() => keybinds.resetAll()}
			>
				{t('dialogs.shortcuts.reset_all')}
			</button>
		</div>
	</Dialog.Content>
</Dialog.Root>
