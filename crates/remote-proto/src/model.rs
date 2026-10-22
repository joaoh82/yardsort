//! What a host says about itself and its work: the read side of the protocol.
//!
//! Each type here is what a remote client sees, not what the host stores. They are built to be
//! safe to send to another device. That rules out local paths (a project's folder, a worktree's
//! location), process arguments, environment variables, a harness's own session ids, labels and
//! terminal output. When a field is added, it has to clear the same bar.
//!
//! Times are milliseconds since the Unix epoch, as the store keeps them.

use serde::{Deserialize, Serialize};

use crate::ids::{HostId, MemoryId, ProjectId, SessionRecordId, TerminalId, WorkspaceId};
use crate::ProtocolVersion;

/// The operating system a host runs. Clients use it to label a host, nothing more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Platform {
    Linux,
    Macos,
    Windows,
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
    pub next: Option<Cursor>,
}

/// `memory.list`: approved entries, most recently written first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPage {
    pub notes: Vec<MemoryNote>,
    pub next: Option<Cursor>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
