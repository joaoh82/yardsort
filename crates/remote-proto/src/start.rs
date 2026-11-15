//! Starting work from a device, and handing an agent a file: since minor 5, and resuming a
//! conversation since minor 6. Since minor 10 a start may also say what the window's composer
//! says: a name for the workspace, a model and an effort for the agent, and the branch to start
//! from or to open.
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

/// The most models, and the most efforts, one harness lists.
pub const MAX_CHOICES: usize = 32;
/// Longest name a start may give a workspace, in characters: what the host's own window allows.
pub const MAX_WORKSPACE_NAME: usize = 80;
/// Longest branch name a request or a reply carries, in bytes.
pub const MAX_BRANCH: usize = 255;

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
    /// Models to suggest for it, as its settings list them: any other may be named too. At most
    /// [`MAX_CHOICES`], each at most [`crate::MAX_LABEL`]. Since minor 10.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// The efforts it takes; none when it has no such thing. At most [`MAX_CHOICES`], each at
    /// most [`crate::MAX_LABEL`]. Since minor 10.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
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
///
/// Since minor 10 it may say more, each as the host's own composer would: `name` for the
/// workspace, `model` and `effort` for the agent, and either `baseBranch`, to start the new
/// branch from, or `existingBranch`, to open a branch as it is. A host before minor 10 does not
/// read them, so a client sends them only to a host that answers `project.branches`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStartParams {
    pub project: ProjectId,
    /// The first message. Not empty, at most [`MAX_PROMPT`] bytes.
    pub prompt: String,
    /// A harness from `harness.list`. Without one, the host's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// The workspace's folder, and its branch after the host's prefix. The host makes it a slug,
    /// and adds a number when it is taken. At most [`MAX_WORKSPACE_NAME`] characters. Not with
    /// `existingBranch`, which names its own. Since minor 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The model the agent is told to use, passed to the harness as its settings say. At most
    /// [`crate::MAX_LABEL`] characters. Since minor 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// One of the harness's `efforts`. Since minor 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The branch the new one starts from: one of `project.branches`. Without it, the project's
    /// default. Since minor 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    /// A branch to open as it is, with no new one made: one of `project.branches` that is not
    /// checked out. Not with `baseBranch` or `name`. Since minor 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_branch: Option<String>,
}

impl WorkspaceStartParams {
    /// A start that says only what every host reads: the project, the message and the harness.
    pub fn plain(project: ProjectId, prompt: impl Into<String>, harness: Option<String>) -> Self {
        Self {
            project,
            prompt: prompt.into(),
            harness,
            name: None,
            model: None,
            effort: None,
            base_branch: None,
            existing_branch: None,
        }
    }

    /// Whether it says anything a host before minor 10 would not read.
    pub fn says_more(&self) -> bool {
        self.name.is_some()
            || self.model.is_some()
            || self.effort.is_some()
            || self.base_branch.is_some()
            || self.existing_branch.is_some()
    }
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

/// The reply to `workspace.start`, `session.start`, `session.resume` and `shell.start`: where
/// the agent or the shell runs, to watch it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Started {
    pub workspace: WorkspaceId,
    pub terminal: TerminalId,
    /// The harness started, by its id. Empty for a shell.
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

/// A branch name as a request may give one: something git could have, and nothing that could be
/// read as an option.
pub(crate) fn branch_valid(branch: &str) -> Result<(), RemoteError> {
    if branch.is_empty()
        || branch.len() > MAX_BRANCH
        || branch.starts_with('-')
        || branch.chars().any(|c| c.is_control() || c == ' ')
    {
        return invalid("That is not a branch's name.");
    }
    Ok(())
}

/// A workspace's name as a request may give one.
pub(crate) fn workspace_name_valid(name: &str) -> Result<(), RemoteError> {
    if name.trim().is_empty()
        || name.chars().count() > MAX_WORKSPACE_NAME
        || name.chars().any(char::is_control)
    {
        return invalid("A workspace name needs 1 to 80 characters.");
    }
    Ok(())
}

fn label_valid(label: Option<&String>, what: &str) -> Result<(), RemoteError> {
    match label {
        Some(l)
            if l.trim().is_empty()
                || l.chars().count() > crate::MAX_LABEL
                || l.starts_with('-')
                || l.chars().any(char::is_control) =>
        {
            invalid(what)
        }
        _ => Ok(()),
    }
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
        harness_valid(self.harness.as_ref())?;
        label_valid(self.model.as_ref(), "That is not a model's name.")?;
        label_valid(self.effort.as_ref(), "That is not an effort.")?;
        if let Some(name) = &self.name {
            workspace_name_valid(name)?;
        }
        for branch in self.base_branch.iter().chain(&self.existing_branch) {
            branch_valid(branch)?;
        }
        if self.existing_branch.is_some() && (self.base_branch.is_some() || self.name.is_some()) {
            return invalid("An existing branch is opened as it is: it takes no name and no base.");
        }
        Ok(())
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
