// The stats page (#396): which window each range asks Rust about, and how the answer is laid out.
// Rust counts; this owns the calendar, because the calendar the page is drawn against is the
// webview's. Pure, so `stats.check.ts` can pin the edges (month ends, a year boundary, a gap).

export type Range = 'week' | 'month' | 'year' | 'all';
export const RANGES: Range[] = ['week', 'month', 'year', 'all'];

/** Where a range starts, unix secs, local time: the last 7 or 30 days counting today, the last 12
 *  months counting this one, or everything (0). Always runs to now. */
export function rangeStart(range: Range, now = new Date()): number {
	const [y, m, d] = [now.getFullYear(), now.getMonth(), now.getDate()];
	const start =
		range === 'week'
			? new Date(y, m, d - 6)
			: range === 'month'
				? new Date(y, m, d - 29)
				: range === 'year'
					? new Date(y, m - 11, 1)
					: null;
	return start ? Math.floor(start.getTime() / 1000) : 0;
}

export interface Bar {
	/** `2026-10-17`, or `2026-10` for a month. */
	key: string;
	/** Its first moment, local. */
	date: Date;
	secs: number;
}

const pad = (n: number) => String(n).padStart(2, '0');
const dayKey = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

/** All time draws days until the history is about three months long, months after that. */
const DAILY_UP_TO = 92;

/**
 * One bar per day of the window, or per month for a year and for a long all time, with nothing
 * left out: a day without a play is a bar of zero, not a gap the chart closes up. `days` is what
 * Rust sent, `[day, secs]` with only the days that had something.
 */
export function bars(days: [string, number][], range: Range, now = new Date()): Bar[] {
	const first =
		range === 'all'
			? days.length
				? new Date(`${days[0][0]}T00:00`)
				: now
			: new Date(rangeStart(range, now) * 1000);
	const monthly =
		range === 'year' || (range === 'all' && (now.getTime() - first.getTime()) / 864e5 > DAILY_UP_TO);
	const secs = new Map<string, number>();
	for (const [day, s] of days) {
		const k = monthly ? day.slice(0, 7) : day;
		secs.set(k, (secs.get(k) ?? 0) + s);
	}
	const out: Bar[] = [];
	const at = new Date(first.getFullYear(), first.getMonth(), monthly ? 1 : first.getDate());
	while (at <= now) {
		const key = monthly ? dayKey(at).slice(0, 7) : dayKey(at);
		out.push({ key, date: new Date(at), secs: secs.get(key) ?? 0 });
		if (monthly) at.setMonth(at.getMonth() + 1);
		else at.setDate(at.getDate() + 1);
	}
	return out;
}

/** A catalog id (`pt_BR`) as a BCP 47 tag (`pt-BR`): Intl throws on the underscore. */
export const tagOf = (locale: string) => locale.replace('_', '-');

/** "4 hr 12 min", "35 min", "0 min": rounded to the minute, hours and minutes in the UI's language. */
export function listened(secs: number, tag: string): string {
	const mins = Math.round(secs / 60);
	const unit = (unit: string, n: number) =>
		new Intl.NumberFormat(tag, { style: 'unit', unit, unitDisplay: 'short' }).format(n);
	const [h, m] = [Math.floor(mins / 60), mins % 60];
	return h ? (m ? `${unit('hour', h)} ${unit('minute', m)}` : unit('hour', h)) : unit('minute', m);
}

/**
 * The headline total as number/unit pairs, so the page can set the numbers large and the units
 * small: `42h 17m` → `[{n:'42', unit:'h'}, {n:'17', unit:'m'}]`. Past a hundred hours the minutes
 * stop mattering and only hours are shown.
 */
export function bigDuration(secs: number, tag: string): { n: string; unit: string }[] {
	const mins = Math.round(secs / 60);
	const [h, m] = [Math.floor(mins / 60), mins % 60];
	const part = (unit: string, v: number) => {
		const parts = new Intl.NumberFormat(tag, {
			style: 'unit',
			unit,
			unitDisplay: 'narrow'
		}).formatToParts(v);
		return {
			n: parts.filter((p) => p.type !== 'unit' && p.type !== 'literal').map((p) => p.value).join(''),
			unit: parts.filter((p) => p.type === 'unit').map((p) => p.value).join('')
		};
	};
	if (h >= 100 || (h && !m)) return [part('hour', h)];
	return h ? [part('hour', h), part('minute', m)] : [part('minute', m)];
}

/** Percent change against the window before, or null when there's nothing before to compare. */
export function change(secs: number, prev: number | null): number | null {
	if (!prev) return null;
	return Math.round(((secs - prev) / prev) * 100);
}

/** The weekday (0 = Sunday) and the hour that hold the most listening, each summed on its own,
 *  so one long Friday night doesn't decide both. Null with nothing heard. */
export function busiest(hours: number[]): { day: number; hour: number } | null {
	const days = Array.from({ length: 7 }, (_, d) =>
		hours.slice(d * 24, d * 24 + 24).reduce((a, b) => a + b, 0)
	);
	const byHour = Array.from({ length: 24 }, (_, h) =>
		days.reduce((sum, _, d) => sum + (hours[d * 24 + h] ?? 0), 0)
	);
	const top = (xs: number[]) => xs.indexOf(Math.max(...xs));
	return days.some((s) => s > 0) ? { day: top(days), hour: top(byHour) } : null;
}

/** 0 for nothing, else 1-4 by the cell's share of the busiest one: the heatmap's five shades. */
export function heat(secs: number, max: number): number {
	return secs <= 0 || max <= 0 ? 0 : Math.min(4, Math.ceil((secs / max) * 4));
}

export type Daypart = 'morning' | 'afternoon' | 'evening' | 'night';
/** The four parts of a day, in order, by the local hour each starts at. Night runs past midnight. */
export const DAYPARTS: { key: Daypart; from: number; to: number }[] = [
	{ key: 'morning', from: 5, to: 12 },
	{ key: 'afternoon', from: 12, to: 17 },
	{ key: 'evening', from: 17, to: 22 },
	{ key: 'night', from: 22, to: 5 }
];

/** The week's `hours` (`hours[day * 24 + hour]`) folded into seconds per part of the day. */
export function dayparts(hours: number[]): Record<Daypart, number> {
	const out: Record<Daypart, number> = { morning: 0, afternoon: 0, evening: 0, night: 0 };
	hours.forEach((secs, i) => {
		const h = i % 24;
		out[DAYPARTS.find((p) => h >= p.from && h < p.to)?.key ?? 'night'] += secs;
	});
	return out;
}
