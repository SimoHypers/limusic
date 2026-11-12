// Self-check for `review.ts`: `node --experimental-strip-types ui/src/lib/review.check.ts`
import { ago, isGone, kindKey, restored, reviewErrorKey, waitingLabel, without } from './review.ts';
import type { LimusicQueueItem } from './api';

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
console.log('ok');
