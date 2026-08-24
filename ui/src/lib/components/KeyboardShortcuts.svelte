<script lang="ts">
	// Ctrl+H: what the keyboard can do. Nothing in the chrome points at the shortcuts, so this is
	// where they are discoverable. It documents the zoom keys too (zoom.ts owns those) — from the
	// outside they are the same feature, and a list that only covers half of them is worse than none.
	import * as Dialog from '$lib/components/ui/dialog';
	import { MOD } from '$lib/shortcuts';
	import { ui } from '$lib/player.svelte';
	import { t } from '$lib/i18n.svelte';

	// $derived, not a plain const: the list is rebuilt when the language changes under it.
	const GROUPS: { title: string; rows: [string, string][] }[] = $derived([
		{ title: t('dialogs.shortcuts.group_search'), rows: [[`${MOD}K`, t('dialogs.shortcuts.search_anywhere')]] },
		{
			title: t('dialogs.shortcuts.group_playback'),
			rows: [
				[`${MOD}E`, t('dialogs.shortcuts.toggle_now_playing')],
				[`${MOD}>`, t('dialogs.shortcuts.volume_up')],
				[`${MOD}<`, t('dialogs.shortcuts.volume_down')]
			]
		},
		{
			title: t('dialogs.shortcuts.group_window'),
			rows: [
				[`${MOD}+`, t('dialogs.shortcuts.zoom_in')],
				[`${MOD}-`, t('dialogs.shortcuts.zoom_out')],
				[`${MOD}0`, t('dialogs.shortcuts.reset_zoom')],
				[`${MOD}H`, t('dialogs.shortcuts.show_this_list')]
			]
		}
	]);
</script>

<Dialog.Root bind:open={ui.shortcutsOpen}>
	<Dialog.Content class="sm:max-w-md">
		<Dialog.Header>
			<Dialog.Title>{t('dialogs.shortcuts.title')}</Dialog.Title>
			<Dialog.Description>{t('dialogs.shortcuts.reopen_hint', { mod: MOD })}</Dialog.Description>
		</Dialog.Header>
		<div class="flex flex-col gap-5">
			{#each GROUPS as group (group.title)}
				<section>
					<h3 class="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
						{group.title}
					</h3>
					<dl class="flex flex-col gap-1.5">
						{#each group.rows as [keys, what] (keys)}
							<div class="flex items-center justify-between gap-4">
								<dt class="min-w-0 truncate text-sm">{what}</dt>
								<dd>
									<kbd
										class="rounded border bg-muted px-1.5 py-0.5 font-mono text-[0.6875rem] font-medium tracking-wide text-muted-foreground"
									>
										{keys}
									</kbd>
								</dd>
							</div>
						{/each}
					</dl>
				</section>
			{/each}
		</div>
	</Dialog.Content>
</Dialog.Root>
