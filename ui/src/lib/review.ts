// The review screen's rules, pure so `review.check.ts` can run them: where a row goes when it is
// decided, what a refusal means, how old a submission is. The page is `routes/review`.
import type { LimusicQueueItem, LyricLine } from './api';
import type { TranslationKey } from './i18n.svelte';

/** The queue without `revision`: a decided row leaves the list at once. */
export const without = (items: LimusicQueueItem[], revision: number): LimusicQueueItem[] =>
	items.filter((i) => i.revision !== revision);

/** `row` back in its place when the service refused. The queue is oldest first, which is by
 *  revision id, so it goes in front of the first newer one. Already there: unchanged. */
export function restored(items: LimusicQueueItem[], row: LimusicQueueItem): LimusicQueueItem[] {
	if (items.some((i) => i.revision === row.revision)) return items;
	const at = items.findIndex((i) => i.revision > row.revision);
	const copy = items.slice();
	copy.splice(at < 0 ? copy.length : at, 0, row);
	return copy;
}

/** "3" or "50+": how many are waiting, when only the first page is known. */
export const waitingLabel = (count: number, more: boolean) => (more ? `${count}+` : String(count));

export function kindKey(kind: string): TranslationKey {
	switch (kind) {
		case 'new':
			return 'review.kind_new';
		case 'reported':
			return 'review.kind_reported';
		default:
			return 'review.kind_edit';
	}
}

/** The service says the revision is no longer there to decide: someone else got to it first, or it
 *  was replaced. Putting the row back would show a dead one. */
export const isGone = (code: string) => ['not_found', 'not_reviewable', 'stale'].includes(code);

/** A refusal or a failure, as a key. */
export function reviewErrorKey(code: string): TranslationKey {
	if (isGone(code)) return 'review.err_gone';
	switch (code) {
		case 'unauthorized':
			return 'review.err_signed_out';
		case 'forbidden':
			return 'review.err_forbidden';
		case 'banned':
			return 'review.err_banned';
		case 'rate_limited':
			return 'review.err_rate_limited';
		case 'invalid':
			return 'review.err_invalid';
		default:
			return 'review.err_unavailable';
	}
}

/** "5 minutes ago", "yesterday", in the UI's language. `created` and `now` are unix seconds. */
export function ago(created: number, now: number, locale: string): string {
	const secs = Math.max(0, now - created);
	const fmt = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
	if (secs < 60) return fmt.format(0, 'second');
	if (secs < 3600) return fmt.format(-Math.floor(secs / 60), 'minute');
	if (secs < 86400) return fmt.format(-Math.floor(secs / 3600), 'hour');
	return fmt.format(-Math.floor(secs / 86400), 'day');
}

/** A cover for a queue row, which carries only the video id: YouTube's thumbnail for it, the same
 *  host every other cover in the app comes from. 16:9, so the row crops its middle square, which
 *  is the whole cover for a song. `undefined` for anything that is not a video id. */
export const coverOf = (videoId: string): string | undefined =>
	/^[\w-]{11}$/.test(videoId) ? `https://i.ytimg.com/vi/${videoId}/mqdefault.jpg` : undefined;

// --- What an edit changes ---------------------------------------------------------------------

export interface Change {
	/** The line's number in the proposed lyric, or in the current one for a removed line. */
	n: number;
	kind: 'reworded' | 'added' | 'removed' | 'retimed';
	text: string;
	/** A reworded line's text as it is live now. */
	old?: string;
	/** A retimed line whose start moved: by how much, in ms. Absent when only its words moved. */
	shiftMs?: number;
}

const norm = (s: string) => s.trim().replace(/\s+/g, ' ');
/** A start that moved less than this is the same start. */
const SAME_MS = 50;

function retimed(a: LyricLine, b: LyricLine): boolean {
	const x = a.words ?? [];
	const y = b.words ?? [];
	if (x.length !== y.length) return true;
	return x.some((w, i) => Math.abs(w.start_ms - y[i].start_ms) >= SAME_MS || Math.abs(w.end_ms - y[i].end_ms) >= SAME_MS);
}

/** The lines of `proposed` that differ from `current`, in order. Lines are matched by their text
 *  (a longest common subsequence), so an added or removed line does not make every line after it
 *  look changed; an unmatched line next to an unmatched one is the same line reworded.
 *  ponytail: a full n*m table, fine for the service's 1000-line cap. */
export function diffLyrics(current: LyricLine[], proposed: LyricLine[]): Change[] {
	const a = current.map((l) => norm(l.text));
	const b = proposed.map((l) => norm(l.text));
	const n = a.length;
	const m = b.length;
	const w = m + 1;
	const lcs = new Uint16Array((n + 1) * w);
	for (let i = n - 1; i >= 0; i--) {
		for (let j = m - 1; j >= 0; j--) {
			lcs[i * w + j] = a[i] === b[j] ? lcs[(i + 1) * w + j + 1] + 1 : Math.max(lcs[(i + 1) * w + j], lcs[i * w + j + 1]);
		}
	}
	const out: Change[] = [];
	let gone: number[] = [];
	let come: number[] = [];
	const flush = () => {
		const both = Math.min(gone.length, come.length);
		for (let k = 0; k < both; k++) out.push({ n: come[k] + 1, kind: 'reworded', text: proposed[come[k]].text, old: current[gone[k]].text });
		for (const i of gone.slice(both)) out.push({ n: i + 1, kind: 'removed', text: current[i].text });
		for (const j of come.slice(both)) out.push({ n: j + 1, kind: 'added', text: proposed[j].text });
		gone = [];
		come = [];
	};
	let i = 0;
	let j = 0;
	while (i < n || j < m) {
		if (i < n && j < m && a[i] === b[j]) {
			flush();
			const shift = (proposed[j].time_ms ?? 0) - (current[i].time_ms ?? 0);
			if (Math.abs(shift) >= SAME_MS) out.push({ n: j + 1, kind: 'retimed', text: proposed[j].text, shiftMs: shift });
			else if (retimed(current[i], proposed[j])) out.push({ n: j + 1, kind: 'retimed', text: proposed[j].text });
			i++;
			j++;
		} else if (j < m && (i === n || lcs[i * w + j + 1] >= lcs[(i + 1) * w + j])) come.push(j++);
		else gone.push(i++);
	}
	flush();
	return out;
}
