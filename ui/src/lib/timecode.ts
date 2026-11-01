// Timecodes: "1:23" in a share field, `t=83` / `t=1m23s` in a YouTube link, "1:23" inside a
// comment. Pure and free of Svelte/Tauri imports so `timecode.check.ts` can run under plain node.

/** Seconds as `m:ss`, or `h:mm:ss` from an hour up. Negative / NaN / Infinity read as 0:00. */
export function formatClock(secs: number): string {
	const s = Number.isFinite(secs) && secs > 0 ? Math.floor(secs) : 0;
	const h = Math.floor(s / 3600);
	const m = Math.floor((s % 3600) / 60);
	const ss = String(s % 60).padStart(2, '0');
	return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

/**
 * What a person types into a "start at" box: `83`, `1:23` or `1:02:03`. `null` for anything else.
 * When a larger unit is present the smaller ones must be below 60 (`1:75` is a typo, not 135 s);
 * a bare number of seconds can be any size.
 */
export function parseClock(input: string): number | null {
	const text = input.trim();
	if (!/^\d+(:\d{1,2}){0,2}$/.test(text)) return null;
	const parts = text.split(':').map(Number);
	if (parts.slice(1).some((p) => p >= 60)) return null;
	return parts.reduce((acc, p) => acc * 60 + p, 0);
}

/** A piece of a comment: plain `text`, or a timecode (`secs` set) that can be jumped to. */
export type TimeSpan = { text: string; secs?: number };

// `m:ss` or `h:mm:ss`, standing alone: not glued to a word, a digit, a colon or a decimal point,
// so "12:30pm", "v1:23abc", "1.5:30" and "10:20:30:40" are not timecodes. The leading group stands
// in for a lookbehind (older WebKit has none) and is skipped over when slicing.
const CLOCK = /(^|[^\w:.])((?:(\d{1,2}):)?(\d{1,3}):(\d{2}))(?![\w:])/g;

/**
 * Cut `text` into plain text and timecodes. A match that can't be a time (`1:75`, `1:2:03`) stays
 * text, and so does one past `maxSecs` when the length is known (> 0), which keeps a "12:30"
 * that means lunch from becoming a button on a three-minute song. Spans join back to `text`.
 */
export function splitTimecodes(text: string, maxSecs = 0): TimeSpan[] {
	if (!text.includes(':')) return [{ text }];
	const out: TimeSpan[] = [];
	const push = (s: string, secs?: number) => {
		if (!s) return;
		const last = out[out.length - 1];
		if (secs === undefined && last && last.secs === undefined) last.text += s;
		else out.push(secs === undefined ? { text: s } : { text: s, secs });
	};
	let at = 0;
	for (const m of text.matchAll(CLOCK)) {
		const [, lead, clock, h, mm, ss] = m;
		if (Number(ss) >= 60) continue;
		if (h !== undefined && (mm.length !== 2 || Number(mm) >= 60)) continue;
		const secs = Number(h ?? 0) * 3600 + Number(mm) * 60 + Number(ss);
		if (maxSecs > 0 && secs > maxSecs) continue;
		const start = m.index + lead.length;
		push(text.slice(at, start));
		push(clock, secs);
		at = start + clock.length;
	}
	push(text.slice(at));
	return out;
}

/** YouTube's `t=` / `start=` value: `83`, `83s`, `1m23s`, `1h2m3s`. `null` for anything else. */
export function parseYtStart(input: string | null | undefined): number | null {
	const text = (input ?? '').trim().toLowerCase();
	if (!text) return null;
	if (/^\d+$/.test(text)) return Number(text);
	const m = /^(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s)?$/.exec(text);
	if (!m || (m[1] === undefined && m[2] === undefined && m[3] === undefined)) return null;
	return Number(m[1] ?? 0) * 3600 + Number(m[2] ?? 0) * 60 + Number(m[3] ?? 0);
}
