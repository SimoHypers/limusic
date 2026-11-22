// Self-check for the seek bar's fill clock (`seekfill.ts`). Same deal as `queue.check.ts`: no test
// runner in `ui/`, node runs TypeScript directly:
//
//     node --experimental-strip-types ui/src/lib/seekfill.check.ts
//
// The bugs this guards are the ones a smoothed bar can have that a stepped one cannot: a fill that
// runs ahead of audio that stopped, one that leaps when playback resumes after a long pause, one
// that stutters backwards on timer jitter, and one that refuses to follow a real seek.
import {
	coast,
	drawGap,
	frame,
	JUMP_SECS,
	MAX_COAST_MS,
	MAX_GAP_MS,
	MIN_GAP_MS,
	restart,
	sample
} from './seekfill.ts';

function ok(cond: boolean, what: string): void {
	if (!cond) throw new Error(`FAIL: ${what}`);
}
const near = (a: number, b: number) => Math.abs(a - b) < 1e-9;

const DUR = 200; // shown seconds
const PER_SEC = 1 / DUR; // pct per second of playing

// --- it advances at the rate of playback -------------------------------------------------------
{
	const c = restart(0.25, 1000);
	ok(near(coast(c, 1000, DUR), 0.25), 'no time passed, no movement');
	ok(near(coast(c, 1250, DUR), 0.25 + 0.25 * PER_SEC), 'a quarter second moves a quarter second');
	ok(coast(c, 1200, DUR) > coast(c, 1100, DUR), 'later is further');
	ok(coast(c, 900, DUR) === 0.25, 'a clock read before its tick does not go backwards');
}

// --- it does not run away from audio that stopped ----------------------------------------------
{
	const c = restart(0.5, 0);
	ok(near(coast(c, 60_000, DUR), 0.5 + (MAX_COAST_MS / 1000) * PER_SEC), 'a long silence holds');
	ok(coast(restart(0.999999, 0), 10_000, 1) === 1, 'never past the end');
	ok(coast(restart(0.4, 0), 5_000, 0) === 0.4, 'no duration, no movement');
}

// --- jitter never steps the fill back; a seek does ---------------------------------------------
{
	// The fill has coasted 200 ms ahead, then a tick arrives that is 150 ms behind it.
	let c = restart(0.5, 0);
	c = frame(c, 200, DUR);
	const ahead = c.shown;
	c = sample(c, 0.5 + 0.05 * PER_SEC, 200, DUR);
	ok(c.shown === ahead, 'a tick a little behind the fill does not pull it back');
	c = frame(c, 250, DUR);
	ok(c.shown >= ahead, 'and the next draw only goes forward');

	// A 10 s seek backwards, on a 200 s track.
	let s = restart(0.5, 0);
	s = frame(s, 100, DUR);
	s = sample(s, 0.45, 100, DUR);
	ok(near(s.shown, 0.45), 'a backwards seek past the threshold shows at once');
	// And forwards.
	s = sample(s, 0.8, 200, DUR);
	ok(near(s.shown, 0.8), 'a forwards seek shows at once');
	// A seek just inside the threshold counts as jitter, which is what the threshold means.
	const t = sample(restart(0.5, 0), 0.5 - (JUMP_SECS * 0.9) / DUR, 0, DUR);
	ok(near(t.shown, 0.5), 'a sub-threshold step back is held');
}

// --- a new track ------------------------------------------------------------------------------
{
	const end = frame(restart(0.97, 0), 100, 180);
	const next = sample(end, 0.0, 120, 180);
	ok(next.shown === 0, 'a new track starts the bar over instead of gliding back');
	const first = sample(restart(0.2, 0), 0.5, 10, 0);
	ok(first.shown === 0.5, 'a tick with no duration yet goes straight there');
}

// --- resume after a pause ---------------------------------------------------------------------
{
	// A pause leaves the last tick's time far behind. Resuming restarts the clock, so the first
	// draw is where the bar was, not a half second on.
	const resumed = restart(0.3, 90_000);
	ok(near(frame(resumed, 90_000, DUR).shown, 0.3), 'resume does not leap');
	// Without the restart, the cap is what bounds the leap: it is the reason the cap exists.
	const stale = restart(0.3, 0);
	ok(frame(stale, 90_000, DUR).shown - 0.3 <= (MAX_COAST_MS / 1000) * PER_SEC + 1e-12, 'a stale tick leaps at most the cap');
}

// --- frame ------------------------------------------------------------------------------------
{
	const c = restart(0.2, 0);
	ok(frame(c, 0, DUR) === c, 'a draw with nothing to add allocates nothing');
	const d = frame(c, 400, DUR);
	ok(d.shown > c.shown && d.pct === c.pct && d.at === c.at, 'a draw moves the fill, not the anchor');
}

// --- redraw rate ------------------------------------------------------------------------------
{
	ok(near(drawGap(180, 1200), 150), 'a 3 minute track in 1200 px: a pixel every 150 ms');
	ok(drawGap(10, 1900) === MIN_GAP_MS, 'a short track in a wide bar is capped at 30 a second');
	ok(drawGap(3600, 300) === MAX_GAP_MS, 'a long track in a narrow bar redraws as often as ticks land');
	ok(drawGap(180, 0) === MAX_GAP_MS, 'width unknown yet: the slow rate');
	ok(drawGap(0, 800) === MAX_GAP_MS, 'duration unknown yet: the slow rate');
}

console.log('ok');
