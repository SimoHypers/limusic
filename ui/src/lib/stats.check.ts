// Self-check for the stats page's calendar and formatting (`stats.ts`), run by `pnpm test`:
//
//     node --experimental-strip-types ui/src/lib/stats.check.ts
//
// Prints "ok" and exits 0, or throws on the first broken invariant.
import { bars, bigDuration, busiest, change, dayparts, heat, listened, rangeStart } from './stats.ts';

function eq<T>(got: T, want: T, what: string): void {
	const [g, w] = [JSON.stringify(got), JSON.stringify(want)];
	if (g !== w) throw new Error(`FAIL: ${what}: got ${g}, want ${w}`);
}

const now = new Date(2026, 2, 3, 15, 30); // 3 March 2026, mid-afternoon, local
const local = (secs: number) => new Date(secs * 1000);

eq(local(rangeStart('week', now)).toDateString(), new Date(2026, 1, 25).toDateString(), '7 days count today');
eq(local(rangeStart('month', now)).toDateString(), new Date(2026, 1, 2).toDateString(), '30 days across February');
eq(local(rangeStart('year', now)).toDateString(), new Date(2025, 3, 1).toDateString(), '12 months from the 1st');
eq(local(rangeStart('week', now)).getHours(), 0, 'from midnight');
eq(rangeStart('all', now), 0, 'all time');

const week = bars([['2026-02-26', 60], ['2026-03-03', 30]], 'week', now);
eq(week.map((b) => b.key), ['2026-02-25', '2026-02-26', '2026-02-27', '2026-02-28', '2026-03-01', '2026-03-02', '2026-03-03'], 'a week of days, gaps filled');
eq(week.map((b) => b.secs), [0, 60, 0, 0, 0, 0, 30], 'secs land on their day');

const year = bars([['2025-04-10', 5], ['2025-04-20', 5], ['2026-03-01', 7]], 'year', now);
eq(year.length, 12, 'twelve months');
eq([year[0].key, year[0].secs, year[11].key, year[11].secs], ['2025-04', 10, '2026-03', 7], 'days roll up into months');

eq(bars([['2026-02-20', 1]], 'all', now).length, 12, 'a short history: days from the first play');
eq(bars([['2025-06-15', 1]], 'all', now)[0].key, '2025-06', 'a long one: months');
eq(bars([], 'all', now).map((b) => b.key), ['2026-03-03'], 'nothing yet: just today');

eq(listened(0, 'en'), '0 min', 'zero');
eq(listened(4 * 3600 + 12 * 60, 'en'), '4 hr 12 min', 'hours and minutes');
eq(listened(2 * 3600 + 10, 'en'), '2 hr', 'whole hours');
eq(bigDuration(42 * 3600 + 17 * 60, 'en'), [{ n: '42', unit: 'h' }, { n: '17', unit: 'm' }], 'number and unit apart');
eq(bigDuration(1234 * 3600 + 60, 'en'), [{ n: '1,234', unit: 'h' }], 'past 100 hours, hours only');
eq(bigDuration(20, 'en'), [{ n: '0', unit: 'm' }], 'under a minute');

eq(change(150, 100), 50, 'up');
eq(change(50, 100), -50, 'down');
eq(change(50, 0), null, 'nothing before');
eq(change(50, null), null, 'all time');

const hours = new Array(168).fill(0);
hours[5 * 24 + 23] = 500; // one long Friday night
hours[2 * 24 + 9] = 300; // Tuesday and Wednesday mornings
hours[3 * 24 + 9] = 300;
eq(busiest(hours), { day: 5, hour: 9 }, 'Friday wins the days, 9am the hours: summed apart');
eq(busiest(new Array(168).fill(0)), null, 'nothing heard');

eq([0, 1, 25, 26, 100, 200].map((s) => heat(s, 100)), [0, 1, 1, 2, 4, 4], 'five shades');

const folded = new Array(168).fill(0);
folded[1 * 24 + 4] = 1; // Monday 4am: still the night before
folded[1 * 24 + 5] = 2; // 5am: morning
folded[3 * 24 + 12] = 4; // noon: afternoon
folded[6 * 24 + 21] = 8; // 9pm: evening
folded[0 * 24 + 23] = 16; // 11pm: night
eq(dayparts(folded), { morning: 2, afternoon: 4, evening: 8, night: 17 }, 'the day folds into four parts');

console.log('ok');
