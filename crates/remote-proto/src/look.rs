//! Reading a workspace's changes and files from a device: since minor 11.
//!
//! What the window's right panel shows for a local workspace, asked of a host by the workspace's
//! id: which files changed (`workspace.changes`), one folder of its file tree (`workspace.files`),
//! and a file's contents, on disk or on one side of a change (`file.read`). Nothing here writes.
//!
//! All three need `terminal:control`. A device that may type into a terminal there can already
//! read any file through it, so this gives it nothing new; a device that only watches gets none
//! of it.
//!
//! A file is named by its path inside the workspace, never by a path on the host. The host checks
//! that it stays inside, symbolic links included, and answers nothing outside it. The file tree
//! leaves out `.git` and what Git ignores, as the window's does by default.
//!
//! A file is larger than one frame carries, so it is read in pieces of at most
//! [`MAX_FILE_CHUNK`] bytes, each asked for by where it starts. A file of more than [`MAX_FILE`]
//! bytes, and one that is not text, is said to be so and not sent.

use serde::{Deserialize, Serialize};

use crate::ids::WorkspaceId;
use crate::{ErrorCode, RemoteError};

/// Longest path inside a workspace a request or a reply carries, in bytes.
pub const MAX_PATH: usize = 1024;
/// The largest file a host sends, in bytes: what its own window shows.
pub const MAX_FILE: u64 = 1024 * 1024;
/// The most one `file.read` reply carries, in bytes before base64: its response stays well
/// within [`crate::MAX_CONTROL_FRAME`].
pub const MAX_FILE_CHUNK: usize = 32 * 1024;
/// How much of a list of paths one reply carries, in bytes of path: what keeps a list of long
/// paths within a frame. A list cut short says so with `more`.
pub const MAX_LISTED_BYTES: usize = 40 * 1024;

/// How a file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    /// New, and not yet known to Git.
    Untracked,
    Conflicted,
    /// A kind from a newer host.
    #[serde(other)]
    Unknown,
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    /// Inside the workspace, with `/` between its parts. At most [`MAX_PATH`] bytes; a file whose
    /// path is longer is left out, and `more` says so.
    pub path: String,
    /// Where a renamed file was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub kind: ChangeKind,
    /// Lines added and removed. Absent for a binary or an untracked file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletions: Option<u32>,
}

/// The reply to `workspace.changes`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChanges {
    /// Changed and not committed.
    pub uncommitted: Vec<ChangedFile>,
    /// Committed on the workspace's branch since `base`.
    pub committed: Vec<ChangedFile>,
    /// The branch `committed` is measured against. Absent when there is nothing to compare with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// There are more changed files than are listed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub more: bool,
}

/// `workspace.files`: one folder of a workspace's file tree. Needs `terminal:control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFilesParams {
    pub workspace: WorkspaceId,
    /// The folder, inside the workspace. Empty, or absent, for its root.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub dir: String,
}

/// One entry of a folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    /// Its name within the folder.
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dir: bool,
}

/// The reply to `workspace.files`: folders first, then files, each by name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileList {
    pub entries: Vec<FileEntry>,
    /// The folder holds more than is listed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub more: bool,
}

/// Which contents of a file `file.read` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileOf {
    /// As it is on disk now: a file of the tree, and the new side of an uncommitted change.
    Working,
    /// Before an uncommitted change: as the last commit has it.
    UncommittedOld,
    /// Before the workspace's commits: as the base branch has it.
    CommittedOld,
    /// After the workspace's commits: as the last commit has it.
    CommittedNew,
}

/// `file.read`: a piece of a file in a workspace. Needs `terminal:control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileReadParams {
    pub workspace: WorkspaceId,
    /// Inside the workspace. At most [`MAX_PATH`] bytes.
    pub path: String,
    /// For one side of a renamed file's change: where it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub of: FileOf,
    /// Where the piece starts: 0, then each reply's `offset` plus what it carried.
    #[serde(default)]
    pub offset: u64,
}

/// What a file's contents are, as a reply can say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileState {
    /// Text, carried in `data`.
    Text,
    /// There is no such file on that side: it was added, or deleted.
    Absent,
    /// Not text. Nothing is carried.
    Binary,
    /// More than [`MAX_FILE`] bytes: `size` says how many. Nothing is carried.
    TooLarge,
    /// Something the host does not show: a folder, a link, a device.
    NotShown,
    /// A state from a newer host.
    #[serde(other)]
    Unknown,
}

/// The reply to `file.read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChunk {
    pub state: FileState,
    /// The whole file's size in bytes. It is the same in every piece of one reading: when it
    /// differs, the file changed meanwhile, and the client reads it again from the start.
    pub size: u64,
    /// Where this piece starts.
    pub offset: u64,
    /// Standard base64, padded, of at most [`MAX_FILE_CHUNK`] bytes of the file. Empty unless
    /// `state` is `text`. A piece may end inside a character: the client joins the pieces before
    /// it reads them as UTF-8.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub data: String,
    /// Nothing comes after this piece.
    pub done: bool,
}

fn invalid(message: &str) -> Result<(), RemoteError> {
    Err(RemoteError::new(ErrorCode::InvalidRequest, message))
}

/// A path as a request may give one: relative, within bounds, and never one that climbs out.
/// Whether it stays inside the workspace on disk, links included, is the host's to check.
pub(crate) fn path_valid(path: &str) -> Result<(), RemoteError> {
    let climbs = path
        .split(['/', '\\'])
        .any(|part| part == ".." || part.is_empty());
    if path.is_empty()
        || path.len() > MAX_PATH
        || path.starts_with(['/', '\\', '-'])
        || path.chars().any(char::is_control)
        // A drive's letter, on a host that has drives.
        || path.as_bytes().get(1) == Some(&b':')
        || climbs
    {
        return invalid("That is not a path inside the workspace.");
    }
    Ok(())
}

impl WorkspaceFilesParams {
    pub(crate) fn validate(&self) -> Result<(), RemoteError> {
        if self.dir.is_empty() {
            return Ok(());
        }
        path_valid(&self.dir)
    }
}

impl FileReadParams {
    pub(crate) fn validate(&self) -> Result<(), RemoteError> {
        path_valid(&self.path)?;
        if let Some(old) = &self.old_path {
            path_valid(old)?;
        }
        if self.offset > MAX_FILE {
            return invalid("That is past the end of any file a computer sends.");
        }
        Ok(())
    }
}

/// Keep a list of changed files within what one reply carries: those whose paths fit, until
/// [`MAX_LISTED_BYTES`] of path are listed, and whether any were left out. The uncommitted come
/// first, since they are what is being worked on.
pub fn bounded_changes(
    uncommitted: Vec<ChangedFile>,
    committed: Vec<ChangedFile>,
    base: Option<String>,
) -> WorkspaceChanges {
    let mut left = MAX_LISTED_BYTES;
    let mut more = false;
    let mut keep = |files: Vec<ChangedFile>| -> Vec<ChangedFile> {
        files
            .into_iter()
            .filter(|file| {
                let old = file.old_path.as_deref().unwrap_or("");
                let fits =
                    path_valid(&file.path).is_ok() && (old.is_empty() || path_valid(old).is_ok());
                // Each entry's JSON around its paths is small and of a fixed size.
                let cost = file.path.len() + old.len() + 96;
                if !fits || cost > left {
                    more = true;
                    return false;
                }
                left -= cost;
                true
            })
            .collect()
    };
    let uncommitted = keep(uncommitted);
    let committed = keep(committed);
    WorkspaceChanges {
        uncommitted,
        committed,
        base: base.filter(|b| b.len() <= crate::start::MAX_BRANCH),
        more,
    }
}

/// Keep a folder's entries within what one reply carries, in the order given.
pub fn bounded_files(entries: Vec<FileEntry>) -> FileList {
    let mut left = MAX_LISTED_BYTES;
    let mut more = false;
    let entries = entries
        .into_iter()
        .filter(|entry| {
            let cost = entry.name.len() + 32;
            let fits = !entry.name.is_empty()
                && entry.name.len() <= 255
                && !entry.name.chars().any(char::is_control);
            if !fits || cost > left {
                more = true;
                return false;
            }
            left -= cost;
            true
        })
        .collect();
    FileList { entries, more }
}

/// The whole of a reply for a file that is not sent: absent, binary, too large or not shown.
pub fn not_sent(state: FileState, size: u64) -> FileChunk {
    FileChunk {
        state,
        size,
        offset: 0,
        data: String::new(),
        done: true,
    }
}
