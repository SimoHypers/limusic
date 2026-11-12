// Self-check for `review.ts`: `node --experimental-strip-types ui/src/lib/review.check.ts`
import { ago, coverOf, diffLyrics, isGone, kindKey, restored, reviewErrorKey, waitingLabel, without } from './review.ts';
import type { LimusicQueueItem, LyricLine } from './api';

function ok(cond: boolean, what: string) {
	if (!cond) throw new Error(`broken: ${what}`);
}
const row = (revision: number, kind = 'new'): LimusicQueueItem => ({
	revision,
	video_id: 'AAAAAAAAAAA',
	title: 't',
	artist: 'a',
	duration: null,
	author: null,
	kind,
	created_at: 0,
	reports: 0
});
const ids = (l: LimusicQueueItem[]) => l.map((i) => i.revision).join(',');

const list = [row(2), row(5), row(9)];
ok(ids(without(list, 5)) === '2,9', 'a decided row leaves');
ok(ids(without(list, 7)) === '2,5,9', 'an unknown row changes nothing');
ok(ids(list) === '2,5,9', 'the list itself is not changed');
ok(ids(restored(without(list, 5), row(5))) === '2,5,9', 'a refused row goes back in its place');
ok(ids(restored(without(list, 2), row(2))) === '2,5,9', 'the first');
ok(ids(restored(without(list, 9), row(9))) === '2,5,9', 'the last');
ok(ids(restored([], row(4))) === '4', 'into an empty list');
ok(restored(list, row(5)) === list, 'already there');
ok(waitingLabel(3, false) === '3' && waitingLabel(50, true) === '50+', 'the count label');
ok(kindKey('new') === 'review.kind_new' && kindKey('edit') === 'review.kind_edit', 'kinds');
ok(kindKey('reported') === 'review.kind_reported' && kindKey('whatever') === 'review.kind_edit', 'kinds, unknown');
for (const c of ['not_found', 'not_reviewable', 'stale']) ok(isGone(c), `${c} is gone`);
for (const c of ['forbidden', 'unavailable', 'rate_limited', 'unauthorized']) ok(!isGone(c), `${c} is not gone`);
ok(reviewErrorKey('stale') === 'review.err_gone', 'gone is worded');
const keys = ['unauthorized', 'forbidden', 'banned', 'rate_limited', 'invalid', 'unavailable', 'stale'].map(reviewErrorKey);
ok(new Set(keys).size === 7 && keys.every((k) => k.startsWith('review.')), 'every refusal has its own words');
ok(reviewErrorKey('anything else') === 'review.err_unavailable', 'unknown means unavailable');
ok(ago(1000, 1030, 'en') === 'now', 'seconds');
ok(ago(0, 5 * 60, 'en') === '5 minutes ago', 'minutes');
ok(ago(0, 3 * 3600, 'en') === '3 hours ago', 'hours');
ok(ago(0, 86400, 'en') === 'yesterday', 'a day');
ok(ago(0, 5 * 86400, 'en') === '5 days ago', 'days');
ok(ago(100, 50, 'en') === 'now', 'a clock that is behind never says the future');
ok(coverOf('AAAAAAAAAAA') === 'https://i.ytimg.com/vi/AAAAAAAAAAA/mqdefault.jpg', 'a cover from a video id');
ok(coverOf('not an id') === undefined && coverOf('') === undefined, 'nothing else makes a URL');

// Placeholder lines, not a real lyric.
const L = (text: string, at: number, shift = 0): LyricLine => ({
	text,
	time_ms: at + shift,
	end_time_ms: at + shift + 900,
	words: text.split(' ').map((t, i) => ({ text: t, start_ms: at + shift + i * 100, end_ms: at + shift + i * 100 + 90 }))
});
const live = [L('one two', 1000), L('three four', 2000), L('five six', 3000), L('seven eight', 4000)];
const kinds = (a: LyricLine[], b: LyricLine[]) => diffLyrics(a, b).map((c) => `${c.n}:${c.kind}`).join(' ');
ok(kinds(live, live) === '', 'the same lyric has no changes');
ok(kinds(live, [live[0], L('three for', 2000), live[2], live[3]]) === '2:reworded', 'one line reworded');
ok(diffLyrics(live, [live[0], L('three for', 2000), live[2], live[3]])[0].old === 'three four', 'a reworded line keeps what it was');
ok(kinds(live, [live[0], live[1], L('new line', 2500), live[2], live[3]]) === '3:added', 'an added line moves nothing after it');
ok(kinds(live, [live[0], live[2], live[3]]) === '2:removed', 'a removed line');
const later = diffLyrics(live, [live[0], L('three four', 2000, 180), live[2], live[3]]);
ok(later.length === 1 && later[0].kind === 'retimed' && later[0].shiftMs === 180, 'a line that starts later');
const inner = { ...live[2], words: live[2].words!.map((w, i) => (i ? { ...w, start_ms: w.start_ms + 60 } : w)) };
const moved = diffLyrics(live, [live[0], live[1], inner, live[3]]);
ok(moved.length === 1 && moved[0].kind === 'retimed' && moved[0].shiftMs === undefined, 'only the words moved');
ok(kinds(live, [live[0], L('three four', 2000, 20), live[2], live[3]]) === '', 'a few ms is not a change');
ok(kinds([], live) === '1:added 2:added 3:added 4:added' && kinds(live, []).split(' ').length === 4, 'against nothing');
console.log('ok');
