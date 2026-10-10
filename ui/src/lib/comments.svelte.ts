// State behind the comments panel, for the playing track (`videoId`). A response is applied only
// if `gen` and the token it was asked with are still current, as in the home page's `loadMore`.
// Errors arrive as a kind word (`api.commentsErrorKind`) and are worded here.
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

/** A composer's text and whether it is waiting for an answer. */
interface Box {
	text: string;
	pending: boolean;
}

function emptyBox(): Box & { target: null } {
	return { text: '', pending: false, target: null };
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
	/** Comments hidden since their deletion was confirmed (back again if it fails). */
	hidden: {} as Record<string, boolean>,
	/** The comment waiting for the viewer to confirm its deletion. */
	confirmDelete: null as Comment | null,
	/** The one inline composer under a comment: a reply, or an edit of the viewer's own comment. */
	draft: emptyBox() as Box & { target: { kind: 'reply' | 'edit'; id: string } | null },
	/** The comment box at the top of the panel. */
	create: emptyBox() as Box
});

/** Bumped on every (re)load and sort switch; a response from an older one is dropped. */
let gen = 0;
/** Bumped only by a (re)load of the track: what a write was started under. A sort switch must not
 *  strand the composer that was sending in its pending state. */
let session = 0;

const view = (t: CommentThread): ThreadView => ({
	comment: t.comment,
	replies: t.replies,
	repliesToken: t.replies_token,
	open: false,
	loading: false,
	error: false
});

/** What of `incoming` is not already in `have`, by id: a keyed list dies on one repeated id. */
function fresh<T>(have: T[], incoming: T[], id: (x: T) => string): T[] {
	const seen = new Set(have.map(id));
	return incoming.filter((x) => !seen.has(id(x)) && seen.add(id(x)));
}

const threadId = (v: ThreadView) => v.comment.id;
const commentId = (c: Comment) => c.id;

/** The comments belong to a session that is gone, so only a reload helps. */
const needsReload = (e: unknown) => api.commentsErrorKind(e) === 'account_changed';

/** A read failed: the reload screen if the session is gone, else `onOther` and a toast. */
function readFailed(e: unknown, onOther: () => void) {
	if (needsReload(e)) {
		comments.status = 'reload';
		return;
	}
	onOther();
	toast.error(t('toasts.could_not_load_comments'));
}

/** Empty the list (before a load or a sort switch). */
function clearList() {
	comments.threads = [];
	comments.continuation = undefined;
	comments.loadingMore = false;
	comments.moreError = false;
	comments.acting = {};
}

function apply(page: api.CommentsPage) {
	comments.state = page.state;
	if (page.header) comments.header = page.header;
	comments.threads = fresh([], page.threads.map(view), threadId);
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
	clearList();
	comments.hidden = {};
	comments.confirmDelete = null;
	comments.draft = emptyBox();
	comments.create = emptyBox();
	session++;
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
		readFailed(e, () => (comments.status = 'error'));
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
		comments.threads = [...comments.threads, ...fresh(comments.threads, page.threads.map(view), threadId)];
		comments.continuation = page.threads.length ? page.continuation : undefined;
	} catch (e) {
		if (g !== gen) return;
		// Stop auto-loading and offer a retry: retrying a visible sentinel by itself would spin.
		readFailed(e, () => (comments.moreError = true));
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
	clearList();
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
		readFailed(e, () => Object.assign(comments, prev, { status: 'ready' }));
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
		v.replies = [...v.replies, ...fresh(v.replies, page.replies, commentId)];
		// An empty page ends the thread's replies, whatever token it carries.
		v.repliesToken = page.replies.length ? page.continuation : undefined;
	} catch (e) {
		if (g !== gen) return;
		readFailed(e, () => (v.error = true));
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

/** Does this comment get like/dislike buttons? Only if the response offered it an action, so
 *  never signed out. Replies are no different from comments. */
export const canAct = (c: Comment) => c.actions.length > 0;

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
	c.vote = VOTE_AFTER[action];
	try {
		const out = await api.commentAction(c.id, action);
		if (g !== gen) return;
		// What Rust says is on offer now.
		c.vote = out.vote;
		c.actions = out.actions;
	} catch (e) {
		if (g !== gen) return;
		const kind = api.commentsErrorKind(e);
		if (needsReload(e)) {
			comments.status = 'reload'; // everything here is replaced by the reload
			return;
		}
		if (kind === 'gone') {
			dropGone(c.id);
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

// --- writing: post, reply, edit, delete -------------------------------------------------------
//
// A post, reply or edit waits for the answer and keeps its text on failure; a delete hides the
// row at once and puts it back on failure. Not reloaded after a write: reads lag behind writes.

/** The comment is gone on YouTube's side (404): drop it, and anything open on it. */
function dropGone(id: string) {
	removeComment(id);
	if (comments.draft.target?.id === id) comments.draft = emptyBox();
	toast.error(t('toasts.comment_gone'));
}

/** Take a comment (and, for a top-level one, its loaded replies) off the screen. */
function removeComment(id: string) {
	comments.hidden[id] = true;
	comments.threads = comments.threads.filter((v) => v.comment.id !== id);
	for (const v of comments.threads) v.replies = v.replies.filter((r) => r.id !== id);
}

/** Every comment on screen, top-level and replies. */
function findComment(id: string): Comment | undefined {
	for (const v of comments.threads) {
		if (v.comment.id === id) return v.comment;
		const reply = v.replies.find((r) => r.id === id);
		if (reply) return reply;
	}
}

/** A write failed: the text stays in its box and the user is told in our words. */
function writeFailed(e: unknown) {
	const kind = api.commentsErrorKind(e);
	if (needsReload(e)) {
		comments.status = 'reload'; // the comments and their commands belong to a session that is gone
		return;
	}
	if (kind === 'busy') return;
	// `uncertain`: a post or reply may have gone through. The text stays; the user checks first.
	toast.error(
		t(
			kind === 'uncertain'
				? 'toasts.comment_write_unconfirmed'
				: kind === 'rejected'
					? 'toasts.comment_write_rejected'
					: 'toasts.comment_write_failed'
		)
	);
}

/** Post what is in the top comment box. */
export async function submitCreate() {
	const box = comments.create;
	const text = box.text.trim();
	if (!text || box.pending || !comments.header?.composer) return;
	const s = session;
	box.pending = true;
	try {
		const posted = await api.commentCreate(text);
		if (s !== session) return;
		box.text = '';
		if (!posted) {
			// The answer did not carry it: it shows up on the next load.
			toast.success(t('toasts.comment_posted'));
			return;
		}
		// The comment may be listed already (a read since carried it).
		if (!comments.threads.some((v) => v.comment.id === posted.comment.id)) {
			comments.threads = [view(posted), ...comments.threads];
		}
		if (comments.state === 'empty') comments.state = 'ok';
	} catch (e) {
		if (s !== session) return;
		writeFailed(e);
	} finally {
		if (s === session) box.pending = false;
	}
}

export function openReply(c: Comment) {
	if (comments.draft.pending) return;
	comments.draft = { ...emptyBox(), target: { kind: 'reply', id: c.id } };
}

export function openEdit(c: Comment) {
	if (comments.draft.pending) return;
	// What the edit dialog pre-fills when the response had it plainly, else the text on screen.
	comments.draft = { ...emptyBox(), text: c.edit_text ?? c.text, target: { kind: 'edit', id: c.id } };
}

/** Close the inline composer, restoring nothing: an edit's original text was never touched. */
export function cancelDraft() {
	if (comments.draft.pending) return;
	comments.draft = emptyBox();
}

/** The thread a comment is on screen in, as the comment or as one of its replies. */
const threadOf = (id: string) =>
	comments.threads.find((v) => v.comment.id === id || v.replies.some((r) => r.id === id));

/** Send what is in the inline composer: a reply, or the edited text of an own comment. */
export async function submitDraft() {
	const d = comments.draft;
	const target = d.target;
	const text = d.text.trim();
	if (!target || !text || d.pending) return;
	const s = session;
	d.pending = true;
	try {
		if (target.kind === 'reply') {
			const posted = await api.commentReply(target.id, text);
			if (s !== session) return;
			comments.draft = emptyBox();
			if (!posted) {
				toast.success(t('toasts.comment_posted'));
				return;
			}
			// Under its thread, open.
			const v = threadOf(target.id);
			if (v) {
				if (!v.replies.some((r) => r.id === posted.id)) v.replies = [...v.replies, posted];
				v.open = true;
			}
		} else {
			await api.commentEdit(target.id, text);
			if (s !== session) return;
			// The text YouTube accepted is the text that was sent: update the row in place.
			const c = findComment(target.id);
			if (c) {
				c.text = text;
				c.edit_text = text;
			}
			comments.draft = emptyBox();
		}
	} catch (e) {
		if (s !== session) return;
		if (api.commentsErrorKind(e) === 'gone') dropGone(target.id);
		else writeFailed(e);
	} finally {
		if (s === session) d.pending = false;
	}
}

/** Ask the viewer to confirm deleting a comment (the dialog is in the panel). */
export const requestDelete = (c: Comment) => {
	comments.confirmDelete = c;
};
export const cancelDelete = () => {
	comments.confirmDelete = null;
};

/** The viewer confirmed: close the dialog, hide the row, send the delete, and put the row back
 *  if it fails. A delete of a comment that is already gone succeeds in Rust. */
export async function confirmDelete() {
	const c = comments.confirmDelete;
	comments.confirmDelete = null;
	if (!c) return;
	const s = session;
	comments.hidden[c.id] = true;
	try {
		await api.commentDelete(c.id);
		if (s !== session) return;
		removeComment(c.id);
	} catch (e) {
		if (s !== session) return;
		delete comments.hidden[c.id];
		if (needsReload(e)) comments.status = 'reload';
		else if (api.commentsErrorKind(e) !== 'busy') toast.error(t('toasts.comment_delete_failed'));
	}
}
