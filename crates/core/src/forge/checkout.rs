//! Getting a pull request's branch into a project's repository, so a workspace can be opened on
//! it: the Pull requests view's "start a workspace on it", and `ys pr checkout`.
//!
//! Nothing is checked out and no worktree is made here. That is the caller's, through
//! [`crate::workspaces::Workspaces::open_branch`].

use std::path::Path;

use serde::Serialize;
use specta::Type;

use super::{parse_remote, PullRequest};
use crate::error::{IpcError, IpcResult};
use crate::git::Git;

/// A local branch a workspace can be opened on for a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PreparedBranch {
    pub branch: String,
    /// Commits the pull request has that this branch does not. Only a branch that was already
    /// here can be behind: it is opened as it is and never moved.
    pub behind: u32,
    /// The pull request comes from a fork, so the branch is `pr/<number>` and follows the pull
    /// request rather than a branch of this project's remote. Yardsort will not push it.
    pub fork: bool,
}

/// Four cases, by where the branch lives:
///
/// 1. Checked out somewhere already — refused, saying where. Git allows a branch one place.
/// 2. Here and not checked out — left exactly as it is, however far behind.
/// 3. In this project's remote and not here — fetched, and a local branch made that tracks it.
/// 4. In a fork — `pr/<number>`, made at the pull request's head and set to follow it.
pub fn prepare_branch(git: &Git, root: &Path, pr: &PullRequest) -> IpcResult<PreparedBranch> {
    let fork = pr
        .details
        .as_ref()
        .is_some_and(|details| details.cross_repository);
    let remote = remote_of(git, root, &pr.url)?.ok_or_else(|| {
        IpcError::new(
            "no_remote",
            "This project has no remote to fetch the pull request from.",
        )
    })?;

    if fork {
        let branch = format!("pr/{}", pr.number);
        not_checked_out(git, root, &branch)?;
        let head = git.fetch_pull_request(root, &remote, pr.number, None)?;
        let behind = if git.branch_exists(root, &branch)? {
            git.commits_between(root, &branch, &head)?
        } else {
            git.branch_create(root, &branch, &head, false)?;
            git.follow_pull_request(root, &branch, &remote, pr.number)?;
            0
        };
        return Ok(PreparedBranch {
            branch,
            behind,
            fork: true,
        });
    }

    let branch = pr.branch.clone();
    // The name is the forge's, which is to say whoever opened the pull request chose it.
    if !git.valid_branch_name(root, &branch)? {
        return Err(IpcError::new(
            "invalid_branch",
            format!("\"{branch}\" is not a branch name git will take."),
        ));
    }
    not_checked_out(git, root, &branch)?;
    git.fetch_branch(root, &remote, &branch)?;
    let tracking = format!("refs/remotes/{remote}/{branch}");
    let behind = if git.branch_exists(root, &branch)? {
        git.commits_between(root, &branch, &tracking)?
    } else {
        git.branch_create(root, &branch, &tracking, true)?;
        0
    };
    Ok(PreparedBranch {
        branch,
        behind,
        fork: false,
    })
}

/// How many commits the pull request has that `branch` does not, fetched fresh. For a branch a
/// workspace already has checked out: only remote-tracking refs and Yardsort's own pull request
/// refs are written, so the branch and the worktree are left exactly as they are.
pub fn behind(git: &Git, root: &Path, pr: &PullRequest, branch: &str) -> IpcResult<u32> {
    let remote = remote_of(git, root, &pr.url)?.ok_or_else(|| {
        IpcError::new(
            "no_remote",
            "This project has no remote to fetch the pull request from.",
        )
    })?;
    let fork = pr
        .details
        .as_ref()
        .is_some_and(|details| details.cross_repository);
    let head = if fork {
        git.fetch_pull_request(root, &remote, pr.number, None)?
    } else {
        git.fetch_branch(root, &remote, &pr.branch)?;
        format!("refs/remotes/{remote}/{}", pr.branch)
    };
    Ok(git.commits_between(root, branch, &head)?)
}

fn not_checked_out(git: &Git, root: &Path, branch: &str) -> IpcResult<()> {
    let holder = git
        .worktrees(root)?
        .into_iter()
        .find(|entry| !entry.prunable && entry.branch.as_deref() == Some(branch));
    match holder {
        Some(entry) => Err(IpcError::new(
            "branch_checked_out",
            format!(
                "\"{branch}\" is already checked out at {}.",
                entry.path.display()
            ),
        )),
        None => Ok(()),
    }
}

/// The remote the pull request's repository is: the one whose URL names the same owner and
/// repository as the pull request's own. In a clone of a fork that is `upstream`, not `origin`
/// — `gh` lists the parent's pull requests there, and only the parent has their branches. When
/// no remote can be matched (a URL this cannot read), the one a push would go to.
pub fn remote_of(git: &Git, root: &Path, pull_request_url: &str) -> IpcResult<Option<String>> {
    let wanted = pull_request_url
        .split_once("/pull/")
        .and_then(|(repository, _)| parse_remote(repository));
    if let Some(wanted) = wanted {
        for remote in git.remotes(root)? {
            let here = git
                .remote_url(root, &remote)?
                .as_deref()
                .and_then(parse_remote);
            if here.is_some_and(|here| {
                here.owner.eq_ignore_ascii_case(&wanted.owner)
                    && here.name.eq_ignore_ascii_case(&wanted.name)
            }) {
                return Ok(Some(remote));
            }
        }
    }
    Ok(git.push_remote(root)?)
}
