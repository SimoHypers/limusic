//! Comments: the `next`-endpoint comment threads (initial page, sort switch, pagination, replies).
//!
//! A comment is not inline in its thread. `commentThreadRenderer.commentViewModel` carries only
//! opaque keys, and the data sits in `frameworkUpdates.entityBatchUpdate.mutations[]`, joined by
//! `entityKey`. Every lookup here is tolerant: a thread whose entity is missing is skipped, and a
//! response that does not look like comments at all becomes an empty/"unavailable" page, never an
//! error.

use std::collections::HashMap;

use serde::Serialize;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Comment {
    pub id: String,
    pub text: String,
    pub author: CommentAuthor,
    /// YouTube's own display string ("6 years ago (edited)"), localized, shown as-is.
    pub published: Option<String>,
    /// Display string ("2.4M"), not a number. `None` when YouTube sends none (zero likes).
    pub like_count: Option<String>,
    pub liked: bool,
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
}

impl CommentsPage {
    /// "Unavailable" first page (no token, or a response we could not read).
    pub fn disabled() -> Self {
        CommentsPage {
            header: None,
            threads: Vec::new(),
            continuation: None,
            state: CommentsState::Disabled,
        }
    }
}

/// One page of replies, flattened.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CommentReplies {
    pub replies: Vec<Comment>,
    pub continuation: Option<String>,
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
    CommentsPage { header, threads, continuation, state }
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

    let liked = state.and_then(|s| str_of(s, "likeState")) == Some("TOOLBAR_LIKE_STATE_LIKED");
    let like_key = if liked { "likeCountLiked" } else { "likeCountNotliked" };
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
        liked,
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
        assert!(page.threads.iter().all(|t| !t.comment.liked), "anonymous read");
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
        assert!(c.liked && !c.hearted && !c.pinned);
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
