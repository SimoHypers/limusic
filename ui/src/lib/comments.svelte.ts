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

/** A composer's text and what it is waiting for. */
interface Box {
	text: string;
	pending: boolean;
	/** The last write may have gone through and the answer was lost: say so, offer a reload. */
	uncertain: boolean;
}

function emptyBox(): Box & { target: null } {
	return { text: '', pending: false, uncertain: false, target: null };
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
	touched: {} as Record<string, boolean>,
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
/** Bumped only by a (re)load of the track: what a write was started under. A sort switch or a
 *  refresh after posting must not strand the composer that caused it in its pending state. */
let session = 0;

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

const emptyDraft = () => emptyBox();

/** The comment is gone on YouTube's side (404): drop it, and anything open on it. */
function dropGone(id: string) {
	removeComment(id);
	if (comments.draft.target?.id === id) comments.draft = emptyDraft();
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

/** A write failed: nothing is lost (the text stays in its box) and the user is told in our words. */
function writeFailed(e: unknown, box: Box) {
	const kind = api.commentsErrorKind(e);
	if (needsReload(e)) {
		comments.status = 'reload'; // the comments and their commands belong to a session that is gone
		return;
	}
	if (kind === 'busy') return;
	if (kind === 'uncertain') {
		box.uncertain = true;
		toast.error(t('toasts.comment_write_uncertain'));
		return;
	}
	toast.error(t(kind === 'rejected' ? 'toasts.comment_write_rejected' : 'toasts.comment_write_failed'));
}

/** Show the list again from the top, in the current sort, drafts kept. */
export async function reloadComments() {
	const videoId = comments.videoId;
	if (!videoId) return;
	const sort = comments.header?.sorts.find((s) => s.selected);
	if (!sort) {
		await loadComments(videoId, true);
		return;
	}
	const g = ++gen;
	comments.status = 'loading';
	comments.threads = [];
	comments.continuation = undefined;
	comments.loadingMore = false;
	comments.moreError = false;
	comments.acting = {};
	comments.hidden = {};
	comments.create.uncertain = false;
	comments.draft.uncertain = false;
	try {
		const page = await api.getCommentsMore(sort.token);
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

/** Post what is in the top comment box. */
export async function submitCreate() {
	const box = comments.create;
	const text = box.text.trim();
	if (!text || box.pending || !comments.header?.composer) return;
	const s = session;
	box.pending = true;
	box.uncertain = false;
	try {
		const posted = await api.commentCreate(text);
		if (s !== session) return;
		box.text = '';
		const row = view(posted ? posted : { comment: localComment(text), replies: [] });
		// The comment may be listed already (the answer carried it, and so did a read since).
		if (!comments.threads.some((v) => v.comment.id === row.comment.id)) {
			comments.threads = [row, ...comments.threads];
		}
		if (comments.state === 'empty') comments.state = 'ok';
	} catch (e) {
		if (s !== session) return;
		writeFailed(e, box);
	} finally {
		if (s === session) box.pending = false;
	}
}

export function openReply(c: Comment) {
	if (comments.draft.pending) return;
	comments.draft = { ...emptyDraft(), target: { kind: 'reply', id: c.id } };
}

export function openEdit(c: Comment) {
	if (comments.draft.pending) return;
	// What the edit dialog pre-fills when the response had it plainly, else the text on screen.
	comments.draft = { ...emptyDraft(), text: c.edit_text ?? c.text, target: { kind: 'edit', id: c.id } };
}

/** Close the inline composer, restoring nothing: an edit's original text was never touched. */
export function cancelDraft() {
	if (comments.draft.pending) return;
	comments.draft = emptyDraft();
}

let localCount = 0;

/** A row for a comment that was just sent when the answer did not carry it: the viewer's own,
 *  with the text that was sent. No actions, so no buttons, until the next read replaces it. */
function localComment(text: string): Comment {
	const me = auth.account;
	return {
		id: `local-${++localCount}`,
		text,
		author: {
			name: me?.name || t('comments.you'),
			avatar: me?.thumbnail ?? undefined,
			verified: false,
			is_creator: false,
			is_artist: false
		},
		published: t('comments.just_now'),
		actions: [],
		hearted: false,
		pinned: false,
		own: true,
		writes: []
	};
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
	d.uncertain = false;
	try {
		if (target.kind === 'reply') {
			const posted = await api.commentReply(target.id, text);
			if (s !== session) return;
			comments.draft = emptyDraft();
			// Under its thread, open, from the answer or from what was sent.
			const v = threadOf(target.id);
			if (v) {
				const row = posted ?? localComment(text);
				if (!v.replies.some((r) => r.id === row.id)) v.replies = [...v.replies, row];
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
			comments.draft = emptyDraft();
		}
	} catch (e) {
		if (s !== session) return;
		if (api.commentsErrorKind(e) === 'gone') dropGone(target.id);
		else writeFailed(e, d);
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

/** The viewer confirmed: hide the row, send the delete, and put it back if it fails. The comment
 *  is passed in because the dialog closes on the same click that confirms it. */
export async function confirmDelete(c: Comment | null) {
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
		const kind = api.commentsErrorKind(e);
		if (needsReload(e)) {
			delete comments.hidden[c.id];
			comments.status = 'reload';
		} else if (kind === 'gone') {
			// Already gone is what a delete is for: nothing to say.
			removeComment(c.id);
		} else {
			delete comments.hidden[c.id];
			if (kind !== 'busy') toast.error(t('toasts.comment_delete_failed'));
		}
	}
}
