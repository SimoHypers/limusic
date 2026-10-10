// State behind the comments panel: the comments of one track, its sort, paging, per-thread
// replies and the signed-in viewer's likes and dislikes. One track at a time, because the panel
// only shows the playing one; `videoId` says which.
//
// Every request is guarded against the world having moved on while it was in flight (a track
// change, a sort switch, a retry, a sign-in or account switch), the way `loadMore` in the home
// page is: a response is applied only if `gen` and the token it was asked with are still current.
//
// Errors arrive as a kind word (`api.commentsErrorKind`), never as text. `account_changed` and
// `stale_token` mean the comments belong to a session that is gone: the panel goes to `reload`
// and shows its own button. Anything else is a toast the UI words itself.
import * as api from './api';
import type {
	Comment,
	CommentAction,
	CommentsHeader,
	CommentsState,
	CommentThread,
	CommentVote
} from './api';
import { auth, toast } from './player.svelte';
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
	/** `error` is a failed request (retry), `reload` a session that changed under the comments
	 *  (reload); `disabled` and `empty` are answers, not errors. */
	status: 'idle' as 'idle' | 'loading' | 'ready' | 'error' | 'reload',
	state: 'ok' as CommentsState,
	header: undefined as CommentsHeader | undefined,
	threads: [] as ThreadView[],
	continuation: undefined as string | undefined,
	loadingMore: false,
	moreError: false,
	/** `auth.epoch` these comments were loaded under. A different one means a sign-in, sign-out or
	 *  account switch since: nothing here may be used, and the next load starts over. */
	epoch: 0,
	/** Comments with a like/dislike request in flight, by id. */
	acting: {} as Record<string, boolean>,
	/** Comments the viewer has voted on in this panel, by id. Their buttons stay (disabled if the
	 *  reverse action is not on offer) instead of vanishing, which would look like a lost vote. */
	touched: {} as Record<string, boolean>
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

/** The comments belong to a session that is gone, so only a reload helps. */
const needsReload = (e: unknown) => {
	const kind = api.commentsErrorKind(e);
	return kind === 'account_changed' || kind === 'stale_token';
};

function apply(page: api.CommentsPage) {
	comments.state = page.state;
	if (page.header) comments.header = page.header;
	comments.threads = freshThreads([], page.threads.map(view));
	comments.continuation = page.continuation;
	comments.status = 'ready';
}

/**
 * Load a track's comments. A no-op when they are already loaded for it under the same sign-in, so
 * closing and reopening the panel on the same track costs nothing; `force` is the retry and
 * reload buttons.
 */
export async function loadComments(videoId: string, force = false) {
	const settled = comments.status !== 'error' && comments.status !== 'reload';
	if (!force && comments.videoId === videoId && settled && comments.epoch === auth.epoch) return;
	const g = ++gen;
	comments.videoId = videoId;
	comments.epoch = auth.epoch;
	comments.header = undefined;
	comments.threads = [];
	comments.continuation = undefined;
	comments.loadingMore = false;
	comments.moreError = false;
	comments.acting = {};
	comments.touched = {};
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
	} catch (e) {
		if (g !== gen) return;
		if (needsReload(e)) {
			comments.status = 'reload';
		} else {
			comments.status = 'error';
			toast.error(t('toasts.could_not_load_comments'));
		}
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
	} catch (e) {
		if (g !== gen) return;
		if (needsReload(e)) {
			comments.status = 'reload';
		} else {
			// Stop auto-loading and offer a retry: retrying a visible sentinel by itself would spin.
			comments.moreError = true;
			toast.error(t('toasts.could_not_load_comments'));
		}
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
	comments.acting = {};
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
	} catch (e) {
		if (g !== gen) return;
		if (needsReload(e)) {
			comments.status = 'reload';
			return;
		}
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
	} catch (e) {
		if (g !== gen) return;
		if (needsReload(e)) {
			comments.status = 'reload';
		} else {
			v.error = true;
			toast.error(t('toasts.could_not_load_comments'));
		}
	} finally {
		if (g === gen) v.loading = false;
	}
}

// --- the viewer's own like/dislike ---------------------------------------------------------

/** The vote a comment has once an action is accepted (what Rust computes too). */
const VOTE_AFTER: Record<CommentAction, CommentVote> = {
	like: 'liked',
	unlike: 'neutral',
	dislike: 'disliked',
	undislike: 'neutral'
};

/** Does this comment get like/dislike buttons? Only if the response offered it an action (so never
 *  signed out) or the viewer has voted on it here. Replies are no different from comments. */
export const canAct = (c: Comment) => c.actions.length > 0 || !!comments.touched[c.id];

export const isActing = (c: Comment) => !!comments.acting[c.id];

/**
 * The like count to show. Counts are display strings ("2.4M"): a vote swaps between the two the
 * comment came with and never adds one. If the variant for the current vote is missing, the count
 * stays as it was sent (and only the highlighted icon shows the vote).
 */
export function shownCount(c: Comment): string | undefined {
	const variant = c.vote === 'liked' ? c.like_count_liked : c.like_count_notliked;
	return variant ?? c.like_count;
}

/**
 * Like, unlike, dislike or undislike a comment, following `setRating`: the vote changes at once,
 * and a failure puts it back. At most one request per comment; the buttons are disabled meanwhile.
 * Like to dislike is one request (the dislike action), so the vote jumps straight across.
 */
export async function actOnComment(c: Comment, action: CommentAction) {
	if (comments.acting[c.id] || !c.actions.includes(action)) return;
	const g = gen;
	const prev = { vote: c.vote, actions: c.actions };
	comments.acting[c.id] = true;
	comments.touched[c.id] = true;
	c.vote = VOTE_AFTER[action];
	try {
		const out = await api.commentAction(c.id, action);
		if (g !== gen) return;
		// What Rust says is on offer now, which may be nothing (no token for the way back).
		c.vote = out.vote;
		c.actions = out.actions;
	} catch (e) {
		if (g !== gen) return;
		const kind = api.commentsErrorKind(e);
		if (needsReload(e)) {
			comments.status = 'reload'; // everything here is replaced by the reload
			return;
		}
		c.vote = prev.vote;
		c.actions = prev.actions;
		// `busy` is a second click racing the first: nothing to say.
		if (kind !== 'busy') {
			toast.error(t(kind === 'rejected' ? 'toasts.comment_action_rejected' : 'toasts.comment_action_failed'));
		}
	} finally {
		if (g === gen) delete comments.acting[c.id];
	}
}
