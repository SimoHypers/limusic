//! Write commands (post, reply, edit, delete) taken from a signed-in comments response and
//! replayed as issued, the way youtubei.js v18.1.0 (MIT) replays a `NavigationEndpoint`. Edit and
//! delete come from the viewer's own comment menu, matched by structure. Commands stay in Rust:
//! `Debug` prints no payload or token, and none is serialized to the UI.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::metadata::find_all;

/// As youtubei.js (MIT) `Comments.createComment`.
pub const COMMENT_CREATE_PATH: &str = "comment/create_comment";

/// Reply path; the reply button names none.
pub const COMMENT_REPLY_PATH: &str = "comment/create_comment_reply";

/// Edit path when the update button names none.
pub const COMMENT_UPDATE_PATH: &str = "comment/update_comment";

/// A reply's edit path when its button names none.
pub const COMMENT_UPDATE_REPLY_PATH: &str = "comment/update_comment_reply";

/// The body field for a write's text.
pub const TEXT_FIELD: &str = "commentText";

/// The body field for a reply's new text (`comment/update_comment_reply`).
pub const REPLY_EDIT_TEXT_FIELD: &str = "replyText";

/// What a viewer can write on a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentWrite {
    Reply,
    Edit,
    Delete,
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
    /// A `performCommentActionEndpoint` token (a delete), replayed like a vote.
    Action(String),
    /// `path` (always `comment/…`), the endpoint's own fields, the body field the text goes in,
    /// and the write's name in log lines.
    Endpoint {
        path: String,
        payload: Map<String, Value>,
        text_field: &'static str,
        kind: &'static str,
    },
}

impl WriteCommand {
    /// The name of a write in log lines (`create`, `reply`, `edit`, `edit_reply`, `delete`).
    pub fn log_kind(&self) -> &'static str {
        match self {
            WriteCommand::Action(_) => "delete",
            WriteCommand::Endpoint { kind, .. } => kind,
        }
    }
}

impl fmt::Debug for WriteCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteCommand::Action(_) => f.write_str("WriteCommand::Action(<redacted>)"),
            WriteCommand::Endpoint { path, kind, .. } => write!(
                f,
                "WriteCommand::Endpoint {{ path: {path:?}, kind: {kind:?}, payload: <redacted> }}"
            ),
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

/// A path under `comment/` made of lowercase letters, digits, `_` and single `/` only. Checked on
/// read and again on send, since the path is appended to the base URL as text.
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

/// A `buttonRenderer`'s command: the first of `serviceEndpoint`, `navigationEndpoint`, `command`.
fn button_command(button: &Value) -> Option<&Value> {
    ["serviceEndpoint", "navigationEndpoint", "command"].iter().find_map(|k| button.get(k))
}

/// The `performCommentActionEndpoint` token under a command: `action`, or the first of `actions`.
/// Searched for, so a wrapper YouTube adds does not lose it. Ported from youtubei.js (MIT)
/// `CommentView.applyMutations` / `PerformCommentActionEndpoint`.
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

/// The comment box (`createRenderer.commentSimpleboxRenderer`): its placeholder and the submit
/// button's `createCommentParams`. The signed-out box has no submit button, so it yields nothing.
pub(crate) fn create_command(header: &Value) -> Option<(Composer, WriteCommand)> {
    let simplebox = header.pointer("/createRenderer/commentSimpleboxRenderer")?;
    let button = simplebox.pointer("/submitButton/buttonRenderer")?;
    let data = button_command(button).map(unwrap_command)?;
    let params = data.pointer("/createCommentEndpoint/createCommentParams")?.as_str()?;
    if params.is_empty() {
        return None;
    }
    let payload =
        Map::from_iter([("createCommentParams".to_owned(), Value::String(params.into()))]);
    Some((
        Composer { placeholder: runs_text(simplebox.get("placeholderText")) },
        WriteCommand::Endpoint {
            path: COMMENT_CREATE_PATH.to_owned(),
            payload,
            text_field: TEXT_FIELD,
            kind: "create",
        },
    ))
}

/// A write a button's command can carry: the endpoint and its params field, the path when the
/// command names none, the body field for the text, and the name in log lines.
struct Shape {
    kind: &'static str,
    endpoint: &'static str,
    field: &'static str,
    fallback: &'static str,
    text_field: &'static str,
}

const REPLY: Shape = Shape {
    kind: "reply",
    endpoint: "createCommentReplyEndpoint",
    field: "createReplyParams",
    fallback: COMMENT_REPLY_PATH,
    text_field: TEXT_FIELD,
};

/// The command under a button's `service` endpoint, if it carries a non-empty `shape` params
/// string. Its path is the plain `comment/…` `apiUrl` the command names, else the fallback.
fn replay(service: &Value, shape: &Shape) -> Option<WriteCommand> {
    let data = unwrap_command(service);
    let params = data.get(shape.endpoint)?.get(shape.field)?.as_str().filter(|p| !p.is_empty())?;
    // The command that holds the endpoint names the path; `service` is tried too, for a wrapper.
    let path = [data, service]
        .iter()
        .find_map(|c| c.pointer("/commandMetadata/webCommandMetadata/apiUrl")?.as_str())
        .and_then(comment_path)
        .unwrap_or_else(|| shape.fallback.to_owned());
    let payload = Map::from_iter([(shape.field.to_owned(), Value::String(params.into()))]);
    Some(WriteCommand::Endpoint { path, payload, text_field: shape.text_field, kind: shape.kind })
}

/// A comment's reply command (`replyCommand…commentReplyDialogRenderer.replyButton`) and the
/// dialog's placeholder.
pub(crate) fn reply_command(surface: &Value) -> Option<(Option<String>, WriteCommand)> {
    let dialog =
        find_all(surface.get("replyCommand")?, "commentReplyDialogRenderer").into_iter().next()?;
    let service = button_command(dialog.pointer("/replyButton/buttonRenderer")?)?;
    let command = replay(service, &REPLY)?;
    Some((runs_text(dialog.get("placeholderText")), command))
}

/// One of the two edit dialogs a menu item can open: the dialog under the item's
/// `navigationEndpoint`, its button's service endpoint, and the write it carries.
struct EditShape {
    dialog: &'static str,
    button: &'static str,
    shape: Shape,
}

const EDIT_SHAPES: [EditShape; 2] = [
    // A top-level comment.
    EditShape {
        dialog: "/updateCommentDialogEndpoint/dialog/commentDialogRenderer",
        button: "/submitButton/buttonRenderer/serviceEndpoint",
        shape: Shape {
            kind: "edit",
            endpoint: "updateCommentEndpoint",
            field: "updateCommentParams",
            fallback: COMMENT_UPDATE_PATH,
            text_field: TEXT_FIELD,
        },
    },
    // A reply.
    EditShape {
        dialog: "/updateCommentReplyDialogEndpoint/dialog/commentReplyDialogRenderer",
        button: "/replyButton/buttonRenderer/serviceEndpoint",
        shape: Shape {
            kind: "edit_reply",
            endpoint: "updateCommentReplyEndpoint",
            field: "updateReplyParams",
            fallback: COMMENT_UPDATE_REPLY_PATH,
            text_field: REPLY_EDIT_TEXT_FIELD,
        },
    },
];

/// What the viewer's own comment's menu offered.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct MenuCommands {
    pub edit: Option<WriteCommand>,
    /// The text the edit dialog is pre-filled with (`editableText`), if it is plain to read.
    pub edit_text: Option<String>,
    pub delete: Option<WriteCommand>,
}

/// The items of every `menuRenderer` under the toolbar surface entity's `menuCommand`.
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

/// Edit, in one of the [`EDIT_SHAPES`]: the command and the dialog's `editableText` prefill.
fn edit_of(navigation: &Value) -> Option<(WriteCommand, Option<String>)> {
    EDIT_SHAPES.iter().find_map(|edit| {
        let dialog = unwrap_command(navigation).pointer(edit.dialog)?;
        let command = replay(dialog.pointer(edit.button)?, &edit.shape)?;
        Some((command, editable_text(dialog.get("editableText"))))
    })
}

/// Delete: a `confirmDialogEndpoint` whose confirm button holds a `performCommentActionEndpoint`
/// token, replayed like a vote.
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

#[cfg(test)]
pub(crate) mod tests {
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
                text_field: TEXT_FIELD,
                kind: "edit",
            }
        );
        for shown in [a, e] {
            assert!(!shown.contains("secret"), "{shown}");
        }
    }

    /// The path, params and text field of an `Endpoint` command.
    fn parts(cmd: &WriteCommand) -> (&str, &Map<String, Value>, &'static str) {
        match cmd {
            WriteCommand::Endpoint { path, payload, text_field, .. } => (path, payload, text_field),
            other => panic!("{other:?}"),
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
        let (path, payload, text_field) = parts(&cmd);
        assert_eq!((path, text_field, cmd.log_kind()), (COMMENT_CREATE_PATH, TEXT_FIELD, "create"));
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

    /// A reply is offered when `createReplyParams` is a non-empty string. Its path is the one the
    /// response names if that is a plain comment path, else the constant.
    #[test]
    fn a_reply_needs_createreplyparams_and_takes_its_path_from_the_response_or_the_constant() {
        let path_and_params = |service: Value| {
            reply_command(&reply_button(service)).map(|(_, c)| {
                let (path, payload, text_field) = parts(&c);
                assert_eq!(payload.len(), 1, "only createReplyParams is sent");
                assert_eq!((text_field, c.log_kind()), (TEXT_FIELD, "reply"));
                (path.to_owned(), payload["createReplyParams"].as_str().unwrap().to_owned())
            })
        };
        // No apiUrl: the constant.
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
        assert_eq!(parts(&cmd).0, COMMENT_REPLY_PATH);
        for dialog in [
            json!({ "cancelButton": { "buttonRenderer": {} } }),
            json!({ "replyButton": { "buttonRenderer": {} } }),
        ] {
            assert!(reply_command(&reply_surface(dialog)).is_none());
        }
        assert!(reply_command(&json!({ "replyCommand": { "innertubeCommand": {} } })).is_none());
        assert!(reply_command(&json!({})).is_none());
    }

    /// A toolbar surface whose menu holds `items`.
    pub(crate) fn menu(items: Vec<Value>) -> Value {
        json!({ "menuCommand": { "innertubeCommand": { "menuEndpoint": { "menu": { "menuRenderer": {
            "items": items } } } } } })
    }

    pub(crate) fn edit_item(icon: Option<&str>, api_url: Option<&str>, params: Value) -> Value {
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

    /// A reply's Edit menu item.
    pub(crate) fn reply_edit_item(
        icon: Option<&str>,
        api_url: Option<&str>,
        params: Value,
    ) -> Value {
        let mut service = json!({ "updateCommentReplyEndpoint": { "updateReplyParams": params } });
        if let Some(url) = api_url {
            service["commandMetadata"] = json!({ "webCommandMetadata": { "apiUrl": url } });
        }
        let mut renderer = json!({ "navigationEndpoint": { "updateCommentReplyDialogEndpoint": {
            "dialog": { "commentReplyDialogRenderer": {
                "editableText": { "runs": [{ "text": "old " }, { "text": "reply" }] },
                "replyButton": { "buttonRenderer": { "serviceEndpoint": service } } } } } } });
        if let Some(icon) = icon {
            renderer["icon"] = json!({ "iconType": icon });
        }
        json!({ "menuNavigationItemRenderer": renderer })
    }

    pub(crate) fn delete_item(icon: Option<&str>, action: Value) -> Value {
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
        let edit = got.edit.as_ref().expect("edit");
        let (path, payload, text_field) = parts(edit);
        assert_eq!(
            (path, text_field, edit.log_kind()),
            ("comment/update_comment", TEXT_FIELD, "edit")
        );
        assert_eq!(payload.len(), 1);
        assert_eq!(payload["updateCommentParams"], "E1");
        assert_eq!(got.edit_text.as_deref(), Some("old text"), "editableText runs");
        assert_eq!(got.delete, Some(WriteCommand::Action("D1".into())));
        assert_eq!(got.delete.unwrap().log_kind(), "delete");

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

        // A delete with no edit beside it.
        let only = menu_commands(&menu(vec![delete_item(None, json!({ "action": "D3" }))]));
        assert_eq!((only.edit, only.delete), (None, Some(WriteCommand::Action("D3".into()))));
    }

    #[test]
    fn an_edit_without_a_usable_path_falls_back_to_the_constant_and_a_hostile_one_is_not_used() {
        let path_of = |item: Value| {
            parts(menu_commands(&menu(vec![item])).edit.as_ref().unwrap()).0.to_owned()
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

    /// A reply's Edit is its own dialog: `updateReplyParams` is sent (nothing else from the
    /// endpoint) with the text as `replyText`, to the path the command names when it is plain,
    /// else to the constant; `editableText` is the prefill. A half of one shape and a half of the
    /// other is not an edit.
    #[test]
    fn a_replys_edit_is_matched_by_its_own_dialog_structure() {
        let edit = |item: Value| menu_commands(&menu(vec![item]));
        let got = edit(reply_edit_item(Some("EDIT"), None, json!("ER1")));
        let cmd = got.edit.as_ref().expect("edit");
        let (path, payload, text_field) = parts(cmd);
        assert_eq!(
            (path, text_field, cmd.log_kind()),
            (COMMENT_UPDATE_REPLY_PATH, REPLY_EDIT_TEXT_FIELD, "edit_reply")
        );
        assert_eq!(payload.len(), 1, "only updateReplyParams is sent");
        assert_eq!(payload["updateReplyParams"], "ER1");
        assert_eq!(got.edit_text.as_deref(), Some("old reply"), "editableText runs");
        let path_of = |item: Value| parts(edit(item).edit.as_ref().unwrap()).0.to_owned();
        assert_eq!(
            path_of(reply_edit_item(
                None,
                Some("/youtubei/v1/comment/update_reply_v2"),
                json!("E")
            )),
            "comment/update_reply_v2"
        );
        for hostile in ["/youtubei/v1/browse", "https://evil.example/youtubei/v1/comment/x"] {
            assert_eq!(
                path_of(reply_edit_item(None, Some(hostile), json!("E"))),
                COMMENT_UPDATE_REPLY_PATH,
                "{hostile}"
            );
        }
        // No params, empty params, or the shapes mixed up: no edit.
        let mixed = [
            json!({ "menuNavigationItemRenderer": { "navigationEndpoint": {
                "updateCommentReplyDialogEndpoint": { "dialog": { "commentReplyDialogRenderer": {
                    "replyButton": { "buttonRenderer": { "serviceEndpoint": {
                        "updateCommentEndpoint": { "updateCommentParams": "E" } } } } } } } } } }),
            json!({ "menuNavigationItemRenderer": { "navigationEndpoint": {
                "updateCommentReplyDialogEndpoint": { "dialog": { "commentReplyDialogRenderer": {
                    "replyButton": { "buttonRenderer": { "serviceEndpoint": {
                        "createCommentReplyEndpoint": { "createReplyParams": "E" } } } } } } } } } }),
            json!({ "menuNavigationItemRenderer": { "navigationEndpoint": {
                "updateCommentDialogEndpoint": { "dialog": { "commentDialogRenderer": {
                    "submitButton": { "buttonRenderer": { "serviceEndpoint": {
                        "updateCommentReplyEndpoint": { "updateReplyParams": "E" } } } } } } } } } }),
        ];
        for item in [reply_edit_item(None, None, json!("")), reply_edit_item(None, None, json!(7))]
            .into_iter()
            .chain(mixed)
        {
            assert_eq!(edit(item.clone()), MenuCommands::default(), "{item}");
        }
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
}
