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
    command_probe, create_command, menu_commands, menu_probe, reply_command, CommentWrite,
    Composer, MenuCommands, WriteCommand, WriteCommands,
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
    /// The viewer's own comment (`author.isCurrentUser`).
    pub own: bool,
    /// What the edit dialog pre-fills (`editableText`), when the response had it in a plain shape;
    /// the UI edits `text` otherwise.
    pub edit_text: Option<String>,
    /// What the viewer can write on it: exactly the commands the response carried. Reply on any
    /// comment that offered it; edit and delete only on one's own, and only if found (UNVERIFIED,
    /// see `comment_write`). Empty read anonymously.
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

/// What a page read as the account turned out to hold, as counts only, for the one debug line per
/// page that says "the viewer state was found" (silence in a log is ambiguous: it could be a
/// filter that never reached the app). No id, token, name, text or path can be in it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ViewerSummary {
    pub comments: usize,
    pub liked: usize,
    pub disliked: usize,
    pub neutral: usize,
    pub vote_missing: usize,
    pub offer_like: usize,
    pub offer_unlike: usize,
    pub offer_dislike: usize,
    pub offer_undislike: usize,
}

impl ViewerSummary {
    pub fn of<'a>(comments: impl IntoIterator<Item = &'a Comment>) -> Self {
        let mut s = ViewerSummary::default();
        for c in comments {
            s.comments += 1;
            match c.vote {
                Some(VoteState::Liked) => s.liked += 1,
                Some(VoteState::Disliked) => s.disliked += 1,
                Some(VoteState::Neutral) => s.neutral += 1,
                None => s.vote_missing += 1,
            }
            for action in &c.actions {
                match action {
                    CommentAction::Like => s.offer_like += 1,
                    CommentAction::Unlike => s.offer_unlike += 1,
                    CommentAction::Dislike => s.offer_dislike += 1,
                    CommentAction::Undislike => s.offer_undislike += 1,
                }
            }
        }
        s
    }

    /// Every comment on a page: the threads' own and the replies that came inline.
    pub fn of_page(page: &CommentsPage) -> Self {
        Self::of(page.threads.iter().flat_map(|t| std::iter::once(&t.comment).chain(&t.replies)))
    }

    pub fn of_replies(replies: &CommentReplies) -> Self {
        Self::of(&replies.replies)
    }
}

impl std::fmt::Display for ViewerSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "comments={} liked={} disliked={} neutral={} vote_missing={} offer_like={} offer_unlike={} offer_dislike={} offer_undislike={}",
            self.comments,
            self.liked,
            self.disliked,
            self.neutral,
            self.vote_missing,
            self.offer_like,
            self.offer_unlike,
            self.offer_dislike,
            self.offer_undislike
        )
    }
}

/// For the debug line that says why edit/delete is or is not offered: the command-like key paths
/// and value types under the first of the viewer's own comments in a response. `None` when the
/// response has no comment of the viewer's. Key names and types only.
pub(crate) fn own_comment_probe(root: &Value) -> Option<String> {
    let entities = Entities::new(root);
    find_all(root, "commentViewModel").into_iter().find_map(|vm| {
        let entity = entities.get(str_of(vm, "commentKey"), "commentEntityPayload")?;
        if entity.pointer("/author/isCurrentUser").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        let surface =
            entities.get(str_of(vm, "toolbarSurfaceKey"), "engagementToolbarSurfaceEntityPayload");
        let comment_surface =
            entities.get(str_of(vm, "commentSurfaceKey"), "commentSurfaceEntityPayload");
        let roots: Vec<(&str, &Value)> = [
            ("surface", surface),
            ("commentSurface", comment_surface),
            ("comment", Some(entity)),
            ("viewModel", Some(vm)),
        ]
        .into_iter()
        .filter_map(|(label, v)| Some((label, v?)))
        .collect();
        let menu = surface.map_or_else(|| "menu: no surface".to_owned(), menu_probe);
        Some(format!("{}; {menu}", command_probe(&roots)))
    })
}

/// The comment a write put on the page, if its answer carries it: the thread itself
/// (`commentThreadRenderer`, with its entities in `frameworkUpdates`), or failing that just the
/// viewer's own comment entity in the answer's mutations, which gives the row with no actions or
/// write commands until the next read. `None` when the answer holds neither, and the caller shows
/// a local row instead. Never reloads anything. UNVERIFIED which of these a live answer holds;
/// what keys an answer has is logged at debug level.
pub fn parse_written_comment(root: &Value) -> Option<CommentThread> {
    let entities = Entities::new(root);
    if let Some(thread) =
        find_all(root, "commentThreadRenderer").into_iter().find_map(|t| parse_thread(t, &entities))
    {
        return Some(thread);
    }
    let mutations = root.pointer("/frameworkUpdates/entityBatchUpdate/mutations")?.as_array()?;
    mutations.iter().find_map(|m| {
        let key = m.get("entityKey")?.as_str()?;
        let payload = m.pointer("/payload/commentEntityPayload")?;
        if payload.pointer("/author/isCurrentUser").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        // The same view-model keys a read has, as far as the entity itself names them.
        let vm = serde_json::json!({
            "commentKey": key,
            "toolbarStateKey": payload.pointer("/properties/toolbarStateKey"),
            "commentId": payload.pointer("/properties/commentId"),
        });
        let comment = parse_comment(&vm, false, &entities)?;
        Some(CommentThread { comment, replies_token: None, replies: Vec::new() })
    })
}

/// JSON PATHS and value TYPES (never values) of what the viewer-state parse depends on, for the
/// debug log line that says why a signed-in page came back without a vote or without actions.
/// Only structure and counts: no id, token, name or text can appear in it.
pub(crate) fn viewer_state_probe(root: &Value) -> String {
    fn ty(v: Option<&Value>) -> &'static str {
        match v {
            None => "missing",
            Some(Value::Null) => "null",
            Some(Value::Bool(_)) => "bool",
            Some(Value::Number(_)) => "number",
            Some(Value::String(_)) => "string",
            Some(Value::Array(_)) => "array",
            Some(Value::Object(_)) => "object",
        }
    }
    let muts = root.pointer("/frameworkUpdates/entityBatchUpdate/mutations");
    let payloads: Vec<&Value> = muts
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("payload"))
        .collect();
    let of = |kind: &str| payloads.iter().filter_map(|p| p.get(kind)).collect::<Vec<_>>();
    let (states, surfaces) =
        (of("engagementToolbarStateEntityPayload"), of("engagementToolbarSurfaceEntityPayload"));

    let mut out = vec![
        format!("/frameworkUpdates/entityBatchUpdate/mutations: {}", ty(muts)),
        format!("mutations with engagementToolbarStateEntityPayload: {}", states.len()),
        format!("mutations with engagementToolbarSurfaceEntityPayload: {}", surfaces.len()),
    ];
    if let Some(vm) =
        find_all(root, "commentViewModel").into_iter().find_map(|w| w.get("commentViewModel"))
    {
        for key in ["commentKey", "toolbarStateKey", "toolbarSurfaceKey"] {
            out.push(format!("first thread commentViewModel/{key}: {}", ty(vm.get(key))));
        }
    }
    if let Some(state) = states.first() {
        out.push(format!("state payload /likeState: {}", ty(state.get("likeState"))));
    }
    if let Some(surface) = surfaces.first() {
        for key in ["likeCommand", "unlikeCommand", "dislikeCommand", "undislikeCommand"] {
            let cmd = surface.get(key);
            let ep =
                cmd.and_then(|c| find_all(c, "performCommentActionEndpoint").into_iter().next());
            out.push(format!(
                "surface payload /{key}: {}, performCommentActionEndpoint: {}, action: {}, actions: {}",
                ty(cmd),
                ty(ep),
                ty(ep.and_then(|e| e.get("action"))),
                ty(ep.and_then(|e| e.get("actions"))),
            ));
        }
        out.push(format!(
            "surface payload /prepareAccountCommand: {}",
            ty(surface.get("prepareAccountCommand"))
        ));
    }
    out.join("; ")
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
    /// SYNTHETIC too: write commands, with ASSUMED shapes for edit and delete.
    const WRITES: &str =
        include_str!("../../tests/fixtures/comments/signed_in_write_synthetic.json");

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

    /// The "read as the account" line is counts only. The synthetic page has 8 comments: 4
    /// neutral (one of them offered nothing, one only `like`), 1 liked, 1 disliked, 2 with no vote.
    #[test]
    fn the_account_read_summary_counts_votes_and_offers_and_nothing_else() {
        let page = synthetic_page();
        let s = ViewerSummary::of_page(&page);
        assert_eq!(
            s,
            ViewerSummary {
                comments: 8,
                liked: 1,
                disliked: 1,
                neutral: 4,
                vote_missing: 2,
                offer_like: 3,
                offer_unlike: 1,
                offer_dislike: 2,
                offer_undislike: 1,
            }
        );
        let line = s.to_string();
        assert_eq!(
            line,
            "comments=8 liked=1 disliked=1 neutral=4 vote_missing=2 offer_like=3 offer_unlike=1 offer_dislike=2 offer_undislike=1"
        );
        for value in ["fixture", "Synthetic", "UCfixture", "@synthetic", "TOOLBAR", "/"] {
            assert!(!line.contains(value), "the summary leaks `{value}`: {line}");
        }
        // A replies page is summarised the same way, and an empty one is all zeros.
        let replies = parse_comment_replies(&load(SIGNED_IN));
        assert_eq!(ViewerSummary::of_replies(&replies).comments, 8);
        assert_eq!(ViewerSummary::of_replies(&CommentReplies::default()), ViewerSummary::default());
    }

    fn writes_page() -> CommentsPage {
        parse_comments_page(&load(WRITES), true)
    }

    fn writes_of(page: &CommentsPage, name: &str) -> (bool, Vec<CommentWrite>) {
        let c = &page
            .threads
            .iter()
            .find(|t| t.comment.text.ends_with(name))
            .unwrap_or_else(|| panic!("no {name}"))
            .comment;
        (c.own, c.writes.clone())
    }

    /// Reply wherever the response carried a reply button with a usable path; edit and delete
    /// only on the viewer's own comments, and only where a command was found.
    #[test]
    fn writes_follow_the_commands_the_response_carried() {
        use CommentWrite::*;
        let page = writes_page();
        assert_eq!(page.threads.len(), 9, "the thread whose entity is missing is skipped");
        let want = [
            // Edit and delete are in this one's menu too, but it is not the viewer's own.
            ("other", false, vec![Reply]),
            ("own_full", true, vec![Reply, Edit, Delete]),
            ("own_bare", true, vec![Reply]),
            ("own_delete_only", true, vec![Reply, Delete]),
            // The update command names no path (or a hostile one): edit still, on the fallback.
            ("own_edit_no_api_url", true, vec![Reply, Edit]),
            ("own_edit_hostile_api_url", true, vec![Reply, Edit]),
            // An update button with no params is not an edit.
            ("own_edit_no_params", true, vec![Reply]),
            // A reply button naming a path outside comment/ offers nothing.
            ("bad_reply_path", false, vec![]),
            ("no_reply_dialog", false, vec![]),
        ];
        for (name, own, writes) in want {
            assert_eq!(writes_of(&page, name), (own, writes), "{name}");
        }
    }

    #[test]
    fn the_write_commands_are_the_servers_and_stay_in_rust() {
        let page = writes_page();
        let by = |name: &str| {
            &page.threads.iter().find(|t| t.comment.text.ends_with(name)).unwrap().comment
        };
        let own = by("own_full");
        match own.commands.reply.as_ref().unwrap() {
            WriteCommand::Endpoint { path, payload } => {
                assert_eq!(path, "comment/create_comment_reply");
                assert_eq!(payload["createReplyParams"], "fixture-reply-params-2");
            }
            other => panic!("{other:?}"),
        }
        match own.commands.edit.as_ref().unwrap() {
            WriteCommand::Endpoint { path, payload } => {
                assert_eq!(path, "comment/update_comment");
                assert_eq!(payload.len(), 1, "only the update params are replayed");
                assert_eq!(payload["updateCommentParams"], "fixture-edit-params-2");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(own.edit_text.as_deref(), Some("Synthetic editable text 2"));
        // Delete is the confirm button's action token, replayed like a vote.
        assert_eq!(
            own.commands.delete,
            Some(WriteCommand::Action("fixture-delete-token-2".into()))
        );
        assert!(by("own_bare").commands.edit.is_none() && by("own_bare").edit_text.is_none());
        // The named path wins when it is a plain comment path; otherwise the constant is used.
        let path = |name: &str| match by(name).commands.edit.as_ref().unwrap() {
            WriteCommand::Endpoint { path, .. } => path.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(path("own_edit_no_api_url"), crate::models::comment_write::COMMENT_UPDATE_PATH);
        assert_eq!(
            path("own_edit_hostile_api_url"),
            crate::models::comment_write::COMMENT_UPDATE_PATH
        );
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
        let header = writes_page().header.unwrap();
        assert_eq!(
            header.composer.as_ref().unwrap().placeholder.as_deref(),
            Some("Add a comment...")
        );
        match header.create.as_ref().unwrap() {
            WriteCommand::Endpoint { path, payload } => {
                assert_eq!(path, crate::models::comment_write::COMMENT_CREATE_PATH);
                assert_eq!(payload["createCommentParams"], "fixture-create-params-1");
            }
            other => panic!("{other:?}"),
        }
        // The anonymous captures have a comment box with no submit button: no composer, no
        // commands, and nothing to write on any comment.
        let anon = parse_comments_page(&load(INITIAL), true);
        let header = anon.header.unwrap();
        assert!(header.composer.is_none() && header.create.is_none());
        assert!(anon.threads.iter().all(|t| t.comment.writes.is_empty() && !t.comment.own));
        assert!(!serde_json::to_string(&header).unwrap().contains("createCommentParams"));
    }

    /// The diagnostics line for a missing edit/delete: key paths and types of the viewer's own
    /// comment's command-like nodes, never a value, and nothing at all without an own comment.
    #[test]
    fn the_own_comment_probe_lists_command_keys_and_types_only() {
        let probe = own_comment_probe(&load(WRITES)).expect("the page has an own comment");
        assert!(probe.contains("surface/menuCommand: object"), "{probe}");
        assert!(probe.contains("menuNavigationItemRenderer"), "{probe}");
        assert!(probe.contains("updateCommentEndpoint: object"), "{probe}");
        assert!(probe.contains("performCommentActionEndpoint: object"), "{probe}");
        // What the structure matched, and the icon enums.
        assert!(
            probe.contains("menu: item[0]: edit, icon=EDIT, item[1]: delete, icon=DELETE, item[2]: other, icon=FLAG"),
            "{probe}"
        );
        for value in ["fixture", "Synthetic", "update_comment", "@synthetic", "Edit", "Delete"] {
            assert!(!probe.contains(value), "the probe leaks `{value}`: {probe}");
        }
        assert_eq!(own_comment_probe(&load(INITIAL)), None, "no own comment, no line");
    }

    /// What a write's answer can hold, by what is in it. The shapes are SYNTHETIC: no live answer
    /// is known to carry the comment (a live create answer has `actions` with an attestation
    /// command and a `frameworkUpdates`), which is why a local row is the last resort.
    fn own_entity(key: &str, id: &str, text: &str, own: bool) -> Value {
        json!({ "entityKey": key, "payload": { "commentEntityPayload": {
            "key": key,
            "properties": { "commentId": id, "content": { "content": text }, "publishedTime": "now",
                            "toolbarStateKey": format!("{key}-state") },
            "author": { "displayName": "@me", "channelId": "UCfixtureme", "isCurrentUser": own },
            "toolbar": { "likeCountNotliked": "", "likeCountLiked": "1", "replyCount": "" } } } })
    }

    #[test]
    fn a_written_comment_is_the_answers_thread_when_it_carries_one() {
        let answer = json!({
            "actions": [{ "createCommentAction": { "contents": { "commentThreadRenderer": {
                "commentViewModel": { "commentViewModel": {
                    "commentKey": "k1", "toolbarStateKey": "k1-state", "toolbarSurfaceKey": "k1-surface",
                    "commentId": "NewOne" } } } } } }],
            "frameworkUpdates": { "entityBatchUpdate": { "mutations": [
                own_entity("k1", "NewOne", "hello there", true),
                { "entityKey": "k1-state", "payload": { "engagementToolbarStateEntityPayload": {
                    "key": "k1-state", "likeState": "TOOLBAR_LIKE_STATE_INDIFFERENT" } } },
                { "entityKey": "k1-surface", "payload": { "engagementToolbarSurfaceEntityPayload": {
                    "likeCommand": { "innertubeCommand": { "performCommentActionEndpoint": { "action": "L" } } } } } },
            ] } },
        });
        let thread = parse_written_comment(&answer).unwrap();
        let c = &thread.comment;
        assert_eq!((c.id.as_str(), c.text.as_str(), c.own), ("NewOne", "hello there", true));
        assert_eq!(c.vote, Some(VoteState::Neutral));
        // Only the like command was in the answer, so only like is on offer.
        assert_eq!(c.actions, [CommentAction::Like]);
        assert!(thread.replies.is_empty() && thread.replies_token.is_none());
    }

    /// No thread in the answer, but the viewer's own comment entity: the row, with nothing to
    /// click until the next read gives it commands.
    #[test]
    fn a_written_comment_falls_back_to_the_answers_own_entity() {
        let answer = json!({
            "actions": [{ "runAttestationCommand": {} }],
            "frameworkUpdates": { "entityBatchUpdate": { "mutations": [
                own_entity("someone", "Other", "not mine", false),
                own_entity("mine", "Mine", "my text", true),
            ] } },
        });
        let c = parse_written_comment(&answer).unwrap().comment;
        assert_eq!((c.id.as_str(), c.text.as_str(), c.own), ("Mine", "my text", true));
        assert!(c.actions.is_empty() && c.writes.is_empty() && c.edit_text.is_none());
    }

    #[test]
    fn an_answer_without_the_comment_gives_none() {
        for answer in [
            json!({}),
            json!(null),
            json!({ "actions": [{ "runAttestationCommand": {} }] }),
            // Entities, but none the viewer's own, or none that parses.
            json!({ "frameworkUpdates": { "entityBatchUpdate": { "mutations": [
                own_entity("a", "A", "x", false)] } } }),
            json!({ "frameworkUpdates": { "entityBatchUpdate": { "mutations": [
                { "entityKey": "z", "payload": { "commentEntityPayload": { "author": { "isCurrentUser": true } } } }] } } }),
            json!({ "frameworkUpdates": 5 }),
            // A thread renderer whose entity is not in the answer.
            json!({ "actions": [{ "x": { "commentThreadRenderer": { "commentViewModel": { "commentViewModel": {
                "commentKey": "gone" } } } } }] }),
        ] {
            assert!(parse_written_comment(&answer).is_none(), "{answer}");
        }
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

    /// The diagnostics line is structure only: no id, token, name or comment text.
    #[test]
    fn the_viewer_state_probe_describes_paths_and_types_and_nothing_else() {
        let probe = viewer_state_probe(&load(SIGNED_IN));
        assert!(probe.contains("/frameworkUpdates/entityBatchUpdate/mutations: array"));
        assert!(probe.contains("engagementToolbarStateEntityPayload: 7"), "{probe}");
        assert!(probe
            .contains("likeCommand: object, performCommentActionEndpoint: object, action: string"));
        assert!(probe.contains("toolbarStateKey: string"));
        for value in ["fixture", "Synthetic", "TOOLBAR_LIKE", "UCfixture", "@synthetic"] {
            assert!(!probe.contains(value), "the probe leaks `{value}`: {probe}");
        }
        // A response with none of it still describes itself instead of failing.
        let bare = viewer_state_probe(&json!({}));
        assert!(bare.contains("mutations: missing"));
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
