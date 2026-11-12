// The review screen's rules, pure so `review.check.ts` can run them: where a row goes when it is
// decided, what a refusal means, how old a submission is. The page is `routes/review`.
import type { LimusicQueueItem } from './api';
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
