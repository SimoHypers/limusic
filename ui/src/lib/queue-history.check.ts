// Queue history scroll anchoring. No DOM or test runner required:
// node --experimental-strip-types ui/src/lib/queue-history.check.ts
import { keepQueueAnchor } from './queue-history.ts';

let assertions = 0;
function ok(condition: boolean, message: string): void {
	assertions++;
	if (!condition) throw new Error(`FAIL: ${message}`);
}

function fixture(scrollTop = 120, contentTop = 120, viewportTop = 30, targetOffset?: () => number) {
	let position = scrollTop;
	let writes = 0;
	let connected = true;
	let headingConnected = true;
	let current = true;
	let resolve!: () => void;
	const rendered = new Promise<void>((done) => { resolve = done; });
	const scroller = {
		get isConnected() { return connected; },
		get scrollTop() { return position; },
		set scrollTop(next: number) { position = Math.max(0, next); writes++; },
		getBoundingClientRect: () => ({ top: viewportTop })
	};
	const heading = {
		get isConnected() { return headingConnected; },
		getBoundingClientRect: () => ({ top: viewportTop + contentTop - position })
	};
	const cancel = keepQueueAnchor(scroller, heading, rendered, () => current, targetOffset);
	return {
		scroller, heading, cancel,
		get writes() { return writes; },
		moveContent: (delta: number) => { contentTop += delta; },
		moveViewport: (delta: number) => { viewportTop += delta; },
		nativeAnchor: (delta: number) => { position = Math.max(0, position + delta); },
		disconnect: () => { connected = false; },
		disconnectHeading: () => { headingConnected = false; },
		supersede: () => { current = false; },
		flush: async () => { resolve(); await rendered; }
	};
}

let f = fixture();
f.moveContent(720);
ok(f.writes === 0, 'wait for rendering before correcting');
await f.flush();
ok(f.scroller.scrollTop === 840, 'expanding history compensates its height');
ok(f.heading.getBoundingClientRect().top === 30, 'current heading stays in place');
ok(f.writes === 1, 'one correction per render');
await f.flush();
ok(f.writes === 1, 'settled update cannot correct again');

f = fixture(840, 840);
f.moveContent(-720);
await f.flush();
ok(f.scroller.scrollTop === 120, 'collapsing history compensates its height');

f = fixture();
f.moveContent(720);
f.nativeAnchor(720);
await f.flush();
ok(f.scroller.scrollTop === 840, 'native scroll anchoring is not applied twice');

f = fixture();
f.moveContent(720);
f.nativeAnchor(400);
await f.flush();
ok(f.scroller.scrollTop === 840, 'only remaining displacement is corrected');

f = fixture(100, 250);
f.moveContent(500);
f.moveViewport(75);
await f.flush();
ok(f.scroller.scrollTop === 600, 'panel movement is not mistaken for history height');
ok(f.heading.getBoundingClientRect().top - f.scroller.getBoundingClientRect().top === 150,
	'a heading below the viewport top keeps its relative position');

f = fixture(100, 50);
f.moveContent(500);
await f.flush();
ok(f.scroller.scrollTop === 600, 'a heading above the viewport is also anchored');

for (const stop of ['cancel', 'disconnect', 'disconnectHeading', 'supersede'] as const) {
	f = fixture();
	f.moveContent(500);
	f[stop]();
	await f.flush();
	ok(f.writes === 0, `${stop} invalidates the pending correction`);
}

// A newer effect can win while an older render callback is still pending.
const old = fixture();
const next = fixture();
old.moveContent(720);
old.cancel();
next.moveContent(360);
await next.flush();
await old.flush();
ok(old.writes === 0 && next.scroller.scrollTop === 480, 'newer visibility change wins');

const sidebar = fixture(120, 120, 30);
const full = fixture(300, 420, 80);
sidebar.moveContent(720);
full.moveContent(1008);
await Promise.all([sidebar.flush(), full.flush()]);
ok(sidebar.scroller.scrollTop === 840, 'sidebar uses its own geometry');
ok(full.scroller.scrollTop === 1308, 'full view uses its own geometry');

f = fixture(100, 100);
await f.flush();
ok(f.scroller.scrollTop === 100, 'empty history causes no displacement');

f = fixture(100, 100);
f.moveContent(-300);
await f.flush();
ok(f.scroller.scrollTop === 0, 'scroll position clamps at the start of the list');

// Large virtualized history is represented by spacers, but follows the same geometry contract.
for (const rows of [1, 199, 200, 201, 5000]) {
	f = fixture();
	f.moveContent(rows * 72 + 40);
	await f.flush();
	ok(f.scroller.scrollTop === 120 + rows * 72 + 40, `${rows} rows preserve heading position`);
}

// A clicked Show button can choose a revealed position after layout, while preserving guards.
let target = 0;
f = fixture(120, 120, 30, () => target);
f.moveContent(720);
target = 300;
await f.flush();
ok(f.heading.getBoundingClientRect().top - f.scroller.getBoundingClientRect().top === 300,
	'the explicit offset reveals history instead of keeping it above the viewport');
f = fixture(120, 120, 30, () => 0);
f.moveContent(720);
await f.flush();
ok(f.scroller.scrollTop === 840, 'zero is a valid target offset');
f = fixture(120, 120, 30, () => { throw new Error('cancelled target must not be read'); });
f.cancel();
await f.flush();
ok(f.writes === 0, 'cancellation guards target-offset computation too');

console.log(`queue history: ${assertions} assertions passed`);
