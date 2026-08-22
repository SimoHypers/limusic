<script lang="ts">
	// Arrange home: drag the sections into the order you want them, hide the ones you don't. Nothing
	// is written until Save, so dismissing the modal any other way (Esc, the overlay, the ✕) throws
	// the edit away — which is why the list below is a working copy and not `personal` itself.
	//
	// ponytail: drag is the only way to reorder, matching the Shortcuts grid. Hiding works from the
	// keyboard; wire arrow-key moves onto the rows if anyone asks.
	import { untrack } from 'svelte';
	import { flip } from 'svelte/animate';
	import { HugeiconsIcon } from '@hugeicons/svelte';
	import {
		Cancel02Icon,
		DragDropHorizontalIcon,
		SaveIcon,
		ViewIcon,
		ViewOffSlashIcon
	} from '@hugeicons/core-free-icons';
	import { Button } from '$lib/components/ui/button';
	import * as Dialog from '$lib/components/ui/dialog';
	import { personal, saveHomeLayout } from '$lib/player.svelte';
	import { hiddenSections } from '$lib/personal';
	import { t } from '$lib/i18n.svelte';

	let {
		open = $bindable(false),
		sections
	}: { open: boolean; sections: { key: string; title: string }[] } = $props();

	type Row = { key: string; title: string; shown: boolean };
	let rows = $state<Row[]>([]);
	// Index of the row being dragged. It follows the row as the list rearranges under it, so the
	// pointer keeps hold of the same section rather than of whatever slid into that slot.
	let dragging = $state<number | null>(null);

	// Snapshotted when the modal opens, not derived: the feed keeps appending shelves as the page
	// behind loads, and a list that grows mid-drag reshuffles under the cursor. Hence `untrack` —
	// `open` is the only thing that may re-run this, or the reset lands in the middle of an edit.
	$effect(() => {
		if (!open) return;
		untrack(() => {
			const hidden = hiddenSections(personal);
			rows = sections.map((s) => ({ ...s, shown: !hidden.has(s.key) }));
			dragging = null;
		});
	});

	/** Live reorder while dragging: the row moves as you pass over its neighbours, no drop marker. */
	function moveTo(to: number) {
		if (dragging === null || dragging === to) return;
		const next = rows.slice();
		next.splice(to, 0, ...next.splice(dragging, 1));
		rows = next;
		dragging = to;
	}

	function save() {
		saveHomeLayout(
			rows.map((r) => r.key),
			rows.filter((r) => !r.shown).map((r) => r.key)
		);
		open = false;
	}
</script>

<Dialog.Root bind:open>
	<Dialog.Content class="gap-0 overflow-hidden p-0 sm:max-w-md">
		<div class="border-b px-5 py-4">
			<Dialog.Title class="text-lg font-semibold">{t('home.edit_home')}</Dialog.Title>
			<Dialog.Description class="text-xs text-muted-foreground">
				{t('home.shortcuts_desc')}
			</Dialog.Description>
		</div>

		<div role="list" class="max-h-[24rem] min-h-[12rem] overflow-y-auto p-2">
			{#each rows as row, i (row.key)}
				<!-- The whole row is the drag source, not just the grip: aiming at a 1rem handle inside a
				     scrolling list is a chore, and the grip still says which part of it to grab. -->
				<div
					role="listitem"
					draggable="true"
					animate:flip={{ duration: 180 }}
					ondragstart={(e) => {
						dragging = i;
						e.dataTransfer?.setData('text/plain', row.key); // some engines refuse a payload-less drag
						if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
					}}
					ondragover={(e) => {
						if (dragging === null) return; // a file or a card from the page behind
						e.preventDefault();
						moveTo(i);
					}}
					ondragend={() => (dragging = null)}
					class="flex cursor-grab items-center gap-2 rounded-lg py-2 pl-3 pr-2 transition-colors hover:bg-muted/50 {dragging ===
					i
						? 'bg-muted opacity-50'
						: ''}"
				>
					<span class="min-w-0 flex-1 truncate text-sm {row.shown ? '' : 'text-muted-foreground'}">
						{row.title}
					</span>
					<button
						onclick={() => (row.shown = !row.shown)}
						title={row.shown ? t('common.remove') : t('common.add')}
						aria-label={row.shown ? `Hide ${row.title}` : `Show ${row.title}`}
						class="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
					>
						<!-- altIcon/showAlt, not a ternary on `icon`: that prop is read once at mount. -->
						<HugeiconsIcon
							icon={ViewIcon}
							altIcon={ViewOffSlashIcon}
							showAlt={!row.shown}
							class="h-4 w-4"
						/>
					</button>
					<span
						aria-hidden="true"
						class="flex h-8 w-8 shrink-0 items-center justify-center text-muted-foreground/60"
					>
						<HugeiconsIcon icon={DragDropHorizontalIcon} class="h-4 w-4" />
					</span>
				</div>
			{/each}
		</div>

		<div class="flex justify-end gap-2 border-t px-5 py-3">
			<Button variant="outline" size="sm" onclick={() => (open = false)}>
				<HugeiconsIcon icon={Cancel02Icon} class="h-4 w-4" />
				{t('common.cancel')}
			</Button>
			<Button size="sm" onclick={save}>
				<HugeiconsIcon icon={SaveIcon} class="h-4 w-4" />
				{t('common.save')}
			</Button>
		</div>
	</Dialog.Content>
</Dialog.Root>
