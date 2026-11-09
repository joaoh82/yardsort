//! Starting work from a device, and handing an agent a file: since minor 5, and resuming a
//! conversation since minor 6.
//!
//! A device that may type into a terminal (`terminal:control`) may also start what a terminal
//! runs: a new workspace with an agent and its first message (`workspace.start`), another agent
//! in a workspace (`session.start`), or an ended conversation again, under its own record
//! (`session.resume`). None names a program, an argument or a folder. The device picks a harness
//! by the id `harness.list` gave it, or a conversation by its record's id, and the host starts it
//! as its own settings say, exactly as its window would.
//!
//! **Uploading** puts a file the device sends — a photo, for an agent to look at — on the host,
//! in a folder of the host's own, outside every worktree. `upload.begin` says how large it is and
//! what it is called; `upload.chunk` carries it in order, as base64, each piece at most
//! [`MAX_UPLOAD_CHUNK`] bytes before encoding. The last piece's reply is [`Uploaded`], with the
//! file's path on the host: the device then pastes that path into a terminal, as a file dropped
//! on the desktop's terminal is. That path is the one place a reply carries one, and only to a
//! device that could type `pwd` anyway. An upload lives as long as its channel: one the channel
//! closes on is deleted.

use serde::{Deserialize, Serialize};

use crate::ids::{ProjectId, SessionRecordId, TerminalId, WorkspaceId};
use crate::{ErrorCode, RemoteError};

/// Longest message a device may send with `workspace.start`, `session.start` or
/// `session.resume`, in bytes.
pub const MAX_PROMPT: usize = 16 * 1024;
/// The largest file one upload carries, in bytes.
pub const MAX_UPLOAD: u64 = 20 * 1024 * 1024;
/// The most one `upload.chunk` carries, in bytes before base64: its request stays well within
/// [`crate::MAX_CONTROL_FRAME`].
pub const MAX_UPLOAD_CHUNK: usize = 32 * 1024;
/// Longest file name an upload may give, in characters.
pub const MAX_UPLOAD_NAME: usize = 128;
/// The most uploads one channel has under way at once.
pub const MAX_UPLOADS: usize = 4;

/// An upload under way, numbered by the host within its channel.
pub type UploadId = u32;

/// An agent the host can start: one its settings have, whose program it found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSummary {
    /// What `workspace.start` and `session.start` name it by. At most [`crate::MAX_LABEL`].
    pub id: String,
    /// What to show. At most [`crate::MAX_TEXT`].
    pub name: String,
}

/// The reply to `harness.list`: the agents the host can start, the one it starts by default
/// first. Empty when it finds none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessList {
    pub harnesses: Vec<HarnessSummary>,
}

/// `workspace.start`: a new workspace in `project` — a branch named after `prompt`, from the
/// project's default branch, and its worktree — with an agent given `prompt` as its first
/// message. Needs `terminal:control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStartParams {
    pub project: ProjectId,
    /// The first message. Not empty, at most [`MAX_PROMPT`] bytes.
    pub prompt: String,
    /// A harness from `harness.list`. Without one, the host's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
}

/// `session.start`: another agent in a workspace, with a first message or none. Needs
/// `terminal:control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStartParams {
    pub workspace: WorkspaceId,
    /// A harness from `harness.list`. Without one, the host's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// At most [`MAX_PROMPT`] bytes. Without one, or empty, the agent waits to be told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

/// `session.resume`: an ended conversation again, in a new terminal, under the same record, with
/// a message to go on with or none. Only one whose summary says it is `resumable`. Needs
/// `terminal:control`. Since minor 6.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResumeParams {
    pub session: SessionRecordId,
    /// At most [`MAX_PROMPT`] bytes. Without one, or empty, the agent waits to be told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

/// The reply to `workspace.start`, `session.start` and `session.resume`: where the agent runs,
/// to watch it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Started {
    pub workspace: WorkspaceId,
    pub terminal: TerminalId,
    /// The harness started, by its id.
    pub harness: String,
}

/// `upload.begin`: a file of `size` bytes is coming, for an agent in `workspace`. Needs
/// `terminal:control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadBeginParams {
    pub workspace: WorkspaceId,
    /// What the file is called on the device. The host keeps its extension, and only letters,
    /// digits, `-` and `_` of the rest. At most [`MAX_UPLOAD_NAME`] characters.
    pub name: String,
    /// From 1 to [`MAX_UPLOAD`].
    pub size: u64,
}

/// The reply to `upload.begin`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Upload {
    pub upload: UploadId,
}

/// `upload.chunk`: the next piece of an upload. Needs `terminal:control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadChunkParams {
    pub upload: UploadId,
    /// Where it goes: all that came before it, so a piece out of order is refused.
    pub offset: u64,
    /// Standard base64, padded, of at most [`MAX_UPLOAD_CHUNK`] bytes. Not empty.
    pub data: String,
}

/// The reply to the `upload.chunk` that completes an upload. Earlier pieces' reply is `done`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Uploaded {
    pub upload: UploadId,
    /// Where the file is on the host, to paste into a terminal there.
    pub path: String,
}

fn invalid(message: &str) -> Result<(), RemoteError> {
    Err(RemoteError::new(ErrorCode::InvalidRequest, message))
}

fn harness_valid(harness: Option<&String>) -> Result<(), RemoteError> {
    match harness {
        Some(h) if h.is_empty() || h.chars().count() > crate::MAX_LABEL => {
            invalid("That is not a harness's id.")
        }
        _ => Ok(()),
    }
}

impl WorkspaceStartParams {
    pub(crate) fn validate(&self) -> Result<(), RemoteError> {
        if self.prompt.trim().is_empty() {
            return invalid("A new workspace needs a first message.");
        }
        if self.prompt.len() > MAX_PROMPT {
            return invalid("That message is longer than one request may carry.");
        }
        harness_valid(self.harness.as_ref())
    }
}

impl SessionStartParams {
    pub(crate) fn validate(&self) -> Result<(), RemoteError> {
        if self.prompt.as_ref().is_some_and(|p| p.len() > MAX_PROMPT) {
            return invalid("That message is longer than one request may carry.");
        }
        harness_valid(self.harness.as_ref())
    }
}

impl SessionResumeParams {
    pub(crate) fn validate(&self) -> Result<(), RemoteError> {
        if self.prompt.as_ref().is_some_and(|p| p.len() > MAX_PROMPT) {
            return invalid("That message is longer than one request may carry.");
        }
        Ok(())
    }
}

impl UploadBeginParams {
    pub(crate) fn validate(&self) -> Result<(), RemoteError> {
        if self.size == 0 || self.size > MAX_UPLOAD {
            return invalid("A file to upload is from 1 byte to 20 MB.");
        }
        if self.name.is_empty() || self.name.chars().count() > MAX_UPLOAD_NAME {
            return invalid("That is not a file name an upload may give.");
        }
        Ok(())
    }
}

impl UploadChunkParams {
    /// Shape only: whether `data` is padded standard base64 of 1 to [`MAX_UPLOAD_CHUNK`] bytes.
    /// The host decodes it.
    pub(crate) fn validate(&self) -> Result<(), RemoteError> {
        let data = self.data.as_bytes();
        if data.is_empty() || data.len() > MAX_UPLOAD_CHUNK.div_ceil(3) * 4 {
            return invalid("A piece of an upload carries from 1 byte to 32 KB.");
        }
        let body = data
            .strip_suffix(b"==")
            .or_else(|| data.strip_suffix(b"="))
            .unwrap_or(data);
        let alphabet = |b: &u8| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'/';
        if !data.len().is_multiple_of(4) || !body.iter().all(alphabet) {
            return invalid("That piece is not base64.");
        }
        Ok(())
    }
}
