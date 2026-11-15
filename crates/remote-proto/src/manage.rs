//! Looking after projects and workspaces from a device: since minor 10.
//!
//! What the window's composer and a workspace's menu do, asked of a host by ids: which branches a
//! project has (`project.branches`), a repository cloned into the host's own projects folder and
//! added as a project (`project.clone`), a shell in a workspace (`shell.start`), and a workspace
//! renamed, archived or deleted (`workspace.rename`, `workspace.archive`, `workspace.delete`).
//!
//! None names a folder. A clone names a repository by its address, which the host checks as it
//! checks one typed into its own window: an HTTPS, `ssh://` or `user@host:path` address, with no
//! credentials in it and nothing that could be read as an option. Where it goes is the host's
//! setting, and it is fetched with the host's own Git credentials.
//!
//! Archiving and deleting keep the branch, as they do in the window. Uncommitted work lives only
//! in the folder that goes, so a host refuses either with `uncommitted_work` until the request
//! says `force`, which a client sends only after its person has been asked a second time.

use serde::{Deserialize, Serialize};

use crate::ids::{ProjectId, WorkspaceId};
use crate::start::{branch_valid, workspace_name_valid, MAX_BRANCH};
use crate::{ErrorCode, RemoteError};

/// The most branches one `project.branches` reply lists.
pub const MAX_BRANCHES: usize = 100;
/// Longest repository address a request or a reply carries, in bytes.
pub const MAX_REMOTE: usize = 512;
/// Longest name a clone may be given, in bytes: what the host's own window allows.
pub const MAX_PROJECT_NAME: usize = 100;

/// `project.branches`: the branches a new workspace in `project` can start from or open. Needs
/// `workspace:read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRef {
    pub project: ProjectId,
}

/// The reply to `project.branches`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBranches {
    /// Local branches, the default first when there is one, then by name. At most
    /// [`MAX_BRANCHES`], each at most [`MAX_BRANCH`] bytes; a longer name is left out.
    pub branches: Vec<String>,
    /// The branch a new workspace starts from when a start names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Those of `branches` a worktree has out, which cannot be opened in another.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checked_out: Vec<String>,
    /// The project has more branches than are listed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub more: bool,
}

/// `project.clone`: clone a repository into the host's projects folder and add it as a project.
/// Needs `terminal:control`, and a grant that reaches every project: the new one is not among
/// the projects a narrower grant names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCloneParams {
    /// The repository's address, with no credentials in it. At most [`MAX_REMOTE`] bytes.
    pub remote: String,
    /// What to call the folder and the project. Without one, the last part of the address. At
    /// most [`MAX_PROJECT_NAME`] bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// The reply to `project.clone`: the project the clone was added as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCloned {
    pub project: ProjectId,
    /// At most [`crate::MAX_TEXT`].
    pub name: String,
}

/// `shell.start`: the host's shell in a workspace. Needs `terminal:control`. The reply is
/// [`crate::Started`], with no harness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellStartParams {
    pub workspace: WorkspaceId,
}

/// `workspace.rename`: change what a workspace is called. Its folder and its branch keep their
/// names. Needs `terminal:control`. Not for `local`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRenameParams {
    pub workspace: WorkspaceId,
    /// From 1 to [`crate::start::MAX_WORKSPACE_NAME`] characters.
    pub name: String,
}

/// `workspace.archive` and `workspace.delete`: take a workspace's folder off the host's disk.
/// Archiving keeps its record and its conversations, to restore it; deleting forgets it. Both
/// keep the branch. Needs `terminal:control`. Not for `local`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRemoveParams {
    pub workspace: WorkspaceId,
    /// Go ahead although the folder has uncommitted work, which is then lost. Without it, such
    /// a workspace is refused with `uncommitted_work`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub force: bool,
}

fn invalid(message: &str) -> Result<(), RemoteError> {
    Err(RemoteError::new(ErrorCode::InvalidRequest, message))
}

impl ProjectCloneParams {
    /// Shape only. Whether it is an address the host will clone from is the host's to say.
    pub(crate) fn validate(&self) -> Result<(), RemoteError> {
        let remote = self.remote.trim();
        if remote.is_empty()
            || remote.len() > MAX_REMOTE
            || remote.starts_with('-')
            || remote.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return invalid("That is not a repository's address.");
        }
        match &self.name {
            Some(name)
                if name.trim().is_empty()
                    || name.len() > MAX_PROJECT_NAME
                    || name
                        .chars()
                        .any(|c| c.is_control() || c == '/' || c == '\\') =>
            {
                invalid("That is not a name a project can have.")
            }
            _ => Ok(()),
        }
    }
}

impl WorkspaceRenameParams {
    pub(crate) fn validate(&self) -> Result<(), RemoteError> {
        workspace_name_valid(&self.name)
    }
}

/// Keep a host's branch list within what one reply carries: names that fit, the default first,
/// at most [`MAX_BRANCHES`], and whether any were left out.
pub fn bounded_branches(
    branches: Vec<String>,
    default: Option<String>,
    checked_out: Vec<String>,
) -> ProjectBranches {
    let fits = |name: &String| branch_valid(name).is_ok() && name.len() <= MAX_BRANCH;
    let default = default.filter(fits);
    let mut listed: Vec<String> = default.iter().cloned().collect();
    let mut more = false;
    for name in branches {
        if Some(&name) == default.as_ref() {
            continue;
        }
        if !fits(&name) || listed.len() >= MAX_BRANCHES {
            more = true;
            continue;
        }
        listed.push(name);
    }
    let checked_out = checked_out
        .into_iter()
        .filter(|name| listed.contains(name))
        .collect();
    ProjectBranches {
        branches: listed,
        default,
        checked_out,
        more,
    }
}
