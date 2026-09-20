//! Workspace and harness commands.

use std::path::PathBuf;

use pty_host::{SessionInfo, TermSize};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;

use super::Workspaces;
use crate::error::{IpcError, IpcResult};
use crate::git::Git;
use crate::harness::{self, HarnessDef};
use crate::projects::{Projects, Workspace};
use crate::state::{blocking, AppState};
use crate::terminal::{spawn_in_workspace, HarnessRequest, Launch};

/// A harness definition plus whether its command can be found on this machine.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessInfo {
    #[serde(flatten)]
    pub def: HarnessDef,
    /// Where `command` resolved to on the user's `PATH`; `None` if it is not installed.
    pub resolved_path: Option<String>,
    /// Ships with Yardsort (as opposed to one the user added).
    pub builtin: bool,
    /// A built-in whose definition the user has changed.
    pub modified: bool,
}

/// The exact command lines a definition produces, for the settings form to show.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessPreview {
    pub resolved_path: Option<String>,
    pub start: Vec<String>,
    pub resume: Vec<String>,
    pub fork: Vec<String>,
    /// Why this definition cannot be saved as it stands, if it cannot.
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSettingsDto {
    /// `None` uses `default_worktree_root`.
    pub worktree_root: Option<String>,
    pub branch_prefix: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SettingsInfo {
    /// The "Open in editor" command; `None` tries the common editors in turn.
    pub editor_command: Option<String>,
    pub notify_when_quiet: bool,
    pub check_for_updates: bool,
    pub workspaces: WorkspaceSettingsDto,
    pub default_worktree_root: String,
    /// Set while `YARDSORT_WORKTREE_ROOT` overrides the setting.
    pub worktree_root_override: Option<String>,
    pub file_path: String,
    /// Why the settings file was ignored, if it was (it is kept, never overwritten).
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BranchList {
    pub branches: Vec<String>,
    /// The branch to offer first: the remote's default, or the one checked out.
    pub default: Option<String>,
    /// Branches checked out in some worktree already. Git allows a branch in one place only, so
    /// these cannot be opened as a workspace.
    pub checked_out: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NewWorkspace {
    pub project_id: String,
    /// `None` starts from the project's default branch.
    pub base_branch: Option<String>,
    /// Open this existing branch instead of creating a new one; `base_branch` is then ignored.
    pub existing_branch: Option<String>,
    pub harness: HarnessRequest,
    pub size: TermSize,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CreatedWorkspace {
    pub workspace: Workspace,
    pub session: SessionInfo,
}

#[tauri::command]
#[specta::specta]
pub async fn harnesses_list(app: AppHandle) -> IpcResult<Vec<HarnessInfo>> {
    blocking(app, |state| Ok(list_harnesses(state))).await
}

pub(crate) fn list_harnesses(state: &AppState) -> Vec<HarnessInfo> {
    let env = state.env();
    let cwd = std::env::current_dir().unwrap_or_default();
    harness::resolve_all(&state.settings.get().harnesses)
        .into_iter()
        .map(|resolved| HarnessInfo {
            resolved_path: env
                .find_program(&resolved.def.command, &cwd)
                .map(|path| path.to_string_lossy().into_owned()),
            def: resolved.def,
            builtin: resolved.builtin,
            modified: resolved.modified,
        })
        .collect()
}

fn save_failed(error: std::io::Error) -> IpcError {
    IpcError::new(
        "settings_write_failed",
        format!("Could not save settings: {error}"),
    )
}

/// Save a harness definition. For a built-in only the differences from the shipped definition
/// are stored; saving one identical to it removes the override.
#[tauri::command]
#[specta::specta]
pub async fn harness_save(app: AppHandle, def: HarnessDef) -> IpcResult<Vec<HarnessInfo>> {
    blocking(app, move |state| {
        harness::validate(&def).map_err(|why| IpcError::new("invalid_harness", why))?;
        let base = harness::builtin()
            .into_iter()
            .find(|builtin| builtin.id == def.id)
            .unwrap_or_else(|| HarnessDef::custom(&def.id));
        let is_builtin = harness::builtin()
            .iter()
            .any(|builtin| builtin.id == def.id);
        let entry = harness::HarnessOverride::between(&base, &def);
        state
            .settings
            .update(|settings| {
                settings.harnesses.retain(|existing| existing.id != def.id);
                // A custom harness needs its entry to exist at all; a built-in only to differ.
                if !is_builtin || !entry.is_empty() {
                    settings.harnesses.push(entry);
                }
            })
            .map_err(save_failed)?;
        Ok(list_harnesses(state))
    })
    .await
}

/// Restore a built-in to its shipped definition, or delete a custom harness.
#[tauri::command]
#[specta::specta]
pub async fn harness_reset(app: AppHandle, id: String) -> IpcResult<Vec<HarnessInfo>> {
    blocking(app, move |state| {
        state
            .settings
            .update(|settings| settings.harnesses.retain(|existing| existing.id != id))
            .map_err(save_failed)?;
        Ok(list_harnesses(state))
    })
    .await
}

/// What would run for this — possibly unsaved — definition, with sample values filled in.
#[tauri::command]
#[specta::specta]
pub async fn harness_preview(app: AppHandle, def: HarnessDef) -> IpcResult<HarnessPreview> {
    blocking(app, move |state| {
        let sample = harness::LaunchValues {
            prompt: Some("Fix the login bug".into()),
            model: Some(
                def.models
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "model-name".into()),
            ),
            effort: def.efforts.first().cloned(),
            session_id: Some("0f8fad5b-d9cb-469f-a165-70867728950e".into()),
            new_session_id: Some("7c9e6679-7425-40de-944b-e07fc1f90ae7".into()),
        };
        let with_command = |args: Vec<String>| {
            std::iter::once(def.command.clone())
                .chain(args)
                .collect::<Vec<_>>()
        };
        let cwd = std::env::current_dir().unwrap_or_default();
        Ok(HarnessPreview {
            resolved_path: state
                .env()
                .find_program(&def.command, &cwd)
                .map(|path| path.to_string_lossy().into_owned()),
            start: with_command(def.start_args(&sample)),
            resume: with_command(def.continue_args(&sample, false)),
            fork: with_command(def.continue_args(&sample, true)),
            problem: harness::validate(&def).err(),
        })
    })
    .await
}

/// Start this — possibly unsaved — definition in the home directory, with no prompt, to see
/// whether it comes up. The caller shows the session and closes it.
#[tauri::command]
#[specta::specta]
pub async fn harness_test(
    app: AppHandle,
    def: HarnessDef,
    size: TermSize,
) -> IpcResult<SessionInfo> {
    blocking(app, move |state| {
        let session_id = (def.session_id_mode == harness::SessionIdMode::Assigned)
            .then(|| uuid::Uuid::new_v4().to_string());
        let args = def.start_args(&harness::LaunchValues {
            session_id,
            ..Default::default()
        });
        let resolved = crate::terminal::ResolvedLaunch {
            program: Some(def.command),
            args,
            labels: Default::default(),
            paste_when_ready: None,
            // A test launch is not a conversation worth remembering.
            record: None,
        };
        crate::terminal::start(state, resolved, None, size)
    })
    .await
}

fn settings_info(state: &AppState) -> IpcResult<SettingsInfo> {
    let settings = state.settings.get();
    let workspaces = settings.workspaces;
    Ok(SettingsInfo {
        notify_when_quiet: settings.general.notify_when_quiet,
        check_for_updates: settings.general.check_for_updates,
        editor_command: settings.general.editor_command,
        workspaces: WorkspaceSettingsDto {
            worktree_root: workspaces.worktree_root,
            branch_prefix: workspaces.branch_prefix,
        },
        default_worktree_root: state
            .default_worktree_root()?
            .to_string_lossy()
            .into_owned(),
        worktree_root_override: crate::legacy::env_var_os("WORKTREE_ROOT")
            .map(|dir| dir.to_string_lossy().into_owned()),
        file_path: state.settings.path().to_string_lossy().into_owned(),
        problem: state.settings.problem(),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn settings_get(app: AppHandle) -> IpcResult<SettingsInfo> {
    blocking(app, settings_info).await
}

#[tauri::command]
#[specta::specta]
pub async fn settings_save_workspaces(
    app: AppHandle,
    workspaces: WorkspaceSettingsDto,
) -> IpcResult<SettingsInfo> {
    blocking(app, move |state| {
        let wanted = crate::settings::WorkspaceSettings {
            worktree_root: workspaces
                .worktree_root
                .map(|root| root.trim().to_owned())
                .filter(|root| !root.is_empty()),
            branch_prefix: workspaces.branch_prefix.trim().to_owned(),
        };
        wanted
            .validate()
            .map_err(|why| IpcError::new("invalid_settings", why))?;
        state
            .settings
            .update(|settings| settings.workspaces = wanted)
            .map_err(save_failed)?;
        settings_info(state)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn project_branches(app: AppHandle, project_id: String) -> IpcResult<BranchList> {
    blocking(app, move |state| {
        let project = state
            .store
            .project(&project_id)?
            .ok_or_else(|| IpcError::new("unknown_project", "That project no longer exists."))?;
        let git = Git::new(&state.env())?;
        let root = PathBuf::from(project.root_path);
        Ok(BranchList {
            branches: git.branches(&root)?,
            default: git.default_branch(&root)?,
            checked_out: git
                .worktrees(&root)?
                .into_iter()
                .filter(|entry| !entry.prunable)
                .filter_map(|entry| entry.branch)
                .collect(),
        })
    })
    .await
}

/// The core loop: make a worktree — on a new branch, or for an existing one — and start a harness in it with the user's
/// first message. If the harness cannot start, the worktree and branch are taken back, so a
/// failed attempt leaves no trace.
#[tauri::command]
#[specta::specta]
pub async fn workspace_create(
    app: AppHandle,
    request: NewWorkspace,
) -> IpcResult<CreatedWorkspace> {
    blocking(app, move |state| {
        // Fail before touching git if the harness cannot possibly start.
        harness::find(&request.harness.id, &state.settings.get().harnesses)
            .filter(|def| def.enabled)
            .ok_or_else(|| IpcError::new("unknown_harness", "That harness is not configured."))?;

        let git = Git::new(&state.env())?;
        let root = state.worktree_root()?;
        let settings = state.settings.get();
        let workspaces = Workspaces {
            store: &state.store,
            git: &git,
            worktree_root: &root,
            settings: &settings.workspaces,
        };
        let prompt = request.harness.prompt.clone().unwrap_or_default();
        let row = match request.existing_branch.as_deref() {
            Some(branch) => workspaces.open_branch(&request.project_id, branch)?,
            None => {
                workspaces.create(&request.project_id, request.base_branch.as_deref(), &prompt)?
            }
        };

        match spawn_in_workspace(
            state,
            &row.id,
            Launch::Harness(request.harness),
            request.size,
        ) {
            Ok(session) => Ok(CreatedWorkspace {
                workspace: Projects {
                    store: &state.store,
                    git: &git,
                }
                .describe_workspace(row),
                session,
            }),
            Err(error) => {
                let _ = workspaces.discard(&row);
                Err(error)
            }
        }
    })
    .await
}

/// Remove a workspace's worktree. The branch is kept. Fails with `worktree_dirty` unless `force`.
#[tauri::command]
#[specta::specta]
pub async fn workspace_delete(app: AppHandle, id: String, force: bool) -> IpcResult<()> {
    blocking(app, move |state| {
        let git = Git::new(&state.env())?;
        let root = state.worktree_root()?;
        let settings = state.settings.get();
        Workspaces {
            store: &state.store,
            git: &git,
            worktree_root: &root,
            settings: &settings.workspaces,
        }
        .delete(&id, force)
    })
    .await
}

fn with_workspaces<T>(
    state: &AppState,
    f: impl FnOnce(&Workspaces<'_>, &Git) -> IpcResult<T>,
) -> IpcResult<T> {
    let git = Git::new(&state.env())?;
    let root = state.worktree_root()?;
    let settings = state.settings.get();
    f(
        &Workspaces {
            store: &state.store,
            git: &git,
            worktree_root: &root,
            settings: &settings.workspaces,
        },
        &git,
    )
}

/// Put a workspace away: the worktree is removed, the branch and session history stay.
/// Fails with `worktree_dirty` unless `force`.
#[tauri::command]
#[specta::specta]
pub async fn workspace_archive(app: AppHandle, id: String, force: bool) -> IpcResult<()> {
    blocking(app, move |state| {
        with_workspaces(state, |workspaces, _| workspaces.archive(&id, force))
    })
    .await
}

/// Bring back an archived workspace, or one whose folder disappeared, at its old path.
#[tauri::command]
#[specta::specta]
pub async fn workspace_restore(app: AppHandle, id: String) -> IpcResult<Workspace> {
    blocking(app, move |state| {
        with_workspaces(state, |workspaces, git| {
            let row = workspaces.restore(&id)?;
            Ok(Projects {
                store: &state.store,
                git,
            }
            .describe_workspace(row))
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn workspace_rename(app: AppHandle, id: String, name: String) -> IpcResult<Workspace> {
    blocking(app, move |state| {
        with_workspaces(state, |workspaces, git| {
            let row = workspaces.rename(&id, &name)?;
            Ok(Projects {
                store: &state.store,
                git,
            }
            .describe_workspace(row))
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn settings_save_general(
    app: AppHandle,
    editor_command: Option<String>,
    notify_when_quiet: bool,
    check_for_updates: bool,
) -> IpcResult<SettingsInfo> {
    blocking(app, move |state| {
        let editor = editor_command
            .map(|command| command.trim().to_owned())
            .filter(|command| !command.is_empty());
        state
            .settings
            .update(|settings| {
                settings.general.editor_command = editor;
                settings.general.notify_when_quiet = notify_when_quiet;
                settings.general.check_for_updates = check_for_updates;
            })
            .map_err(save_failed)?;
        settings_info(state)
    })
    .await
}
