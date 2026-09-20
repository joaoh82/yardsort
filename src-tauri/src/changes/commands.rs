//! Right-panel commands: changes, diffs, the file tree, the watcher, "open in editor".

use std::path::{Path, PathBuf};
use std::sync::PoisonError;

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;
use tauri_specta::Event;

use super::files::{list_dir, FileEntry};
use super::watch::WorkspaceWatcher;
use super::{read_working_file, resolve_inside, ChangeSet, Changes, Content, FileDiff, Scope};
use crate::error::{IpcError, IpcResult};
use crate::git::Git;
use crate::state::{blocking, AppState};
use crate::store::WorkspaceRow;

/// Something changed on disk in the watched workspace; ask again.
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFilesChanged {
    pub workspace_id: String,
}

pub(crate) fn workspace(state: &AppState, id: &str) -> IpcResult<(WorkspaceRow, PathBuf)> {
    let row = state
        .store
        .workspace(id)?
        .ok_or_else(|| IpcError::new("unknown_workspace", "That workspace no longer exists."))?;
    let root = PathBuf::from(&row.path);
    if !root.is_dir() {
        return Err(IpcError::new(
            "workspace_missing",
            format!("{} does not exist any more.", row.path),
        ));
    }
    Ok((row, root))
}

#[tauri::command]
#[specta::specta]
pub async fn workspace_changes(app: AppHandle, workspace_id: String) -> IpcResult<ChangeSet> {
    blocking(app, move |state| {
        let (row, root) = workspace(state, &workspace_id)?;
        let git = Git::new(&state.env())?;
        Changes {
            git: &git,
            root: &root,
            base_branch: row.base_branch.as_deref(),
        }
        .list()
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn workspace_diff(
    app: AppHandle,
    workspace_id: String,
    path: String,
    old_path: Option<String>,
    scope: Scope,
) -> IpcResult<FileDiff> {
    blocking(app, move |state| {
        let (row, root) = workspace(state, &workspace_id)?;
        let git = Git::new(&state.env())?;
        Changes {
            git: &git,
            root: &root,
            base_branch: row.base_branch.as_deref(),
        }
        .diff(&path, old_path.as_deref(), scope)
    })
    .await
}

/// One folder of the workspace's file tree (`dir` is relative; empty for the root). With
/// `show_ignored`, `.git` and ignored entries are included and flagged.
#[tauri::command]
#[specta::specta]
pub async fn workspace_files(
    app: AppHandle,
    workspace_id: String,
    dir: String,
    show_ignored: bool,
) -> IpcResult<Vec<FileEntry>> {
    blocking(app, move |state| {
        list_dir(&workspace(state, &workspace_id)?.1, &dir, show_ignored)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn workspace_file(
    app: AppHandle,
    workspace_id: String,
    path: String,
) -> IpcResult<Content> {
    blocking(app, move |state| {
        read_working_file(&workspace(state, &workspace_id)?.1, &path)
    })
    .await
}

/// Watch this workspace's files (replacing any previous watch); `None` stops watching.
#[tauri::command]
#[specta::specta]
pub async fn workspace_watch(app: AppHandle, workspace_id: Option<String>) -> IpcResult<()> {
    let handle = app.clone();
    blocking(app, move |state| {
        let mut slot = state.watcher.lock().unwrap_or_else(PoisonError::into_inner);
        // Stop the old watch first: two watchers must never signal for different workspaces.
        *slot = None;
        let Some(id) = workspace_id else {
            return Ok(());
        };
        let (_, root) = workspace(state, &id)?;
        let root = crate::git::normalize(&root);
        let git_dir = Git::new(&state.env())
            .and_then(|git| git.run(&root, &["rev-parse", "--absolute-git-dir"]))
            .ok()
            .map(|dir| crate::git::normalize(Path::new(&dir)));

        let watcher = WorkspaceWatcher::start(&root, git_dir.as_deref(), move || {
            let _ = WorkspaceFilesChanged {
                workspace_id: id.clone(),
            }
            .emit(&handle);
        })
        .map_err(|error| {
            IpcError::new(
                "watch_failed",
                format!("Cannot watch for file changes: {error}"),
            )
        })?;
        *slot = Some(watcher);
        Ok(())
    })
    .await
}

/// Open a file (or the workspace folder, when `path` is `None`) in the user's editor.
#[tauri::command]
#[specta::specta]
pub async fn open_in_editor(
    app: AppHandle,
    workspace_id: String,
    path: Option<String>,
) -> IpcResult<()> {
    blocking(app, move |state| {
        let (_, root) = workspace(state, &workspace_id)?;
        let target = match &path {
            Some(path) => resolve_inside(&root, path)?,
            None => root.clone(),
        };
        let env = state.env();
        let configured = state.settings.get().general.editor_command;
        let editor = configured
            .iter()
            .map(String::as_str)
            .chain(EDITORS.iter().copied())
            .find_map(|command| {
                let mut words = command.split_whitespace();
                let program = env.find_program(words.next()?, &root)?;
                Some((program, words.map(str::to_owned).collect::<Vec<_>>()))
            });
        let Some((program, args)) = editor else {
            return Err(IpcError::new(
                "no_editor",
                "No editor found. Set one in Settings → General (for example \"code\" or \"zed\").",
            ));
        };
        let mut command = std::process::Command::new(program);
        if env.replaces_inherited() {
            command.env_clear();
        }
        command
            .args(args)
            // Editors take the folder first so the file opens in that project's window.
            .arg(&root)
            .args(path.is_some().then_some(&target))
            .current_dir(&root)
            .envs(env.vars.iter())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(drop)
            .map_err(|error| {
                IpcError::new(
                    "editor_failed",
                    format!("Could not start the editor: {error}"),
                )
            })
    })
    .await
}

/// Tried in order when no editor is configured.
const EDITORS: &[&str] = &["cursor", "code", "zed", "windsurf", "subl", "idea"];
