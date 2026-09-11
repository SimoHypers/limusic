import type { SongItem } from './api';

export interface TrackEntry {
	key: string;
	song: SongItem;
}

/** Reconcile occurrences, never just video IDs. Ambiguous refreshed duplicates get new keys. */
export function reconcileTracks(
	songs: SongItem[],
	previous: TrackEntry[],
	newKey: () => string
): TrackEntry[] {
	const byOccurrence = new Map<string, TrackEntry[]>();
	const byObject = new Map<SongItem, TrackEntry[]>();
	const byVideo = new Map<string, TrackEntry[]>();
	const counts = new Map<string, number>();
	const occurrence = (s: SongItem) => s.set_video_id ? JSON.stringify([s.video_id, s.set_video_id]) : '';
	const add = <K>(map: Map<K, TrackEntry[]>, key: K, entry: TrackEntry) => {
		const bucket = map.get(key);
		if (bucket) bucket.push(entry);
		else map.set(key, [entry]);
	};
	for (const entry of previous) {
		if (occurrence(entry.song)) add(byOccurrence, occurrence(entry.song), entry);
		add(byObject, entry.song, entry);
		add(byVideo, entry.song.video_id, entry);
	}
	for (const song of songs) counts.set(song.video_id, (counts.get(song.video_id) ?? 0) + 1);
	const used = new Set<string>();
	return songs.map((song) => {
		const exact = byOccurrence.get(occurrence(song))?.find((e) => !used.has(e.key));
		const sameObject = byObject.get(song)?.find((e) => !used.has(e.key));
		const candidates = byVideo.get(song.video_id) ?? [];
		// A unique track may have been re-fetched or acquired its playlist ID after an add.
		// Two different explicit playlist IDs always denote different occurrences.
		const unique = counts.get(song.video_id) === 1 && candidates.length === 1 &&
			!(song.set_video_id && candidates[0].song.set_video_id &&
				song.set_video_id !== candidates[0].song.set_video_id) ? candidates[0] : undefined;
		const old = exact ?? sameObject ?? unique;
		const key = old && !used.has(old.key) ? old.key : newKey();
		used.add(key);
		return { key, song };
	});
}

/** Filtering/sorting keeps the source objects. Buckets also handle repeated object references. */
export function visibleTrackKeys(entries: TrackEntry[], visible: SongItem[]): string[] {
	const buckets = new Map<SongItem, string[]>();
	for (const { key, song } of entries) {
		const bucket = buckets.get(song);
		if (bucket) bucket.push(key);
		else buckets.set(song, [key]);
	}
	return visible.flatMap((song) => {
		const key = buckets.get(song)?.shift();
		return key === undefined ? [] : [key];
	});
}

export interface SelectionState {
	keys: ReadonlySet<string>;
	anchor: string | null;
}

export const emptySelection = (): SelectionState => ({ keys: new Set(), anchor: null });

export function toggleTrack(
	state: SelectionState, key: string, visible: string[], range = false
): SelectionState {
	if (!visible.includes(key)) return state;
	const keys = new Set(state.keys);
	const from = state.anchor === null ? -1 : visible.indexOf(state.anchor);
	if (range && from >= 0) {
		const to = visible.indexOf(key);
		for (const k of visible.slice(Math.min(from, to), Math.max(from, to) + 1)) keys.add(k);
		return { keys, anchor: state.anchor };
	}
	if (range || !keys.delete(key)) keys.add(key);
	return { keys, anchor: key };
}

export function retainSelection(state: SelectionState, entries: TrackEntry[]): SelectionState {
	const available = new Set(entries.map((e) => e.key));
	return {
		keys: new Set([...state.keys].filter((key) => available.has(key))),
		anchor: state.anchor !== null && available.has(state.anchor) ? state.anchor : null
	};
}

/** A partial server reload cannot prove that a selected occurrence was removed. */
export function reconcileSelection(
	songs: SongItem[], previous: TrackEntry[], state: SelectionState,
	newKey: () => string, complete = true
) {
	const loaded = reconcileTracks(songs, previous, newKey);
	const loadedKeys = new Set(loaded.map((e) => e.key));
	const waiting = complete ? [] : previous.filter((e) =>
		state.keys.has(e.key) && !loadedKeys.has(e.key));
	const entries = [...loaded, ...waiting];
	return { entries, loadedKeys, selection: retainSelection(state, entries) };
}
