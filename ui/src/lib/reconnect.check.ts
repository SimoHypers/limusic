// Self-check for the retry ladder in `reconnect.ts`:
//
//     node --experimental-strip-types ui/src/lib/reconnect.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant. Not imported by the app.
// Timers and the reachability probe are stubbed so the ladder is stepped by hand.
export {};

const delays: number[] = [];
globalThis.setTimeout = ((_fn: () => void, ms: number) => delays.push(ms)) as never;
globalThis.clearTimeout = (() => {}) as never;
let now = 0;
Date.now = () => now;

const { whenBackOnline, _test } = await import('./reconnect.ts');
let up = false;
let probes = 0;
_test.probeWith(async () => (probes++, up));

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}

let calls = 0;
const cancel = whenBackOnline(() => calls++);
whenBackOnline(() => calls++);
ok(delays.length === 1 && delays[0] === 2_000, 'two callers share one timer on the first rung');

// Offline: the probe says no, nothing is retried, and the ladder climbs.
await _test.check();
ok(calls === 0 && _test.state().waiting === 2, 'nothing is retried while the network is down');
ok(delays.at(-1) === 4_000, 'a failed probe waits on the next rung');
for (let i = 0; i < 9; i++) await _test.check();
ok(probes === 10, 'one probe per rung, however many callers wait');
ok(!_test.state().pending && delays.length === 10, 'the clock gives up at the top');
await _test.check();
ok(probes === 11, 'focus and online still probe after that');

// Back online: everyone waiting runs once.
up = true;
await _test.check();
ok(calls === 2 && _test.state().waiting === 0, 'a probe that answers runs and clears everyone');

// Cancelled callers are not run.
calls = 0;
cancel();
const c2 = whenBackOnline(() => calls++);
c2();
await _test.check();
ok(calls === 0, 'a cancelled retry never runs');

// After a quiet spell the next outage starts from the bottom again.
now += 6 * 60_000;
whenBackOnline(() => {});
ok(delays.at(-1) === 2_000, 'a fresh outage starts on the first rung');

console.log('ok');
