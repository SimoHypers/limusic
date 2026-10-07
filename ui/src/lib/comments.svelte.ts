// State behind the comments panel: the comments of one track, its sort, paging and per-thread
// replies. One track at a time, because the panel only shows the playing one; `videoId` says which.
//
// Every request is guarded against the world having moved on while it was in flight (a track
// change, a sort switch, a retry), the way `loadMore` in the home page is: a response is applied
// only if `gen` and the token it was asked with are still current.
import * as api from './api';
import type { Comment, CommentsHeader, CommentsState, CommentThread } from './api';
import { toast } from './player.svelte';
import { t } from './i18n.svelte';

/** A thread plus what the panel is doing with its replies. */
export interface ThreadView {
	comment: Comment;
	/** Replies loaded so far, nested levels already flattened by the backend. */
	replies: Comment[];
	/** Next replies page; absent when there is nothing (more) to fetch. */
	repliesToken?: string;
	open: boolean;
	loading: boolean;
	error: boolean;
}

export const comments = $state({
	videoId: null as string | null,
	/** `error` is a failed request (retry); `disabled` and `empty` are answers, not errors. */
	status: 'idle' as 'idle' | 'loading' | 'ready' | 'error',
	state: 'ok' as CommentsState,
	header: undefined as CommentsHeader | undefined,
	threads: [] as ThreadView[],
	continuation: undefined as string | undefined,
	loadingMore: false,
	moreError: false
});

/** Bumped on every (re)load and sort switch; a response from an older one is dropped. */
let gen = 0;

const view = (t: CommentThread): ThreadView => ({
	comment: t.comment,
	replies: t.replies,
	repliesToken: t.replies_token,
	open: false,
	loading: false,
	error: false
});

/** Append what is not already there: a keyed list dies on one repeated id. */
function fresh<T extends { id: string }>(have: T[], incoming: T[]): T[] {
	const seen = new Set(have.map((c) => c.id));
	return incoming.filter((c) => !seen.has(c.id) && seen.add(c.id));
}

const freshThreads = (have: ThreadView[], incoming: ThreadView[]) => {
	const seen = new Set(have.map((v) => v.comment.id));
	return incoming.filter((v) => !seen.has(v.comment.id) && seen.add(v.comment.id));
};

function apply(page: api.CommentsPage) {
	comments.state = page.state;
	if (page.header) comments.header = page.header;
	comments.threads = freshThreads([], page.threads.map(view));
	comments.continuation = page.continuation;
	comments.status = 'ready';
}

/**
 * Load a track's comments. A no-op when they are already loaded for it, so closing and reopening
 * the panel on the same track costs nothing; `force` is the retry button.
 */
export async function loadComments(videoId: string, force = false) {
	if (!force && comments.videoId === videoId && comments.status !== 'error') return;
	const g = ++gen;
	comments.videoId = videoId;
	comments.header = undefined;
	comments.threads = [];
	comments.continuation = undefined;
	comments.loadingMore = false;
	comments.moreError = false;
	comments.state = 'ok';
	// A local file has no YouTube comments to ask for.
	if (api.isLocalId(videoId)) {
		comments.state = 'disabled';
		comments.status = 'ready';
		return;
	}
	comments.status = 'loading';
	try {
		const page = await api.getComments(videoId);
		if (g !== gen) return;
		apply(page);
	} catch {
		if (g !== gen) return;
		comments.status = 'error';
		toast.error(t('toasts.could_not_load_comments'));
	}
}

/** The next page of top-level comments. An empty page is the end of the list. */
export async function loadMoreComments() {
	const token = comments.continuation;
	if (!token || comments.loadingMore || comments.status !== 'ready') return;
	const g = gen;
	comments.loadingMore = true;
	comments.moreError = false;
	try {
		const page = await api.getCommentsMore(token);
		if (g !== gen || comments.continuation !== token) return; // stale
		comments.threads = [...comments.threads, ...freshThreads(comments.threads, page.threads.map(view))];
		comments.continuation = page.threads.length ? page.continuation : undefined;
	} catch {
		if (g !== gen) return;
		// Stop auto-loading and offer a retry: retrying a visible sentinel by itself would spin.
		comments.moreError = true;
		toast.error(t('toasts.could_not_load_comments'));
	} finally {
		if (g === gen) comments.loadingMore = false;
	}
}

/** Reload the comments in the other order. The header's sort entries carry their own tokens. */
export async function setCommentSort(key: 'top' | 'newest') {
	const sort = comments.header?.sorts.find((s) => s.key === key);
	if (!sort || sort.selected || comments.status === 'loading') return;
	const g = ++gen;
	// What to put back if the switch fails: the old order is still the truth.
	const prev = { threads: comments.threads, continuation: comments.continuation, state: comments.state };
	comments.status = 'loading';
	comments.threads = [];
	comments.continuation = undefined;
	comments.loadingMore = false;
	comments.moreError = false;
	try {
		const page = await api.getCommentsMore(sort.token);
		if (g !== gen) return;
		apply(page);
		// A response without a header keeps the old sort entries: flip the selection by hand.
		if (!page.header && comments.header) {
			comments.header = {
				...comments.header,
				sorts: comments.header.sorts.map((s) => ({ ...s, selected: s.key === key }))
			};
		}
	} catch {
		if (g !== gen) return;
		Object.assign(comments, prev, { status: 'ready' });
		toast.error(t('toasts.could_not_load_comments'));
	}
}

/** Show or hide a thread's replies, fetching the first page the first time. */
export async function toggleReplies(v: ThreadView) {
	if (v.open) {
		v.open = false;
		return;
	}
	v.open = true;
	if (!v.replies.length && v.repliesToken) await loadMoreReplies(v);
}

/** The next page of a thread's replies. */
export async function loadMoreReplies(v: ThreadView) {
	const token = v.repliesToken;
	if (!token || v.loading) return;
	const g = gen;
	v.loading = true;
	v.error = false;
	try {
		const page = await api.getCommentReplies(token);
		if (g !== gen || v.repliesToken !== token) return; // stale
		v.replies = [...v.replies, ...fresh(v.replies, page.replies)];
		// An empty page ends the thread's replies, whatever token it carries.
		v.repliesToken = page.replies.length ? page.continuation : undefined;
	} catch {
		if (g !== gen) return;
		v.error = true;
		toast.error(t('toasts.could_not_load_comments'));
	} finally {
		if (g === gen) v.loading = false;
	}
}
