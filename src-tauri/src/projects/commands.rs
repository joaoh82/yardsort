//! Project and UI-state commands.

use std::collections::BTreeMap;
use std::path::PathBuf;

use tauri::AppHandle;

use super::{AddedProject, Project, Projects};
use crate::error::IpcResult;
use crate::git::Git;
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

/// Forget a project. Files on disk are never touched.
#[tauri::command]
#[specta::specta]
pub async fn project_remove(app: AppHandle, id: String) -> IpcResult<()> {
    blocking(app, move |state| {
        Ok(state.store.remove_project(&id).map(drop)?)
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
