//! Comments: the `next`-endpoint comment threads (initial page, sort switch, pagination, replies).
//!
//! A comment is not inline in its thread. `commentThreadRenderer.commentViewModel` carries only
//! opaque keys, and the data sits in `frameworkUpdates.entityBatchUpdate.mutations[]`, joined by
//! `entityKey`. Every lookup here is tolerant: a thread whose entity is missing is skipped, and a
//! response that does not look like comments at all becomes an empty/"unavailable" page, never an
//! error.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::metadata::find_all;

/// Which of the two sorts a header entry switches to. Identified by position (Top first, Newest
/// second), never by `title`, which is localized by `hl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentSortKey {
    Top,
    Newest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommentSort {
    pub key: CommentSortKey,
    pub selected: bool,
    /// Pass to `comments_continuation` to load the comments in this order.
    pub token: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CommentsHeader {
    /// The short count YouTube prints ("7.8K"), display only.
    pub count_text: Option<String>,
    pub sorts: Vec<CommentSort>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommentAuthor {
    pub name: String,
    pub channel_id: Option<String>,
    pub avatar: Option<String>,
    pub verified: bool,
    pub is_creator: bool,
    pub is_artist: bool,
}

/// What the viewer has voted on a comment, from `engagementToolbarStateEntityPayload.likeState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoteState {
    Neutral,
    Liked,
    Disliked,
}

/// The four things the viewer can do to a comment's vote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentAction {
    Like,
    Unlike,
    Dislike,
    Undislike,
}

/// The opaque, server-minted token behind each action, taken from the toolbar surface entity
/// (`likeCommand` / `unlikeCommand` / `dislikeCommand` / `undislikeCommand`). They stay in Rust:
/// the field that holds them is `#[serde(skip)]`, so the type that goes to the UI cannot carry
/// one, and `Debug` says only which are present, so a stray `{:?}` cannot log one.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ActionTokens {
    like: Option<String>,
    unlike: Option<String>,
    dislike: Option<String>,
    undislike: Option<String>,
}

impl ActionTokens {
    /// The token for `action`, if the response carried one.
    pub fn get(&self, action: CommentAction) -> Option<&str> {
        match action {
            CommentAction::Like => &self.like,
            CommentAction::Unlike => &self.unlike,
            CommentAction::Dislike => &self.dislike,
            CommentAction::Undislike => &self.undislike,
        }
        .as_deref()
    }

    /// Read the four commands off an `engagementToolbarSurfaceEntityPayload`. A surface that is
    /// the signed-out one (`prepareAccountCommand`), or any command that is empty or shaped
    /// differently, simply yields no token for that action.
    fn from_surface(surface: &Value) -> Self {
        if surface.get("prepareAccountCommand").is_some() {
            return Self::default();
        }
        let token = |key: &str| surface.get(key).and_then(action_token);
        ActionTokens {
            like: token("likeCommand"),
            unlike: token("unlikeCommand"),
            dislike: token("dislikeCommand"),
            undislike: token("undislikeCommand"),
        }
    }
}

impl std::fmt::Debug for ActionTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionTokens")
            .field("like", &self.like.is_some())
            .field("unlike", &self.unlike.is_some())
            .field("dislike", &self.dislike.is_some())
            .field("undislike", &self.undislike.is_some())
            .finish()
    }
}

/// The token inside one toolbar command: `performCommentActionEndpoint.action` (a string), or the
/// first of `.actions` (an array of them), whichever the response uses, under the command's
/// `innertubeCommand` wrapper (or `command` / `performOnceCommand`). Searched for rather than
/// pinned to one path, so a wrapper YouTube adds does not lose the action.
///
/// FROM-REFERENCE youtubei.js v18.1.0 (MIT): `CommentView.applyMutations` takes the four
/// commands off the surface entity, `NavigationEndpoint` unwraps `innertubeCommand`/`command`/
/// `performOnceCommand`, and `PerformCommentActionEndpoint.buildRequest` reads `action` or
/// `actions`. The shape of a signed-in command is NOT a capture of ours: UNVERIFIED.
fn action_token(command: &Value) -> Option<String> {
    let endpoint = find_all(command, "performCommentActionEndpoint").into_iter().next()?;
    let token = endpoint
        .get("action")
        .and_then(Value::as_str)
        .or_else(|| endpoint.get("actions")?.as_array()?.first()?.as_str())?;
    (!token.is_empty()).then(|| token.to_owned())
}

/// What the viewer can do to a comment right now, given its vote and the tokens it has. An
/// action needs a token AND a vote it makes sense from; with no vote state at all (anonymous
/// read, or the state entity missing) nothing is offered, because the UI would not know which
/// button to light.
pub fn available_actions(vote: Option<VoteState>, tokens: &ActionTokens) -> Vec<CommentAction> {
    use CommentAction::*;
    let Some(vote) = vote else { return Vec::new() };
    [
        (Like, vote != VoteState::Liked),
        (Unlike, vote == VoteState::Liked),
        (Dislike, vote != VoteState::Disliked),
        (Undislike, vote == VoteState::Disliked),
    ]
    .into_iter()
    .filter(|&(action, fits)| fits && tokens.get(action).is_some())
    .map(|(action, _)| action)
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Comment {
    pub id: String,
    pub text: String,
    pub author: CommentAuthor,
    /// YouTube's own display string ("6 years ago (edited)"), localized, shown as-is.
    pub published: Option<String>,
    /// Display string ("2.4M"), not a number: the count as it reads for the viewer's current vote
    /// (the liked variant while liked). `None` when YouTube sends none (zero likes).
    pub like_count: Option<String>,
    /// The count if the viewer has not liked it, and if they have: both display strings, so an
    /// optimistic update swaps one for the other instead of adding one. Either may be missing
    /// (YouTube sends `""` for none); never guess a number.
    pub like_count_notliked: Option<String>,
    pub like_count_liked: Option<String>,
    /// The viewer's vote, from the response's own state entity. `None` when it carried none.
    pub vote: Option<VoteState>,
    /// What the viewer can do to this comment now. Empty when read anonymously, when the vote is
    /// unknown, or when the response had no token for an action.
    pub actions: Vec<CommentAction>,
    /// Never serialized. Kept on the type so the command layer can cache them in Rust.
    #[serde(skip)]
    pub tokens: ActionTokens,
    /// Display string, `None` when there are no replies.
    pub reply_count: Option<String>,
    pub hearted: bool,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommentThread {
    pub comment: Comment,
    /// Loads (more of) this thread's replies via `comment_replies`. `None` when there is nothing
    /// further to fetch.
    pub replies_token: Option<String>,
    /// Replies that came inline with the thread, nested levels flattened in order.
    pub replies: Vec<Comment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentsState {
    Ok,
    /// Comments are on, but there are none yet.
    Empty,
    /// Turned off, or the response could not be read: both read as "unavailable".
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommentsPage {
    /// Only on the first page and on a sort switch.
    pub header: Option<CommentsHeader>,
    pub threads: Vec<CommentThread>,
    /// Next page of top-level comments; `None` is the end of the list.
    pub continuation: Option<String>,
    pub state: CommentsState,
    /// `true` when the request that produced this page was sent as the signed-in account (cookie,
    /// auth header, `onBehalfOfUser`). It says NOTHING about whether viewer state is present: a
    /// request sent as the account can still come back without it, and an anonymous page can
    /// carry neutral state. What a comment offers is in its own `vote` and `actions`, and that is
    /// all the UI may go by. It is also how a token must be sent back (`as_account`). Set by the
    /// endpoint, never the parser.
    pub read_as_account: bool,
}

impl CommentsPage {
    /// "Unavailable" first page (no token, or a response we could not read).
    pub fn disabled() -> Self {
        CommentsPage {
            header: None,
            threads: Vec::new(),
            continuation: None,
            state: CommentsState::Disabled,
            read_as_account: false,
        }
    }
}

/// One page of replies, flattened.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CommentReplies {
    pub replies: Vec<Comment>,
    pub continuation: Option<String>,
    /// As [`CommentsPage::read_as_account`].
    pub read_as_account: bool,
}

/// The first-comments token off a `next` response: the one tab whose content is a section list
/// with a `reloadContinuationData`. The lyrics and related tabs are browse endpoints with no
/// content, and a disabled comments tab holds a `messageRenderer`, so "no token" is what disabled
/// means. Not matched on the tab title, which is localized.
pub fn parse_comments_token(root: &Value) -> Option<String> {
    find_all(root, "tabRenderer").into_iter().find_map(|tab| {
        tab.pointer("/content/sectionListRenderer/continuations")?.as_array()?.iter().find_map(
            |c| {
                c.pointer("/reloadContinuationData/continuation")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            },
        )
    })
}

/// Parse a comments response: the initial load or a sort switch (`header` + body), or a
/// pagination page (body only). `first` says how to read an empty result: on a first page, no
/// header means the response was not comments at all ("unavailable"); on a later page it is just
/// the end of the list.
pub fn parse_comments_page(root: &Value, first: bool) -> CommentsPage {
    let entities = Entities::new(root);
    let mut header = None;
    let mut threads = Vec::new();
    let mut continuation = None;
    for items in find_all(root, "continuationItems") {
        let Some(items) = items.as_array() else { continue };
        for item in items {
            if let Some(h) = item.get("commentsHeaderRenderer") {
                header = header.or_else(|| Some(parse_header(h)));
            } else if let Some(t) = item.get("commentThreadRenderer") {
                threads.extend(parse_thread(t, &entities));
            } else if let Some(c) = item.get("continuationItemRenderer") {
                continuation = continuation.or_else(|| item_token(c));
            }
        }
    }
    let state = if !threads.is_empty() {
        CommentsState::Ok
    } else if header.is_some() || !first {
        CommentsState::Empty
    } else {
        CommentsState::Disabled
    };
    CommentsPage { header, threads, continuation, state, read_as_account: false }
}

/// Parse a replies response: every comment in order (nested levels flattened) and the "Show more
/// replies" token, whose path differs from a normal page's.
pub fn parse_comment_replies(root: &Value) -> CommentReplies {
    let entities = Entities::new(root);
    let mut out = CommentReplies::default();
    for items in find_all(root, "continuationItems") {
        let Some(items) = items.as_array() else { continue };
        for item in items {
            if let Some(t) = item.get("commentThreadRenderer") {
                if let Some(thread) = parse_thread(t, &entities) {
                    out.replies.push(thread.comment);
                    out.replies.extend(thread.replies);
                }
            } else if let Some(c) = item.get("continuationItemRenderer") {
                out.continuation = out.continuation.take().or_else(|| item_token(c));
            }
        }
    }
    out
}

/// `frameworkUpdates` mutations by entity key.
struct Entities<'a>(HashMap<&'a str, &'a Value>);

impl<'a> Entities<'a> {
    fn new(root: &'a Value) -> Self {
        let mut map = HashMap::new();
        let muts = root.pointer("/frameworkUpdates/entityBatchUpdate/mutations");
        for m in muts.and_then(Value::as_array).into_iter().flatten() {
            if let (Some(key), Some(payload)) =
                (m.get("entityKey").and_then(Value::as_str), m.get("payload"))
            {
                map.insert(key, payload);
            }
        }
        Entities(map)
    }

    /// The `kind` payload stored under `key`.
    fn get(&self, key: Option<&str>, kind: &str) -> Option<&'a Value> {
        self.0.get(key?)?.get(kind)
    }
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// A non-empty string field (YouTube sends `""` for "none").
fn non_empty(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn parse_header(h: &Value) -> CommentsHeader {
    let count_text = h
        .pointer("/commentsCount/runs/0/text")
        .and_then(Value::as_str)
        .or_else(|| h.pointer("/countText/runs/0/text").and_then(Value::as_str))
        .map(str::to_owned);
    let items = h
        .pointer("/sortMenu/sortFilterSubMenuRenderer/subMenuItems")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let sorts = items
        .iter()
        .zip([CommentSortKey::Top, CommentSortKey::Newest])
        .filter_map(|(item, key)| {
            let token = item.pointer("/continuation/reloadContinuationData/continuation")?;
            Some(CommentSort {
                key,
                selected: item.get("selected").and_then(Value::as_bool).unwrap_or(false),
                token: token.as_str()?.to_owned(),
            })
        })
        .collect();
    CommentsHeader { count_text, sorts }
}

/// A `continuationItemRenderer`'s token. A normal page's trailing item keeps it under
/// `continuationEndpoint`; the replies' "Show more replies" item under `button`.
fn item_token(c: &Value) -> Option<String> {
    let cmd = c
        .pointer("/continuationEndpoint/continuationCommand")
        .or_else(|| c.pointer("/button/buttonRenderer/command/continuationCommand"))?;
    str_of(cmd, "token").map(str::to_owned)
}

/// A thread and its inline replies, or `None` when its comment's entity is missing.
fn parse_thread(t: &Value, entities: &Entities) -> Option<CommentThread> {
    let vm = t.pointer("/commentViewModel/commentViewModel")?;
    let pinned = vm.get("pinnedText").is_some()
        || str_of(t, "renderingPriority") == Some("RENDERING_PRIORITY_PINNED_COMMENT");
    let comment = parse_comment(vm, pinned, entities)?;

    let mut replies = Vec::new();
    let mut replies_token = None;
    let subs = t.pointer("/replies/commentRepliesRenderer/subThreads").and_then(Value::as_array);
    for sub in subs.into_iter().flatten() {
        if let Some(inner) = sub.get("commentThreadRenderer") {
            if let Some(reply) = parse_thread(inner, entities) {
                replies.push(reply.comment);
                replies.extend(reply.replies);
            }
        } else if let Some(c) = sub.get("continuationItemRenderer") {
            replies_token = replies_token.or_else(|| item_token(c));
        }
    }
    Some(CommentThread { comment, replies_token, replies })
}

fn parse_comment(vm: &Value, pinned: bool, entities: &Entities) -> Option<Comment> {
    let payload = entities.get(str_of(vm, "commentKey"), "commentEntityPayload")?;
    let props = payload.get("properties")?;
    let author = payload.get("author")?;
    let toolbar = payload.get("toolbar");
    let state = entities.get(str_of(vm, "toolbarStateKey"), "engagementToolbarStateEntityPayload");

    // A value this does not know is "no state", not an error and not neutral.
    let vote = match state.and_then(|s| str_of(s, "likeState")) {
        Some("TOOLBAR_LIKE_STATE_LIKED") => Some(VoteState::Liked),
        Some("TOOLBAR_LIKE_STATE_DISLIKED") => Some(VoteState::Disliked),
        Some("TOOLBAR_LIKE_STATE_INDIFFERENT") => Some(VoteState::Neutral),
        _ => None,
    };
    let like_key =
        if vote == Some(VoteState::Liked) { "likeCountLiked" } else { "likeCountNotliked" };
    let tokens = entities
        .get(str_of(vm, "toolbarSurfaceKey"), "engagementToolbarSurfaceEntityPayload")
        .map(ActionTokens::from_surface)
        .unwrap_or_default();
    let flag = |k: &str| author.get(k).and_then(Value::as_bool).unwrap_or(false);

    Some(Comment {
        id: str_of(props, "commentId").or_else(|| str_of(vm, "commentId"))?.to_owned(),
        text: props.pointer("/content/content").and_then(Value::as_str)?.to_owned(),
        author: CommentAuthor {
            name: str_of(author, "displayName")?.to_owned(),
            channel_id: non_empty(author.get("channelId")),
            avatar: non_empty(author.get("avatarThumbnailUrl")),
            verified: flag("isVerified"),
            is_creator: flag("isCreator"),
            is_artist: flag("isArtist"),
        },
        published: non_empty(props.get("publishedTime")),
        like_count: non_empty(toolbar.and_then(|t| t.get(like_key))),
        like_count_notliked: non_empty(toolbar.and_then(|t| t.get("likeCountNotliked"))),
        like_count_liked: non_empty(toolbar.and_then(|t| t.get("likeCountLiked"))),
        vote,
        actions: available_actions(vote, &tokens),
        tokens,
        reply_count: non_empty(toolbar.and_then(|t| t.get("replyCount"))),
        hearted: state.and_then(|s| str_of(s, "heartState")) == Some("TOOLBAR_HEART_STATE_HEARTED"),
        pinned,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const INITIAL: &str = include_str!("../../tests/fixtures/comments/initial_pinned_hearted.json");
    const PAGE2: &str = include_str!("../../tests/fixtures/comments/page2_append.json");
    const REPLIES: &str = include_str!("../../tests/fixtures/comments/replies.json");
    const EMPTY: &str = include_str!("../../tests/fixtures/comments/empty_zero.json");
    const TABS: &str = include_str!("../../tests/fixtures/comments/next_tabs.json");
    /// SYNTHETIC (see the note inside it): not a live signed-in capture.
    const SIGNED_IN: &str = include_str!("../../tests/fixtures/comments/signed_in_synthetic.json");

    fn load(s: &str) -> Value {
        serde_json::from_str(s).expect("fixture is valid JSON")
    }

    #[test]
    fn initial_page_has_header_sorts_pinned_hearted_and_verified() {
        let page = parse_comments_page(&load(INITIAL), true);
        assert_eq!(page.state, CommentsState::Ok);
        assert!(page.continuation.is_some(), "trailing continuationEndpoint token");

        let header = page.header.expect("header");
        assert!(header.count_text.is_some());
        let keys: Vec<_> = header.sorts.iter().map(|s| (s.key, s.selected)).collect();
        assert_eq!(keys, [(CommentSortKey::Top, true), (CommentSortKey::Newest, false)]);
        assert!(header.sorts.iter().all(|s| !s.token.is_empty()));

        let first = &page.threads[0];
        assert!(first.comment.pinned, "renderingPriority / pinnedText");
        assert!(first.comment.hearted, "heartState");
        assert!(first.replies_token.is_some(), "replies token off subThreads");
        assert!(first.comment.reply_count.is_some());
        assert!(page.threads.iter().any(|t| t.comment.author.verified));
        assert!(page.threads.iter().filter(|t| t.comment.pinned).count() == 1);
        // Anonymous captures: the state entity says neutral, and every command is empty, so
        // there is nothing to offer and no token.
        for t in &page.threads {
            let c = &t.comment;
            assert_eq!(c.vote, Some(VoteState::Neutral), "anonymous read");
            assert!(c.actions.is_empty());
            assert_eq!(c.tokens, ActionTokens::default());
        }
    }

    #[test]
    fn page2_append_has_threads_and_a_further_token_but_no_header() {
        let page = parse_comments_page(&load(PAGE2), false);
        assert_eq!(page.state, CommentsState::Ok);
        assert!(page.header.is_none());
        assert!(!page.threads.is_empty());
        assert!(page.continuation.is_some());
    }

    #[test]
    fn replies_flatten_nested_levels_and_read_both_token_paths() {
        let replies = parse_comment_replies(&load(REPLIES));
        assert!(replies.replies.len() >= 3, "got {}", replies.replies.len());
        // The "Show more replies" item keeps its token under button.buttonRenderer.command.
        assert!(replies.continuation.is_some());

        // First reply carries two inline level-2 replies, which come right after it.
        let page = parse_comments_page(&load(REPLIES), false);
        assert!(page.continuation.is_some(), "same button-path token via the page parser");
        assert!(replies.replies.len() > page.threads.len(), "nested replies are flattened in");

        // The normal path is covered by the initial and page-2 fixtures.
        assert!(parse_comments_page(&load(PAGE2), false).continuation.is_some());
    }

    #[test]
    fn zero_comments_is_empty_not_disabled() {
        let page = parse_comments_page(&load(EMPTY), true);
        assert_eq!(page.state, CommentsState::Empty);
        assert!(page.header.is_some());
        assert!(page.threads.is_empty());
        assert!(page.continuation.is_none());
    }

    #[test]
    fn comments_tab_token_present_when_enabled_and_absent_when_disabled() {
        let tabs = load(TABS);
        let enabled = json!({ "tabs": [{ "tabRenderer": tabs["enabledTab"].clone() }] });
        let disabled = json!({ "tabs": [{ "tabRenderer": tabs["disabledTab"].clone() }] });
        assert!(parse_comments_token(&enabled).is_some());
        assert_eq!(parse_comments_token(&disabled), None);
    }

    #[test]
    fn other_tabs_are_not_mistaken_for_comments() {
        let root = json!({ "tabs": [
            { "tabRenderer": { "title": "Lyrics", "endpoint": { "browseEndpoint": { "browseId": "MPLYtX" } } } },
            { "tabRenderer": { "title": "Up next", "content": { "musicQueueRenderer": {} } } },
        ] });
        assert_eq!(parse_comments_token(&root), None);
    }

    #[test]
    fn a_thread_without_its_entity_is_skipped() {
        let mut root = load(INITIAL);
        let all = parse_comments_page(&root, true).threads.len();
        // Drop every mutation: no thread can resolve.
        root["frameworkUpdates"]["entityBatchUpdate"]["mutations"] = json!([]);
        let page = parse_comments_page(&root, true);
        assert!(all > 0 && page.threads.is_empty());
        assert_eq!(page.state, CommentsState::Empty, "header still there, nothing to show");

        // Drop one comment's entity: only that thread goes.
        let mut root = load(INITIAL);
        let victim = root["onResponseReceivedEndpoints"][1]["reloadContinuationItemsCommand"]
            ["continuationItems"][1]["commentThreadRenderer"]["commentViewModel"]
            ["commentViewModel"]["commentKey"]
            .as_str()
            .unwrap()
            .to_owned();
        root["frameworkUpdates"]["entityBatchUpdate"]["mutations"]
            .as_array_mut()
            .unwrap()
            .retain(|m| m["entityKey"] != victim.as_str());
        assert_eq!(parse_comments_page(&root, true).threads.len(), all - 1);
    }

    /// `isCreator` / `isArtist` were never true in the live samples.
    #[test]
    fn creator_and_artist_flags_are_read() {
        let root = json!({
            "onResponseReceivedActions": [{ "appendContinuationItemsAction": { "continuationItems": [
                { "commentThreadRenderer": { "commentViewModel": { "commentViewModel": {
                    "commentKey": "k1", "toolbarStateKey": "s1", "commentId": "c1" } } } },
            ] } }],
            "frameworkUpdates": { "entityBatchUpdate": { "mutations": [
                { "entityKey": "k1", "payload": { "commentEntityPayload": {
                    "properties": { "commentId": "c1", "content": { "content": "hi" },
                                    "publishedTime": "1 day ago" },
                    "author": { "displayName": "@a", "isCreator": true, "isArtist": true,
                                "isVerified": false },
                    "toolbar": { "likeCountNotliked": "", "likeCountLiked": "1", "replyCount": "" },
                } } },
                { "entityKey": "s1", "payload": { "engagementToolbarStateEntityPayload": {
                    "likeState": "TOOLBAR_LIKE_STATE_LIKED",
                    "heartState": "TOOLBAR_HEART_STATE_UNHEARTED" } } },
            ] } },
        });
        let page = parse_comments_page(&root, false);
        let c = &page.threads[0].comment;
        assert!(c.author.is_creator && c.author.is_artist && !c.author.verified);
        assert!(c.vote == Some(VoteState::Liked) && !c.hearted && !c.pinned);
        assert_eq!(c.like_count.as_deref(), Some("1"), "the liked count while liked");
        assert_eq!(c.reply_count, None, "empty string is no count");
        assert_eq!(c.author.channel_id, None);
    }

    /// A thread whose commentViewModel points at entity `key`, replying inline with `subs`.
    fn thread(key: &str, subs: Vec<Value>) -> Value {
        let mut t = json!({ "commentViewModel": { "commentViewModel": {
            "commentKey": key, "toolbarStateKey": "none", "commentId": key } } });
        if !subs.is_empty() {
            t["replies"] = json!({ "commentRepliesRenderer": { "subThreads": subs } });
        }
        json!({ "commentThreadRenderer": t })
    }

    fn mutation(key: &str) -> Value {
        json!({ "entityKey": key, "payload": { "commentEntityPayload": {
            "properties": { "commentId": key, "content": { "content": key } },
            "author": { "displayName": "@a" },
        } } })
    }

    fn response(items: Vec<Value>, mutation_keys: &[&str]) -> Value {
        json!({
            "onResponseReceivedActions": [{ "appendContinuationItemsAction": {
                "continuationItems": items } }],
            "frameworkUpdates": { "entityBatchUpdate": { "mutations":
                mutation_keys.iter().map(|k| mutation(k)).collect::<Vec<_>>() } },
        })
    }

    /// Threads come out in the order of `continuationItems`: not the order of the entity
    /// mutations, not alphabetical, and not whatever a hash map happens to iterate in.
    #[test]
    fn threads_keep_the_order_of_the_continuation_items() {
        let items = vec![thread("c3", vec![]), thread("c1", vec![]), thread("c2", vec![])];
        let root = response(items, &["c2", "c3", "c1"]);
        let ids: Vec<_> =
            parse_comments_page(&root, false).threads.into_iter().map(|t| t.comment.id).collect();
        assert_eq!(ids, ["c3", "c1", "c2"]);
    }

    /// Same for replies, nested levels included: a reply is followed by its own inline replies,
    /// in the order they were sent.
    #[test]
    fn replies_keep_response_order_with_nested_levels_in_place() {
        let items = vec![
            thread("r2", vec![thread("n2b", vec![]), thread("n2a", vec![])]),
            thread("r1", vec![]),
        ];
        let root = response(items, &["r1", "n2a", "r2", "n2b"]);
        let flat: Vec<_> = parse_comment_replies(&root).replies.into_iter().map(|c| c.id).collect();
        assert_eq!(flat, ["r2", "n2b", "n2a", "r1"]);

        // And inline replies on a top-level thread.
        let page = parse_comments_page(&root, false);
        let inline: Vec<_> = page.threads[0].replies.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(inline, ["n2b", "n2a"]);
    }

    /// Top is the first entry and Newest the second whatever the titles say (they are localized),
    /// and `selected` is whatever the response says, not a default.
    #[test]
    fn sorts_follow_menu_order_and_selected_comes_from_the_response() {
        let menu = |items: Value| {
            json!({ "continuationItems": [{ "commentsHeaderRenderer": { "sortMenu": {
                "sortFilterSubMenuRenderer": { "subMenuItems": items } } } }] })
        };
        let entry = |title: &str, selected: bool, token: &str| {
            json!({ "title": title, "selected": selected,
                    "continuation": { "reloadContinuationData": { "continuation": token } } })
        };
        let root =
            menu(json!([entry("Beste", false, "tok-top"), entry("Neueste", true, "tok-new")]));
        let page = parse_comments_page(&root, true);
        let sorts = page.header.unwrap().sorts;
        assert_eq!(
            sorts,
            [
                CommentSort { key: CommentSortKey::Top, selected: false, token: "tok-top".into() },
                CommentSort {
                    key: CommentSortKey::Newest,
                    selected: true,
                    token: "tok-new".into()
                },
            ]
        );

        // A missing `selected` is "not selected", and an entry with no token is dropped without
        // shifting the other one into its slot.
        let root = menu(json!([{ "title": "x" }, entry("y", true, "tok-new")]));
        let sorts = parse_comments_page(&root, true).header.unwrap().sorts;
        assert_eq!(sorts.len(), 1);
        assert_eq!((sorts[0].key, sorts[0].selected), (CommentSortKey::Newest, true));
    }

    fn synthetic_page() -> CommentsPage {
        parse_comments_page(&load(SIGNED_IN), false)
    }

    fn actions_of(page: &CommentsPage, name: &str) -> (Option<VoteState>, Vec<CommentAction>) {
        let c = &page
            .threads
            .iter()
            .find(|t| t.comment.text.ends_with(name))
            .unwrap_or_else(|| panic!("no {name} thread"))
            .comment;
        (c.vote, c.actions.clone())
    }

    /// What each synthetic case reads as. The vote comes from the state entity and decides which
    /// actions make sense; a token has to be present for each one.
    #[test]
    fn viewer_state_and_available_actions_follow_the_response() {
        use CommentAction::*;
        let page = synthetic_page();
        assert_eq!(page.threads.len(), 8);
        let n = VoteState::Neutral;
        let want = [
            ("neutral", Some(n), vec![Like, Dislike]),
            ("liked", Some(VoteState::Liked), vec![Unlike, Dislike]),
            ("disliked", Some(VoteState::Disliked), vec![Like, Undislike]),
            // Commands empty: the signed-out shape. State is neutral, nothing is offered.
            ("empty_commands", Some(n), vec![]),
            // State entity missing: tokens exist, but with no vote nothing can be offered.
            ("state_missing", None, vec![]),
            // Only the like command exists: only like is offered.
            ("like_only", Some(n), vec![Like]),
            // A signed-out surface (prepareAccountCommand) offers nothing even beside a command.
            ("signed_out_surface", Some(n), vec![]),
            // A likeState this parser does not know is no state, not an error, not neutral.
            ("unknown_state", None, vec![]),
        ];
        for (name, vote, actions) in want {
            assert_eq!(actions_of(&page, name), (vote, actions), "{name}");
        }
    }

    /// Both count strings come through as sent, and `like_count` is the one for the current vote.
    /// `""` is no count, never zero.
    #[test]
    fn both_count_strings_are_exposed_and_never_invented() {
        let page = synthetic_page();
        let by = |name: &str| {
            &page.threads.iter().find(|t| t.comment.text.ends_with(name)).unwrap().comment
        };
        let neutral = by("neutral");
        assert_eq!(neutral.like_count_notliked.as_deref(), Some("41"));
        assert_eq!(neutral.like_count_liked.as_deref(), Some("42"));
        assert_eq!(neutral.like_count.as_deref(), Some("41"));
        assert_eq!(by("liked").like_count.as_deref(), Some("42"), "the liked variant while liked");
        assert_eq!(by("disliked").like_count.as_deref(), Some("41"));
        let zero = by("empty_commands");
        assert_eq!(zero.like_count_notliked, None, "\"\" is no count");
        assert_eq!(zero.like_count_liked.as_deref(), Some("1"));
        assert_eq!(zero.like_count, None);
    }

    /// The tokens are in Rust and nowhere else: not in what the UI is sent, not in `Debug`.
    #[test]
    fn tokens_are_kept_but_never_serialized_or_printed() {
        let page = synthetic_page();
        let neutral = &page.threads[0].comment;
        assert_eq!(neutral.tokens.get(CommentAction::Like), Some("fixture-token-like-1"));
        assert_eq!(neutral.tokens.get(CommentAction::Unlike), Some("fixture-token-unlike-1"));
        assert_eq!(neutral.tokens.get(CommentAction::Dislike), Some("fixture-token-dislike-1"));
        assert_eq!(neutral.tokens.get(CommentAction::Undislike), Some("fixture-token-undislike-1"));
        // Present even where nothing is offered, so a later step can decide on its own.
        let missing_state = page.threads.iter().find(|t| t.comment.vote.is_none()).unwrap();
        assert!(missing_state.comment.tokens.get(CommentAction::Like).is_some());

        let sent = serde_json::to_string(&page).unwrap();
        assert!(!sent.contains("fixture-token"), "a token reached the serialized page");
        assert!(sent.contains("\"actions\""), "the actions themselves do");
        for printed in [format!("{page:?}"), format!("{:?}", neutral.tokens)] {
            assert!(!printed.contains("fixture-token"), "a token reached Debug output");
        }
    }

    /// The array form of the endpoint (`actions: [token]`) and a differently wrapped command read
    /// too; a token that is not a string, or empty, is no token.
    #[test]
    fn token_extraction_is_tolerant_about_shape() {
        let wrapped =
            |inner: Value| json!({ "innertubeCommand": { "performCommentActionEndpoint": inner } });
        assert_eq!(action_token(&wrapped(json!({ "action": "T1" }))).as_deref(), Some("T1"));
        assert_eq!(
            action_token(&wrapped(json!({ "actions": ["T2", "T3"] }))).as_deref(),
            Some("T2")
        );
        let bare = json!({ "command": { "performCommentActionEndpoint": { "action": "T4" } } });
        assert_eq!(action_token(&bare).as_deref(), Some("T4"));
        for junk in [
            json!(null),
            json!(5),
            json!({ "innertubeCommand": {} }),
            wrapped(json!({ "action": "" })),
            wrapped(json!({ "action": 7 })),
            wrapped(json!({ "actions": [] })),
            wrapped(json!({ "actions": [null] })),
            wrapped(json!("not an object")),
        ] {
            assert_eq!(action_token(&junk), None, "{junk}");
        }
    }

    /// Replies share the toolbar payloads with top-level comments: the same parser gives them
    /// their own vote and tokens (UNVERIFIED that a live reply carries them).
    #[test]
    fn a_reply_with_the_same_toolbar_payloads_gets_the_same_state() {
        let root = load(SIGNED_IN);
        let replies = parse_comment_replies(&root);
        assert_eq!(replies.replies.len(), 8);
        assert!(!replies.read_as_account, "the endpoint sets that, not the parser");
        let liked = replies.replies.iter().find(|c| c.text.ends_with("liked")).unwrap();
        assert_eq!(liked.vote, Some(VoteState::Liked));
        assert_eq!(liked.actions, [CommentAction::Unlike, CommentAction::Dislike]);
    }

    #[test]
    fn availability_needs_both_a_vote_and_a_token() {
        let none = ActionTokens::default();
        for vote in
            [None, Some(VoteState::Neutral), Some(VoteState::Liked), Some(VoteState::Disliked)]
        {
            assert!(available_actions(vote, &none).is_empty(), "no tokens, nothing: {vote:?}");
        }
        let all = ActionTokens {
            like: Some("a".into()),
            unlike: Some("b".into()),
            dislike: Some("c".into()),
            undislike: Some("d".into()),
        };
        assert!(available_actions(None, &all).is_empty(), "no vote, nothing");
        // Never an action that does not fit the vote, even with every token present.
        assert_eq!(
            available_actions(Some(VoteState::Liked), &all),
            [CommentAction::Unlike, CommentAction::Dislike]
        );
        assert_eq!(
            available_actions(Some(VoteState::Disliked), &all),
            [CommentAction::Like, CommentAction::Undislike]
        );
    }

    #[test]
    fn unexpected_json_never_panics() {
        for v in [
            json!(null),
            json!([]),
            json!("x"),
            json!({ "onResponseReceivedEndpoints": "no" }),
            json!({ "x": { "continuationItems": 3 } }),
            json!({ "x": { "continuationItems": [null, 1, { "commentThreadRenderer": 5 },
                { "commentThreadRenderer": { "commentViewModel": { "commentViewModel": 7 } } },
                { "continuationItemRenderer": { "continuationEndpoint": 9 } },
                { "commentsHeaderRenderer": { "sortMenu": 4 } }] } }),
            json!({ "frameworkUpdates": { "entityBatchUpdate": { "mutations": [null, 3] } } }),
            json!({ "frameworkUpdates": { "entityBatchUpdate": { "mutations": [
                { "entityKey": "s", "payload": { "engagementToolbarSurfaceEntityPayload": {
                    "likeCommand": 5, "unlikeCommand": [], "dislikeCommand": { "innertubeCommand": 7 },
                    "undislikeCommand": { "performCommentActionEndpoint": { "actions": "x" } } } } },
                { "entityKey": "t", "payload": { "engagementToolbarStateEntityPayload": {
                    "likeState": 3 } } },
            ] } } }),
        ] {
            let _ = parse_comments_page(&v, true);
            let _ = parse_comments_page(&v, false);
            let _ = parse_comment_replies(&v);
            let _ = parse_comments_token(&v);
        }
        // Unreadable first page = unavailable; unreadable later page = end of list.
        assert_eq!(parse_comments_page(&json!({}), true).state, CommentsState::Disabled);
        assert_eq!(parse_comments_page(&json!({}), false).state, CommentsState::Empty);
    }
}
