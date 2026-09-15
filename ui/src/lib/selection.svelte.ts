import { tick, untrack } from 'svelte';
import type { SongItem } from './api';
import { emptySelection, reconcileSelection, toggleTrack, visibleTrackKeys,
	type TrackEntry } from './selection';

/** One list owns selection. Rows may unmount freely; navigation/account changes reset the scope. */
export function trackSelection(
	items: () => SongItem[], visible: () => SongItem[], scope: () => string,
	complete: () => boolean = () => true,
	/** The list's own length while pages are still missing, and the walk that fetches them. Only a
	 *  playlist has either: everywhere else the rows on screen are all the rows there are. */
	total: () => number | undefined = () => undefined,
	loadRest: () => Promise<boolean> = async () => true
) {
	let entries = $state.raw<TrackEntry[]>([]);
	let loadedKeys = $state.raw<ReadonlySet<string>>(new Set());
	let selected = $state.raw(emptySelection());
	let lost = $state(0);
	// Off by default: checkboxes and the bulk bar only exist once the list is put in select mode.
	let active = $state(false);
	let selectingAll = $state(false);
	// Bumped every time the selection is dropped out from under an in-flight selectAll: a walk that
	// comes back to a cleared list, another playlist, or a list no longer in select mode has nothing
	// left to select.
	let dropped = 0;
	let lastScope: string | undefined;
	let nextKey = 0;
	$effect(() => {
		const songs = items();
		const currentScope = scope();
		const allLoaded = complete();
		untrack(() => {
			if (lastScope !== currentScope) {
				entries = [];
				selected = emptySelection();
				lost = 0;
				active = false;
				dropped++;
				lastScope = currentScope;
			}
			// A server sort or cached-page refresh may replace a long list with its first page.
			// Missing selected occurrences are unresolved until the final page, not deleted.
			const next = reconcileSelection(songs, entries, selected, () => String(++nextKey), allLoaded);
			entries = next.entries;
			loadedKeys = next.loadedKeys;
			lost += selected.keys.size - next.selection.keys.size;
			selected = next.selection;
		});
	});
	const visibleKeys = $derived(visibleTrackKeys(entries, visible()));
	const songs = $derived(entries.filter((e) => selected.keys.has(e.key)).map((e) => e.song));
	const pending = $derived(entries.filter((e) => selected.keys.has(e.key) && !loadedKeys.has(e.key)).length);
	const hidden = $derived(selected.keys.size - pending - visibleKeys.filter((k) => selected.keys.has(k)).length);
	// What Select all is offering. Until the last page is in, the rows on screen are not the count
	// to put on the button, so the list's own total stands in.
	const selectAllCount = $derived(complete() ? visibleKeys.length : (total() ?? visibleKeys.length));
	// Never while pages are outstanding: every row on screen being ticked is not the whole list.
	const allSelected = $derived(
		complete() && visibleKeys.length > 0 && visibleKeys.every((k) => selected.keys.has(k))
	);
	return {
		get active() { return active; },
		enter() { active = true; },
		exit() { active = false; selected = emptySelection(); lost = 0; dropped++; },
		get count() { return selected.keys.size; },
		get songs() { return songs; },
		get visibleKeys() { return visibleKeys; },
		get hidden() { return hidden; },
		get pending() { return pending; },
		get lost() { return lost; },
		has(key: string | undefined) { return key !== undefined && selected.keys.has(key); },
		toggle(key: string, range = false) {
			selected = toggleTrack(selected, key, visibleKeys, range);
		},
		get selectAllCount() { return selectAllCount; },
		get allSelected() { return allSelected; },
		get selectingAll() { return selectingAll; },
		/**
		 * Select every row, pulling in the pages still missing first. Hidden selections survive it,
		 * and the first visible key becomes the anchor (the anchor clears if none are visible).
		 * A walk that gives up short selects what did arrive rather than nothing.
		 */
		async selectAll() {
			if (selectingAll) return;
			const at = dropped;
			selectingAll = true;
			try {
				if (!complete()) {
					await loadRest();
					// `entries` is written by the effect above, not derived, so the new pages are not
					// in `visibleKeys` until it has run.
					await tick();
				}
				// Cleared, navigated, switched account, or left select mode while the pages were in
				// the air. Whichever it was is the newer instruction, so it wins.
				if (dropped !== at) return;
				selected = { keys: new Set([...selected.keys, ...visibleKeys]), anchor: visibleKeys[0] ?? null };
			} finally {
				selectingAll = false;
			}
		},
		clear() { selected = emptySelection(); lost = 0; dropped++; }
	};
}

export type TrackSelection = ReturnType<typeof trackSelection>;
