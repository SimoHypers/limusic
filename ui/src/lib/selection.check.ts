// node --experimental-strip-types ui/src/lib/selection.check.ts
import type { SongItem } from './api.ts';
import { emptySelection, reconcileSelection, reconcileTracks, retainSelection, toggleTrack, visibleTrackKeys } from './selection.ts';

function ok(value: boolean, message: string) {
	if (!value) throw new Error(message);
}
const song = (id: string, occurrence?: string): SongItem => ({
	video_id: id, title: id, artists: 'Artist', set_video_id: occurrence
});
let seq = 0;
const key = () => String(++seq);
const a = song('same', 'first');
const b = song('other');
const c = song('same', 'second');
const d = song('last');
let entries = reconcileTracks([a, b, c, d], [], key);
const originalKeys = entries.map((e) => e.key);
let state = toggleTrack(emptySelection(), originalKeys[2], originalKeys);
ok(state.keys.size === 1 && !state.keys.has(originalKeys[0]), 'Duplicate occurrences select independently');
state = toggleTrack(state, originalKeys[0], originalKeys);
ok(state.keys.size === 2, 'Both copies can be selected');

// Bulk actions read the source order, even when selected in reverse order.
ok(entries.filter((e) => state.keys.has(e.key)).map((e) => e.song.set_video_id).join(',') === 'first,second',
	'Queue order follows the list, not click order');

// Filtering and virtualization only affect projection, never selection retention.
const visible = visibleTrackKeys(entries, [b, c]);
ok(visible.join(',') === [originalKeys[1], originalKeys[2]].join(','), 'Filtered keys address source occurrences');
state = retainSelection(state, entries);
ok(state.keys.size === 2, 'Hidden and unmounted rows remain selected');
state = toggleTrack(state, originalKeys[1], visible, true);
ok(state.keys.size === 3, 'A hidden range anchor does not select filtered-out rows');

// Shift ranges span the complete matching model, not the rendered viewport.
const many = Array.from({ length: 5000 }, (_, i) => song(`track-${i}`));
const longEntries = reconcileTracks(many, [], key);
const longKeys = longEntries.map((e) => e.key);
let range = toggleTrack(emptySelection(), longKeys[12], longKeys);
range = toggleTrack(range, longKeys[4900], longKeys, true);
ok(range.keys.size === 4889, 'Shift range includes unmounted rows across a large list');
range = toggleTrack(range, longKeys[4900], longKeys);
ok(range.keys.size === 4888, 'Toggling removes exactly one occurrence');

let backward = toggleTrack(emptySelection(), longKeys[20], longKeys);
backward = toggleTrack(backward, longKeys[5], longKeys, true);
ok(backward.keys.size === 16 && backward.keys.has(longKeys[5]) && backward.keys.has(longKeys[20]) &&
	!backward.keys.has(longKeys[4]) && !backward.keys.has(longKeys[21]),
	'Backward Shift range includes both endpoints and excludes neighboring rows');

// Sorting and appending pages preserve existing keys; new rows are not auto-selected.
entries = reconcileTracks([d, c, b, a, song('new-page')], entries, key);
ok(entries.slice(0, 4).map((e) => e.key).join(',') === originalKeys.slice().reverse().join(','), 'Sort keeps keys');
state = retainSelection(state, entries);
ok(state.keys.size === 3 && !state.keys.has(entries[4].key), 'Continuation does not change selection');

// A fresh response with explicit occurrence IDs retains the correct duplicate, not its position.
const refreshed = reconcileTracks([song('same', 'second'), song('same', 'first'), song('other')], entries, key);
ok(refreshed[0].key === originalKeys[2] && refreshed[1].key === originalKeys[0], 'Fresh playlist IDs match exact copies');
ok(refreshed[2].key === originalKeys[1], 'A unique song survives object replacement');
const removed = retainSelection(state, refreshed.slice(1));
ok(!removed.keys.has(originalKeys[2]) && removed.keys.size === 2, 'Removed occurrence is pruned alone');

// With no occurrence IDs, identical refreshes are ambiguous; retaining by index would select the wrong copy.
const twins = reconcileTracks([song('duplicate'), song('duplicate')], [], key);
const ambiguous = reconcileTracks([song('duplicate'), song('duplicate')], twins, key);
const oneTwin = toggleTrack(emptySelection(), twins[0].key, twins.map((e) => e.key));
ok(retainSelection(oneTwin, ambiguous).keys.size === 0, 'Ambiguous refreshed duplicates clear safely');
const sameTwins = reconcileTracks([twins[1].song, twins[0].song], twins, key);
ok(sameTwins[0].key === twins[1].key, 'Duplicate objects keep identity through a local sort');

const beforeBackfill = reconcileTracks([song('optimistic')], [], key);
const afterBackfill = reconcileTracks([song('optimistic', 'server-id')], beforeBackfill, key);
ok(afterBackfill[0].key === beforeBackfill[0].key, 'An unambiguous playlist ID backfill preserves selection');
const replaced = reconcileTracks([song('optimistic', 'different-occurrence')], afterBackfill, key);
ok(replaced[0].key !== afterBackfill[0].key, 'Different explicit occurrence IDs never match');

const repeated = song('same-object');
const repeatEntries = reconcileTracks([repeated, repeated], [], key);
ok(new Set(visibleTrackKeys(repeatEntries, [repeated, repeated])).size === 2, 'Repeated object references remain distinct');
ok(toggleTrack(emptySelection(), 'stale', originalKeys).keys.size === 0, 'Stale row events cannot select missing entries');
ok(retainSelection(state, []).anchor === null, 'Removing the anchor clears it');

// A server sort returns its first page before the selected occurrences' new positions are known.
const beforeSort = reconcileTracks([a, b, c], [], key);
const beforeKeys = beforeSort.map((e) => e.key);
let chosen = toggleTrack(emptySelection(), beforeKeys[0], beforeKeys);
chosen = toggleTrack(chosen, beforeKeys[2], beforeKeys);
const firstPage = reconcileSelection([song('other')], beforeSort, chosen, key, false);
ok(firstPage.selection.keys.size === 2, 'Partial server reload retains selections on unloaded pages');
ok([...chosen.keys].every((k) => !firstPage.loadedKeys.has(k)), 'Unresolved selections are identified');
const lastPage = reconcileSelection([song('other'), song('same', 'second'), song('same', 'first')],
	firstPage.entries, firstPage.selection, key, true);
ok(lastPage.entries.filter((e) => chosen.keys.has(e.key)).map((e) => e.song.set_video_id).join(',') === 'second,first',
	'Continuation resolves selected occurrences in their new server order');
const deleted = reconcileSelection([song('other')], firstPage.entries, chosen, key, true);
ok(deleted.selection.keys.size === 0, 'Only a completed reload prunes missing selections');
const cancelled = reconcileSelection([song('other')], firstPage.entries, emptySelection(), key, false);
ok(cancelled.entries.length === 1, 'Cleared selections do not retain orphan entries');
console.log('selection.check: ok');
