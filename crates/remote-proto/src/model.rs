//! What a host says about itself and its work: the read side of the protocol.
//!
//! Each type here is what a remote client sees, not what the host stores. They are built to be
//! safe to send to another device. That rules out local paths (a project's folder, a worktree's
//! location), process arguments, environment variables, a harness's own session ids, labels and
//! terminal output. When a field is added, it has to clear the same bar.
//!
//! Times are milliseconds since the Unix epoch, as the store keeps them.

use serde::{Deserialize, Serialize};

use crate::ids::{
    EventId, HostId, MemoryId, ProjectId, RunId, SessionRecordId, TerminalId, WorkspaceId,
};
use crate::ProtocolVersion;

/// Longest name or title in a reply, in characters: a host's, a project's, a workspace's, a
/// branch, a conversation's title, a task's key and title. A longer one is [`clip`]ped.
pub const MAX_TEXT: usize = 256;
/// Longest label in a reply, in characters: a harness, a model, an effort, a program, an
/// author, a task's source. A longer one is [`clip`]ped.
pub const MAX_LABEL: usize = 64;
/// Longest memory note in a reply, in characters. Hosts accept far shorter notes; this bounds one
/// they kept from before they did.
pub const MAX_NOTE: usize = 1000;
/// The most tasks one workspace lists.
pub const MAX_TASKS: usize = 8;
/// Longest task link, in bytes. A task whose link is longer, or holds a control character, is
/// left out: a shortened link would lead somewhere else.
pub const MAX_LINK: usize = 512;
/// The most other terminals one [`SessionPage`] lists.
pub const MAX_TERMINALS: usize = 32;

/// `text`, cut to `max` characters when it is longer, the last of them `…`.
///
/// These limits are what keep one item of any list within [`crate::MAX_CONTROL_FRAME`], so a
/// page can always hold at least one.
pub fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_owned(),
        Some(_) => {
            let keep = text
                .char_indices()
                .nth(max.saturating_sub(1))
                .map_or(0, |(at, _)| at);
            format!("{}…", &text[..keep])
        }
    }
}

/// The operating system a host or a device runs. Used to label one, nothing more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Platform {
    Linux,
    Macos,
    Windows,
    /// A phone's, for a device that signed in from one.
    Ios,
    Android,
    #[serde(other)]
    Other,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "ios") {
            Self::Ios
        } else if cfg!(target_os = "android") {
            Self::Android
        } else {
            Self::Other
        }
    }
}

/// `host.describe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostDescription {
    pub host: HostId,
    /// What the person who enrolled the host called it.
    pub name: String,
    pub platform: Platform,
    /// The Yardsort release running there. For display and support; compatibility is decided
    /// by `protocol`, never by this.
    pub app_version: String,
    pub protocol: ProtocolVersion,
    /// The operations this host will answer. See [`crate::capability`].
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: ProjectId,
    pub name: String,
    /// The project's folder is gone from its host (deleted, moved, an unmounted drive).
    pub missing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceKind {
    /// The project's own checkout.
    Local,
    /// A git worktree on a branch of its own.
    Worktree,
}

/// What is checked out, asked of git when the list was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchRef {
    /// A branch name, or an abbreviated commit when detached.
    pub label: String,
    pub detached: bool,
    /// The branch has no commits yet.
    pub unborn: bool,
}

/// A task a workspace was started from, on its forge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskLink {
    /// Where the task is kept: `github`.
    pub source: String,
    /// What a person calls it there: `#91`.
    pub key: String,
    pub title: String,
    /// The task's page on its forge.
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    pub id: WorkspaceId,
    pub project: ProjectId,
    pub name: String,
    pub kind: WorkspaceKind,
    /// `None` when the folder is not on disk, or git would not say.
    pub branch: Option<BranchRef>,
    /// Put away on purpose: no folder, branch and history kept.
    pub archived: bool,
    /// The folder should be there and is not.
    pub missing: bool,
    /// At most [`MAX_TASKS`].
    pub tasks: Vec<TaskLink>,
    /// Conversations whose record says they are running. Whether a process is really there is
    /// on each [`SessionSummary`]; this count is for a list, and only as fresh as the list.
    pub running_sessions: u32,
}

/// `workspace.describe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDetail {
    pub workspace: WorkspaceSummary,
    pub project: ProjectSummary,
}

/// Whether a conversation's process is there, as far as its host can tell.
///
/// The record and the terminal daemon can disagree; this says which way rather than picking one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessState {
    /// The daemon has the process, running.
    Running,
    /// The record says it ended.
    Ended,
    /// The record says running, but the daemon has no running process for it.
    Gone,
    /// The record says running, and there was no daemon to ask.
    Unknown,
}

/// What a terminal has been doing, from when it last printed — never from what it printed.
///
/// `Quiet` means only that: nothing printed for a while. It is not "waiting for permission" or
/// "needs your answer". A program that asks a question goes quiet, and so does one that is done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TerminalActivity {
    /// Has printed nothing yet.
    Starting,
    /// Printing now.
    Busy,
    /// Has printed nothing for the host's quiet period.
    Quiet,
    /// The process has exited; the last screen can still be viewed.
    Exited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

/// A terminal a session runs in, while its host still has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSummary {
    pub id: TerminalId,
    pub activity: TerminalActivity,
    /// Milliseconds since it last printed, when the list was made.
    pub idle_ms: u32,
    /// The size the program draws at. Viewers render at this size; only a controller changes it.
    pub size: TermSize,
    /// The exit code, once exited.
    pub exit_code: Option<u32>,
}

/// One agent conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: SessionRecordId,
    pub workspace: WorkspaceId,
    /// Which harness: `claude`, `codex`, or a custom definition's id.
    pub harness: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The tab title the desktop shows. It may quote the start of the first message, so it is
    /// user content: shown, never logged.
    pub title: String,
    /// The conversation this one was forked from.
    pub forked_from: Option<SessionRecordId>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    /// What the record says the exit code was.
    pub exit_code: Option<i64>,
    pub process: ProcessState,
    /// Present while the host's daemon still has the terminal, exited or not.
    pub terminal: Option<TerminalSummary>,
}

/// What a terminal outside any recorded conversation runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TerminalPurpose {
    /// The user's shell.
    Shell,
    /// The project's run command.
    RunCommand,
    /// Some other program started in the workspace.
    Program,
    /// An agent whose conversation record is gone: forgotten, while its process ran on.
    Agent,
    /// The host cannot tell: it was started while activity recording was off. Also what a
    /// purpose from a newer host reads as.
    #[serde(other)]
    Other,
}

/// A terminal in a workspace that is not an agent conversation, or whose conversation the host
/// no longer has a record of.
///
/// Only while the host's daemon has it, running or exited: nothing is recorded about a shell
/// after it is closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTerminal {
    pub purpose: TerminalPurpose,
    /// The program's name, without its folder, extension or arguments: `zsh`, `npm`.
    pub program: String,
    /// Which harness, for an [`TerminalPurpose::Agent`].
    pub harness: Option<String>,
    pub terminal: TerminalSummary,
}

/// An approved memory entry. The only state a remote client is ever shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryNote {
    pub id: MemoryId,
    pub project: ProjectId,
    pub text: String,
    /// Who wrote the text: `user`, or `agent` for a proposal a person then approved.
    pub author: String,
    /// The harness that proposed it, for an agent's.
    pub harness: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// An opaque position in a list, from one page's reply, for the next page's request.
///
/// A client stores it and sends it back; it never builds or reads one. A host that no longer
/// understands a cursor answers `cursor_expired`, and the client starts the list again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Cursor(String);

/// Longest cursor accepted.
pub const MAX_CURSOR_LEN: usize = 256;

impl Cursor {
    pub fn new(text: impl Into<String>) -> Result<Self, crate::ids::IdError> {
        let text = text.into();
        if text.is_empty() {
            return Err(crate::ids::IdError::Empty);
        }
        if text.len() > MAX_CURSOR_LEN {
            return Err(crate::ids::IdError::TooLong);
        }
        if !text.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(crate::ids::IdError::BadCharacter);
        }
        Ok(Self(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Cursor {
    type Error = crate::ids::IdError;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::new(text)
    }
}

impl From<Cursor> for String {
    fn from(cursor: Cursor) -> String {
        cursor.0
    }
}

/// `workspace.list`.
///
/// Every page, of this and the other lists, fits in [`crate::MAX_CONTROL_FRAME`]. A page of
/// large items holds fewer than the request's `limit`, and its cursor goes on from the last of
/// them: a short page is not the last one, a page without a cursor is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePage {
    /// Every project in the grant that has a workspace on this page, so a client can group
    /// without a second request.
    pub projects: Vec<ProjectSummary>,
    pub workspaces: Vec<WorkspaceSummary>,
    /// `None` on the last page.
    pub next: Option<Cursor>,
}

/// `session.list`: most recently started first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPage {
    pub sessions: Vec<SessionSummary>,
    /// The workspace's other terminals: shells, the run command, and agents whose record is
    /// gone. On the first page only (a request without a cursor), since there are few and they
    /// have no order worth paging by; empty on later pages, and when there is no daemon to ask.
    /// At most [`MAX_TERMINALS`].
    #[serde(default)]
    pub terminals: Vec<WorkspaceTerminal>,
    pub next: Option<Cursor>,
}

/// `memory.list`: approved entries, most recently written first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPage {
    pub notes: Vec<MemoryNote>,
    pub next: Option<Cursor>,
}

/// Who recorded an event, how, and how far to trust it: `yardsort`/`lifecycle`/`observed` for
/// what Yardsort saw from outside a process, `claude`/`hook`/`reported` for what an agent said
/// about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub producer: String,
    pub method: String,
    pub fidelity: String,
}

/// One recorded fact about a workspace's processes and agents: what Yardsort saw (a process
/// started, exited, could not start) or what an agent reported through an adapter the user
/// switched on (a tool ran, a turn ended). Never anything read from a terminal.
///
/// A client shows a kind it does not know by its name, with whatever details it does know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub id: EventId,
    pub workspace: WorkspaceId,
    /// The conversation it belongs to, when it belongs to one the host still has a record of.
    pub session: Option<SessionRecordId>,
    /// The process it belongs to.
    pub run: Option<RunId>,
    /// The producer's clock, in milliseconds since the Unix epoch. Order by the page, not this:
    /// clocks disagree.
    pub occurred_at: i64,
    /// `process.started`, `tool.completed`, `turn.completed`, …
    pub kind: String,
    pub source: Provenance,
    pub detail: ActivityDetail,
}

/// What an event says, field by field. Every field is optional, and which ones an event has
/// depends on its kind and on who recorded it.
///
/// This is a list of allowed fields, not a copy of what the host stores. A host fills only
/// these, and only with values that pass its checks: names and short codes, counts, durations,
/// and paths relative to the workspace. Never a prompt, a command, a tool's input or output,
/// an error's text, an agent's own session id, or a path outside the workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDetail {
    /// Which harness: `claude`, `codex`, or a custom definition's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// For `process.*`: `harness`, `shell` or `program`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_kind: Option<String>,
    /// The program's name, without its folder or arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// `fresh`, `resumed` or `forked`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<String>,
    /// `app` or `cli`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launched_by: Option<String>,
    /// How the run was given to report about itself (`hook`, `notify`, …), when it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<String>,
    /// For `session.forked`: the conversation it was copied from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<SessionRecordId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
    /// How an exit came to be known: `live`, `spool`, `settle` or `reconcile`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// A short code saying why, where the producer gives one: `exited`, `interrupted`,
    /// `prompt_input_exit`. Never free text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// A tool's name, as the agent gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_type: Option<String>,
    /// A file, relative to the workspace's folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The file was outside the workspace; which one is not said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_outside_workspace: Option<bool>,
    /// For `file.reported_write`: `add`, `update` or `delete`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// For `prompt.submitted`: how long the message was. Its text is never recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chars: Option<u64>,
    /// For `approval.resolved`: `allow`, `deny`, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    /// How long the agent waited for that decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u64>,
    /// For `agent.notified`: what the agent says it wants, as its own code
    /// (`permission_prompt`, `idle_prompt`). Reported by the agent, never inferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_number: Option<u64>,
    /// How much context the turn's last request carried, and of how large a window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// A short code qualifying the event, such as `notify` for a Codex turn known only from
    /// its notification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `activity.list`: events oldest first, whichever way the request walked.
///
/// The first request (no cursor) gets the newest events. `older` walks back from there, a page
/// at a time; `newer` returns only what was recorded after a page, which is how a client that
/// was away catches up without fetching everything again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    pub events: Vec<ActivityEvent>,
    /// The page before this one. `None` when nothing older is kept.
    pub older: Option<Cursor>,
    /// What was recorded after this page. A client catching up keeps the one from the newest
    /// page it holds; asking with it returns an empty page and the same cursor when nothing is
    /// new. `None` only when there was nothing at all.
    pub newer: Option<Cursor>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipped_text_keeps_its_limit_in_characters_and_says_it_was_cut() {
        assert_eq!(clip("short", 5), "short");
        assert_eq!(clip("longer", 5), "long…");
        assert_eq!(clip("\u{1f600}\u{1f600}\u{1f600}", 2), "\u{1f600}…");
        assert_eq!(
            clip(&"\u{e9}".repeat(300), MAX_TEXT).chars().count(),
            MAX_TEXT
        );
    }

    #[test]
    fn a_platform_from_the_future_is_other() {
        let platform: Platform = serde_json::from_str("\"plan9\"").unwrap();
        assert_eq!(platform, Platform::Other);
    }

    #[test]
    fn a_cursor_is_bounded_and_printable() {
        assert!(Cursor::new("k1.abc").is_ok());
        assert!(Cursor::new("").is_err());
        assert!(Cursor::new("a b").is_err());
        assert!(Cursor::new("x".repeat(MAX_CURSOR_LEN + 1)).is_err());
    }
}
