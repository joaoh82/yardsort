//! Commit, push and open a pull request; and what the forge says about one afterwards.

use std::path::Path;

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use super::{state, ProjectPullRequests, PublishState};
use crate::changes::commands::workspace;
use crate::error::{IpcError, IpcResult};
use crate::forge::{ForgeError, Gh};
use crate::git::Git;
use crate::state::{blocking, AppState};
use crate::store::WorkspaceRow;

fn failed(error: ForgeError) -> IpcError {
    IpcError::new(error.code(), error.to_string())
}

/// The project's checkout, which is where `gh` is asked: one answer serves every workspace, and
/// a worktree whose folder has gone cannot be asked at all.
fn project_root(state: &AppState, project_id: &str) -> IpcResult<std::path::PathBuf> {
    let project = state
        .store
        .project(project_id)?
        .ok_or_else(|| IpcError::new("unknown_project", "That project is no longer open."))?;
    Ok(std::path::PathBuf::from(project.root_path))
}

fn found(state: &AppState, project_id: &str, refresh: bool) -> ProjectPullRequests {
    let Ok(root) = project_root(state, project_id) else {
        return ProjectPullRequests::default();
    };
    let gh = Gh::find(&state.env());
    state
        .forge
        .pull_requests(gh.as_ref(), &root, project_id, refresh)
}

fn publish_state(
    state: &AppState,
    row: &WorkspaceRow,
    root: &Path,
    refresh: bool,
) -> IpcResult<PublishState> {
    let git = Git::new(&state.env())?;
    let found = found(state, &row.project_id, refresh);
    super::state(&git, root, row, &found)
}

/// Where the selected workspace stands with its remote: what it can commit, push and open.
#[tauri::command]
#[specta::specta]
pub async fn workspace_publish_state(
    app: AppHandle,
    workspace_id: String,
    refresh: bool,
) -> IpcResult<PublishState> {
    blocking(app, move |state| {
        let (row, root) = workspace(state, &workspace_id)?;
        publish_state(state, &row, &root, refresh)
    })
    .await
}

/// Every pull request `gh` knows for a project, so each workspace row can show its own.
#[tauri::command]
#[specta::specta]
pub async fn project_pull_requests(
    app: AppHandle,
    project_id: String,
    refresh: bool,
) -> IpcResult<ProjectPullRequests> {
    blocking(app, move |state| Ok(found(state, &project_id, refresh))).await
}

/// Commit everything the workspace has changed.
///
/// Everything, because the panel offers no way to leave a file out — see
/// [`Git::commit_all`](crate::git::Git::commit_all). Nothing is staged until git has an identity
/// to commit with, so the common first failure does not leave a half-staged tree behind.
#[tauri::command]
#[specta::specta]
pub async fn workspace_commit(
    app: AppHandle,
    workspace_id: String,
    message: String,
) -> IpcResult<PublishState> {
    let message = message.trim().to_owned();
    if message.is_empty() {
        return Err(IpcError::new(
            "empty_commit_message",
            "Write a commit message first.",
        ));
    }
    blocking(app, move |state| {
        let (row, root) = workspace(state, &workspace_id)?;
        let git = Git::new(&state.env())?;
        if git.identity(&root)?.is_none() {
            return Err(IpcError::new(
                "no_git_identity",
                "git has no name and email to commit with. Set them with \
                 `git config --global user.name` and `user.email`.",
            ));
        }
        git.commit_all(&root, &message)?;
        publish_state(state, &row, &root, false)
    })
    .await
}

/// Push the workspace's branch, setting its upstream the first time.
#[tauri::command]
#[specta::specta]
pub async fn workspace_push(app: AppHandle, workspace_id: String) -> IpcResult<PublishState> {
    blocking(app, move |state| {
        let (row, root) = workspace(state, &workspace_id)?;
        let git = Git::new(&state.env())?;
        push(&git, &root, &publishable(&git, &root, &row)?)?;
        // The remote moved, so what gh last said about this project is out of date.
        state.forge.forget(&row.project_id);
        publish_state(state, &row, &root, true)
    })
    .await
}

/// A pull request that now exists, or the form to fill in to make one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestOpened {
    pub url: String,
    /// `gh` opened it. When false the URL is the forge's own form, to finish in a browser.
    pub created: bool,
}

/// Open a pull request for the workspace's branch, pushing first if it needs it.
///
/// Pushing first because the alternative is a button that fails and tells you to press another
/// one: a branch the forge has never seen cannot have a pull request. With `gh` this ends on the
/// pull request; without it, on the forge's form with both branches already filled in.
#[tauri::command]
#[specta::specta]
pub async fn workspace_open_pull_request(
    app: AppHandle,
    workspace_id: String,
    title: String,
    body: String,
    draft: bool,
) -> IpcResult<PullRequestOpened> {
    let title = title.trim().to_owned();
    blocking(app, move |state| {
        let (row, root) = workspace(state, &workspace_id)?;
        let git = Git::new(&state.env())?;
        let publishable = publishable(&git, &root, &row)?;
        let base = publishable.base.clone().ok_or_else(|| {
            IpcError::new(
                "no_base_branch",
                "This workspace is on the branch it would merge into.",
            )
        })?;

        if publishable.upstream.is_none() || publishable.ahead > 0 {
            push(&git, &root, &publishable)?;
        }
        state.forge.forget(&row.project_id);

        let Some(gh) = Gh::find(&state.env()) else {
            return fall_back(&publishable);
        };
        if title.is_empty() {
            return Err(IpcError::new(
                "empty_pull_request_title",
                "Give the pull request a title first.",
            ));
        }
        match gh.create_pull_request(&root, &base, &title, &body, draft) {
            Ok(url) => Ok(PullRequestOpened { url, created: true }),
            // Not logged in is not a failure: it is exactly the case the link is there for.
            Err(error) if error.is_logged_out() => fall_back(&publishable),
            Err(error) => Err(failed(error)),
        }
    })
    .await
}

/// The forge's own form, for when `gh` cannot or will not.
fn fall_back(publishable: &PublishState) -> IpcResult<PullRequestOpened> {
    publishable
        .compare_url()
        .map(|url| PullRequestOpened {
            url,
            created: false,
        })
        .ok_or_else(|| {
            IpcError::new(
                "no_forge",
                "This project's remote is not a forge Yardsort can open a pull request on.",
            )
        })
}

fn push(git: &Git, root: &Path, publishable: &PublishState) -> IpcResult<()> {
    let (Some(remote), Some(branch)) = (&publishable.remote, &publishable.branch) else {
        return Err(IpcError::new(
            "nothing_to_push",
            match publishable.branch {
                Some(_) => "This project has no remote to push to.",
                None => "This workspace is not on a branch.",
            },
        ));
    };
    git.push(root, remote, branch)?;
    Ok(())
}

/// What a push or a pull request is decided from — read without asking the forge, because
/// neither decision needs it and both would pay for the round trip.
fn publishable(git: &Git, root: &Path, row: &WorkspaceRow) -> IpcResult<PublishState> {
    state(git, root, row, &ProjectPullRequests::default())
}
