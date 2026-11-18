// Self-check for the cast picker's logic (`cast.ts`). Same deal as `queue.check.ts`: no test
// runner in `ui/`, node 22 runs TypeScript directly:
//
//     node --experimental-strip-types ui/src/lib/cast.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant.
import type { CastDevice } from './api.ts';
import { deviceSubtitle, isPlaying } from './cast.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

const device = (id: string, kind: string, model: string | null): CastDevice => ({
	id,
	name: id,
	kind,
	model,
	host: '192.168.1.5',
	port: 8009
});

// --- what a device is called ------------------------------------------------------------------
// A speaker that names its model says it; one that does not says what it is. A renderer on the
// network is not a Cast device, and the picker must not call it one.
ok(
	deviceSubtitle(device('a', 'chromecast', 'Google Nest Mini'), 'Google Cast', 'DLNA renderer') ===
		'Google Nest Mini',
	'a model is shown when the device reports one'
);
ok(
	deviceSubtitle(device('a', 'chromecast', null), 'Google Cast', 'DLNA renderer') === 'Google Cast',
	'a Cast device without a model is named by its kind'
);
ok(
	deviceSubtitle(device('a', 'dlna', null), 'Google Cast', 'DLNA renderer') === 'DLNA renderer',
	'a renderer is named by its kind'
);

// --- which device is playing ------------------------------------------------------------------
ok(isPlaying(device('a', 'chromecast', null), 'a'), 'the device that is playing is the one named');
ok(!isPlaying(device('b', 'chromecast', null), 'a'), 'another device is not the one playing');
ok(!isPlaying(device('a', 'chromecast', null), null), 'nothing is playing when no device is');

console.log('ok');
