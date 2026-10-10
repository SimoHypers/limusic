//! What the app remembers about the comments on screen, none of which the UI is allowed to see:
//!
//! - **Provenance of every token handed to the UI** (next page, sort, replies): which identity
//!   issued it, anonymous or a specific account. A token goes back the way it came, and never as
//!   an account that is no longer the active one.
//! - **Action tokens** (like/unlike/dislike/undislike) per `(account identity, comment id)`, with
//!   the vote they were read at. A token issued to one account or channel is never sent as another.
//!
//! Plain data, no I/O: the app wraps it in a `Mutex` and clears it on every auth change and when
//! the loaded track changes. Both maps are bounded and drop their oldest entries first.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;

use crate::models::comment_write::{CommentWrite, WriteCommand, WriteCommands};
use crate::models::comments::{
    available_actions, ActionTokens, Comment, CommentAction, CommentReplies, CommentsPage,
    VoteState,
};

/// Who a token was issued to.
#[derive(Clone, PartialEq, Eq)]
pub enum Provenance {
    /// An anonymous read.
    Anonymous,
    /// A read as the account whose [`crate::InnerTube::comments_identity`] this is.
    Account(String),
}

/// The identity is a hash of the account's secret, so `Debug` (a stray `{:?}` in a log line) says
/// only that there is one.
impl std::fmt::Debug for Provenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provenance::Anonymous => f.write_str("Anonymous"),
            Provenance::Account(_) => f.write_str("Account(<redacted>)"),
        }
    }
}

/// Why an action was not sent. Plain reasons for the caller to word; never a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionError {
    /// Nothing is remembered for this comment under this identity (never loaded, evicted, or
    /// loaded as someone else).
    Unknown,
    /// The previous action on this comment has not finished.
    Busy,
    /// The comment's vote and its tokens do not offer this action.
    Unavailable,
}

/// An action approved to be sent. Holds the token, so it is deliberately not `Debug`.
pub struct ActionTicket {
    token: String,
    /// What the vote becomes if YouTube accepts it.
    pub resulting_vote: VoteState,
}

impl ActionTicket {
    pub fn token(&self) -> &str {
        &self.token
    }
}

/// A map that forgets its oldest key first once it is over `cap`.
struct Bounded<K, V> {
    map: HashMap<K, V>,
    order: VecDeque<K>,
    cap: usize,
}

impl<K: Clone + Eq + Hash, V> Bounded<K, V> {
    fn new(cap: usize) -> Self {
        Bounded { map: HashMap::new(), order: VecDeque::new(), cap }
    }

    fn insert(&mut self, key: K, value: V) {
        if self.map.insert(key.clone(), value).is_none() {
            self.order.push_back(key);
        }
        while self.map.len() > self.cap {
            match self.order.pop_front() {
                Some(oldest) => {
                    self.map.remove(&oldest);
                }
                None => break,
            }
        }
    }

    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }
}

/// How much of each is kept. A page is ~20 comments, so this is dozens of pages of both.
const MAX_TOKENS: usize = 512;
const MAX_COMMENTS: usize = 1500;

struct Entry {
    vote: Option<VoteState>,
    tokens: ActionTokens,
    writes: WriteCommands,
    entity_key: Option<String>,
}

/// What stands in for a comment id in the busy set while the top-level comment is being posted.
const COMPOSER: &str = "\0composer";

/// A write approved to be sent: the server-issued command to replay. Not `Debug`, as it holds
/// params or a token.
pub struct WriteTicket {
    command: WriteCommand,
    entity_key: Option<String>,
}

impl WriteTicket {
    pub fn command(&self) -> &WriteCommand {
        &self.command
    }

    /// The comment's entity key, to look for its delete mutation in an answer (debug line only).
    pub fn entity_key(&self) -> Option<&str> {
        self.entity_key.as_deref()
    }
}

pub struct CommentsSession {
    tokens: Bounded<String, Provenance>,
    comments: Bounded<(String, String), Entry>,
    busy: HashSet<(String, String)>,
    /// The command that posts a top-level comment on the track on screen, and who it was issued
    /// to. Replaced by every page read as an account that carries one.
    composer: Option<(String, WriteCommand)>,
}

impl Default for CommentsSession {
    fn default() -> Self {
        Self::with_caps(MAX_TOKENS, MAX_COMMENTS)
    }
}

impl CommentsSession {
    fn with_caps(tokens: usize, comments: usize) -> Self {
        CommentsSession {
            tokens: Bounded::new(tokens),
            comments: Bounded::new(comments),
            busy: HashSet::new(),
            composer: None,
        }
    }

    /// Forget everything: a sign-in, sign-out, account or channel switch, a heal that changed the
    /// session, or a different track.
    pub fn clear(&mut self) {
        self.tokens.clear();
        self.comments.clear();
        self.busy.clear();
        self.composer = None;
    }

    /// Remember a page just handed to the UI: where each of its tokens came from and, for a page
    /// read as an account, each comment's vote and action tokens.
    pub fn remember_page(&mut self, provenance: &Provenance, page: &CommentsPage) {
        if let (Provenance::Account(identity), Some(create)) =
            (provenance, page.header.as_ref().and_then(|h| h.create.as_ref()))
        {
            self.composer = Some((identity.clone(), create.clone()));
        }
        let sort_tokens = page.header.iter().flat_map(|h| h.sorts.iter()).map(|s| s.token.as_str());
        let thread_tokens = page.threads.iter().filter_map(|t| t.replies_token.as_deref());
        for token in
            page.continuation.as_deref().into_iter().chain(sort_tokens).chain(thread_tokens)
        {
            self.tokens.insert(token.to_owned(), provenance.clone());
        }
        for thread in &page.threads {
            self.remember_comment(provenance, &thread.comment);
            for reply in &thread.replies {
                self.remember_comment(provenance, reply);
            }
        }
    }

    /// A comment (and its replies) a write put on the page: its tokens and commands are kept like
    /// any read's, under the identity that wrote it.
    pub fn remember_thread(&mut self, provenance: &Provenance, thread: &crate::CommentThread) {
        self.remember_comment(provenance, &thread.comment);
        for reply in &thread.replies {
            self.remember_comment(provenance, reply);
        }
    }

    /// The same for a page of replies.
    pub fn remember_replies(&mut self, provenance: &Provenance, replies: &CommentReplies) {
        if let Some(token) = &replies.continuation {
            self.tokens.insert(token.clone(), provenance.clone());
        }
        for reply in &replies.replies {
            self.remember_comment(provenance, reply);
        }
    }

    fn remember_comment(&mut self, provenance: &Provenance, comment: &Comment) {
        // Only an account has tokens worth keeping. Never overwrite a comment mid-action: its
        // vote is about to be decided by the answer.
        let Provenance::Account(identity) = provenance else { return };
        let key = (identity.clone(), comment.id.clone());
        if self.busy.contains(&key) {
            return;
        }
        self.comments.insert(
            key,
            Entry {
                vote: comment.vote,
                tokens: comment.tokens.clone(),
                writes: comment.commands.clone(),
                entity_key: comment.entity_key.get().map(str::to_owned),
            },
        );
    }

    /// Who issued `token`. `None` for a token this session never handed out (or has forgotten).
    pub fn provenance(&self, token: &str) -> Option<Provenance> {
        self.tokens.map.get(token).cloned()
    }

    /// Approve `action` on a comment for the account `identity`, or say why not. Marks the comment
    /// busy until [`Self::finish_action`], so a second click cannot start a second request.
    ///
    /// Which actions are offered is exactly [`available_actions`] for the comment's vote and
    /// tokens. A transition between like and dislike is therefore one request with the target
    /// action's own token (what youtubei.js v18.1.0 does too), and an action with no token in the
    /// current state is simply `Unavailable`: no two-step transition is ever invented.
    pub fn begin_action(
        &mut self,
        identity: &str,
        comment_id: &str,
        action: CommentAction,
    ) -> Result<ActionTicket, ActionError> {
        let key = (identity.to_owned(), comment_id.to_owned());
        let entry = self.comments.map.get(&key).ok_or(ActionError::Unknown)?;
        if self.busy.contains(&key) {
            return Err(ActionError::Busy);
        }
        if !available_actions(entry.vote, &entry.tokens).contains(&action) {
            return Err(ActionError::Unavailable);
        }
        let token = entry.tokens.get(action).ok_or(ActionError::Unavailable)?.to_owned();
        self.busy.insert(key);
        Ok(ActionTicket { token, resulting_vote: action.resulting_vote() })
    }

    /// The request is over. On success the comment's vote becomes the ticket's, and what it
    /// offers next is returned; on failure nothing changes. Either way the comment is free again.
    pub fn finish_action(
        &mut self,
        identity: &str,
        comment_id: &str,
        accepted: Option<VoteState>,
    ) -> Option<(VoteState, Vec<CommentAction>)> {
        let key = (identity.to_owned(), comment_id.to_owned());
        self.busy.remove(&key);
        let vote = accepted?;
        let entry = self.comments.map.get_mut(&key)?;
        entry.vote = Some(vote);
        Some((vote, available_actions(entry.vote, &entry.tokens)))
    }
}

impl CommentsSession {
    /// Approve a reply, edit or delete of a comment for the account `identity`: only if the
    /// response issued that command to this identity. Marks the comment busy until
    /// [`Self::finish_write`].
    pub fn begin_write(
        &mut self,
        identity: &str,
        comment_id: &str,
        write: CommentWrite,
    ) -> Result<WriteTicket, ActionError> {
        let key = (identity.to_owned(), comment_id.to_owned());
        let entry = self.comments.map.get(&key).ok_or(ActionError::Unknown)?;
        if self.busy.contains(&key) {
            return Err(ActionError::Busy);
        }
        let command = entry.writes.get(write).ok_or(ActionError::Unavailable)?.clone();
        let entity_key = entry.entity_key.clone();
        self.busy.insert(key);
        Ok(WriteTicket { command, entity_key })
    }

    /// The write is over. A delete that YouTube accepted forgets the comment, so nothing can be
    /// sent for it again; anything else leaves it as it was.
    pub fn finish_write(&mut self, identity: &str, comment_id: &str, removed: bool) {
        let key = (identity.to_owned(), comment_id.to_owned());
        self.busy.remove(&key);
        if removed {
            self.comments.map.remove(&key);
        }
    }

    /// YouTube said the comment is gone (404): forget what is kept for it, so nothing can be sent
    /// for it again.
    pub fn forget(&mut self, identity: &str, comment_id: &str) {
        let key = (identity.to_owned(), comment_id.to_owned());
        self.busy.remove(&key);
        self.comments.map.remove(&key);
    }

    /// Approve posting a top-level comment for `identity`: only with the command the page read as
    /// that identity carried. One post at a time.
    pub fn begin_create(&mut self, identity: &str) -> Result<WriteTicket, ActionError> {
        let command = match &self.composer {
            Some((who, command)) if who == identity => command.clone(),
            _ => return Err(ActionError::Unknown),
        };
        if !self.busy.insert((identity.to_owned(), COMPOSER.to_owned())) {
            return Err(ActionError::Busy);
        }
        Ok(WriteTicket { command, entity_key: None })
    }

    pub fn finish_create(&mut self, identity: &str) {
        self.busy.remove(&(identity.to_owned(), COMPOSER.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::comments::{parse_comment_replies, parse_comments_page};
    use CommentAction::*;

    const SIGNED_IN: &str = include_str!("../tests/fixtures/comments/signed_in_synthetic.json");
    const INITIAL: &str = include_str!("../tests/fixtures/comments/initial_pinned_hearted.json");
    const REPLIES: &str = include_str!("../tests/fixtures/comments/replies.json");

    fn account(id: &str) -> Provenance {
        Provenance::Account(id.to_owned())
    }

    fn signed_in_page() -> CommentsPage {
        let mut page = parse_comments_page(&serde_json::from_str(SIGNED_IN).unwrap(), false);
        page.read_as_account = true;
        page
    }

    fn comment_id(page: &CommentsPage, ends_with: &str) -> String {
        page.threads
            .iter()
            .find(|t| t.comment.text.ends_with(ends_with))
            .unwrap_or_else(|| panic!("no {ends_with}"))
            .comment
            .id
            .clone()
    }

    #[test]
    fn every_token_handed_out_remembers_who_it_was_issued_to() {
        let mut s = CommentsSession::default();
        let page = parse_comments_page(&serde_json::from_str(INITIAL).unwrap(), true);
        s.remember_page(&Provenance::Anonymous, &page);
        let next = page.continuation.clone().unwrap();
        assert_eq!(s.provenance(&next), Some(Provenance::Anonymous));
        for sort in &page.header.as_ref().unwrap().sorts {
            assert_eq!(s.provenance(&sort.token), Some(Provenance::Anonymous), "sort token");
        }
        let replies = page.threads.iter().find_map(|t| t.replies_token.clone()).unwrap();
        assert_eq!(s.provenance(&replies), Some(Provenance::Anonymous), "replies token");
        assert_eq!(s.provenance("never-issued"), None);

        // The same session can hold tokens of two identities, each remembered as its own.
        let mut replies_page = parse_comment_replies(&serde_json::from_str(REPLIES).unwrap());
        replies_page.continuation = Some("tok-more-replies".into());
        s.remember_replies(&account("a"), &replies_page);
        assert_eq!(s.provenance("tok-more-replies"), Some(account("a")));
        assert_eq!(s.provenance(&next), Some(Provenance::Anonymous));
    }

    /// Neither the identity hash nor an action token can reach a log line through `Debug`.
    #[test]
    fn debug_output_redacts_the_identity_and_the_tokens() {
        const WHO: &str = "0123456789abcdef-identity-hash";
        assert_eq!(format!("{:?}", Provenance::Account(WHO.into())), "Account(<redacted>)");
        assert_eq!(format!("{:?}", Provenance::Anonymous), "Anonymous");

        let mut s = CommentsSession::default();
        let page = signed_in_page();
        let prov = account(WHO);
        s.remember_page(&prov, &page);
        let id = comment_id(&page, "neutral");
        let ticket = s.begin_action(WHO, &id, Like).ok().unwrap();
        // The page (which carries every token) and anything holding the provenance, however it is
        // formatted, including pretty-printed and inside an Option.
        for shown in [
            format!("{prov:?}"),
            format!("{:#?}", Some(&prov)),
            format!("{:?}", s.provenance(page.continuation.as_deref().unwrap())),
            format!("{page:?}"),
            format!("{:#?}", page.threads[0].comment),
        ] {
            assert!(!shown.contains(WHO), "the identity reached Debug output: {shown}");
            assert!(!shown.contains("fixture-token"), "a token reached Debug output");
        }
        assert_eq!(ticket.token(), "fixture-token-like-1", "and the ticket still has its token");
    }

    #[test]
    fn clearing_forgets_tokens_comments_and_busy_marks() {
        let mut s = CommentsSession::default();
        let page = signed_in_page();
        s.remember_page(&account("a"), &page);
        let id = comment_id(&page, "neutral");
        let _ticket = s.begin_action("a", &id, Like).ok().unwrap();
        s.clear();
        assert_eq!(s.provenance(page.continuation.as_deref().unwrap()), None);
        assert_eq!(s.begin_action("a", &id, Like).err(), Some(ActionError::Unknown));
    }

    /// The core rule: a token issued to one account or channel is never sent as another.
    #[test]
    fn action_tokens_are_only_usable_by_the_identity_they_were_issued_to() {
        let mut s = CommentsSession::default();
        let page = signed_in_page();
        s.remember_page(&account("account-a:channel-1"), &page);
        let id = comment_id(&page, "neutral");
        for other in ["account-b:channel-1", "account-a:channel-2", ""] {
            assert_eq!(
                s.begin_action(other, &id, Like).err(),
                Some(ActionError::Unknown),
                "{other}"
            );
        }
        assert!(s.begin_action("account-a:channel-1", &id, Like).is_ok());
    }

    /// Anonymous pages never put action tokens in the cache.
    #[test]
    fn an_anonymous_page_caches_no_action_tokens() {
        let mut s = CommentsSession::default();
        let page = signed_in_page();
        s.remember_page(&Provenance::Anonymous, &page);
        let id = comment_id(&page, "neutral");
        assert_eq!(s.begin_action("anything", &id, Like).err(), Some(ActionError::Unknown));
    }

    #[test]
    fn only_offered_actions_are_approved_and_each_is_a_single_request() {
        let mut s = CommentsSession::default();
        let page = signed_in_page();
        s.remember_page(&account("a"), &page);

        // Neutral: like and dislike are offered, unlike is not.
        let neutral = comment_id(&page, "neutral");
        assert_eq!(s.begin_action("a", &neutral, Unlike).err(), Some(ActionError::Unavailable));
        let t = s.begin_action("a", &neutral, Like).ok().unwrap();
        assert_eq!(t.token(), "fixture-token-like-1");
        assert_eq!(t.resulting_vote, VoteState::Liked);
        let (vote, next) = s.finish_action("a", &neutral, Some(t.resulting_vote)).unwrap();
        assert_eq!(vote, VoteState::Liked);
        assert_eq!(next, [Unlike, Dislike], "what a liked comment offers with these tokens");

        // Liked -> disliked is the single `dislike` request, with its own token: no unlike first.
        let t = s.begin_action("a", &neutral, Dislike).ok().unwrap();
        assert_eq!(t.token(), "fixture-token-dislike-1");
        assert_eq!(t.resulting_vote, VoteState::Disliked);
        s.finish_action("a", &neutral, Some(t.resulting_vote)).unwrap();
        // ...and back: disliked -> liked is the single `like` request.
        let t = s.begin_action("a", &neutral, Like).ok().unwrap();
        assert_eq!(t.token(), "fixture-token-like-1");
        s.finish_action("a", &neutral, None);

        // A comment with only a like token: dislike is unavailable, never faked in two steps.
        let like_only = comment_id(&page, "like_only");
        assert_eq!(s.begin_action("a", &like_only, Dislike).err(), Some(ActionError::Unavailable));
        assert!(s.begin_action("a", &like_only, Like).is_ok());
        // No vote (state entity missing) offers nothing, even with tokens.
        let missing = comment_id(&page, "state_missing");
        assert_eq!(s.begin_action("a", &missing, Like).err(), Some(ActionError::Unavailable));
    }

    #[test]
    fn a_comment_takes_one_action_at_a_time_and_a_failure_changes_nothing() {
        let mut s = CommentsSession::default();
        let page = signed_in_page();
        s.remember_page(&account("a"), &page);
        let id = comment_id(&page, "neutral");
        let first = s.begin_action("a", &id, Like).ok().unwrap();
        assert_eq!(s.begin_action("a", &id, Like).err(), Some(ActionError::Busy));
        assert_eq!(s.begin_action("a", &id, Dislike).err(), Some(ActionError::Busy));
        // The request failed: free again, same vote.
        assert!(s.finish_action("a", &id, None).is_none());
        let again = s.begin_action("a", &id, Like).ok().unwrap();
        assert_eq!(first.resulting_vote, again.resulting_vote);
    }

    /// A page re-read while an action is in flight must not overwrite the vote the answer decides.
    #[test]
    fn a_reload_does_not_clobber_a_comment_mid_action() {
        let mut s = CommentsSession::default();
        let page = signed_in_page();
        s.remember_page(&account("a"), &page);
        let id = comment_id(&page, "neutral");
        let t = s.begin_action("a", &id, Like).ok().unwrap();
        s.remember_page(&account("a"), &page);
        let (vote, _) = s.finish_action("a", &id, Some(t.resulting_vote)).unwrap();
        assert_eq!(vote, VoteState::Liked);
    }

    const WRITES: &str = include_str!("../tests/fixtures/comments/signed_in_write_synthetic.json");

    fn writes_page() -> CommentsPage {
        let mut page = parse_comments_page(&serde_json::from_str(WRITES).unwrap(), true);
        page.read_as_account = true;
        page
    }

    fn command_path(t: &WriteTicket) -> String {
        match t.command() {
            WriteCommand::Endpoint { path, .. } => path.clone(),
            WriteCommand::Action(_) => "action".into(),
        }
    }

    /// The same rule as for votes: a command issued to one account or channel is never replayed
    /// as another, and an anonymous read caches none.
    #[test]
    fn write_commands_are_only_usable_by_the_identity_they_were_issued_to() {
        let mut s = CommentsSession::default();
        let page = writes_page();
        s.remember_page(&account("a:1"), &page);
        let own = comment_id(&page, "own_full");
        for other in ["b:1", "a:2", ""] {
            assert!(matches!(
                s.begin_write(other, &own, CommentWrite::Edit),
                Err(ActionError::Unknown)
            ));
            assert!(matches!(s.begin_create(other), Err(ActionError::Unknown)), "{other}");
        }
        let t = s.begin_write("a:1", &own, CommentWrite::Edit).ok().unwrap();
        assert_eq!(command_path(&t), "comment/update_comment");
        s.finish_write("a:1", &own, false);
        assert!(s.begin_create("a:1").is_ok());

        let mut anon = CommentsSession::default();
        anon.remember_page(&Provenance::Anonymous, &page);
        assert!(matches!(
            anon.begin_write("a:1", &own, CommentWrite::Edit),
            Err(ActionError::Unknown)
        ));
        assert!(matches!(anon.begin_create("a:1"), Err(ActionError::Unknown)));
    }

    #[test]
    fn only_the_commands_the_response_carried_are_approved() {
        let mut s = CommentsSession::default();
        let page = writes_page();
        s.remember_page(&account("a"), &page);
        // A comment that is not the viewer's has a reply command and nothing else, even though
        // edit and delete are in its menu.
        let other = comment_id(&page, "other");
        assert!(s.begin_write("a", &other, CommentWrite::Reply).is_ok());
        s.finish_write("a", &other, false);
        for w in [CommentWrite::Edit, CommentWrite::Delete] {
            assert!(matches!(s.begin_write("a", &other, w), Err(ActionError::Unavailable)));
        }
        let bare = comment_id(&page, "own_bare");
        assert!(matches!(
            s.begin_write("a", &bare, CommentWrite::Edit),
            Err(ActionError::Unavailable)
        ));
        let bad = comment_id(&page, "bad_reply_path");
        assert!(matches!(
            s.begin_write("a", &bad, CommentWrite::Reply),
            Err(ActionError::Unavailable)
        ));
    }

    #[test]
    fn a_write_holds_its_comment_busy_and_a_delete_forgets_it() {
        let mut s = CommentsSession::default();
        let page = writes_page();
        s.remember_page(&account("a"), &page);
        let own = comment_id(&page, "own_full");

        let first = s.begin_write("a", &own, CommentWrite::Edit).ok().unwrap();
        assert!(matches!(s.begin_write("a", &own, CommentWrite::Delete), Err(ActionError::Busy)));
        assert!(
            matches!(s.begin_action("a", &own, Like), Err(ActionError::Busy)),
            "votes share the lock"
        );
        s.finish_write("a", &own, false);
        drop(first);

        let t = s.begin_write("a", &own, CommentWrite::Delete).ok().unwrap();
        assert!(matches!(t.command(), WriteCommand::Action(_)));
        s.finish_write("a", &own, true);
        assert!(matches!(s.begin_write("a", &own, CommentWrite::Edit), Err(ActionError::Unknown)));
        assert!(matches!(s.begin_action("a", &own, Like), Err(ActionError::Unknown)));
    }

    #[test]
    fn a_comment_that_is_gone_is_forgotten_whatever_was_in_flight() {
        let mut s = CommentsSession::default();
        let page = writes_page();
        s.remember_page(&account("a"), &page);
        let own = comment_id(&page, "own_full");
        let _t = s.begin_write("a", &own, CommentWrite::Edit).ok().unwrap();
        s.forget("a", &own);
        for w in [CommentWrite::Edit, CommentWrite::Delete, CommentWrite::Reply] {
            assert!(matches!(s.begin_write("a", &own, w), Err(ActionError::Unknown)));
        }
        assert!(matches!(s.begin_action("a", &own, Like), Err(ActionError::Unknown)));
        // Someone else's entry for the same id is untouched.
        s.remember_page(&account("b"), &page);
        s.forget("a", &own);
        assert!(s.begin_write("b", &own, CommentWrite::Edit).is_ok());
    }

    #[test]
    fn posting_is_one_at_a_time_and_clearing_forgets_the_composer() {
        let mut s = CommentsSession::default();
        s.remember_page(&account("a"), &writes_page());
        let t = s.begin_create("a").ok().unwrap();
        assert_eq!(command_path(&t), crate::COMMENT_CREATE_PATH);
        assert!(matches!(s.begin_create("a"), Err(ActionError::Busy)));
        s.finish_create("a");
        assert!(s.begin_create("a").is_ok());
        s.clear();
        s.finish_create("a");
        assert!(matches!(s.begin_create("a"), Err(ActionError::Unknown)));
    }

    #[test]
    fn both_maps_are_bounded_and_forget_the_oldest_first() {
        let mut s = CommentsSession::with_caps(3, 2);
        for n in 0..5 {
            s.tokens.insert(format!("t{n}"), Provenance::Anonymous);
        }
        assert_eq!(s.provenance("t0"), None);
        assert_eq!(s.provenance("t1"), None);
        for n in 2..5 {
            assert!(s.provenance(&format!("t{n}")).is_some());
        }
        assert_eq!(s.tokens.map.len(), 3);

        let page = signed_in_page();
        s.remember_page(&account("a"), &page);
        assert_eq!(s.comments.map.len(), 2, "capped");
        // Re-inserting a key already there does not grow the order queue.
        let before = s.comments.order.len();
        s.remember_page(&account("a"), &page);
        assert_eq!(s.comments.order.len(), before);
    }
}
