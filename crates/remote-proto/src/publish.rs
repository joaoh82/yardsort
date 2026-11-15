//! Sending a workspace's work on from a device: since minor 12.
//!
//! What the window's commit box and publish bar do for a local workspace, asked of a host by the
//! workspace's id: where it stands with its remote (`workspace.publish`), commit everything
//! changed in it (`workspace.commit`), push its branch (`workspace.push`), open a pull request
//! for it (`pull_request.open`) and merge that pull request (`pull_request.merge`).
//!
//! All need `terminal:control`: a device that may type into a terminal there could run `git` and
//! `gh` in it. The host commits with its own Git identity, pushes with its own credentials and
//! asks the forge with its own sign-in. No request carries a credential, a remote, a branch to
//! push to, or a path: the host pushes the workspace's own branch to the project's own remote.
//!
//! Each of these but `workspace.publish`, which only reads, changes a repository, so a client
//! sends one only at its person's press. A merge
//! names the commit the person saw at the head of the pull request, and the host merges only if
//! it is still there.

use serde::{Deserialize, Serialize};

use crate::ids::WorkspaceId;
use crate::{ErrorCode, RemoteError};

/// Longest commit message a request carries, in bytes.
pub const MAX_COMMIT_MESSAGE: usize = 16 * 1024;
/// Longest pull request title, in characters.
pub const MAX_PULL_REQUEST_TITLE: usize = 256;
/// Longest pull request body a request carries, in bytes.
pub const MAX_PULL_REQUEST_BODY: usize = 32 * 1024;
/// The most unpushed commits a reply names.
pub const MAX_UNPUSHED: usize = 20;
/// Longest commit id a request or a reply carries, in characters.
pub const MAX_OID: usize = 64;
/// Longest link in a reply, in bytes.
pub const MAX_URL: usize = 1024;

/// `workspace.publish`: where a workspace stands with its remote. Needs `terminal:control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePublishParams {
    pub workspace: WorkspaceId,
    /// Ask the forge again about its pull request, which takes the host a network round trip.
    /// Without it, only what Git knows is answered, and `pullRequest` is absent.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub forge: bool,
}

/// What the forge's checks say about a pull request's head commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Checks {
    None,
    Running,
    Passing,
    Failing,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PullRequestState {
    Open,
    Merged,
    Closed,
    #[serde(other)]
    Unknown,
}

/// The pull request of a workspace's branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u32,
    /// Its page on the forge. At most [`MAX_URL`] bytes.
    pub url: String,
    /// At most [`MAX_PULL_REQUEST_TITLE`] characters.
    pub title: String,
    pub state: PullRequestState,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub draft: bool,
    pub checks: Checks,
    /// The commit at its head, to name when merging it: 7 to [`MAX_OID`] hexadecimal characters.
    /// Absent when the forge did not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_oid: Option<String>,
    /// The forge says it does not merge cleanly into its base.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub conflicting: bool,
}

/// The reply to `workspace.publish`, `workspace.commit` and `workspace.push`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishState {
    /// Absent when the workspace is not on a branch: there is nothing to push or open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// What a pull request would merge into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Who commits there would be by. Absent when Git has no name and email on the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// The project has a remote to push to.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remote: bool,
    /// The branch has been pushed, and tracks its remote branch.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pushed: bool,
    /// Commits the remote does not have.
    pub ahead: u32,
    pub behind: u32,
    /// The subjects of those commits, newest first. At most [`MAX_UNPUSHED`], each at most
    /// [`crate::MAX_TEXT`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unpushed: Vec<String>,
    /// Absent when there is none, and when the forge was not asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<PullRequest>,
    /// Opening one makes sense: a branch that is not its own base, on a forge, with none open.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub can_open: bool,
    /// The host has the forge's own program and can open and merge without a browser.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gh: bool,
    /// Why the forge gave no answer, when it was asked. At most [`crate::MAX_TEXT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// `workspace.commit`: commit everything changed in a workspace. Needs `terminal:control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCommitParams {
    pub workspace: WorkspaceId,
    /// Not empty, at most [`MAX_COMMIT_MESSAGE`] bytes.
    pub message: String,
}

/// `pull_request.open`: open a pull request for a workspace's branch, pushing it first when the
/// remote does not have it all. Needs `terminal:control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestOpenParams {
    pub workspace: WorkspaceId,
    /// Not empty, at most [`MAX_PULL_REQUEST_TITLE`] characters.
    pub title: String,
    /// At most [`MAX_PULL_REQUEST_BODY`] bytes.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub draft: bool,
}

/// The reply to `pull_request.open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestOpened {
    /// The pull request, or the forge's own form to finish in a browser. At most [`MAX_URL`].
    pub url: String,
    /// The host opened it. When false, `url` is the form, with both branches filled in.
    pub created: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeMethod {
    Squash,
    Merge,
    Rebase,
}

/// `pull_request.merge`: merge the pull request of a workspace's branch, if it is still open,
/// not a draft, and at the commit named. Needs `terminal:control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestMergeParams {
    pub workspace: WorkspaceId,
    pub number: u32,
    /// The commit the person saw at its head: 7 to [`MAX_OID`] characters, all hexadecimal.
    pub head_oid: String,
    pub method: MergeMethod,
}

fn invalid(message: &str) -> Result<(), RemoteError> {
    Err(RemoteError::new(ErrorCode::InvalidRequest, message))
}

impl WorkspaceCommitParams {
    pub(crate) fn validate(&self) -> Result<(), RemoteError> {
        if self.message.trim().is_empty() {
            return invalid("A commit needs a message.");
        }
        if self.message.len() > MAX_COMMIT_MESSAGE || self.message.contains('\0') {
            return invalid("That is not a commit message one request may carry.");
        }
        Ok(())
    }
}

impl PullRequestOpenParams {
    pub(crate) fn validate(&self) -> Result<(), RemoteError> {
        let title = self.title.trim();
        if title.is_empty()
            || title.chars().count() > MAX_PULL_REQUEST_TITLE
            || title.starts_with('-')
            || title.chars().any(char::is_control)
        {
            return invalid("A pull request needs a title of 1 to 256 characters on one line.");
        }
        if self.body.len() > MAX_PULL_REQUEST_BODY || self.body.contains('\0') {
            return invalid("That is not a description one request may carry.");
        }
        Ok(())
    }
}

impl PullRequestMergeParams {
    pub(crate) fn validate(&self) -> Result<(), RemoteError> {
        let oid = &self.head_oid;
        if self.number == 0
            || oid.len() < 7
            || oid.len() > MAX_OID
            || !oid.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return invalid("That is not a pull request and the commit at its head.");
        }
        Ok(())
    }
}
