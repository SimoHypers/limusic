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

use super::comment_write::{
    action_token, create_command, menu_commands, reply_command, CommentWrite, Composer,
    MenuCommands, WriteCommand, WriteCommands,
};
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
    /// The comment box, only when the response carried a command to post with (signed in).
    pub composer: Option<Composer>,
    /// That command. Never serialized: it stays in Rust.
    #[serde(skip)]
    pub create: Option<WriteCommand>,
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

impl CommentAction {
    /// The word for it in log lines.
    pub fn name(self) -> &'static str {
        match self {
            CommentAction::Like => "like",
            CommentAction::Unlike => "unlike",
            CommentAction::Dislike => "dislike",
            CommentAction::Undislike => "undislike",
        }
    }

    /// The vote a comment has once YouTube accepts this action.
    pub fn resulting_vote(self) -> VoteState {
        match self {
            CommentAction::Like => VoteState::Liked,
            CommentAction::Dislike => VoteState::Disliked,
            CommentAction::Unlike | CommentAction::Undislike => VoteState::Neutral,
        }
    }
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
    /// The viewer's own comment (`author.isCurrentUser`).
    pub own: bool,
    /// What the edit dialog pre-fills (`editableText`), when the response had it in a plain shape;
    /// the UI edits `text` otherwise.
    pub edit_text: Option<String>,
    /// What the viewer can write on it: exactly the commands the response carried. Edit and
    /// delete only on one's own. Empty read anonymously.
    pub writes: Vec<CommentWrite>,
    /// The reply box's placeholder, if the response sent one.
    pub reply_placeholder: Option<String>,
    /// The commands behind `writes`. Never serialized.
    #[serde(skip)]
    pub commands: WriteCommands,
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
    /// The request was sent as the signed-in account, so its tokens go back that way. Set by the
    /// endpoint; not sent to the UI.
    #[serde(skip)]
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
    let (composer, create) = match create_command(h) {
        Some((composer, create)) => (Some(composer), Some(create)),
        None => (None, None),
    };
    CommentsHeader { count_text, sorts, composer, create }
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
    let surface =
        entities.get(str_of(vm, "toolbarSurfaceKey"), "engagementToolbarSurfaceEntityPayload");
    let tokens = surface.map(ActionTokens::from_surface).unwrap_or_default();
    let own = author.get("isCurrentUser").and_then(Value::as_bool).unwrap_or(false);
    let (reply_placeholder, reply) = match surface.and_then(reply_command) {
        Some((placeholder, command)) => (placeholder, Some(command)),
        None => (None, None),
    };
    // Edit and delete: the viewer's own comment's menu, by structure, and nowhere else.
    let menu = match (own, surface) {
        (true, Some(surface)) => menu_commands(surface),
        _ => MenuCommands::default(),
    };
    let edit_text = menu.edit_text;
    let commands = WriteCommands { reply, edit: menu.edit, delete: menu.delete };
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
        own,
        edit_text,
        writes: commands.offered(),
        reply_placeholder,
        commands,
        reply_count: non_empty(toolbar.and_then(|t| t.get("replyCount"))),
        hearted: state.and_then(|s| str_of(s, "heartState")) == Some("TOOLBAR_HEART_STATE_HEARTED"),
        pinned,
    })
}

/// The comment a write put on the page: the answer's `commentThreadRenderer`, with its entities in
/// `frameworkUpdates`. `None` when the answer holds none.
pub fn parse_written_comment(root: &Value) -> Option<CommentThread> {
    let entities = Entities::new(root);
    find_all(root, "commentThreadRenderer").into_iter().find_map(|t| parse_thread(t, &entities))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::models::comment_write::tests::{delete_item, edit_item, menu, reply_edit_item};
    use serde_json::json;

    const INITIAL: &str = include_str!("../../tests/fixtures/comments/initial_pinned_hearted.json");
    const REPLIES: &str = include_str!("../../tests/fixtures/comments/replies.json");
    const TABS: &str = include_str!("../../tests/fixtures/comments/next_tabs.json");

    pub(crate) fn load(s: &str) -> Value {
        serde_json::from_str(s).expect("fixture is valid JSON")
    }

    // --- builders for signed-in pages ------------------------------------------------------

    /// One comment as a read carries it: its thread item and its entity mutations.
    pub(crate) struct Case {
        pub item: Value,
        pub mutations: Vec<Value>,
    }

    /// A comment with id `id` (and text `Comment <id>`), the viewer's own or not, with a toolbar
    /// state entity when `like_state` is given, a toolbar surface entity when `surface` is, and
    /// `subs` as its inline replies.
    pub(crate) fn case(
        id: &str,
        own: bool,
        like_state: Option<&str>,
        surface: Option<Value>,
        subs: Vec<Case>,
    ) -> Case {
        let mut vm = json!({ "commentKey": format!("{id}-key"),
            "toolbarStateKey": format!("{id}-state"), "commentId": id });
        let mut mutations = vec![json!({ "entityKey": format!("{id}-key"), "payload": {
            "commentEntityPayload": {
                "properties": { "commentId": id, "content": { "content": format!("Comment {id}") },
                                "publishedTime": "1 day ago" },
                "author": { "displayName": format!("@{id}"), "channelId": "UCfixture",
                            "isCurrentUser": own },
                "toolbar": { "likeCountNotliked": "41", "likeCountLiked": "42", "replyCount": "" } } } })];
        if let Some(state) = like_state {
            mutations.push(json!({ "entityKey": format!("{id}-state"), "payload": {
                "engagementToolbarStateEntityPayload": { "likeState": state,
                    "heartState": "TOOLBAR_HEART_STATE_UNHEARTED" } } }));
        }
        if let Some(surface) = surface {
            vm["toolbarSurfaceKey"] = json!(format!("{id}-surface"));
            mutations.push(json!({ "entityKey": format!("{id}-surface"), "payload": {
                "engagementToolbarSurfaceEntityPayload": surface } }));
        }
        let mut thread = json!({ "commentViewModel": { "commentViewModel": vm } });
        if !subs.is_empty() {
            let items: Vec<Value> = subs.iter().map(|s| s.item.clone()).collect();
            thread["replies"] = json!({ "commentRepliesRenderer": { "subThreads": items } });
            mutations.extend(subs.into_iter().flat_map(|s| s.mutations));
        }
        Case { item: json!({ "commentThreadRenderer": thread }), mutations }
    }

    /// A first page or a replies page: an optional header, the cases, and a trailing item.
    pub(crate) fn page(header: Option<Value>, cases: Vec<Case>, trailing: Option<Value>) -> Value {
        let mut items: Vec<Value> =
            header.into_iter().map(|h| json!({ "commentsHeaderRenderer": h })).collect();
        let mut mutations = Vec::new();
        for c in cases {
            items.push(c.item);
            mutations.extend(c.mutations);
        }
        items.extend(trailing);
        json!({
            "onResponseReceivedEndpoints": [{ "reloadContinuationItemsCommand": {
                "continuationItems": items } }],
            "frameworkUpdates": { "entityBatchUpdate": { "mutations": mutations } },
        })
    }

    /// A normal page's trailing "next page" item.
    pub(crate) fn continuation_item(token: &str) -> Value {
        json!({ "continuationItemRenderer": { "continuationEndpoint": {
            "continuationCommand": { "token": token } } } })
    }

    pub(crate) const ALL_VOTES: [&str; 4] = ["like", "unlike", "dislike", "undislike"];
    const NEUTRAL: Option<&str> = Some("TOOLBAR_LIKE_STATE_INDIFFERENT");

    /// A toolbar surface with a `<action>Command` for each of `actions`, whose token is
    /// `fixture-token-<action>-<id>`.
    pub(crate) fn votes(id: &str, actions: &[&str]) -> Value {
        let mut surface = json!({});
        for a in actions {
            surface[format!("{a}Command")] = json!({ "innertubeCommand": {
                "performCommentActionEndpoint": { "action": format!("fixture-token-{a}-{id}") } } });
        }
        surface
    }

    /// Every vote case a signed-in read can hold, by id, and a next-page token.
    pub(crate) fn vote_page() -> Value {
        let all = |id: &str| Some(votes(id, &ALL_VOTES));
        let mut empty = json!({});
        for a in ALL_VOTES {
            empty[format!("{a}Command")] = json!({ "innertubeCommand": {} });
        }
        let mut signed_out = votes("signed_out_surface", &["like"]);
        signed_out["prepareAccountCommand"] = json!({});
        let cases = vec![
            case("neutral", false, NEUTRAL, all("neutral"), vec![]),
            case("liked", false, Some("TOOLBAR_LIKE_STATE_LIKED"), all("liked"), vec![]),
            case("disliked", false, Some("TOOLBAR_LIKE_STATE_DISLIKED"), all("disliked"), vec![]),
            case("empty_commands", false, NEUTRAL, Some(empty), vec![]),
            case("state_missing", false, None, all("state_missing"), vec![]),
            case("like_only", false, NEUTRAL, Some(votes("like_only", &["like"])), vec![]),
            case("signed_out_surface", false, NEUTRAL, Some(signed_out), vec![]),
            case(
                "unknown_state",
                false,
                Some("TOOLBAR_LIKE_STATE_NEW"),
                all("unknown_state"),
                vec![],
            ),
        ];
        page(None, cases, Some(continuation_item("fixture-next")))
    }

    /// A toolbar surface with every vote, a reply button and a menu of `items`.
    pub(crate) fn write_surface(id: &str, items: Vec<Value>) -> Value {
        let mut surface = votes(id, &ALL_VOTES);
        surface["replyCommand"] = json!({ "innertubeCommand": { "createCommentReplyDialogEndpoint": {
            "dialog": { "commentReplyDialogRenderer": {
                "placeholderText": { "runs": [{ "text": "Add a reply..." }] },
                "replyButton": { "buttonRenderer": { "serviceEndpoint": { "createCommentReplyEndpoint": {
                    "createReplyParams": format!("fixture-reply-params-{id}") } } } } } } } } });
        if !items.is_empty() {
            surface["menuCommand"] = menu(items)["menuCommand"].clone();
        }
        surface
    }

    /// An Edit and a Delete menu item, as a comment's own menu carries them.
    fn own_menu(id: &str) -> Vec<Value> {
        vec![
            edit_item(
                Some("EDIT"),
                Some("/youtubei/v1/comment/update_comment"),
                json!(format!("fixture-edit-params-{id}")),
            ),
            delete_item(Some("DELETE"), json!({ "action": format!("fixture-delete-token-{id}") })),
        ]
    }

    /// A signed-in page with a comment box: the viewer's own comment and someone else's with
    /// identical menus, an own comment with no menu, and a comment with no reply button.
    pub(crate) fn write_page() -> Value {
        let header = json!({ "createRenderer": { "commentSimpleboxRenderer": {
            "placeholderText": { "runs": [{ "text": "Add a comment..." }] },
            "submitButton": { "buttonRenderer": { "serviceEndpoint": { "createCommentEndpoint": {
                "createCommentParams": "fixture-create-params" } } } } } } });
        let cases = vec![
            case(
                "own_full",
                true,
                NEUTRAL,
                Some(write_surface("own_full", own_menu("own_full"))),
                vec![],
            ),
            case("other", false, NEUTRAL, Some(write_surface("other", own_menu("other"))), vec![]),
            case("own_bare", true, NEUTRAL, Some(write_surface("own_bare", vec![])), vec![]),
            case("no_reply", false, NEUTRAL, Some(votes("no_reply", &ALL_VOTES)), vec![]),
        ];
        page(Some(header), cases, None)
    }

    /// A replies page: the viewer's own reply and someone else's, the latter with an own and an
    /// other nested reply, and a "Show more replies" button.
    pub(crate) fn replies_write_page() -> Value {
        let reply_menu = |id: &str, api_url: Option<&str>| {
            vec![
                reply_edit_item(
                    Some("EDIT"),
                    api_url,
                    json!(format!("fixture-edit-reply-params-{id}")),
                ),
                delete_item(
                    Some("DELETE"),
                    json!({ "action": format!("fixture-delete-token-{id}") }),
                ),
            ]
        };
        let reply = |id: &str, own: bool, api_url: Option<&str>, subs: Vec<Case>| {
            case(id, own, NEUTRAL, Some(write_surface(id, reply_menu(id, api_url))), subs)
        };
        let nested_url = Some("/youtubei/v1/comment/update_comment_reply");
        let cases = vec![
            reply("own_reply", true, None, vec![]),
            reply(
                "other_reply",
                false,
                None,
                vec![
                    reply("nested_own_reply", true, nested_url, vec![]),
                    reply("nested_other_reply", false, nested_url, vec![]),
                ],
            ),
        ];
        let more = json!({ "continuationItemRenderer": { "button": { "buttonRenderer": {
            "command": { "continuationCommand": { "token": "fixture-more-replies" } } } } } });
        page(None, cases, Some(more))
    }

    fn comment<'a>(page: &'a CommentsPage, id: &str) -> &'a Comment {
        &page
            .threads
            .iter()
            .find(|t| t.comment.id == id)
            .unwrap_or_else(|| panic!("no {id}"))
            .comment
    }

    // --- anonymous reads (anonymised real responses) --------------------------------------

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
        // Read anonymously: the state is neutral and every command is empty, so nothing is
        // offered.
        for t in &page.threads {
            let c = &t.comment;
            assert_eq!(c.vote, Some(VoteState::Neutral), "anonymous read");
            assert!(c.actions.is_empty());
            assert_eq!(c.tokens, ActionTokens::default());
        }
    }

    #[test]
    fn an_appended_page_has_threads_and_a_further_token_but_no_header() {
        let mut items = vec![thread("c1", vec![]), thread("c2", vec![])];
        items.push(continuation_item("tok-page-3"));
        let page = parse_comments_page(&response(items, &["c1", "c2"]), false);
        assert_eq!(page.state, CommentsState::Ok);
        assert!(page.header.is_none());
        assert_eq!(page.threads.len(), 2);
        assert_eq!(page.continuation.as_deref(), Some("tok-page-3"));
    }

    #[test]
    fn replies_flatten_nested_levels_and_read_the_button_token_path() {
        let replies = parse_comment_replies(&load(REPLIES));
        assert!(replies.replies.len() >= 3, "got {}", replies.replies.len());
        // The "Show more replies" item keeps its token under button.buttonRenderer.command.
        assert!(replies.continuation.is_some());

        // First reply carries two inline level-2 replies, which come right after it.
        let page = parse_comments_page(&load(REPLIES), false);
        assert!(page.continuation.is_some(), "same button-path token via the page parser");
        assert!(replies.replies.len() > page.threads.len(), "nested replies are flattened in");
    }

    #[test]
    fn zero_comments_is_empty_not_disabled() {
        let root = json!({ "onResponseReceivedEndpoints": [
            { "reloadContinuationItemsCommand": { "slot": "RELOAD_CONTINUATION_SLOT_HEADER",
                "continuationItems": [{ "commentsHeaderRenderer": {
                    "countText": { "runs": [{ "text": "0" }] } } }] } },
            { "reloadContinuationItemsCommand": { "slot": "RELOAD_CONTINUATION_SLOT_BODY" } },
        ] });
        let page = parse_comments_page(&root, true);
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

    // --- signed-in votes ---------------------------------------------------------------------

    /// The vote comes from the state entity and decides which actions make sense; a token has to
    /// be present for each one.
    #[test]
    fn viewer_state_and_available_actions_follow_the_response() {
        use CommentAction::*;
        let page = parse_comments_page(&vote_page(), false);
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
        for (id, vote, actions) in want {
            let c = comment(&page, id);
            assert_eq!((c.vote, c.actions.clone()), (vote, actions), "{id}");
        }
    }

    /// Both count strings come through as sent, and `like_count` is the one for the current vote.
    /// `""` is no count, never zero.
    #[test]
    fn both_count_strings_are_exposed_and_never_invented() {
        let page = parse_comments_page(&vote_page(), false);
        let neutral = comment(&page, "neutral");
        assert_eq!(neutral.like_count_notliked.as_deref(), Some("41"));
        assert_eq!(neutral.like_count_liked.as_deref(), Some("42"));
        assert_eq!(neutral.like_count.as_deref(), Some("41"));
        assert_eq!(comment(&page, "liked").like_count.as_deref(), Some("42"), "liked variant");
        assert_eq!(comment(&page, "disliked").like_count.as_deref(), Some("41"));

        let mut zero = case("zero", false, NEUTRAL, None, vec![]);
        zero.mutations[0]["payload"]["commentEntityPayload"]["toolbar"] =
            json!({ "likeCountNotliked": "", "likeCountLiked": "1", "replyCount": "" });
        let page = parse_comments_page(&page_of(zero), false);
        let zero = comment(&page, "zero");
        assert_eq!(zero.like_count_notliked, None, "\"\" is no count");
        assert_eq!(zero.like_count_liked.as_deref(), Some("1"));
        assert_eq!(zero.like_count, None);
    }

    fn page_of(c: Case) -> Value {
        page(None, vec![c], None)
    }

    /// The tokens are in Rust and nowhere else: not in what the UI is sent, not in `Debug`.
    #[test]
    fn tokens_are_kept_but_never_serialized_or_printed() {
        let page = parse_comments_page(&vote_page(), false);
        let neutral = comment(&page, "neutral");
        for (action, name) in [
            (CommentAction::Like, "like"),
            (CommentAction::Unlike, "unlike"),
            (CommentAction::Dislike, "dislike"),
            (CommentAction::Undislike, "undislike"),
        ] {
            assert_eq!(
                neutral.tokens.get(action),
                Some(format!("fixture-token-{name}-neutral").as_str())
            );
        }
        // Kept even where nothing is offered.
        assert!(comment(&page, "state_missing").tokens.get(CommentAction::Like).is_some());

        let sent = serde_json::to_string(&page).unwrap();
        assert!(!sent.contains("fixture-token"), "a token reached the serialized page");
        assert!(sent.contains("\"actions\""), "the actions themselves do");
        assert!(!sent.contains("read_as_account"), "how a page was read stays in Rust");
        for printed in [format!("{page:?}"), format!("{:?}", neutral.tokens)] {
            assert!(!printed.contains("fixture-token"), "a token reached Debug output");
        }
    }

    /// Replies share the toolbar payloads with top-level comments: the same parser gives them
    /// their own vote and tokens.
    #[test]
    fn a_reply_with_the_same_toolbar_payloads_gets_the_same_state() {
        let replies = parse_comment_replies(&vote_page());
        assert_eq!(replies.replies.len(), 8);
        let liked = replies.replies.iter().find(|c| c.id == "liked").unwrap();
        assert_eq!(liked.vote, Some(VoteState::Liked));
        assert_eq!(liked.actions, [CommentAction::Unlike, CommentAction::Dislike]);
    }

    // --- signed-in writes --------------------------------------------------------------------

    /// Reply wherever the response carried a reply button; edit and delete only on the viewer's
    /// own comments, and only where the menu has them. Someone else's comment with the very same
    /// menu offers neither.
    #[test]
    fn writes_follow_the_commands_the_response_carried() {
        use CommentWrite::*;
        let page = parse_comments_page(&write_page(), true);
        for (id, own, writes) in [
            ("own_full", true, vec![Reply, Edit, Delete]),
            ("other", false, vec![Reply]),
            ("own_bare", true, vec![Reply]),
            ("no_reply", false, vec![]),
        ] {
            let c = comment(&page, id);
            assert_eq!((c.own, c.writes.clone()), (own, writes), "{id}");
        }
    }

    #[test]
    fn the_write_commands_are_the_servers_and_stay_in_rust() {
        let page = parse_comments_page(&write_page(), true);
        let own = comment(&page, "own_full");
        match own.commands.reply.as_ref().unwrap() {
            WriteCommand::Endpoint { path, payload, .. } => {
                assert_eq!(path, "comment/create_comment_reply");
                assert_eq!(payload["createReplyParams"], "fixture-reply-params-own_full");
            }
            other => panic!("{other:?}"),
        }
        match own.commands.edit.as_ref().unwrap() {
            WriteCommand::Endpoint { path, payload, .. } => {
                assert_eq!(path, "comment/update_comment");
                assert_eq!(payload.len(), 1, "only the update params are replayed");
                assert_eq!(payload["updateCommentParams"], "fixture-edit-params-own_full");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(own.edit_text.as_deref(), Some("old text"));
        // Delete is the confirm button's action token, replayed like a vote.
        assert_eq!(
            own.commands.delete,
            Some(WriteCommand::Action("fixture-delete-token-own_full".into()))
        );
        let bare = comment(&page, "own_bare");
        assert!(bare.commands.edit.is_none() && bare.edit_text.is_none());
        assert_eq!(own.reply_placeholder.as_deref(), Some("Add a reply..."));

        // The UI is sent which writes are offered and nothing that could replay one.
        let sent = serde_json::to_string(&page).unwrap();
        assert!(sent.contains("\"writes\":[\"reply\",\"edit\",\"delete\"]"), "{sent}");
        // Params and tokens are in neither the serialized page nor Debug output (the endpoint
        // path, which is not secret, may show in Debug but never reaches the UI).
        for secret in [
            "fixture-reply-params",
            "fixture-edit-params",
            "fixture-delete",
            "fixture-create-params",
        ] {
            assert!(!sent.contains(secret), "`{secret}` reached the serialized page");
            assert!(!format!("{page:?}").contains(secret), "`{secret}` reached Debug output");
        }
        assert!(!sent.contains("update_comment") && !sent.contains("create_comment"));
    }

    #[test]
    fn the_composer_comes_from_the_header_and_only_when_it_can_post() {
        let header = parse_comments_page(&write_page(), true).header.unwrap();
        assert_eq!(
            header.composer.as_ref().unwrap().placeholder.as_deref(),
            Some("Add a comment...")
        );
        match header.create.as_ref().unwrap() {
            WriteCommand::Endpoint { path, payload, .. } => {
                assert_eq!(path, crate::models::comment_write::COMMENT_CREATE_PATH);
                assert_eq!(payload["createCommentParams"], "fixture-create-params");
            }
            other => panic!("{other:?}"),
        }
        // Read anonymously, the comment box has no submit button: no composer, no commands, and
        // nothing to write on any comment.
        let anon = parse_comments_page(&load(INITIAL), true);
        let header = anon.header.unwrap();
        assert!(header.composer.is_none() && header.create.is_none());
        assert!(anon.threads.iter().all(|t| t.comment.writes.is_empty() && !t.comment.own));
        assert!(!serde_json::to_string(&header).unwrap().contains("createCommentParams"));
    }

    /// Replies are parsed like any comment: on a replies page, and for the inline nested levels,
    /// the viewer's own get Edit and Delete from their menu, and someone else's do not.
    #[test]
    fn a_replies_page_offers_edit_and_delete_on_the_viewers_own_replies_only() {
        use CommentWrite::*;
        let replies = parse_comment_replies(&replies_write_page());
        // The nested replies of "other_reply" follow it, flattened.
        let order: Vec<&str> = replies.replies.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(order, ["own_reply", "other_reply", "nested_own_reply", "nested_other_reply"]);
        let writes: Vec<(bool, Vec<CommentWrite>)> =
            replies.replies.iter().map(|c| (c.own, c.writes.clone())).collect();
        assert_eq!(
            writes,
            [
                (true, vec![Reply, Edit, Delete]),
                (false, vec![Reply]),
                (true, vec![Reply, Edit, Delete]),
                (false, vec![Reply]),
            ]
        );
        assert_eq!(replies.continuation.as_deref(), Some("fixture-more-replies"));
        assert_eq!(replies.replies[0].edit_text.as_deref(), Some("old reply"));
        // A reply's Edit is its own dialog: the first own reply names no path (the constant),
        // the nested one names it.
        let edit = |i: usize| match &replies.replies[i].commands.edit {
            Some(WriteCommand::Endpoint { path, payload, .. }) => {
                assert_eq!(payload.len(), 1, "only updateReplyParams is sent");
                (path.clone(), payload["updateReplyParams"].as_str().unwrap().to_owned())
            }
            other => panic!("{other:?}"),
        };
        let reply_path = crate::models::comment_write::COMMENT_UPDATE_REPLY_PATH;
        assert_eq!(edit(0), (reply_path.to_owned(), "fixture-edit-reply-params-own_reply".into()));
        assert_eq!(
            edit(2),
            (reply_path.to_owned(), "fixture-edit-reply-params-nested_own_reply".into())
        );
        assert_eq!(replies.replies[0].commands.edit.as_ref().unwrap().log_kind(), "edit_reply");
        assert!(replies.replies[1].commands.edit.is_none());
        assert!(replies.replies[3].commands.edit.is_none());
    }

    // --- a write's answer ----------------------------------------------------------------------

    /// A write's answer carrying `case`'s thread and entities.
    fn answer_with(case: Case) -> Value {
        json!({ "actions": [{ "createCommentAction": { "contents": case.item } }],
                "frameworkUpdates": { "entityBatchUpdate": { "mutations": case.mutations } } })
    }

    #[test]
    fn a_written_comment_is_the_answers_thread_when_it_carries_one() {
        let posted = case("NewOne", true, NEUTRAL, Some(votes("NewOne", &["like"])), vec![]);
        let thread = parse_written_comment(&answer_with(posted)).unwrap();
        let c = &thread.comment;
        assert_eq!((c.id.as_str(), c.text.as_str(), c.own), ("NewOne", "Comment NewOne", true));
        assert_eq!(c.vote, Some(VoteState::Neutral));
        // Only the like command was in the answer, so only like is on offer.
        assert_eq!(c.actions, [CommentAction::Like]);
        assert!(thread.replies.is_empty() && thread.replies_token.is_none());
    }

    /// A comment inserted from a write's answer has its menu parsed exactly like a read's, if
    /// the answer's toolbar surface carries one; if it does not, it has none until the next read.
    #[test]
    fn a_written_comment_takes_edit_and_delete_from_its_surface_in_the_answer() {
        use CommentWrite::*;
        let full = case(
            "own_full",
            true,
            NEUTRAL,
            Some(write_surface("own_full", own_menu("own_full"))),
            vec![],
        );
        let with_menu = parse_written_comment(&answer_with(full)).unwrap().comment;
        assert_eq!(with_menu.writes, [Reply, Edit, Delete]);
        let bare = case("own_bare", true, NEUTRAL, Some(write_surface("own_bare", vec![])), vec![]);
        assert_eq!(parse_written_comment(&answer_with(bare)).unwrap().comment.writes, [Reply]);
    }

    #[test]
    fn an_answer_without_the_comment_gives_none() {
        let entity_only = case("Mine", true, NEUTRAL, None, vec![]);
        for answer in [
            json!({}),
            json!(null),
            json!({ "actions": [{ "runAttestationCommand": {} }] }),
            // The viewer's own comment entity, but no thread for it.
            json!({ "frameworkUpdates": { "entityBatchUpdate": { "mutations": entity_only.mutations } } }),
            json!({ "frameworkUpdates": 5 }),
            // A thread renderer whose entity is not in the answer.
            json!({ "actions": [{ "x": { "commentThreadRenderer": { "commentViewModel": { "commentViewModel": {
                "commentKey": "gone" } } } } }] }),
        ] {
            assert!(parse_written_comment(&answer).is_none(), "{answer}");
        }
    }

    // --- actions -------------------------------------------------------------------------------

    #[test]
    fn actions_have_log_names() {
        assert_eq!(
            [
                CommentAction::Like,
                CommentAction::Unlike,
                CommentAction::Dislike,
                CommentAction::Undislike
            ]
            .map(CommentAction::name),
            ["like", "unlike", "dislike", "undislike"]
        );
    }

    #[test]
    fn each_action_has_one_resulting_vote() {
        use CommentAction::*;
        let got: Vec<_> =
            [Like, Unlike, Dislike, Undislike].map(CommentAction::resulting_vote).into();
        assert_eq!(
            got,
            [VoteState::Liked, VoteState::Neutral, VoteState::Disliked, VoteState::Neutral]
        );
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
