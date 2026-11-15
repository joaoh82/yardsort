//! Sync, in the window: a project's **Sync** in Project settings, turning it on after a preview,
//! linking it to a project another computer syncs, stopping and removing. The loop and its rules
//! are `remote_host::sync` and `yardsort_core::sync`, shared with a headless host. Only these
//! commands change sync; `ys` only reads it.

use std::path::Path;

use remote_host::sync;
use remote_proto::sync::SyncId;
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;
use yardsort_core::sync::{
    approved_notes, project_hint, project_sync, same_remote, synced_workspace_count, ProjectSync,
    SyncPreview, WorkspaceTask,
};

use super::{ipc, store_failed, AppHome};
use crate::error::{IpcError, IpcResult};
use crate::state::{blocking, AppState};

/// Something sync shows changed: refresh what shows it.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
pub struct SyncChanged {}

/// How a project stands for sync, and how the last pass went.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSyncStatus {
    pub sync: ProjectSync,
    /// When a pass last went through, in milliseconds.
    #[specta(type = Option<f64>)]
    pub synced_at: Option<i64>,
    /// Why the last pass did not.
    pub problem: Option<String>,
}

/// A project another computer syncs, offered for linking.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncOffer {
    pub id: String,
    pub name: String,
    pub remote: Option<String>,
    /// Its remote is this project's.
    pub matches: bool,
}

/// A workspace on another computer of the account, of a project that syncs here. Shown, never
/// made here: nothing of it is on this disk.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncedWorkspace {
    pub id: String,
    /// The computer it lives on, by name.
    pub host: String,
    pub name: String,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub archived: bool,
    /// The tasks it was started from.
    pub tasks: Vec<WorkspaceTask>,
}

fn hint(state: &AppState, root: &str) -> Option<String> {
    let git = yardsort_core::git::Git::new(&state.env()).ok()?;
    project_hint(&git, Path::new(root))
}

fn status(state: &AppState, project_id: &str) -> IpcResult<ProjectSyncStatus> {
    let store = &state.store;
    let project = store
        .project(project_id)
        .map_err(store_failed)?
        .ok_or_else(|| IpcError::new("sync_refused", "There is no such project."))?;
    let remote_on = store.remote_host().map_err(store_failed)?.is_some();
    let notes = approved_notes(store, project_id).map_err(store_failed)?;
    let workspaces = synced_workspace_count(store, project_id).map_err(store_failed)?;
    let sync = project_sync(store, project_id, remote_on, || SyncPreview {
        remote: hint(state, &project.root_path),
        name: project.name.clone(),
        notes,
        workspaces,
    })
    .map_err(store_failed)?;
    let pass = store.sync_state().map_err(store_failed)?;
    Ok(ProjectSyncStatus {
        sync,
        synced_at: pass.synced_at,
        problem: pass.problem,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn project_sync_get(app: AppHandle, project_id: String) -> IpcResult<ProjectSyncStatus> {
    blocking(app, move |state| status(state, &project_id)).await
}

/// Turn sync on for a project: its name and remote hint go up on the next pass.
#[tauri::command]
#[specta::specta]
pub async fn project_sync_on(app: AppHandle, project_id: String) -> IpcResult<ProjectSyncStatus> {
    let handle = app.clone();
    blocking(app, move |state| {
        let root = state
            .store
            .project(&project_id)
            .map_err(store_failed)?
            .ok_or_else(|| IpcError::new("sync_refused", "There is no such project."))?
            .root_path;
        sync::turn_on(&AppHome::new(&handle), &project_id, hint(state, &root)).map_err(ipc)?;
        status(state, &project_id)
    })
    .await
}

/// The synced projects another computer of the account has, that this one could link
/// `project_id` to. Those whose remote is this project's come first.
#[tauri::command]
#[specta::specta]
pub async fn project_sync_offers(app: AppHandle, project_id: String) -> IpcResult<Vec<SyncOffer>> {
    let offers = sync::offered(&AppHome::new(&app)).await.map_err(ipc)?;
    blocking(app, move |state| {
        let root = state
            .store
            .project(&project_id)
            .map_err(store_failed)?
            .ok_or_else(|| IpcError::new("sync_refused", "There is no such project."))?
            .root_path;
        let ours = hint(state, &root);
        let mut offers: Vec<SyncOffer> = offers
            .into_iter()
            .map(|offer| SyncOffer {
                matches: match (&ours, &offer.remote) {
                    (Some(ours), Some(theirs)) => same_remote(ours, theirs),
                    _ => false,
                },
                id: offer.id.as_str().to_owned(),
                name: offer.name,
                remote: offer.remote,
            })
            .collect();
        offers.sort_by(|a, b| b.matches.cmp(&a.matches).then(a.name.cmp(&b.name)));
        Ok(offers)
    })
    .await
}

/// Link a project here to the synced one the person chose.
#[tauri::command]
#[specta::specta]
pub async fn project_sync_link(
    app: AppHandle,
    project_id: String,
    synced: String,
) -> IpcResult<ProjectSyncStatus> {
    let synced = SyncId::new(synced)
        .map_err(|_| IpcError::new("sync_refused", "That is not a synced project's id."))?;
    sync::link(&AppHome::new(&app), &project_id, &synced)
        .await
        .map_err(ipc)?;
    blocking(app, move |state| status(state, &project_id)).await
}

/// Stop syncing a project here. Its records stay on the server for the account's other hosts.
#[tauri::command]
#[specta::specta]
pub async fn project_sync_stop(app: AppHandle, project_id: String) -> IpcResult<ProjectSyncStatus> {
    sync::stop(&AppHome::new(&app), &project_id)
        .await
        .map_err(ipc)?;
    blocking(app, move |state| status(state, &project_id)).await
}

/// Remove a project's records from the server, for every host. Each keeps its own copy.
#[tauri::command]
#[specta::specta]
pub async fn project_sync_remove(
    app: AppHandle,
    project_id: String,
) -> IpcResult<ProjectSyncStatus> {
    let handle = app.clone();
    blocking(app, move |state| {
        sync::remove(&AppHome::new(&handle), &project_id).map_err(ipc)?;
        status(state, &project_id)
    })
    .await
}

/// The other computers' workspaces of a project that syncs here.
#[tauri::command]
#[specta::specta]
pub async fn project_sync_workspaces(
    app: AppHandle,
    project_id: String,
) -> IpcResult<Vec<SyncedWorkspace>> {
    blocking(app, move |state| {
        Ok(state
            .store
            .synced_workspaces(&project_id)
            .map_err(store_failed)?
            .into_iter()
            .map(|w| SyncedWorkspace {
                id: w.id,
                host: w.host,
                name: w.name,
                branch: w.branch,
                base_branch: w.base_branch,
                archived: w.archived,
                tasks: w.tasks,
            })
            .collect())
    })
    .await
}
