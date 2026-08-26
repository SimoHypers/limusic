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
		{
			title: t('dialogs.shortcuts.group_playback'),
			rows: [
				[t('dialogs.shortcuts.play_pause'), 'SPACE or ;'],
				[t('dialogs.shortcuts.next_song'), `${MOD}F`],
				[t('dialogs.shortcuts.previous_song'), `${MOD}D`],
				[t('dialogs.shortcuts.shuffle_queue'), `${MOD}S`],
				[t('dialogs.shortcuts.toggle_repeat'), `${MOD}R`],
				[t('dialogs.shortcuts.mute_unmute'), `${MOD}M`],
				[t('dialogs.shortcuts.volume_up'), `${MOD}>`],
				[t('dialogs.shortcuts.volume_down'), `${MOD}<`]
			]
		},
		{
			title: t('dialogs.shortcuts.group_general'),
			rows: [
				[t('dialogs.shortcuts.search_anywhere'), `${MOD}K`],
				[t('dialogs.shortcuts.toggle_now_playing'), `${MOD}E`],
				[t('dialogs.shortcuts.zoom_in'), `${MOD}+`],
				[t('dialogs.shortcuts.zoom_out'), `${MOD}-`],
				[t('dialogs.shortcuts.reset_zoom'), `${MOD}0`],
				[t('dialogs.shortcuts.show_this_list'), `${MOD}H`]
			]
		}
	]);
</script>

<Dialog.Root bind:open={ui.shortcutsOpen}>
	<Dialog.Content class="sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>{t('dialogs.shortcuts.title')}</Dialog.Title>
			<Dialog.Description>{t('dialogs.shortcuts.reopen_hint', { mod: MOD })}</Dialog.Description>
		</Dialog.Header>
		<!-- Two columns that flow, so adding a row never means rebalancing the layout by hand. -->
		<div class="gap-x-10 sm:columns-2">
			{#each GROUPS as group (group.title)}
				<section class="mb-6 break-inside-avoid">
					<h3 class="mb-2 text-base font-semibold">{group.title}</h3>
					<dl>
						{#each group.rows as [what, keys] (what)}
							<div class="grid grid-cols-2 items-center gap-4 border-b py-2 last:border-0">
								<dt class="text-sm text-muted-foreground">{what}</dt>
								<dd class="font-mono text-xs font-medium">{keys}</dd>
							</div>
						{/each}
					</dl>
				</section>
			{/each}
		</div>
	</Dialog.Content>
</Dialog.Root>
