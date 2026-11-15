//! Checking out here a workspace another computer has: its branch, fetched from the project's
//! remote with Git, and a workspace of this computer's own on it, made as `open_branch` makes
//! any. The other computer's record is untouched, and nothing here is made unless the person
//! asks. See [42](../../../../docs/design/42-remote-r6.md) §13.
//!
//! The branch comes only from the remote, never from the server, which has no commits: a branch
//! the other computer has not pushed cannot be checked out. A branch of that name here is kept
//! as it is when it has a commit the remote's has not, since moving it would lose that commit;
//! one that is only behind is moved up to the remote's.

use std::path::Path;

use crate::error::{IpcError, IpcResult};
use crate::forge::checkout::not_checked_out;
use crate::git::Git;
use crate::store::WorkspaceRow;
use crate::workspaces::Workspaces;

/// What became of the branch here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Branch {
    /// Made, from the remote's, and tracking it.
    Made,
    /// Here already, and moved up to the remote's: it was behind and had nothing of its own.
    MovedUp,
    /// Here already, and at the remote's.
    AsItWas,
}

/// Fetch `branch` from the project's remote and have a local branch of that name at it, for a
/// workspace to be opened on. `host` names the computer it is from, for what is said when it
/// cannot be.
pub fn prepare_branch(git: &Git, root: &Path, branch: &str, host: &str) -> IpcResult<Branch> {
    if !git.valid_branch_name(root, branch)? {
        return Err(IpcError::new(
            "invalid_branch",
            format!("\"{branch}\" is not a branch name git will take."),
        ));
    }
    not_checked_out(git, root, branch)?;
    let remote = git.push_remote(root)?.ok_or_else(|| {
        IpcError::new(
            "no_remote",
            "This project has no remote to fetch the branch from.",
        )
    })?;
    if git.fetch_branch(root, &remote, branch).is_err() {
        return Err(IpcError::new(
            "branch_not_pushed",
            format!(
                "{remote} has no branch \"{branch}\", or cannot be reached. Push it from {host} \
                 first. Nothing here was changed."
            ),
        ));
    }
    let tracking = format!("refs/remotes/{remote}/{branch}");
    if !git.branch_exists(root, branch)? {
        git.branch_create(root, branch, &tracking, true)?;
        return Ok(Branch::Made);
    }
    let own = git.commits_between(root, &tracking, branch)?;
    if own > 0 {
        return Err(IpcError::new(
            "branch_diverged",
            format!(
                "\"{branch}\" here has {own} commit{} that {remote}'s has not. It was left as it is: \
                 bring the two together with Git first, then check it out again.",
                if own == 1 { "" } else { "s" }
            ),
        ));
    }
    if git.commits_between(root, branch, &tracking)? > 0 {
        git.branch_move(root, branch, &tracking)?;
        return Ok(Branch::MovedUp);
    }
    Ok(Branch::AsItWas)
}

/// Check out here the workspace `synced` another computer has of `project_id`: its branch, and a
/// workspace on it. Refused when one here is on that branch already.
pub fn check_out_here(
    workspaces: &Workspaces<'_>,
    project_id: &str,
    synced: &str,
) -> IpcResult<(WorkspaceRow, Branch)> {
    let store = workspaces.store;
    let other = store
        .synced_workspaces(project_id)?
        .into_iter()
        .find(|w| w.id == synced)
        .ok_or_else(|| {
            IpcError::new(
                "unknown_workspace",
                "That workspace is not on your other computers any more.",
            )
        })?;
    let branch = other.branch.ok_or_else(|| {
        IpcError::new(
            "no_branch",
            format!(
                "{} on {} has no branch to check out.",
                other.name, other.host
            ),
        )
    })?;
    if let Some(here) = store.workspaces()?.into_iter().find(|w| {
        w.project_id == project_id && !w.forgotten && w.branch.as_deref() == Some(branch.as_str())
    }) {
        return Err(IpcError::new(
            "branch_here",
            format!("\"{branch}\" is already a workspace here: {}.", here.name),
        ));
    }
    let project = store
        .project(project_id)?
        .ok_or_else(|| IpcError::new("unknown_project", "That project no longer exists."))?;
    let prepared = prepare_branch(
        workspaces.git,
        Path::new(&project.root_path),
        &branch,
        &other.host,
    )?;
    let row = workspaces.open_branch(project_id, &branch)?;
    Ok((row, prepared))
}

#[cfg(test)]
mod tests;
