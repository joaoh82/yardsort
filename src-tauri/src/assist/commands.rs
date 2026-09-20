//! The Assist IPC surface: the key, the switches, and the two things Jev is asked.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};

use super::jev::{Jev, JevError};
use super::key::{self, KeySource};
use super::review::{self, FileInput, Review};
use super::suggest::{self, Candidate, Suggestion};
use super::Thresholds;
use crate::changes::{Changes, Scope};
use crate::error::{IpcError, IpcResult};
use crate::git::Git;
use crate::state::{blocking, AppState};

/// How much of what was asked is worth sending. Long enough for several messages, short enough
/// to leave Jev's attention on the diff.
const MAX_TASK_CHARS: usize = 6000;

/// [`Thresholds`] as the webview sees them. The settings file keeps snake_case names people can
/// read and edit; IPC is camelCase like everything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ThresholdsDto {
    pub flag_at_percent: u8,
    pub off_task_at_percent: u8,
    pub suggest_at_percent: u8,
    /// What "Restore defaults" goes back to, so the form need not repeat them.
    pub defaults: [u8; 3],
    /// The lowest and highest either end may be.
    pub range: [u8; 2],
}

impl From<Thresholds> for ThresholdsDto {
    fn from(thresholds: Thresholds) -> Self {
        let defaults = Thresholds::default();
        Self {
            flag_at_percent: thresholds.flag_at_percent,
            off_task_at_percent: thresholds.off_task_at_percent,
            suggest_at_percent: thresholds.suggest_at_percent,
            defaults: [
                defaults.flag_at_percent,
                defaults.off_task_at_percent,
                defaults.suggest_at_percent,
            ],
            range: [*Thresholds::RANGE.start(), *Thresholds::RANGE.end()],
        }
    }
}

impl From<ThresholdsDto> for Thresholds {
    fn from(dto: ThresholdsDto) -> Self {
        Self {
            flag_at_percent: dto.flag_at_percent,
            off_task_at_percent: dto.off_task_at_percent,
            suggest_at_percent: dto.suggest_at_percent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AssistStatus {
    pub key_source: KeySource,
    /// The end of the key in force, so it can be told apart without being shown.
    pub key_hint: Option<String>,
    /// Why the credential store could not be used, if it could not.
    pub problem: Option<String>,
    pub review_changes: bool,
    pub suggest_in_composer: bool,
    /// How sure Jev must be before an answer becomes a badge or a suggestion.
    pub thresholds: ThresholdsDto,
    /// The model every request names.
    pub model: String,
}

fn failed(error: JevError) -> IpcError {
    IpcError::new(error.code(), error.to_string())
}

fn status(state: &AppState) -> AssistStatus {
    let settings = state.settings.get().assist;
    let (key, key_source, problem) = state.assist.key(state.env().get(key::ENV_VAR));
    AssistStatus {
        key_source,
        key_hint: key.as_deref().map(key::hint),
        problem,
        review_changes: settings.review_changes,
        suggest_in_composer: settings.suggest_in_composer,
        thresholds: settings.thresholds.into(),
        model: super::jev::MODEL.to_owned(),
    }
}

/// The key in force, or the reason there is nothing to ask with.
fn key_in_force(state: &AppState) -> IpcResult<String> {
    let (key, _, problem) = state.assist.key(state.env().get(key::ENV_VAR));
    key.ok_or_else(|| {
        IpcError::new(
            "assist_no_key",
            problem
                .unwrap_or_else(|| "No TypeSafe API key. Add one in Settings → Assist.".to_owned()),
        )
    })
}

#[tauri::command]
#[specta::specta]
pub async fn assist_status(app: AppHandle) -> IpcResult<AssistStatus> {
    blocking(app, |state| Ok(status(state))).await
}

/// Save an API key, once TypeSafe confirms it works. It goes to the OS credential store and is
/// never sent back to the webview.
#[tauri::command]
#[specta::specta]
pub async fn assist_save_key(app: AppHandle, key: String) -> IpcResult<AssistStatus> {
    let key = key.trim().to_owned();
    if key.is_empty() {
        return Err(IpcError::new("assist_no_key", "Paste an API key first."));
    }
    Jev::new(&key)
        .map_err(failed)?
        .check_key()
        .await
        .map_err(failed)?;
    blocking(app, move |state| {
        state
            .assist
            .save_key(&key)
            .map_err(|problem| IpcError::new("assist_key_store", problem))?;
        Ok(status(state))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn assist_forget_key(app: AppHandle) -> IpcResult<AssistStatus> {
    blocking(app, |state| {
        state
            .assist
            .forget_key()
            .map_err(|problem| IpcError::new("assist_key_store", problem))?;
        Ok(status(state))
    })
    .await
}

/// Ask TypeSafe whether the key in force still works.
#[tauri::command]
#[specta::specta]
pub async fn assist_test_key(app: AppHandle) -> IpcResult<()> {
    let key = blocking(app, key_in_force).await?;
    Jev::new(&key)
        .map_err(failed)?
        .check_key()
        .await
        .map_err(failed)
}

#[tauri::command]
#[specta::specta]
pub async fn assist_save_settings(
    app: AppHandle,
    review_changes: bool,
    suggest_in_composer: bool,
    thresholds: ThresholdsDto,
) -> IpcResult<AssistStatus> {
    let thresholds = Thresholds::from(thresholds);
    thresholds
        .validate()
        .map_err(|why| IpcError::new("invalid_thresholds", why))?;
    blocking(app, move |state| {
        state
            .settings
            .update(|settings| {
                settings.assist.review_changes = review_changes;
                settings.assist.suggest_in_composer = suggest_in_composer;
                settings.assist.thresholds = thresholds;
            })
            .map_err(|error| {
                IpcError::new(
                    "settings_write_failed",
                    format!("Could not save settings: {error}"),
                )
            })?;
        Ok(status(state))
    })
    .await
}

/// What a review needs before anything is sent: the key, the task, and one entry per changed file.
struct Prepared {
    key: String,
    task: Option<String>,
    inputs: Vec<FileInput>,
    thresholds: Thresholds,
}

fn prepare_review(state: &AppState, workspace_id: &str) -> IpcResult<Prepared> {
    if !state.settings.get().assist.review_changes {
        return Err(IpcError::new(
            "assist_off",
            "Reviewing changes is switched off in Settings → Assist.",
        ));
    }
    let key = key_in_force(state)?;
    let thresholds = state.settings.get().assist.thresholds;
    let (row, root) = crate::changes::commands::workspace(state, workspace_id)?;
    let git = Git::new(&state.env())?;
    let changes = Changes {
        git: &git,
        root: &root,
        base_branch: row.base_branch.as_deref(),
    };
    let set = changes.list()?;
    let mut inputs = Vec::with_capacity(set.uncommitted.len() + set.committed.len());
    for (scope, list) in [
        (Scope::Uncommitted, &set.uncommitted),
        (Scope::Committed, &set.committed),
    ] {
        for change in list {
            // A file we cannot read a diff for is still listed, as "not checked".
            let patch = changes.patch(change, scope).unwrap_or(None);
            inputs.push(FileInput::new(change, scope, patch));
        }
    }

    let mut task = state.store.session_prompts(workspace_id)?.join("\n\n");
    if task.chars().count() > MAX_TASK_CHARS {
        task = task.chars().take(MAX_TASK_CHARS).collect();
    }
    Ok(Prepared {
        key,
        task: Some(task).filter(|task| !task.trim().is_empty()),
        inputs,
        thresholds,
    })
}

/// Check a workspace's changed files against what was asked, and for risky edits. Sends the
/// diffs of those files to TypeSafe.
#[tauri::command]
#[specta::specta]
pub async fn assist_review(app: AppHandle, workspace_id: String) -> IpcResult<Review> {
    let prepared = blocking(app.clone(), move |state| {
        prepare_review(state, &workspace_id)
    })
    .await?;
    let jev = Arc::new(Jev::new(&prepared.key).map_err(failed)?);
    let state = app.state::<AppState>();
    review::review(
        jev,
        &state.assist.reviews,
        prepared.task,
        prepared.inputs,
        prepared.thresholds,
    )
    .await
    .map_err(failed)
}

/// Suggest a harness and an effort for the message being typed. Sends the message and the
/// harnesses' "Good at" descriptions.
#[tauri::command]
#[specta::specta]
pub async fn assist_suggest(app: AppHandle, message: String) -> IpcResult<Suggestion> {
    let (key, candidates, thresholds) = blocking(app, |state| {
        if !state.settings.get().assist.suggest_in_composer {
            return Err(IpcError::new(
                "assist_off",
                "Composer suggestions are switched off in Settings → Assist.",
            ));
        }
        let key = key_in_force(state)?;
        let candidates: Vec<Candidate> = crate::workspaces::commands::list_harnesses(state)
            .into_iter()
            .filter(|harness| harness.def.enabled && harness.resolved_path.is_some())
            .map(|harness| Candidate {
                id: harness.def.id,
                label: harness.def.label,
                strengths: harness.def.strengths,
                efforts: harness.def.efforts,
            })
            .collect();
        Ok((key, candidates, state.settings.get().assist.thresholds))
    })
    .await?;
    // An empty suggestion is "nothing worth offering": the composer then shows nothing.
    suggest::suggest(
        &Jev::new(&key).map_err(failed)?,
        &message,
        &candidates,
        thresholds,
    )
    .await
    .map(Option::unwrap_or_default)
    .map_err(failed)
}
