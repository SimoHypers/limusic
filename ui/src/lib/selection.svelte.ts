import { untrack } from 'svelte';
import type { SongItem } from './api';
import { emptySelection, reconcileSelection, toggleTrack, visibleTrackKeys,
	type TrackEntry } from './selection';

/** One list owns selection. Rows may unmount freely; navigation/account changes reset the scope. */
export function trackSelection(
	items: () => SongItem[], visible: () => SongItem[], scope: () => string,
	complete: () => boolean = () => true
) {
	let entries = $state.raw<TrackEntry[]>([]);
	let loadedKeys = $state.raw<ReadonlySet<string>>(new Set());
	let selected = $state.raw(emptySelection());
	let lost = $state(0);
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
	return {
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
		selectAll() {
			selected = { keys: new Set([...selected.keys, ...visibleKeys]), anchor: visibleKeys[0] ?? null };
		},
		clear() { selected = emptySelection(); lost = 0; }
	};
}

export type TrackSelection = ReturnType<typeof trackSelection>;
