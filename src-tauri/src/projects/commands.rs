//! Project and UI-state commands.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tauri::AppHandle;
use yardsort_core::repositories::{
    parse_link, CloneProtocol, RemoteRepository, Repositories, ResolvedLink,
};

use super::{AddedProject, Project, Projects};
use crate::error::{IpcError, IpcResult};
use crate::forge::{self, ForgeError, Gh, Repo};
use crate::git::Git;
use crate::publish::commands::failed as forge_failed;
use crate::state::{blocking, AppState};

fn with_projects<T>(
    state: &AppState,
    f: impl FnOnce(&Projects<'_>) -> IpcResult<T>,
) -> IpcResult<T> {
    let git = Git::new(&state.env())?;
    // Catch up with git first: worktrees of ours that Yardsort lost track of — orphaned when
    // their project was removed and added again — become workspaces once more. Worktrees made
    // elsewhere wait to be imported. Failing to look must not block the list.
    let worktree_root = state.worktree_root()?;
    let _one_at_a_time = state
        .reconciling
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for project in state.store.projects()? {
        if let Err(error) =
            crate::workspaces::adopt_unknown(&state.store, &git, &project.id, &worktree_root)
        {
            eprintln!(
                "could not reconcile worktrees of {}: {}",
                project.name, error.message
            );
        }
    }
    f(&Projects {
        store: &state.store,
        git: &git,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn projects_list(app: AppHandle) -> IpcResult<Vec<Project>> {
    blocking(app, |state| {
        with_projects(state, |projects| projects.list())
    })
    .await
}

/// Add the repository containing `path`. Fails with `not_a_git_repo` unless `init_git` is set.
#[tauri::command]
#[specta::specta]
pub async fn project_open(app: AppHandle, path: String, init_git: bool) -> IpcResult<AddedProject> {
    blocking(app, move |state| {
        with_projects(state, |projects| {
            projects.open(&PathBuf::from(path), init_git)
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn project_create(
    app: AppHandle,
    name: String,
    parent: String,
) -> IpcResult<AddedProject> {
    blocking(app, move |state| {
        with_projects(state, |projects| {
            projects.create(&name, &PathBuf::from(parent))
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn project_clone(
    app: AppHandle,
    repository: String,
    name: String,
    parent: String,
    upstream: Option<String>,
) -> IpcResult<AddedProject> {
    blocking(app, move |state| {
        let git = Git::new(&state.env())?;
        Projects {
            store: &state.store,
            git: &git,
        }
        .clone_from(
            &repository,
            &name,
            &PathBuf::from(parent),
            upstream.as_deref(),
        )
    })
    .await
}

/// Each project's remotes, read as repositories on a forge and in `gh`'s order, for matching
/// against the forge's answers. A project with no remote, or a folder that has gone, is simply
/// not matched.
fn project_repos(state: &AppState) -> IpcResult<Vec<(String, Vec<Repo>)>> {
    let git = Git::new(&state.env())?;
    Ok(state
        .store
        .projects()?
        .into_iter()
        .map(|project| {
            let repos = forge::repos_at(&git, Path::new(&project.root_path));
            (project.id, repos)
        })
        .filter(|(_, repos)| !repos.is_empty())
        .collect())
}

/// A pasted link to an issue or a pull request, settled: which project has its repository,
/// and whether it is an issue or a pull request — asked of GitHub for `owner/repo#12`, which
/// does not say.
#[tauri::command]
#[specta::specta]
pub async fn forge_resolve_link(app: AppHandle, text: String) -> IpcResult<ResolvedLink> {
    blocking(app, move |state| {
        let link = parse_link(&text).ok_or_else(|| {
            IpcError::new(
                "invalid_link",
                "Paste a link to an issue or a pull request, or owner/repository#number on GitHub.",
            )
        })?;
        let kind = match link.kind {
            Some(_) => None,
            None => {
                let (gh, cwd, _) = gh_for_the_account(state)?;
                Some(
                    gh.link_kind(&cwd, &link.repo, link.number, Duration::from_secs(20))
                        .map_err(forge_failed)?,
                )
            }
        };
        let projects = project_repos(state)?;
        Ok(link.resolve(
            projects
                .iter()
                .map(|(id, repos)| (id.as_str(), repos.as_slice())),
            kind,
        ))
    })
    .await
}

/// `gh` is asked from the profile's folder: these questions name no repository, so no checkout
/// is needed, and a project's folder may have gone.
fn gh_for_the_account(state: &AppState) -> IpcResult<(Gh, PathBuf, CloneProtocol)> {
    let gh = Gh::find(&state.env()).ok_or_else(|| forge_failed(ForgeError::NotInstalled))?;
    let cwd = state.data_dir.clone();
    let protocol = gh.git_protocol(&cwd);
    Ok((gh, cwd, protocol))
}

/// The repositories the `gh` account owns or collaborates on, most recently pushed first, up to
/// 200, with the ones that are already projects marked. Never an error once `gh` is there: a
/// page that fails keeps the pages before it and says why in `problem`.
#[tauri::command]
#[specta::specta]
pub async fn forge_repositories(app: AppHandle) -> IpcResult<Repositories> {
    blocking(app, move |state| {
        let (gh, cwd, protocol) = gh_for_the_account(state)?;
        let mut found = gh.repositories(&cwd, protocol, 200, Duration::from_secs(20));
        let projects = project_repos(state)?;
        found.mark_projects(
            projects
                .iter()
                .map(|(id, repos)| (id.as_str(), repos.as_slice())),
        );
        Ok(found)
    })
    .await
}

/// Repositories on the forge whose name contains `text` — the account's, its organisations',
/// anyone's public ones — the twenty the forge ranks first.
#[tauri::command]
#[specta::specta]
pub async fn forge_search_repositories(
    app: AppHandle,
    text: String,
) -> IpcResult<Vec<RemoteRepository>> {
    blocking(app, move |state| {
        let (gh, cwd, protocol) = gh_for_the_account(state)?;
        let mut found = gh
            .search_repositories(&cwd, &text, protocol, Duration::from_secs(20))
            .map_err(forge_failed)?;
        let projects = project_repos(state)?;
        let mut marked = Repositories {
            repositories: std::mem::take(&mut found),
            ..Default::default()
        };
        marked.mark_projects(
            projects
                .iter()
                .map(|(id, repos)| (id.as_str(), repos.as_slice())),
        );
        Ok(marked.repositories)
    })
    .await
}

/// Take a project off the list. With `keep_history` its workspaces and their conversations
/// wait for the folder to be opened again. Files on disk are never touched.
#[tauri::command]
#[specta::specta]
pub async fn project_remove(app: AppHandle, id: String, keep_history: bool) -> IpcResult<()> {
    blocking(app, move |state| {
        Ok(state.store.remove_project(&id, keep_history).map(drop)?)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn projects_reorder(app: AppHandle, ordered_ids: Vec<String>) -> IpcResult<()> {
    blocking(app, move |state| {
        Ok(state.store.reorder_projects(&ordered_ids)?)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn ui_state_load(app: AppHandle) -> IpcResult<BTreeMap<String, String>> {
    blocking(app, |state| Ok(state.store.ui_state()?)).await
}

#[tauri::command]
#[specta::specta]
pub async fn ui_state_save(app: AppHandle, key: String, value: String) -> IpcResult<()> {
    blocking(app, move |state| {
        Ok(state.store.set_ui_state(&key, &value)?)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn project_automation_get(
    app: AppHandle,
    project_id: String,
) -> IpcResult<yardsort_core::project_automation::ProjectAutomation> {
    blocking(app, move |state| {
        yardsort_core::project_automation::ProjectAutomation::load(&state.store, &project_id)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn project_automation_save(
    app: AppHandle,
    project_id: String,
    config: yardsort_core::project_automation::ProjectAutomation,
) -> IpcResult<()> {
    blocking(app, move |state| config.save(&state.store, &project_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn workspace_run(
    app: AppHandle,
    workspace_id: String,
    size: pty_host::TermSize,
) -> IpcResult<pty_host::SessionInfo> {
    blocking(app, move |state| {
        // Serialize the lookup/spawn pair so double clicks cannot start duplicate servers.
        let _guard = state
            .reconciling
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::terminal::with_launcher(state, |launcher| launcher.project_run(&workspace_id, size))
    })
    .await
}
