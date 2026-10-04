//! Commit, push and open a pull request; and what the forge says about one afterwards.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use super::{pull_requests_of, state, ProjectPullRequests, PublishState};
use crate::changes::commands::workspace;
use crate::error::{IpcError, IpcResult};
use crate::forge::{ForgeError, Gh, MergeMethod, PullRequest, PullRequestState, Repo};
use crate::git::Git;
use crate::state::{blocking, AppState};
use crate::store::WorkspaceRow;

pub(super) fn failed(error: ForgeError) -> IpcError {
    IpcError::new(error.code(), error.to_string())
}

/// The project's checkout, which is where `gh` is asked: one answer serves every workspace, and
/// a worktree whose folder has gone cannot be asked at all.
pub(super) fn project_root(state: &AppState, project_id: &str) -> IpcResult<std::path::PathBuf> {
    let project = state
        .store
        .project(project_id)?
        .ok_or_else(|| IpcError::new("unknown_project", "That project is no longer open."))?;
    Ok(std::path::PathBuf::from(project.root_path))
}

pub(super) fn found(state: &AppState, project_id: &str, refresh: bool) -> ProjectPullRequests {
    look(state, project_id, refresh, false)
}

/// What the forge says about a project's pull requests. `full` is the Pull requests view
/// asking: every open one, not only the newest fifty.
fn look(state: &AppState, project_id: &str, refresh: bool, full: bool) -> ProjectPullRequests {
    let Ok(root) = project_root(state, project_id) else {
        return ProjectPullRequests::default();
    };
    let gh = Gh::find(&state.env());
    state
        .forge
        .pull_requests(gh.as_ref(), &root, project_id, refresh, full, &|| {
            repo_at(state, &root)
        })
}

/// The project's remote as a repository on a forge: the one a push would go to.
pub(super) fn repo_at(state: &AppState, root: &Path) -> Option<Repo> {
    crate::forge::repo_at(&Git::new(&state.env()).ok()?, root)
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
///
/// `full` reads every open pull request as well as the newest fifty: what the Pull requests
/// view lists. It is several questions to the forge on a busy repository, so only that view
/// asks for it.
#[tauri::command]
#[specta::specta]
pub async fn project_pull_requests(
    app: AppHandle,
    project_id: String,
    refresh: bool,
    full: bool,
) -> IpcResult<ProjectPullRequests> {
    blocking(app, move |state| {
        let mut found = look(state, &project_id, refresh, full);
        // The whole list is in hand: the moment to let go of what was fetched for pull requests
        // that are no longer on it.
        if full {
            if let (Ok(git), Ok(root)) = (Git::new(&state.env()), project_root(state, &project_id))
            {
                super::pull_requests::prune_refs(&git, &root, &found);
            }
        }
        let mut owned = state
            .forge
            .owned(&project_id, &found.pull_requests, refresh, || {
                by_workspace(state, &project_id, &found.pull_requests)
            });
        // A full look has just read every open one. Otherwise the ones workspaces show that
        // only the open tier has are asked about, or they would stay as they were at startup.
        if refresh && !full {
            let shown: Vec<u32> = owned.values().flatten().copied().collect();
            let followed = match (project_root(state, &project_id), Gh::find(&state.env())) {
                (Ok(root), Some(gh)) => state.forge.follow_open(&project_id, &shown, |number| {
                    gh.pull_request_within(&root, number, super::PAGE_LIMIT)
                }),
                _ => false,
            };
            if followed {
                // The recent tier is fresh in the cache, so this asks nobody.
                found = look(state, &project_id, false, false);
                owned = state
                    .forge
                    .owned(&project_id, &found.pull_requests, false, || {
                        by_workspace(state, &project_id, &found.pull_requests)
                    });
            }
        }
        found.workspaces = owned;
        Ok(found)
    })
    .await
}

/// Each workspace's own pull requests out of the project's. One `git reflog` per workspace, and
/// none at all when the project has no pull requests to share out.
fn by_workspace(
    state: &AppState,
    project_id: &str,
    requests: &[PullRequest],
) -> BTreeMap<String, Vec<u32>> {
    let mut found = BTreeMap::new();
    if requests.is_empty() {
        return found;
    }
    let (Ok(git), Ok(rows)) = (Git::new(&state.env()), state.store.workspaces()) else {
        return found;
    };
    for row in rows
        .into_iter()
        .filter(|row| row.project_id == project_id && !row.archived)
    {
        let root = Path::new(&row.path);
        if !root.is_dir() {
            continue;
        }
        if let Ok(numbers) = pull_requests_of(&git, root, &row, requests) {
            found.insert(row.id, numbers);
        }
    }
    found
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

pub(super) fn push(git: &Git, root: &Path, publishable: &PublishState) -> IpcResult<()> {
    if let Some(number) = publishable.follows_pull_request {
        return Err(IpcError::new(
            "follows_pull_request",
            format!(
                "This branch is a checkout of pull request #{number}, which comes from a fork. \
                 Yardsort does not push to forks: a push from here would make a new branch on \
                 this project's remote instead."
            ),
        ));
    }
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

/// Merge a confirmed PR only if it is still open, still one of this workspace's, and still at
/// the head the user confirmed.
#[tauri::command]
#[specta::specta]
pub async fn workspace_merge_pull_request(
    app: AppHandle,
    workspace_id: String,
    number: u32,
    head_oid: String,
    method: MergeMethod,
) -> IpcResult<()> {
    blocking(app, move |state| {
        let (row, root) = workspace(state, &workspace_id)?;
        let found = found(state, &row.project_id, true);
        let git = Git::new(&state.env())?;
        let own = pull_requests_of(&git, &root, &row, &found.pull_requests)?;
        let pr = found
            .pull_requests
            .iter()
            .filter(|pr| own.contains(&pr.number))
            .find(|pr| pr.number == number && still_mergeable(pr, &head_oid))
            .ok_or_else(pull_request_changed)?;
        let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
        // Use the same repository context as the project-wide query.
        let result = gh.merge_pull_request(
            &project_root(state, &row.project_id)?,
            pr.number,
            &head_oid,
            method,
        );
        state.forge.forget(&row.project_id);
        result.map_err(failed)
    })
    .await
}

/// Whether `pr` is still what the user confirmed merging: open, not a draft, and at the head
/// commit the confirmation showed.
pub(super) fn still_mergeable(pr: &PullRequest, head_oid: &str) -> bool {
    pr.state == PullRequestState::Open
        && !pr.draft
        && !head_oid.is_empty()
        && pr
            .details
            .as_ref()
            .is_some_and(|details| details.head_oid == head_oid)
}

pub(super) fn pull_request_changed() -> IpcError {
    IpcError::new(
        "pull_request_changed",
        "The pull request changed or is no longer ready to merge. Refresh and review it again.",
    )
}
