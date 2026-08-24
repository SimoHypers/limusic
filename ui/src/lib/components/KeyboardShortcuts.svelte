<script lang="ts">
	// Ctrl+H: what the keyboard can do. Nothing in the chrome points at the shortcuts, so this is
	// where they are discoverable. It documents the zoom keys too (zoom.ts owns those) — from the
	// outside they are the same feature, and a list that only covers half of them is worse than none.
	import * as Dialog from '$lib/components/ui/dialog';
	import { MOD } from '$lib/shortcuts';
	import { ui } from '$lib/player.svelte';

	const GROUPS: { title: string; rows: [string, string][] }[] = [
		{
			title: 'Playback',
			rows: [
				['Play or pause', 'SPACE or ;'],
				['Next song', `${MOD}F`],
				['Previous song', `${MOD}D`],
				['Shuffle queue', `${MOD}S`],
				['Toggle repeat', `${MOD}R`],
				['Mute or unmute', `${MOD}M`],
				['Volume up', `${MOD}>`],
				['Volume down', `${MOD}<`]
			]
		},
		{
			title: 'General',
			rows: [
				['Search from anywhere', `${MOD}K`],
				['Toggle the now-playing view', `${MOD}E`],
				['Zoom in', `${MOD}+`],
				['Zoom out', `${MOD}-`],
				['Reset zoom', `${MOD}0`],
				['Show this list', `${MOD}H`]
			]
		}
	];
</script>

<Dialog.Root bind:open={ui.shortcutsOpen}>
	<Dialog.Content class="sm:max-w-2xl">
		<Dialog.Header>
			<Dialog.Title>Keyboard shortcuts</Dialog.Title>
			<Dialog.Description>{MOD}H brings this back at any time.</Dialog.Description>
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
