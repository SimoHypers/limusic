//! Writing comments: finding, in a signed-in read response, the commands the server issued for
//! posting a comment, replying, and (for the viewer's own comments) editing and deleting.
//!
//! Nothing here builds a request out of thin air. A command is whatever the response carried,
//! replayed the way youtubei.js v18.1.0 (MIT) replays one (`NavigationEndpoint`): unwrap
//! `innertubeCommand`, take the one key ending in `Endpoint`, send its fields (plus the text) to the
//! `apiUrl` the response names. What the reference has, and does not have:
//!
//! - **Create**: `Comments.createComment` takes the comment box's `submitButton` endpoint
//!   (`createCommentEndpoint.createCommentParams`) from the response and calls `comment/create_comment`
//!   with it and `commentText`. FROM-REFERENCE.
//! - **Reply**: `CommentView.reply` takes the reply dialog's `replyButton` endpoint from the
//!   response and sends its own payload plus `commentText` to the `apiUrl` the endpoint names (it has
//!   no class and no path constant of its own). FROM-REFERENCE. That dialog and button exist in a
//!   live signed-in response (VERIFIED); what is inside the button's endpoint is not.
//! - **Edit and delete**: youtubei.js v18.1.0 has neither (searched the whole tag). They are read
//!   from the viewer's OWN comment's menu, by exact structure, as a live signed-in response shows
//!   it (VERIFIED): `engagementToolbarSurfaceEntityPayload.menuCommand…menuRenderer.items[]`, an
//!   item whose `navigationEndpoint` opens an `updateCommentDialogEndpoint` (edit) or a
//!   `confirmDialogEndpoint` (delete). Delete replays the confirm button's action token; edit
//!   sends `{context, commentText, updateCommentParams}`. What a successful edit or delete answer
//!   looks like is not known (UNVERIFIED).
//!
//! Every command is opaque and stays in Rust: `Debug` prints no payload and no token, and none of
//! these types is serialized to the UI.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::metadata::find_all;

/// `Comments.createComment` / `CreateCommentEndpoint` in youtubei.js v18.1.0. UNVERIFIED live.
pub const COMMENT_CREATE_PATH: &str = "comment/create_comment";

/// What a viewer can write on a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentWrite {
    Reply,
    Edit,
    Delete,
}

impl CommentWrite {
    /// The word for it in log lines.
    pub fn name(self) -> &'static str {
        match self {
            CommentWrite::Reply => "reply",
            CommentWrite::Edit => "edit",
            CommentWrite::Delete => "delete",
        }
    }
}

/// The comment box at the top of the panel, as the UI needs it. Present only when the response
/// carried a command to post with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Composer {
    pub placeholder: Option<String>,
}

/// A command the server issued, ready to replay.
#[derive(Clone, PartialEq, Eq)]
pub enum WriteCommand {
    /// A `performCommentActionEndpoint` token: `comment/perform_comment_action {actions:[token]}`.
    Action(String),
    /// `path` (under `/youtubei/v1/`, always `comment/…`) and the endpoint's own fields.
    Endpoint { path: String, payload: Map<String, Value> },
}

impl fmt::Debug for WriteCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteCommand::Action(_) => f.write_str("WriteCommand::Action(<redacted>)"),
            WriteCommand::Endpoint { path, .. } => {
                write!(f, "WriteCommand::Endpoint {{ path: {path:?}, payload: <redacted> }}")
            }
        }
    }
}

/// The write commands one comment offered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteCommands {
    pub reply: Option<WriteCommand>,
    pub edit: Option<WriteCommand>,
    pub delete: Option<WriteCommand>,
}

impl WriteCommands {
    pub fn get(&self, write: CommentWrite) -> Option<&WriteCommand> {
        match write {
            CommentWrite::Reply => &self.reply,
            CommentWrite::Edit => &self.edit,
            CommentWrite::Delete => &self.delete,
        }
        .as_ref()
    }

    /// What the UI is told is available: exactly what has a command.
    pub fn offered(&self) -> Vec<CommentWrite> {
        [CommentWrite::Reply, CommentWrite::Edit, CommentWrite::Delete]
            .into_iter()
            .filter(|w| self.get(*w).is_some())
            .collect()
    }
}

/// A path the response names for a command, kept only if it is a plain `comment/…` path under
/// `/youtubei/v1/`: a command can never send us somewhere else.
fn comment_path(api_url: &str) -> Option<String> {
    let path = api_url.strip_prefix("/youtubei/v1/")?;
    is_plain_comment_path(path).then(|| path.to_owned())
}

/// A path relative to `https://music.youtube.com/youtubei/v1/` that stays under `comment/`: lowercase
/// letters, digits, `_` and `/` only, so no scheme, host, port, `.`, `?`, `#`, `%`, `@`, backslash,
/// whitespace or non-ASCII can be in it, and no empty segment (`//`). Checked when a command is
/// read AND again when one is sent, since the path is glued onto the fixed base URL as text.
pub(crate) fn is_plain_comment_path(path: &str) -> bool {
    path.starts_with("comment/")
        && path.len() > "comment/".len()
        && !path.contains("//")
        && !path.ends_with('/')
        && path
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'/')
}

/// `NavigationEndpoint`'s unwrapping: the first of `innertubeCommand`, `command`,
/// `performOnceCommand`, or the node itself.
fn unwrap_command(node: &Value) -> &Value {
    ["innertubeCommand", "command", "performOnceCommand"]
        .iter()
        .find_map(|k| node.get(k))
        .unwrap_or(node)
}

/// The one key ending in `Endpoint` or `Command` in a command, and what is under it.
fn endpoint_of(data: &Value) -> Option<(&str, &Value)> {
    data.as_object()?
        .iter()
        .find(|(k, _)| k.ends_with("Endpoint") || k.ends_with("Command"))
        .map(|(k, v)| (k.as_str(), v))
}

/// The `performCommentActionEndpoint` token, as for the like/dislike commands: `action`, or the
/// first of `actions`.
pub(crate) fn action_token(command: &Value) -> Option<String> {
    let endpoint = find_all(command, "performCommentActionEndpoint").into_iter().next()?;
    let token = endpoint
        .get("action")
        .and_then(Value::as_str)
        .or_else(|| endpoint.get("actions")?.as_array()?.first()?.as_str())?;
    (!token.is_empty()).then(|| token.to_owned())
}

fn runs_text(v: Option<&Value>) -> Option<String> {
    v?.pointer("/runs/0/text").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
}

/// The comment box in `commentsHeaderRenderer.createRenderer.commentSimpleboxRenderer`: its
/// placeholder, and the `createCommentEndpoint.createCommentParams` the submit button carries.
/// Both are required for a composer to be offered; the signed-out box has a sign-in endpoint and
/// no submit button, so it yields nothing. The params are server-issued: this never builds them
/// (youtubei.js's `InteractionManager.comment` does build a protobuf; that path is not taken).
pub(crate) fn create_command(header: &Value) -> Option<(Composer, WriteCommand)> {
    let simplebox = header.pointer("/createRenderer/commentSimpleboxRenderer")?;
    let button = simplebox.pointer("/submitButton/buttonRenderer")?;
    let data = ["serviceEndpoint", "navigationEndpoint", "command"]
        .iter()
        .find_map(|k| button.get(k))
        .map(unwrap_command)?;
    let params = data.pointer("/createCommentEndpoint/createCommentParams")?.as_str()?;
    if params.is_empty() {
        return None;
    }
    let payload =
        Map::from_iter([("createCommentParams".to_owned(), Value::String(params.into()))]);
    Some((
        Composer { placeholder: runs_text(simplebox.get("placeholderText")) },
        WriteCommand::Endpoint { path: COMMENT_CREATE_PATH.to_owned(), payload },
    ))
}

/// Where a reply goes when its button's command names no plain `comment/…` path (a live response
/// does not). UNVERIFIED, and not from a reference: youtubei.js v18.1.0 has no reply path (it
/// replays the `apiUrl` the response names, and a live one names none). It is inferred from the
/// naming of the verified paths (`comment/create_comment`, `comment/update_comment`,
/// `comment/perform_comment_action`), exactly like [`COMMENT_UPDATE_PATH`]. A path the response
/// does name always wins. A 404 on it is never taken to mean the comment is gone.
pub const COMMENT_REPLY_PATH: &str = "comment/create_comment_reply";

/// A comment's reply command, off its toolbar surface entity's `replyCommand` (VERIFIED live:
/// `…createCommentReplyDialogEndpoint.dialog.commentReplyDialogRenderer` with `replyButton`), and
/// the dialog's placeholder.
pub(crate) fn reply_command(surface: &Value) -> Option<(Option<String>, WriteCommand)> {
    let dialog =
        find_all(surface.get("replyCommand")?, "commentReplyDialogRenderer").into_iter().next()?;
    let button = dialog.pointer("/replyButton/buttonRenderer")?;
    let (command, _) = reply_of(button)?;
    Some((runs_text(dialog.get("placeholderText")), command))
}

/// The reply button's command, and where its path came from (`apiUrl` or `constant`). VERIFIED
/// live: its `createCommentReplyEndpoint` carries `createReplyParams` and no `apiUrl`. Sent as
/// `{context, commentText, createReplyParams}`; offered only when `createReplyParams` is a
/// non-empty string.
fn reply_of(button: &Value) -> Option<(WriteCommand, &'static str)> {
    let service =
        ["serviceEndpoint", "navigationEndpoint", "command"].iter().find_map(|k| button.get(k))?;
    let data = unwrap_command(service);
    let params = data
        .pointer("/createCommentReplyEndpoint/createReplyParams")?
        .as_str()
        .filter(|p| !p.is_empty())?;
    let named = [data, service]
        .iter()
        .find_map(|c| c.pointer("/commandMetadata/webCommandMetadata/apiUrl")?.as_str())
        .and_then(comment_path);
    let (path, source) = match named {
        Some(path) => (path, "apiUrl"),
        None => (COMMENT_REPLY_PATH.to_owned(), "constant"),
    };
    let payload = Map::from_iter([("createReplyParams".to_owned(), Value::String(params.into()))]);
    Some((WriteCommand::Endpoint { path, payload }, source))
}

/// Key names (never values) of what a comment's reply command carries, for the debug line that
/// says why Reply is or is not offered: the button, its service endpoint, the endpoint's payload
/// fields, the command metadata, and whether the `apiUrl` is there and is a plain `comment/…`
/// path, and whether `createReplyParams` is there. `source` is where the path comes from:
/// `apiUrl`, `constant` (the unverified fallback) or `none` (no reply is offered).
pub(crate) fn reply_probe(surface: &Value) -> String {
    fn names(v: Option<&Value>) -> String {
        v.and_then(Value::as_object)
            .map(|o| o.keys().cloned().collect::<Vec<_>>().join(","))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "-".into())
    }
    let Some(command) = surface.get("replyCommand") else {
        return "reply: no replyCommand".into();
    };
    let Some(dialog) = find_all(command, "commentReplyDialogRenderer").into_iter().next() else {
        return "reply: replyCommand has no commentReplyDialogRenderer".into();
    };
    let button = dialog.pointer("/replyButton/buttonRenderer");
    let service = button.and_then(|b| {
        ["serviceEndpoint", "navigationEndpoint", "command"].iter().find_map(|k| b.get(k))
    });
    let data = service.map(unwrap_command);
    let endpoint = data.and_then(endpoint_of);
    let api_url = data
        .and_then(|d| d.pointer("/commandMetadata/webCommandMetadata/apiUrl"))
        .and_then(Value::as_str);
    let plain = api_url.and_then(comment_path).is_some();
    let params = match data
        .and_then(|d| d.pointer("/createCommentReplyEndpoint/createReplyParams"))
        .and_then(Value::as_str)
    {
        Some("") => "empty",
        Some(_) => "string",
        None => "missing",
    };
    let source = button.and_then(reply_of).map_or("none", |(_, source)| source);
    format!(
        "reply: button={} service={} endpoint={}:{} commandMetadata={} webCommandMetadata={} apiUrl={} apiUrl_plain={plain} createReplyParams={params} source={source}",
        names(button),
        names(service),
        endpoint.map_or("-", |(name, _)| name),
        names(endpoint.map(|(_, payload)| payload)),
        names(data.and_then(|d| d.get("commandMetadata"))),
        names(data.and_then(|d| d.pointer("/commandMetadata/webCommandMetadata"))),
        if api_url.is_some() { "string" } else { "missing" },
    )
}

const MAX_DEPTH: usize = 24;

/// Where editing falls back to when the update button's command names no usable path. UNVERIFIED:
/// it is the name `updateCommentEndpoint` suggests and the pattern of the other comment paths
/// (`comment/create_comment`, `comment/perform_comment_action`), not something a response said.
/// A live response so far carries `commandMetadata.webCommandMetadata.apiUrl` for it, which wins.
pub const COMMENT_UPDATE_PATH: &str = "comment/update_comment";

/// What the viewer's own comment's menu offered.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct MenuCommands {
    pub edit: Option<WriteCommand>,
    /// The text the edit dialog is pre-filled with (`editableText`), if it is plain to read.
    pub edit_text: Option<String>,
    pub delete: Option<WriteCommand>,
}

/// The items of every `menuRenderer` under the toolbar surface entity's `menuCommand`. VERIFIED in
/// a live response: `menuCommand/innertubeCommand/menuEndpoint/menu/menuRenderer/items[]`.
fn menu_items(surface: &Value) -> Vec<&Value> {
    let Some(menu) = surface.get("menuCommand") else { return Vec::new() };
    find_all(menu, "menuRenderer")
        .into_iter()
        .filter_map(|m| m.get("items")?.as_array())
        .flatten()
        .collect()
}

/// The renderer inside a menu item (`menuNavigationItemRenderer`, whatever it is called) that
/// carries the `navigationEndpoint`.
fn item_renderer(item: &Value) -> Option<&Value> {
    item.as_object()?.values().find(|v| v.get("navigationEndpoint").is_some())
}

/// Edit, by structure (VERIFIED live): the item's `navigationEndpoint` is an
/// `updateCommentDialogEndpoint` whose `commentDialogRenderer.submitButton…serviceEndpoint` holds an
/// `updateCommentEndpoint` with an `updateCommentParams` string. Sent as `{context, commentText,
/// updateCommentParams}` to the path that command names, or to [`COMMENT_UPDATE_PATH`] when it
/// names none (UNVERIFIED fallback; which one was used is logged at debug level).
fn edit_of(navigation: &Value) -> Option<(WriteCommand, Option<String>)> {
    let dialog = unwrap_command(navigation)
        .pointer("/updateCommentDialogEndpoint/dialog/commentDialogRenderer")?;
    let service = dialog.pointer("/submitButton/buttonRenderer/serviceEndpoint")?;
    let data = unwrap_command(service);
    let params = data
        .pointer("/updateCommentEndpoint/updateCommentParams")?
        .as_str()
        .filter(|p| !p.is_empty())?;
    // The command that holds the endpoint names its path; `service` itself is also tried in case
    // there is a wrapper in between.
    let named = [data, service]
        .iter()
        .find_map(|c| c.pointer("/commandMetadata/webCommandMetadata/apiUrl")?.as_str());
    let (path, source) = match named.map(comment_path) {
        Some(Some(path)) => (path, "apiUrl"),
        Some(None) => {
            (COMMENT_UPDATE_PATH.to_owned(), "constant (the apiUrl was not a comment path)")
        }
        None => (COMMENT_UPDATE_PATH.to_owned(), "constant (no apiUrl)"),
    };
    // A path that passed `comment_path` or is our constant: not secret.
    tracing::debug!(source, path = %path, "edit command path");
    let payload =
        Map::from_iter([("updateCommentParams".to_owned(), Value::String(params.into()))]);
    Some((WriteCommand::Endpoint { path, payload }, editable_text(dialog.get("editableText"))))
}

/// Delete, by structure (VERIFIED live): the item's `navigationEndpoint` is a `confirmDialogEndpoint`
/// whose `confirmDialogRenderer.confirmButton…serviceEndpoint` is a `performCommentActionEndpoint`
/// with an action token: replayed like a vote.
fn delete_of(navigation: &Value) -> Option<WriteCommand> {
    let service = unwrap_command(navigation).pointer(
        "/confirmDialogEndpoint/content/confirmDialogRenderer/confirmButton/buttonRenderer/serviceEndpoint",
    )?;
    unwrap_command(service).get("performCommentActionEndpoint")?;
    Some(WriteCommand::Action(action_token(service)?))
}

/// A plain reading of `editableText`: `simpleText`, the text of its `runs`, or `content`. Anything
/// else (attachments, mentions in a shape not known) is `None`, and the comment's own text is
/// used.
fn editable_text(v: Option<&Value>) -> Option<String> {
    let v = v?;
    let text = v
        .get("simpleText")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| v.get("content").and_then(Value::as_str).map(str::to_owned))
        .or_else(|| {
            v.get("runs")?
                .as_array()?
                .iter()
                .map(|r| r.get("text").and_then(Value::as_str))
                .collect::<Option<String>>()
        })?;
    (!text.is_empty()).then_some(text)
}

/// The viewer's own comment's edit and delete commands, from the toolbar surface entity's menu,
/// matched by structure and never by (localized) text or icon. Call it for a comment whose
/// `author.isCurrentUser` is true and nowhere else. Anything that does not match offers nothing.
pub(crate) fn menu_commands(surface: &Value) -> MenuCommands {
    let mut out = MenuCommands::default();
    for item in menu_items(surface) {
        let Some(nav) = item_renderer(item).and_then(|r| r.get("navigationEndpoint")) else {
            continue;
        };
        if out.edit.is_none() {
            if let Some((edit, text)) = edit_of(nav) {
                out.edit = Some(edit);
                out.edit_text = text;
                continue;
            }
        }
        if out.delete.is_none() {
            out.delete = delete_of(nav);
        }
    }
    out
}

/// One entry per menu item for the debug line that says why edit or delete is missing: which kind
/// the structure matched (`edit`, `delete`, `other`) and the item's `icon.iconType` enum when it
/// is a plain enum string. Enum values only; no text, id or token.
pub(crate) fn menu_probe(surface: &Value) -> String {
    let items = menu_items(surface);
    if items.is_empty() {
        return "menu: no items".into();
    }
    let entries: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let renderer = item_renderer(item);
            let nav = renderer.and_then(|r| r.get("navigationEndpoint"));
            let kind = match nav {
                Some(n) if edit_of(n).is_some() => "edit",
                Some(n) if delete_of(n).is_some() => "delete",
                _ => "other",
            };
            let icon = renderer
                .and_then(|r| r.pointer("/icon/iconType"))
                .and_then(Value::as_str)
                .map_or("none", |s| {
                    if s.len() <= 48
                        && s.bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                    {
                        s
                    } else {
                        "unexpected"
                    }
                });
            format!("item[{i}]: {kind}, icon={icon}")
        })
        .collect();
    format!("menu: {}", entries.join(", "))
}

/// Key names and value types (never values) of everything under a comment's entities that looks
/// like a command or a menu, for the one debug line that says why edit or delete is missing.
pub(crate) fn command_probe(roots: &[(&str, &Value)]) -> String {
    fn ty(v: &Value) -> &'static str {
        match v {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
    fn interesting(k: &str) -> bool {
        let k = k.to_lowercase();
        k.ends_with("endpoint")
            || k.ends_with("command")
            || k.ends_with("action")
            || ["menu", "edit", "delete", "remove", "update"].iter().any(|w| k.contains(w))
    }
    fn go(node: &Value, path: &mut Vec<String>, out: &mut Vec<String>, depth: usize) {
        if depth > MAX_DEPTH || out.len() >= 80 {
            return;
        }
        match node {
            Value::Object(map) => {
                for (k, v) in map {
                    path.push(k.clone());
                    if interesting(k) {
                        out.push(format!("{}: {}", path.join("/"), ty(v)));
                    }
                    go(v, path, out, depth + 1);
                    path.pop();
                }
            }
            Value::Array(items) => {
                path.push("[]".into());
                items.iter().for_each(|e| go(e, path, out, depth + 1));
                path.pop();
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (label, root) in roots {
        go(root, &mut vec![(*label).to_owned()], &mut out, 0);
    }
    out.sort();
    out.dedup();
    if out.is_empty() {
        "no command-like keys".into()
    } else {
        out.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths_must_be_plain_comment_paths() {
        assert_eq!(
            comment_path("/youtubei/v1/comment/create_comment_reply").as_deref(),
            Some("comment/create_comment_reply")
        );
        for bad in [
            "/youtubei/v1/browse",
            "/youtubei/v1/comment/",
            "/youtubei/v1/comment",
            "/youtubei/v1/comment//x",
            "/youtubei/v1/comment/x/",
            "/youtubei/v1/comment/../browse",
            "/youtubei/v1/comment/X",
            "/youtubei/v1/comment/a?b=c",
            "https://evil.example/youtubei/v1/comment/x",
            "//evil.example/youtubei/v1/comment/x",
            "/youtubei/v1/comment/x#frag",
            "/youtubei/v1/comment/x\\y",
            "/youtubei/v1/comment/%2e%2e/browse",
            "/youtubei/v1/comment/x@evil.example",
            "/youtubei/v1/comment/x:8080",
            "/youtubei/v1/comment/x y",
            "/youtubei/v1/comment/x\n",
            "/youtubei/v1/comment/\u{e9}",
            "/youtubei/v1//comment/x",
            "/youtubei/v1/comment/x.json",
            "comment/x",
            "",
        ] {
            assert_eq!(comment_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn debug_never_shows_a_token_or_a_payload() {
        let a = format!("{:?}", WriteCommand::Action("secret-token".into()));
        let e = format!(
            "{:#?}",
            WriteCommand::Endpoint {
                path: "comment/x".into(),
                payload: Map::from_iter([("p".to_owned(), json!("secret-params"))]),
            }
        );
        for shown in [a, e] {
            assert!(!shown.contains("secret"), "{shown}");
        }
    }

    #[test]
    fn the_composer_needs_a_submit_endpoint_with_server_issued_params() {
        let header = |button: Value| {
            json!({ "createRenderer": { "commentSimpleboxRenderer": {
                "placeholderText": { "runs": [{ "text": "Add a comment..." }] },
                "submitButton": button } } })
        };
        let good = header(json!({ "buttonRenderer": { "serviceEndpoint": {
            "commandMetadata": {}, "createCommentEndpoint": { "createCommentParams": "P1" } } } }));
        let (composer, cmd) = create_command(&good).unwrap();
        assert_eq!(composer.placeholder.as_deref(), Some("Add a comment..."));
        let WriteCommand::Endpoint { path, payload } = cmd else { panic!() };
        assert_eq!(path, COMMENT_CREATE_PATH);
        assert_eq!(payload.len(), 1);
        assert_eq!(payload["createCommentParams"], "P1");
        // Signed out: a sign-in endpoint and no submit button. Or no params. Or empty ones.
        assert!(create_command(&json!({ "createRenderer": { "commentSimpleboxRenderer": {
            "placeholderText": { "runs": [{ "text": "x" }] },
            "prepareAccountEndpoint": { "signInEndpoint": {} } } } }))
        .is_none());
        for button in [
            json!({ "buttonRenderer": { "serviceEndpoint": { "createCommentEndpoint": {} } } }),
            json!({ "buttonRenderer": { "serviceEndpoint": { "createCommentEndpoint": { "createCommentParams": "" } } } }),
            json!({ "buttonRenderer": { "serviceEndpoint": { "somethingElseEndpoint": { "createCommentParams": "P" } } } }),
            json!("x"),
        ] {
            assert!(create_command(&header(button)).is_none());
        }
        assert!(create_command(&json!({})).is_none());
    }

    fn reply_button(service: Value) -> Value {
        reply_surface(
            json!({ "replyButton": { "buttonRenderer": { "serviceEndpoint": service } } }),
        )
    }

    fn reply_service(api_url: Option<&str>, params: Value) -> Value {
        let mut service = json!({ "createCommentReplyEndpoint": { "createReplyParams": params } });
        if let Some(url) = api_url {
            service["commandMetadata"] = json!({ "webCommandMetadata": { "apiUrl": url } });
        }
        service
    }

    /// The probe says which keys the live reply endpoint has and where the path comes from, and
    /// never a value.
    #[test]
    fn the_reply_probe_names_keys_and_the_path_source_and_never_values() {
        let named = reply_surface(json!({ "replyButton": { "buttonRenderer": { "text": "secret",
            "serviceEndpoint": reply_service(Some("/youtubei/v1/comment/secret_path"), json!("secret-value")) } } }));
        assert_eq!(
            reply_probe(&named),
            "reply: button=serviceEndpoint,text service=commandMetadata,createCommentReplyEndpoint endpoint=createCommentReplyEndpoint:createReplyParams commandMetadata=webCommandMetadata webCommandMetadata=apiUrl apiUrl=string apiUrl_plain=true createReplyParams=string source=apiUrl"
        );
        // The live case: params, and no apiUrl.
        let live = reply_button(reply_service(None, json!("secret-value")));
        let probe = reply_probe(&live);
        assert!(
            probe.ends_with(
                "apiUrl=missing apiUrl_plain=false createReplyParams=string source=constant"
            ),
            "{probe}"
        );
        // Params missing or empty: nothing is offered.
        let empty = reply_probe(&reply_button(reply_service(None, json!(""))));
        assert!(empty.ends_with("createReplyParams=empty source=none"), "{empty}");
        let none = reply_probe(&reply_button(json!({ "createCommentReplyEndpoint": {} })));
        assert!(none.ends_with("createReplyParams=missing source=none"), "{none}");
        for shown in [&probe, &empty, &none, &reply_probe(&named)] {
            assert!(!shown.contains("secret-value") && !shown.contains("secret_path"), "{shown}");
        }
        assert_eq!(reply_probe(&json!({})), "reply: no replyCommand");
        assert_eq!(
            reply_probe(&json!({ "replyCommand": {} })),
            "reply: replyCommand has no commentReplyDialogRenderer"
        );
    }

    /// A reply is offered when `createReplyParams` is a non-empty string. Its path is the one the
    /// response names if that is a plain comment path, else the (unverified) constant.
    #[test]
    fn a_reply_needs_createreplyparams_and_takes_its_path_from_the_response_or_the_constant() {
        let path_and_params =
            |service: Value| match reply_command(&reply_button(service)).map(|(_, c)| c) {
                Some(WriteCommand::Endpoint { path, payload }) => {
                    assert_eq!(payload.len(), 1, "only createReplyParams is sent");
                    Some((path, payload["createReplyParams"].as_str().unwrap().to_owned()))
                }
                Some(other) => panic!("{other:?}"),
                None => None,
            };
        // The live shape: no apiUrl, so the constant.
        assert_eq!(
            path_and_params(reply_service(None, json!("R1"))),
            Some((COMMENT_REPLY_PATH.to_owned(), "R1".to_owned()))
        );
        // A plain path the response names wins; a hostile or foreign one is not used.
        assert_eq!(
            path_and_params(reply_service(Some("/youtubei/v1/comment/reply_v2"), json!("R2")))
                .unwrap()
                .0,
            "comment/reply_v2"
        );
        for hostile in [
            "/youtubei/v1/browse",
            "https://evil.example/youtubei/v1/comment/x",
            "/youtubei/v1/comment/../x",
        ] {
            assert_eq!(
                path_and_params(reply_service(Some(hostile), json!("R3"))).unwrap().0,
                COMMENT_REPLY_PATH,
                "{hostile}"
            );
        }
        // No params (missing, empty, not a string): no reply, whatever the path says.
        for params in [json!(""), json!(null), json!(7), json!({})] {
            assert_eq!(
                path_and_params(reply_service(Some("/youtubei/v1/comment/x"), params)),
                None
            );
        }
        assert_eq!(path_and_params(json!({ "createCommentReplyEndpoint": {} })), None);
        // Other fields of the endpoint are not sent.
        assert_eq!(
            path_and_params(
                json!({ "createCommentReplyEndpoint": { "createReplyParams": "R4", "other": "x" } })
            ),
            Some((COMMENT_REPLY_PATH.to_owned(), "R4".to_owned()))
        );
    }

    fn reply_surface(dialog: Value) -> Value {
        json!({ "replyCommand": { "innertubeCommand": { "createCommentReplyDialogEndpoint": {
            "dialog": { "commentReplyDialogRenderer": dialog } } } } })
    }

    #[test]
    fn a_reply_needs_a_dialog_with_a_reply_button() {
        let ok = reply_surface(json!({
            "placeholderText": { "runs": [{ "text": "Add a reply..." }] },
            "replyButton": { "buttonRenderer": { "serviceEndpoint": reply_service(None, json!("R1")) } },
            "cancelButton": { "buttonRenderer": {} } }));
        let (placeholder, cmd) = reply_command(&ok).unwrap();
        assert_eq!(placeholder.as_deref(), Some("Add a reply..."));
        assert!(
            matches!(cmd, WriteCommand::Endpoint { ref path, .. } if path == COMMENT_REPLY_PATH)
        );
        for dialog in [
            json!({ "cancelButton": { "buttonRenderer": {} } }),
            json!({ "replyButton": { "buttonRenderer": {} } }),
        ] {
            assert!(reply_command(&reply_surface(dialog)).is_none());
        }
        assert!(reply_command(&json!({ "replyCommand": { "innertubeCommand": {} } })).is_none());
        assert!(reply_command(&json!({})).is_none());
    }

    /// The real structure, with placeholders (SYNTHETIC values on a VERIFIED shape).
    fn menu(items: Vec<Value>) -> Value {
        json!({ "menuCommand": { "innertubeCommand": { "menuEndpoint": { "menu": { "menuRenderer": {
            "items": items } } } } } })
    }

    fn edit_item(icon: Option<&str>, api_url: Option<&str>, params: Value) -> Value {
        let mut service = json!({ "updateCommentEndpoint": { "updateCommentParams": params } });
        if let Some(url) = api_url {
            service["commandMetadata"] = json!({ "webCommandMetadata": { "apiUrl": url } });
        }
        let mut renderer = json!({ "navigationEndpoint": { "updateCommentDialogEndpoint": { "dialog": {
            "commentDialogRenderer": {
                "editableText": { "runs": [{ "text": "old " }, { "text": "text" }] },
                "submitButton": { "buttonRenderer": { "serviceEndpoint": service } } } } } } });
        if let Some(icon) = icon {
            renderer["icon"] = json!({ "iconType": icon });
        }
        json!({ "menuNavigationItemRenderer": renderer })
    }

    fn delete_item(icon: Option<&str>, action: Value) -> Value {
        let mut renderer = json!({ "navigationEndpoint": { "confirmDialogEndpoint": { "content": {
            "confirmDialogRenderer": { "confirmButton": { "buttonRenderer": { "serviceEndpoint": {
                "performCommentActionEndpoint": action } } } } } } } });
        if let Some(icon) = icon {
            renderer["icon"] = json!({ "iconType": icon });
        }
        json!({ "menuNavigationItemRenderer": renderer })
    }

    #[test]
    fn edit_and_delete_are_matched_by_the_menus_structure_not_by_words() {
        let surface = menu(vec![
            // Neither a keyword nor an icon is needed: the shape alone decides.
            edit_item(None, Some("/youtubei/v1/comment/update_comment"), json!("E1")),
            delete_item(None, json!({ "action": "D1" })),
        ]);
        let got = menu_commands(&surface);
        let Some(WriteCommand::Endpoint { path, payload }) = got.edit else { panic!("edit") };
        assert_eq!(path, "comment/update_comment");
        assert_eq!(payload.len(), 1);
        assert_eq!(payload["updateCommentParams"], "E1");
        assert_eq!(got.edit_text.as_deref(), Some("old text"), "editableText runs");
        assert_eq!(got.delete, Some(WriteCommand::Action("D1".into())));

        // Order and extra items do not matter, and a localized label is never looked at.
        let surface = menu(vec![
            json!({ "menuNavigationItemRenderer": { "text": { "runs": [{ "text": "Supprimer" }] },
                "icon": { "iconType": "FLAG" }, "navigationEndpoint": { "reportEndpoint": {} } } }),
            delete_item(Some("DELETE"), json!({ "actions": ["D2"] })),
            edit_item(Some("EDIT"), Some("/youtubei/v1/comment/update_comment"), json!("E2")),
        ]);
        let got = menu_commands(&surface);
        assert_eq!(got.delete, Some(WriteCommand::Action("D2".into())));
        assert!(got.edit.is_some());
    }

    #[test]
    fn an_edit_without_a_usable_path_falls_back_to_the_constant_and_a_hostile_one_is_not_used() {
        let path_of = |item: Value| {
            let Some(WriteCommand::Endpoint { path, .. }) = menu_commands(&menu(vec![item])).edit
            else {
                panic!("expected an edit")
            };
            path
        };
        assert_eq!(path_of(edit_item(None, None, json!("E"))), COMMENT_UPDATE_PATH);
        for hostile in [
            "/youtubei/v1/browse",
            "https://evil.example/youtubei/v1/comment/x",
            "/youtubei/v1/comment/../browse",
            "/youtubei/v1/comment/x?y",
        ] {
            assert_eq!(
                path_of(edit_item(None, Some(hostile), json!("E"))),
                COMMENT_UPDATE_PATH,
                "{hostile}"
            );
        }
        // A plain path the response names wins over the fallback.
        assert_eq!(
            path_of(edit_item(None, Some("/youtubei/v1/comment/update_comment_v2"), json!("E"))),
            "comment/update_comment_v2"
        );
    }

    #[test]
    fn anything_that_does_not_match_the_structure_offers_nothing() {
        for surface in [
            json!({}),
            json!(null),
            menu(vec![]),
            // An edit with no params, empty params, or non-string params.
            menu(vec![edit_item(None, None, json!(""))]),
            menu(vec![edit_item(None, None, json!(7))]),
            // A delete whose button is not a perform-action, or has no token.
            menu(vec![delete_item(None, json!({}))]),
            menu(vec![delete_item(None, json!({ "action": "" }))]),
            menu(vec![json!({ "menuNavigationItemRenderer": { "navigationEndpoint": {
                "confirmDialogEndpoint": { "content": { "confirmDialogRenderer": { "confirmButton": {
                    "buttonRenderer": { "serviceEndpoint": { "somethingElseEndpoint": { "action": "X" } } } } } } } } } })]),
            // Items that are not shaped like items.
            menu(vec![json!(1), json!({}), json!({ "x": { "navigationEndpoint": 3 } })]),
            // Edit and delete commands anywhere but the menu are not looked at.
            json!({ "elsewhere": menu(vec![edit_item(None, None, json!("E"))]) }),
        ] {
            assert_eq!(menu_commands(&surface), MenuCommands::default(), "{surface}");
        }
    }

    /// Each kind is the first match, and the probe says what each item matched and its icon enum.
    #[test]
    fn the_menu_probe_names_kinds_and_icon_enums_and_never_text() {
        let surface = menu(vec![
            edit_item(Some("EDIT"), None, json!("secret-edit-params")),
            delete_item(Some("DELETE"), json!({ "action": "secret-token" })),
            json!({ "menuNavigationItemRenderer": { "icon": { "iconType": "not an enum!" },
                "text": { "runs": [{ "text": "secret label" }] }, "navigationEndpoint": {} } }),
            json!({ "menuNavigationItemRenderer": { "navigationEndpoint": {} } }),
        ]);
        let probe = menu_probe(&surface);
        assert_eq!(
            probe,
            "menu: item[0]: edit, icon=EDIT, item[1]: delete, icon=DELETE, item[2]: other, icon=unexpected, item[3]: other, icon=none"
        );
        assert!(!probe.contains("secret"));
        assert_eq!(menu_probe(&json!({})), "menu: no items");
    }

    #[test]
    fn editable_text_is_read_only_when_it_is_plain() {
        assert_eq!(editable_text(Some(&json!({ "simpleText": "a" }))).as_deref(), Some("a"));
        assert_eq!(editable_text(Some(&json!({ "content": "b" }))).as_deref(), Some("b"));
        assert_eq!(
            editable_text(Some(&json!({ "runs": [{ "text": "c" }, { "text": "d" }] }))).as_deref(),
            Some("cd")
        );
        for odd in [
            json!({ "runs": [{ "text": "a" }, { "emoji": 1 }] }),
            json!({ "runs": [] }),
            json!({}),
            json!(5),
        ] {
            assert_eq!(editable_text(Some(&odd)), None, "{odd}");
        }
        assert_eq!(editable_text(None), None);
    }

    #[test]
    fn the_probe_lists_command_like_keys_and_types_and_never_values() {
        let node = json!({ "menu": { "items": [{ "deleteCommentCommand": { "innertubeCommand": {
            "performCommentActionEndpoint": { "action": "secret-token" } } } }] },
            "text": "secret comment text" });
        let probe = command_probe(&[("surface", &node)]);
        assert!(probe.contains("surface/menu: object"), "{probe}");
        assert!(probe.contains("deleteCommentCommand: object"), "{probe}");
        assert!(probe.contains("performCommentActionEndpoint: object"));
        assert!(!probe.contains("secret"), "{probe}");
        assert_eq!(command_probe(&[("x", &json!({}))]), "no command-like keys");
    }
}
