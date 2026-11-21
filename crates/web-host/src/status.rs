//! The statuses a hosted app shows in its title bar (storytold/craft-libs#2079): whether the host
//! stored the document, the version it stored, how many people are in it, and whether the session
//! is read-only.
//!
//! Every input is a message of the hosting contract (storytold/craft-libs#358) or a fact the app
//! already has, so nothing here opens a network path and nothing is specific to one host:
//!
//! | Input | Where the app gets it |
//! |---|---|
//! | [`StatusInput::Edited`] | its own change tracking: the document differs from what it last posted |
//! | [`StatusInput::SavePosted`] | it posted `save`, with the `id` it put in the message |
//! | [`StatusInput::Saved`] | `saved {id}` from the host |
//! | [`StatusInput::SaveFailed`] | `save-failed {id, error}` from the host, or the crate's own timeout (storytold/craft-libs#1338) |
//! | [`StatusInput::ConfirmsSaves`] | `host`'s `confirmsSaves`: the host answers every save (storytold/craft-libs#1338) |
//! | [`StatusInput::CommitPosted`] | it posted `commit` |
//! | [`StatusInput::Committed`] | `committed {version, etag}` from the host |
//! | [`StatusInput::SessionState`] | the `session-state {status, participants}` it posts |
//! | [`StatusInput::Presence`] | the `presence {participants}` it posts |
//! | [`StatusInput::ReadOnly`] | `session`'s `viewer` role, or a host-controlled read-only session (storytold/craft-libs#609) |
//!
//! [`HostStatus::apply`] keeps the state and takes the app's monotonic clock (`now`) on every input,
//! so the crate owns the save timeout and no app keeps its own timer: a posted save the host has not
//! answered within the timeout (30 s by default, configurable) turns into a [`SaveFailure::TimedOut`]
//! and the document shows as unsaved again. [`HostStatus::cluster`] turns the state into what the
//! title bar draws, in reading order, leaving out what does not apply. A host that never sends the
//! collaboration messages never produces those inputs: the cluster then shows the save state alone.
//! The crate carries no display text: the app's language pack supplies it, and the English source
//! texts are noted on each variant (`Saved`, `Unsaved`, `Saving…`, `Read-only`, and a count of
//! people in the document). A host's refusal text is the one exception: it is shown as it came.

use std::collections::VecDeque;
use std::time::Duration;

/// The sync status the title bar never shows: a synced session needs no word of its own, so only a
/// session that is not [`SYNCED`] puts one there.
pub const SYNCED: &str = "synced";

/// How many posted saves the state still tells apart. Autosave may post several before a host
/// answers, and a host that stops answering must not make the queue grow: past this many the oldest
/// posted save is forgotten, and its late answer confirms nothing.
pub const MAX_PENDING_SAVES: usize = 8;

/// How long a posted save may wait for `saved` or `save-failed` before [`HostStatus`] counts it
/// failed: the 30 s of storytold/wordcraft#4490, kept by the crate instead of each app
/// (storytold/craft-libs#1338). A host that writes a new revision on every save may take longer;
/// [`HostStatus::set_save_timeout`] changes it.
pub const DEFAULT_SAVE_TIMEOUT: Duration = Duration::from_secs(30);

/// How the host holds the document, as a title bar says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveState {
    /// The document has changes the host does not hold, or the host refused the last save. English
    /// source text: `Unsaved`.
    Unsaved,
    /// A save is posted and unanswered. English source text: `Saving…`.
    ///
    /// A save in flight outranks a save already confirmed: the person sees `Saving…`, not `Saved`,
    /// while any posted save waits for its answer.
    Saving,
    /// The host confirmed the last save, or committed the session: it holds the document. English
    /// source text: `Saved`.
    Saved,
}

/// The version a host stored when it committed the session (`committed {version, etag}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredVersion {
    /// The session version the host stored: it counts commits, so a later commit has a higher one.
    pub version: u64,
    /// The host's `etag` for that stored content, for the app to send with its next commit.
    pub etag: String,
}

/// Why the last save is not stored, while the document shows as unsaved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveFailure {
    /// The host refused the save (`save-failed`): its free text, shown to the person as it is.
    Refused(String),
    /// No `saved` or `save-failed` arrived within [`HostStatus::save_timeout`]. The words are the
    /// app's: the crate carries no display text.
    TimedOut,
}

impl SaveFailure {
    /// The host's own text, when the refusal came from the host; `None` for a timeout, which the
    /// app words itself.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Refused(text) => Some(text),
            Self::TimedOut => None,
        }
    }
}

/// What the app tells [`HostStatus`]: a contract message it received or posted, its own change
/// tracking, or the session's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusInput<'a> {
    /// The document changed: what the host holds is no longer what the app shows.
    Edited,
    /// The app posted a `save`. `id` is the message's `id`, or `None` when it carries none: the
    /// contract has a host answer only a save with an `id`, so one without is stored once posted. An
    /// empty `id` counts as none. A host that confirms saves (`ConfirmsSaves`) expects an `id` on
    /// every save.
    SavePosted { id: Option<&'a str> },
    /// The host stored the save with this `id` (`saved`).
    Saved { id: &'a str },
    /// A save of this `id` was refused, and this is why (`save-failed`). An empty `error` still says
    /// the save did not happen. The app words a timeout itself: the crate raises that one.
    SaveFailed { id: Option<&'a str>, error: &'a str },
    /// The host set `confirmsSaves` in `host`: it answers every save. The app then puts an `id` in
    /// each `save` and keeps the document unsaved until the answer.
    ConfirmsSaves(bool),
    /// The app asked the host to commit the session (it posted `commit`): the document it sent is
    /// the document as it is now.
    CommitPosted,
    /// The host committed the session and now holds it as this version (`committed`).
    Committed { version: u64, etag: &'a str },
    /// The app's own sync word and who is in the session, as it posted in `session-state`.
    SessionState { status: &'a str, participants: usize },
    /// Who is in the session, as the app posted in `presence`.
    Presence { participants: usize },
    /// The session is read-only: the host's decision, not the app's (storytold/craft-libs#609).
    ReadOnly(bool),
}

/// Where the cluster starts: the session's mode, or how the host holds the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Headline {
    /// Read-only: the app cannot change the document. Drawn instead of a save state, so the cluster
    /// cannot promise a save the person may not make. English source text: `Read-only`.
    ReadOnly,
    /// Whether the host holds the document.
    Save(SaveState),
}

/// One thing the cluster draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterPart<'a> {
    /// The read-only notice or the save state; always drawn.
    Headline(Headline),
    /// How many people are in the document, this app included: a count, never the list. English
    /// source text: a plural count of people in the document.
    Participants(usize),
    /// The app's own sync word (`pending`, `offline`, `error`), shown as the app spells it.
    Sync(&'a str),
    /// The version the host stored at the last commit.
    Version(u64),
}

/// What the title bar's cluster draws, built by [`HostStatus::cluster`]. The app supplies the words
/// and the layout; this says which parts are there and in what order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cluster<'a> {
    /// The first part: `Read-only`, or the save state.
    pub headline: Headline,
    /// People in the document, this app included; `0` when no session reports any.
    pub participants: usize,
    /// The app's own sync word, and only while it is not [`SYNCED`].
    pub sync: Option<&'a str>,
    /// The stored version after a commit.
    pub version: Option<u64>,
    /// Why the last save is not stored, while the document still shows as unsaved: the host's
    /// refusal text, or the crate's timeout for the app to word.
    ///
    /// It is not a [`ClusterPart`]: it belongs to the headline, so the app draws it where it draws
    /// the failure (a tooltip on the label, or beside it).
    pub failure: Option<&'a SaveFailure>,
}

impl<'a> Cluster<'a> {
    /// The parts in the order the title bar draws them, leaving out what does not apply: no
    /// participant count without a session, no sync word while `synced`, no version before a commit.
    pub fn parts(self) -> impl Iterator<Item = ClusterPart<'a>> {
        let headline = Some(ClusterPart::Headline(self.headline));
        let participants = (self.participants > 0).then_some(ClusterPart::Participants(self.participants));
        let sync = self.sync.map(ClusterPart::Sync);
        let version = self.version.map(ClusterPart::Version);
        [headline, participants, sync, version].into_iter().flatten()
    }
}

/// A posted save the host has not answered for, and when it was posted on the app's clock.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingSave {
    id: String,
    posted_at: Duration,
}

/// The host statuses of one document, from the messages its app posts and receives.
///
/// Plain data: it builds everywhere, so it is tested natively. The bridge (`web`, wasm32 only) turns
/// a host's messages into [`StatusInput`] values; the app turns its own tracking into the rest and
/// passes its monotonic clock to [`HostStatus::apply`].
#[derive(Debug, Clone)]
pub struct HostStatus {
    save: SaveState,
    /// Posted saves the host has not answered for, oldest first, with the time they were posted.
    pending: VecDeque<PendingSave>,
    /// The app asked for a commit and the host has not answered yet.
    committing: bool,
    /// The document has changes the last posted save did not carry.
    dirty: bool,
    /// Why the last save is not stored, until another attempt is posted or one succeeds.
    failure: Option<SaveFailure>,
    /// How long an unanswered save may wait; [`Duration::ZERO`] turns the timer off.
    timeout: Duration,
    /// The host said `confirmsSaves`: it answers every save.
    confirms_saves: bool,
    stored: Option<StoredVersion>,
    /// The app's own sync word, kept as it was given.
    sync: Option<String>,
    participants: usize,
    read_only: bool,
}

impl Default for HostStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl HostStatus {
    /// A document the host opened and has not changed: `Saved`, synced, with no participants and no
    /// committed version, and the default [`DEFAULT_SAVE_TIMEOUT`]. The app reports its first change
    /// with [`StatusInput::Edited`].
    pub fn new() -> Self {
        Self {
            save: SaveState::Saved,
            pending: VecDeque::new(),
            committing: false,
            dirty: false,
            failure: None,
            timeout: DEFAULT_SAVE_TIMEOUT,
            confirms_saves: false,
            stored: None,
            sync: None,
            participants: 0,
            read_only: false,
        }
    }

    /// [`HostStatus::new`] with another save timeout, for a host whose storage takes longer (a new
    /// revision on every save) or an app that wants the timeout off ([`Duration::ZERO`]).
    pub fn with_save_timeout(timeout: Duration) -> Self {
        Self { timeout, ..Self::new() }
    }

    /// How long an unanswered save may wait. [`Duration::ZERO`] turns the timer off.
    pub fn save_timeout(&self) -> Duration {
        self.timeout
    }

    /// Change the save timeout. A shorter one applies at the next [`HostStatus::apply`], to the
    /// saves already waiting too.
    pub fn set_save_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Whether the host set `confirmsSaves`: it answers every save, so the app puts an `id` in each.
    pub fn confirms_saves(&self) -> bool {
        self.confirms_saves
    }

    /// Take one input at `now`, the app's monotonic clock in any unit as long as every call uses
    /// the same one (a browser's `Date.now()` in milliseconds).
    ///
    /// Before the input is applied, posted saves older than [`HostStatus::save_timeout`] expire:
    /// the document shows as unsaved with [`SaveFailure::TimedOut`] once none of them is left. Out
    /// of order the inputs are safe: a save in flight stays in flight, an edit made while it was
    /// posted keeps the document unsaved after its confirmation, and an answer for a save that is
    /// no longer told apart confirms nothing.
    pub fn apply(&mut self, input: StatusInput<'_>, now: Duration) {
        self.expire(now);
        match input {
            StatusInput::Edited => self.edited(),
            StatusInput::SavePosted { id } => self.posted(id.filter(|id| !id.is_empty()), now),
            StatusInput::Saved { id } => self.confirmed(id),
            StatusInput::SaveFailed { id, error } => self.refused(id.filter(|id| !id.is_empty()), error),
            StatusInput::ConfirmsSaves(confirms) => self.confirms_saves = confirms,
            StatusInput::CommitPosted => self.commit_posted(),
            StatusInput::Committed { version, etag } => self.committed(version, etag),
            StatusInput::SessionState { status, participants } => {
                self.sync = Some(status.to_string());
                self.participants = participants;
            }
            StatusInput::Presence { participants } => self.participants = participants,
            StatusInput::ReadOnly(read_only) => self.read_only = read_only,
        }
    }

    /// How the host holds the document, ignoring the session's mode.
    pub fn save_state(&self) -> SaveState {
        self.save
    }

    /// Why the last save is not stored, while no save has succeeded since.
    pub fn failure(&self) -> Option<&SaveFailure> {
        self.failure.as_ref()
    }

    /// The version the host stored at its last commit.
    pub fn stored_version(&self) -> Option<&StoredVersion> {
        self.stored.as_ref()
    }

    /// The app's own sync word as it was given, `synced` included.
    pub fn sync_status(&self) -> Option<&str> {
        self.sync.as_deref()
    }

    /// People in the document, this app included; `0` when no session reported any.
    pub fn participants(&self) -> usize {
        self.participants
    }

    /// The `id`s of posted saves the host has not answered for, oldest first.
    ///
    /// The state remembers which ids still wait and expires those that passed the timeout, so the
    /// app does not keep a second list that drifts from this one (storytold/craft-libs#1338).
    pub fn unanswered_saves(&self) -> impl Iterator<Item = &str> {
        self.pending.iter().map(|queued| queued.id.as_str())
    }

    /// Whether the host made the session read-only.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// What the title bar draws from this state.
    pub fn cluster(&self) -> Cluster<'_> {
        Cluster {
            headline: if self.read_only { Headline::ReadOnly } else { Headline::Save(self.save) },
            participants: self.participants,
            sync: self.sync.as_deref().filter(|status| *status != SYNCED),
            version: self.stored.as_ref().map(|stored| stored.version),
            failure: if self.save == SaveState::Unsaved && !self.read_only { self.failure.as_ref() } else { None },
        }
    }

    fn edited(&mut self) {
        self.dirty = true;
        self.settle();
    }

    fn posted(&mut self, id: Option<&str>, now: Duration) {
        // The bytes just posted carry every change so far; what is edited from now on they do not.
        self.dirty = false;
        // A new attempt replaces the last failure's message; if it fails too, its own arrives.
        self.failure = None;
        let Some(id) = id else {
            // Nothing will answer it: the contract has a host answer only a save with an `id`.
            self.settle();
            return;
        };
        if self.pending.len() >= MAX_PENDING_SAVES {
            self.pending.pop_front();
        }
        self.pending.push_back(PendingSave { id: id.to_string(), posted_at: now });
        self.save = SaveState::Saving;
    }

    fn confirmed(&mut self, id: &str) {
        // A duplicate `id` removes both, so a repeated answer cannot leave one behind for ever.
        self.pending.retain(|queued| queued.id != id);
        self.settle();
    }

    fn refused(&mut self, id: Option<&str>, error: &str) {
        if let Some(id) = id {
            self.pending.retain(|queued| queued.id != id);
        }
        self.failure = Some(SaveFailure::Refused(error.to_string()));
        if self.pending.is_empty() && !self.committing {
            // The bytes posted most recently are not stored, whatever the app has changed since.
            self.save = SaveState::Unsaved;
        }
    }

    /// Forget the posted saves that waited longer than the timeout and count the latest one failed.
    /// `now` must be monotonic: a clock that goes backwards expires nothing.
    fn expire(&mut self, now: Duration) {
        if self.timeout.is_zero() {
            return;
        }
        let mut expired = false;
        while self.pending.front().is_some_and(|queued| now.saturating_sub(queued.posted_at) >= self.timeout) {
            self.pending.pop_front();
            expired = true;
        }
        if !expired {
            return;
        }
        if self.pending.is_empty() && !self.committing {
            self.save = SaveState::Unsaved;
            self.failure = Some(SaveFailure::TimedOut);
        }
    }

    fn commit_posted(&mut self) {
        self.committing = true;
        self.dirty = false;
        self.failure = None;
        self.save = SaveState::Saving;
    }

    fn committed(&mut self, version: u64, etag: &str) {
        self.committing = false;
        // The commit covers the document the app sent, like a save: an edit made after it was posted
        // is not in the stored version, and `settle` keeps the document unsaved for it.
        self.stored = Some(StoredVersion { version, etag: etag.to_string() });
        self.settle();
    }

    /// What an answer leaves the save state as: `Saving…` while a posted save or a commit waits,
    /// otherwise `Unsaved` when changes came after the last post, otherwise `Saved`.
    fn settle(&mut self) {
        if !self.pending.is_empty() || self.committing {
            self.save = SaveState::Saving;
            return;
        }
        if self.dirty {
            self.save = SaveState::Unsaved;
            return;
        }
        self.save = SaveState::Saved;
        self.failure = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The clock every input that does not test the timeout is taken at.
    const T0: Duration = Duration::ZERO;

    trait ApplyAt {
        fn at(&mut self, input: StatusInput<'_>);
    }

    impl ApplyAt for HostStatus {
        fn at(&mut self, input: StatusInput<'_>) {
            self.apply(input, T0);
        }
    }

    fn parts(status: &HostStatus) -> Vec<ClusterPart<'_>> {
        status.cluster().parts().collect()
    }

    fn refused(text: &str) -> SaveFailure {
        SaveFailure::Refused(text.to_string())
    }

    #[test]
    fn a_posted_save_is_saving_and_its_answer_is_saved() {
        let mut status = HostStatus::new();
        assert_eq!(status.save_state(), SaveState::Saved);

        status.at(StatusInput::Edited);
        assert_eq!(status.save_state(), SaveState::Unsaved);
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Unsaved))]);

        status.at(StatusInput::SavePosted { id: Some("a") });
        assert_eq!(status.save_state(), SaveState::Saving);
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saving))]);

        status.at(StatusInput::Saved { id: "a" });
        assert_eq!(status.save_state(), SaveState::Saved);
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saved))]);
    }

    #[test]
    fn a_save_in_flight_outranks_a_confirmed_one() {
        let mut status = HostStatus::new();
        status.at(StatusInput::SavePosted { id: Some("a") });
        status.at(StatusInput::SavePosted { id: Some("b") });
        assert_eq!(status.save_state(), SaveState::Saving);

        // The first save's answer must not turn the cluster to `Saved` while the second waits.
        status.at(StatusInput::Saved { id: "a" });
        assert_eq!(status.save_state(), SaveState::Saving);
        status.at(StatusInput::Saved { id: "b" });
        assert_eq!(status.save_state(), SaveState::Saved);
    }

    #[test]
    fn a_refused_save_returns_to_unsaved_and_keeps_the_error() {
        let mut status = HostStatus::new();
        status.at(StatusInput::SavePosted { id: Some("a") });
        status.at(StatusInput::SaveFailed { id: Some("a"), error: "disk full" });

        assert_eq!(status.save_state(), SaveState::Unsaved);
        assert_eq!(status.failure(), Some(&refused("disk full")));
        let cluster = status.cluster();
        assert_eq!(cluster.headline, Headline::Save(SaveState::Unsaved));
        assert_eq!(cluster.failure, Some(&refused("disk full")), "the app can say why it is unsaved");

        // A new attempt starts without the old message; success forgets it for good.
        status.at(StatusInput::SavePosted { id: Some("b") });
        assert_eq!(status.failure(), None);
        status.at(StatusInput::Saved { id: "b" });
        assert_eq!(status.failure(), None);
        assert_eq!(status.save_state(), SaveState::Saved);
    }

    #[test]
    fn a_refusal_of_a_save_without_an_id_is_still_unsaved() {
        let mut status = HostStatus::new();
        status.at(StatusInput::SavePosted { id: None });
        assert_eq!(status.save_state(), SaveState::Saved, "no id: stored once posted");

        status.at(StatusInput::SaveFailed { id: None, error: "no write access" });
        assert_eq!(status.save_state(), SaveState::Unsaved);
        assert_eq!(status.failure(), Some(&refused("no write access")));
    }

    #[test]
    fn an_edit_made_while_a_save_was_in_flight_keeps_it_unsaved() {
        let mut status = HostStatus::new();
        status.at(StatusInput::Edited);
        status.at(StatusInput::SavePosted { id: Some("a") });
        status.at(StatusInput::Edited);
        assert_eq!(status.save_state(), SaveState::Saving, "the save in flight still runs");

        status.at(StatusInput::Saved { id: "a" });
        assert_eq!(status.save_state(), SaveState::Unsaved, "the edit came after the posted bytes");
    }

    #[test]
    fn a_commit_shows_saving_until_the_host_answers_with_its_version() {
        let mut status = HostStatus::new();
        status.at(StatusInput::Edited);
        assert_eq!(status.save_state(), SaveState::Unsaved);

        status.at(StatusInput::CommitPosted);
        assert_eq!(status.save_state(), SaveState::Saving, "the host has the commit, not the answer");
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saving))]);

        status.at(StatusInput::Committed { version: 7, etag: "\"v7\"" });
        assert_eq!(status.save_state(), SaveState::Saved);
        assert_eq!(status.stored_version().map(|stored| (stored.version, stored.etag.as_str())), Some((7, "\"v7\"")));
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saved)), ClusterPart::Version(7)]);

        // A later commit replaces the version: the newest stored one is the one to show.
        status.at(StatusInput::Committed { version: 9, etag: "\"v9\"" });
        assert_eq!(status.cluster().version, Some(9));
    }

    #[test]
    fn an_edit_made_while_a_commit_was_in_flight_keeps_it_unsaved() {
        let mut status = HostStatus::new();
        status.at(StatusInput::Edited);
        status.at(StatusInput::CommitPosted);
        status.at(StatusInput::Edited);
        assert_eq!(status.save_state(), SaveState::Saving, "the commit still waits");

        status.at(StatusInput::Committed { version: 4, etag: "\"v4\"" });
        assert_eq!(status.save_state(), SaveState::Unsaved, "the edit came after the committed bytes");
        assert_eq!(status.stored_version().map(|stored| stored.version), Some(4), "the stored version is still shown");
    }

    #[test]
    fn an_edit_after_a_commit_is_unsaved_again() {
        let mut status = HostStatus::new();
        status.at(StatusInput::Committed { version: 3, etag: "\"v3\"" });
        assert_eq!(status.save_state(), SaveState::Saved);

        status.at(StatusInput::Edited);
        assert_eq!(status.save_state(), SaveState::Unsaved);
        assert_eq!(
            status.stored_version().map(|stored| stored.version),
            Some(3),
            "the stored version is what was committed, not what is being edited"
        );
    }

    #[test]
    fn the_participant_count_comes_from_session_state_and_presence() {
        let mut status = HostStatus::new();
        assert_eq!(status.participants(), 0);
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saved))], "no session, no count");

        status.at(StatusInput::SessionState { status: SYNCED, participants: 3 });
        assert_eq!(status.participants(), 3);
        status.at(StatusInput::Presence { participants: 2 });
        assert_eq!(status.participants(), 2, "someone left");
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saved)), ClusterPart::Participants(2)]);

        status.at(StatusInput::Presence { participants: 1 });
        assert_eq!(parts(&status)[1], ClusterPart::Participants(1));

        // The last person leaving takes the count with it, leaving the save state alone.
        status.at(StatusInput::Presence { participants: 0 });
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saved))]);
    }

    #[test]
    fn two_clients_in_one_room_show_the_same_count() {
        let mut left = HostStatus::new();
        let mut right = HostStatus::new();
        for status in [&mut left, &mut right] {
            status.at(StatusInput::SessionState { status: SYNCED, participants: 2 });
        }
        assert_eq!(left.cluster().participants, right.cluster().participants);

        // The left client sees the other leave; the count it shows is the session's, not its own.
        left.at(StatusInput::Presence { participants: 1 });
        assert_eq!(left.cluster().participants, 1);
        assert_eq!(right.cluster().participants, 2);
    }

    #[test]
    fn the_sync_word_is_shown_only_when_it_is_not_synced() {
        let mut status = HostStatus::new();
        status.at(StatusInput::SessionState { status: SYNCED, participants: 0 });
        assert_eq!(status.cluster().sync, None, "synced says nothing");

        status.at(StatusInput::SessionState { status: "pending", participants: 0 });
        assert_eq!(status.cluster().sync, Some("pending"));
        status.at(StatusInput::SessionState { status: "offline", participants: 0 });
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::Save(SaveState::Saved)), ClusterPart::Sync("offline")]);

        // The word is the app's own: it is kept as given, not translated here.
        status.at(StatusInput::SessionState { status: "error", participants: 0 });
        assert_eq!(status.sync_status(), Some("error"));
        status.at(StatusInput::SessionState { status: SYNCED, participants: 0 });
        assert_eq!(status.sync_status(), Some(SYNCED), "the word is kept");
        assert_eq!(status.cluster().sync, None, "and hidden");
    }

    #[test]
    fn a_read_only_session_says_so_instead_of_a_save_state() {
        let mut status = HostStatus::new();
        status.at(StatusInput::ReadOnly(true));
        status.at(StatusInput::Edited);
        status.at(StatusInput::SaveFailed { id: None, error: "viewers cannot save" });

        assert_eq!(status.save_state(), SaveState::Unsaved, "the state is still tracked");
        let cluster = status.cluster();
        assert_eq!(cluster.headline, Headline::ReadOnly);
        assert_eq!(cluster.failure, None, "a read-only session shows no save failure");
        assert_eq!(parts(&status), [ClusterPart::Headline(Headline::ReadOnly)]);

        // Leaving read-only shows the save state that was tracked meanwhile.
        status.at(StatusInput::ReadOnly(false));
        assert_eq!(status.cluster().headline, Headline::Save(SaveState::Unsaved));
        assert_eq!(status.cluster().failure, Some(&refused("viewers cannot save")));
    }

    #[test]
    fn the_cluster_draws_the_parts_in_one_order() {
        let mut status = HostStatus::new();
        status.at(StatusInput::SavePosted { id: Some("a") });
        status.at(StatusInput::SessionState { status: "pending", participants: 4 });
        status.at(StatusInput::Committed { version: 12, etag: "\"v12\"" });

        assert_eq!(
            parts(&status),
            [
                ClusterPart::Headline(Headline::Save(SaveState::Saving)),
                ClusterPart::Participants(4),
                ClusterPart::Sync("pending"),
                ClusterPart::Version(12),
            ],
            "headline, people, sync, version: the same order in every app"
        );
    }

    #[test]
    fn an_answer_for_a_forgotten_save_confirms_nothing_and_the_queue_is_bounded() {
        let mut status = HostStatus::new();
        for id in 0..MAX_PENDING_SAVES {
            status.at(StatusInput::SavePosted { id: Some(&id.to_string()) });
        }
        // More saves than the queue tells apart: the first is forgotten, and its answer changes nothing.
        status.at(StatusInput::SavePosted { id: Some("extra") });
        status.at(StatusInput::Saved { id: "0" });
        assert_eq!(status.save_state(), SaveState::Saving, "the newer saves still wait");

        // Answering the saves it remembers settles it, one at a time.
        for id in 1..MAX_PENDING_SAVES {
            status.at(StatusInput::Saved { id: &id.to_string() });
        }
        assert_eq!(status.save_state(), SaveState::Saving);
        status.at(StatusInput::Saved { id: "extra" });
        assert_eq!(status.save_state(), SaveState::Saved);
    }

    #[test]
    fn an_answer_for_an_id_that_was_never_posted_confirms_the_document_it_covers() {
        let mut status = HostStatus::new();
        status.at(StatusInput::Edited);
        // A host that answers a save the app posted before this state existed: nothing is in flight,
        // so the answer is for the bytes as they are now.
        status.at(StatusInput::Saved { id: "from-before" });
        assert_eq!(status.save_state(), SaveState::Unsaved, "the edit is still not stored");

        status.at(StatusInput::SavePosted { id: Some("from-before") });
        status.at(StatusInput::Saved { id: "from-before" });
        assert_eq!(status.save_state(), SaveState::Saved);
    }

    #[test]
    fn the_unanswered_saves_are_the_ids_an_answer_has_not_removed() {
        let mut status = HostStatus::new();
        assert_eq!(status.unanswered_saves().collect::<Vec<_>>(), Vec::<&str>::new());

        status.at(StatusInput::SavePosted { id: Some("a") });
        status.at(StatusInput::SavePosted { id: Some("b") });
        assert_eq!(status.unanswered_saves().collect::<Vec<_>>(), ["a", "b"], "oldest first");

        status.at(StatusInput::Saved { id: "a" });
        assert_eq!(status.unanswered_saves().collect::<Vec<_>>(), ["b"]);
        status.at(StatusInput::SaveFailed { id: Some("b"), error: "timeout" });
        assert_eq!(status.unanswered_saves().collect::<Vec<_>>(), Vec::<&str>::new());

        // A save without an id is never answered, so it never waits.
        status.at(StatusInput::SavePosted { id: None });
        assert_eq!(status.unanswered_saves().collect::<Vec<_>>(), Vec::<&str>::new());
    }

    #[test]
    fn a_repeated_answer_does_not_leave_a_save_in_flight() {
        let mut status = HostStatus::new();
        status.at(StatusInput::SavePosted { id: Some("a") });
        status.at(StatusInput::SavePosted { id: Some("a") });
        status.at(StatusInput::Saved { id: "a" });
        assert_eq!(status.save_state(), SaveState::Saved);

        status.at(StatusInput::Edited);
        status.at(StatusInput::SavePosted { id: Some("") });
        assert_eq!(status.save_state(), SaveState::Saved, "an empty id is no id: stored once posted");
    }

    #[test]
    fn a_refusal_while_another_save_waits_does_not_end_the_saving() {
        let mut status = HostStatus::new();
        status.at(StatusInput::SavePosted { id: Some("a") });
        status.at(StatusInput::SavePosted { id: Some("b") });
        status.at(StatusInput::SaveFailed { id: Some("a"), error: "conflict" });

        assert_eq!(status.save_state(), SaveState::Saving, "b was not answered yet");
        assert_eq!(status.failure(), Some(&refused("conflict")));

        status.at(StatusInput::Saved { id: "b" });
        assert_eq!(status.save_state(), SaveState::Saved);
        assert_eq!(status.failure(), None, "the newest save is stored");
    }

    #[test]
    fn an_unanswered_save_times_out_after_the_default_thirty_seconds() {
        let mut status = HostStatus::new();
        assert_eq!(status.save_timeout(), DEFAULT_SAVE_TIMEOUT);

        status.apply(StatusInput::SavePosted { id: Some("a") }, T0);
        status.apply(StatusInput::Edited, Duration::from_secs(29));
        assert_eq!(status.save_state(), SaveState::Saving, "still inside the timeout");
        assert_eq!(status.unanswered_saves().collect::<Vec<_>>(), ["a"]);

        status.apply(StatusInput::Presence { participants: 0 }, Duration::from_secs(30));
        assert_eq!(status.save_state(), SaveState::Unsaved, "the wait is over");
        assert_eq!(status.failure(), Some(&SaveFailure::TimedOut));
        assert_eq!(status.unanswered_saves().count(), 0, "the state forgot the expired save");
        let cluster = status.cluster();
        assert_eq!(cluster.headline, Headline::Save(SaveState::Unsaved));
        assert!(matches!(cluster.failure, Some(SaveFailure::TimedOut)));
    }

    #[test]
    fn the_timeout_is_configurable_and_zero_turns_it_off() {
        let mut status = HostStatus::with_save_timeout(Duration::from_secs(5));
        status.apply(StatusInput::SavePosted { id: Some("a") }, T0);
        status.apply(StatusInput::Presence { participants: 0 }, Duration::from_millis(4_999));
        assert_eq!(status.save_state(), SaveState::Saving);
        status.apply(StatusInput::Presence { participants: 0 }, Duration::from_secs(5));
        assert_eq!(status.failure(), Some(&SaveFailure::TimedOut));

        // A changed timeout applies to the saves already waiting.
        let mut status = HostStatus::with_save_timeout(Duration::from_secs(60));
        status.apply(StatusInput::SavePosted { id: Some("a") }, T0);
        status.set_save_timeout(Duration::from_secs(10));
        status.apply(StatusInput::Presence { participants: 0 }, Duration::from_secs(10));
        assert_eq!(status.failure(), Some(&SaveFailure::TimedOut));

        // Zero means the crate does not time saves at all; the app keeps its own rule.
        let mut status = HostStatus::with_save_timeout(Duration::ZERO);
        status.apply(StatusInput::SavePosted { id: Some("a") }, T0);
        status.apply(StatusInput::Presence { participants: 0 }, Duration::from_secs(3_600));
        assert_eq!(status.save_state(), SaveState::Saving);
        assert_eq!(status.failure(), None);
    }

    #[test]
    fn a_saved_that_arrives_after_the_timeout_still_counts() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::SavePosted { id: Some("a") }, T0);
        status.apply(StatusInput::Presence { participants: 0 }, Duration::from_secs(31));
        assert_eq!(status.failure(), Some(&SaveFailure::TimedOut));

        // The host was slow, not refusing: the bytes are stored, and the document is saved.
        status.apply(StatusInput::Saved { id: "a" }, Duration::from_secs(32));
        assert_eq!(status.save_state(), SaveState::Saved);
        assert_eq!(status.failure(), None);
    }

    #[test]
    fn a_refusal_after_a_timeout_replaces_it_with_the_hosts_text() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::SavePosted { id: Some("a") }, T0);
        status.apply(StatusInput::Presence { participants: 0 }, Duration::from_secs(31));
        assert_eq!(status.failure(), Some(&SaveFailure::TimedOut));

        let text = "someone else saved this file in the meantime";
        status.apply(StatusInput::SaveFailed { id: Some("a"), error: text }, Duration::from_secs(32));
        assert_eq!(status.save_state(), SaveState::Unsaved);
        assert_eq!(status.failure(), Some(&refused(text)));
        assert_eq!(status.cluster().failure.and_then(SaveFailure::text), Some(text), "shown as it came");
    }

    #[test]
    fn an_older_save_times_out_while_a_newer_one_still_waits() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::SavePosted { id: Some("a") }, T0);
        status.apply(StatusInput::SavePosted { id: Some("b") }, Duration::from_secs(20));

        status.apply(StatusInput::Presence { participants: 0 }, Duration::from_secs(31));
        assert_eq!(status.save_state(), SaveState::Saving, "b is younger than the timeout");
        assert_eq!(status.unanswered_saves().collect::<Vec<_>>(), ["b"]);
        assert_eq!(status.failure(), None, "a failure is not shown while a save is in flight");

        status.apply(StatusInput::Saved { id: "b" }, Duration::from_secs(32));
        assert_eq!(status.save_state(), SaveState::Saved);
    }

    #[test]
    fn a_save_without_an_id_never_times_out() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::SavePosted { id: None }, T0);
        status.apply(StatusInput::Presence { participants: 0 }, Duration::from_secs(3_600));
        assert_eq!(status.save_state(), SaveState::Saved, "no id: stored once posted");
        assert_eq!(status.failure(), None);
        assert_eq!(status.unanswered_saves().count(), 0);
    }

    #[test]
    fn a_timeout_does_not_end_a_commit_in_flight() {
        let mut status = HostStatus::new();
        status.apply(StatusInput::SavePosted { id: Some("a") }, T0);
        status.apply(StatusInput::CommitPosted, Duration::from_secs(1));
        status.apply(StatusInput::Presence { participants: 0 }, Duration::from_secs(31));
        assert_eq!(status.save_state(), SaveState::Saving, "the commit still waits");
        assert_eq!(status.failure(), None);

        status.apply(StatusInput::Committed { version: 4, etag: "\"v4\"" }, Duration::from_secs(40));
        assert_eq!(status.save_state(), SaveState::Saved);
    }

    #[test]
    fn the_host_confirms_saves_only_when_it_says_so() {
        let mut status = HostStatus::new();
        assert!(!status.confirms_saves(), "the bridges before the handshake do not");

        status.at(StatusInput::ConfirmsSaves(true));
        assert!(status.confirms_saves(), "the app now puts an id in every save");
        status.at(StatusInput::ConfirmsSaves(false));
        assert!(!status.confirms_saves());
    }

    #[test]
    fn a_refusals_text_is_kept_as_it_came_and_an_empty_one_still_fails() {
        let mut status = HostStatus::new();
        status.at(StatusInput::SavePosted { id: Some("a") });
        status.at(StatusInput::SaveFailed { id: Some("a"), error: "you can only view this file" });
        assert_eq!(status.failure().and_then(SaveFailure::text), Some("you can only view this file"));

        let mut status = HostStatus::new();
        status.at(StatusInput::SavePosted { id: Some("b") });
        status.at(StatusInput::SaveFailed { id: Some("b"), error: "" });
        assert_eq!(status.save_state(), SaveState::Unsaved, "a refusal without words still refuses");
        assert_eq!(status.failure(), Some(&refused("")));
    }
}
