//! The Memory view's side of IPC: a project's entries with their history, the user's decisions,
//! the switch that shares them with agents, the counts on the project rows, and Jev's checks.
//! The rules live in the core (`yardsort_core::memory`); nothing here decides what an entry may
//! become.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use yardsort_core::memory::{self, Decision};
use yardsort_core::store::MemoryRow;

use crate::assist::memory::MemoryCheck;
use crate::error::{IpcError, IpcResult};
use crate::state::{blocking, AppState};

/// One change to an entry.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MemoryHistoryItem {
    pub at: f64,
    /// `proposed`, `written`, `approved`, `edited`, `rejected`, `revoked` or `restored`.
    pub action: String,
    /// `user` or `agent`.
    pub by: String,
    /// The text an edit replaced.
    pub previous_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: String,
    /// The first 8 characters of the id, as agents see it cited.
    pub short_id: String,
    pub text: String,
    /// `candidate`, `approved`, `rejected` or `revoked`.
    pub state: String,
    /// `user` or `agent`.
    pub author: String,
    /// Where it came from, in words: "the user", or "claude in fix-login".
    pub from: String,
    pub created_at: f64,
    pub updated_at: f64,
    pub history: Vec<MemoryHistoryItem>,
}

/// A project's memory, as the view shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMemory {
    pub project_id: String,
    /// Whether approved entries go into this project's agents' first messages.
    pub shared: bool,
    /// Oldest first.
    pub entries: Vec<MemoryEntry>,
    /// The section a first message would get now, or `None` when nothing would be added.
    pub preview: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MemoryDecision {
    Approve,
    Reject,
    Revoke,
    Restore,
}

/// How many proposals wait in a project.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MemoryWaiting {
    pub project_id: String,
    pub count: u32,
}

fn entry(state: &AppState, row: MemoryRow) -> IpcResult<MemoryEntry> {
    let history = state
        .store
        .memory_history(&row.id)?
        .into_iter()
        .map(|item| MemoryHistoryItem {
            at: item.at as f64,
            action: item.action,
            by: item.by,
            previous_text: item.previous_text,
        })
        .collect();
    Ok(MemoryEntry {
        short_id: memory::short_id(&row.id).to_owned(),
        from: memory::citation(&row),
        id: row.id,
        text: row.text,
        state: row.state,
        author: row.author,
        created_at: row.created_at as f64,
        updated_at: row.updated_at as f64,
        history,
    })
}

fn project_memory(state: &AppState, project_id: &str) -> IpcResult<ProjectMemory> {
    let entries = state
        .store
        .memory_entries(project_id)?
        .into_iter()
        .map(|row| entry(state, row))
        .collect::<IpcResult<Vec<_>>>()?;
    Ok(ProjectMemory {
        project_id: project_id.to_owned(),
        shared: state.store.memory_shared(project_id)?,
        preview: memory::prompt_section(&state.store, project_id)?,
        entries,
    })
}

/// The project an entry belongs to, for answering with that project's memory.
fn project_of(state: &AppState, id: &str) -> IpcResult<String> {
    Ok(state
        .store
        .memory_entry(id)?
        .ok_or_else(|| IpcError::new("memory_unknown", "That memory entry no longer exists."))?
        .project_id)
}

#[tauri::command]
#[specta::specta]
pub async fn memory_get(app: AppHandle, project_id: String) -> IpcResult<ProjectMemory> {
    blocking(app, move |state| project_memory(state, &project_id)).await
}

/// The user writes an entry: approved as written.
#[tauri::command]
#[specta::specta]
pub async fn memory_write(
    app: AppHandle,
    project_id: String,
    text: String,
) -> IpcResult<ProjectMemory> {
    blocking(app, move |state| {
        memory::write(&state.store, &project_id, &text)?;
        project_memory(state, &project_id)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn memory_edit(app: AppHandle, id: String, text: String) -> IpcResult<ProjectMemory> {
    blocking(app, move |state| {
        let project = project_of(state, &id)?;
        memory::edit(&state.store, &id, &text)?;
        project_memory(state, &project)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn memory_decide(
    app: AppHandle,
    id: String,
    decision: MemoryDecision,
) -> IpcResult<ProjectMemory> {
    blocking(app, move |state| {
        let project = project_of(state, &id)?;
        let decision = match decision {
            MemoryDecision::Approve => Decision::Approve,
            MemoryDecision::Reject => Decision::Reject,
            MemoryDecision::Revoke => Decision::Revoke,
            MemoryDecision::Restore => Decision::Restore,
        };
        memory::decide(&state.store, &id, decision)?;
        project_memory(state, &project)
    })
    .await
}

/// Turn sharing a project's approved entries with its agents on or off.
#[tauri::command]
#[specta::specta]
pub async fn memory_share(
    app: AppHandle,
    project_id: String,
    shared: bool,
) -> IpcResult<ProjectMemory> {
    blocking(app, move |state| {
        state.store.set_memory_shared(&project_id, shared)?;
        project_memory(state, &project_id)
    })
    .await
}

/// How many proposals wait, per project that has any. Agents propose through `ys`, which writes
/// the database directly, so the window asks again when it regains focus and when activity lands.
#[tauri::command]
#[specta::specta]
pub async fn memory_waiting(app: AppHandle) -> IpcResult<Vec<MemoryWaiting>> {
    blocking(app, |state| {
        Ok(state
            .store
            .memory_waiting()?
            .into_iter()
            .map(|(project_id, count)| MemoryWaiting { project_id, count })
            .collect())
    })
    .await
}

/// Ask Jev whether each proposal waiting in a project repeats or contradicts an approved entry.
/// Only with the Assist switch on and a key in force; sends the proposals and the approved
/// entries, nothing else.
#[tauri::command]
#[specta::specta]
pub async fn memory_check(app: AppHandle, project_id: String) -> IpcResult<Vec<MemoryCheck>> {
    let (key, candidates, approved, thresholds) = blocking(app.clone(), move |state| {
        let settings = state.settings.get().assist;
        if !settings.check_memory {
            return Err(IpcError::new(
                "assist_off",
                "Checking memory proposals is switched off in Settings → Assist.",
            ));
        }
        let (key, _, problem) = state
            .assist
            .key(state.env().get(crate::assist::key::ENV_VAR));
        let key = key.ok_or_else(|| {
            IpcError::new(
                "no_key",
                problem.unwrap_or_else(|| "No TypeSafe API key is set.".to_owned()),
            )
        })?;
        let entries = state.store.memory_entries(&project_id)?;
        let candidates: Vec<(String, String)> = entries
            .iter()
            .filter(|e| e.state == "candidate")
            .map(|e| (e.id.clone(), e.text.clone()))
            .collect();
        let approved: Vec<String> = memory::approved(&state.store, &project_id)?
            .into_iter()
            .map(|e| e.text)
            .collect();
        Ok((key, candidates, approved, settings.thresholds))
    })
    .await?;
    let jev =
        crate::assist::jev::Jev::new(&key).map_err(|e| IpcError::new(e.code(), e.to_string()))?;
    let state = app.state::<AppState>();
    crate::assist::memory::check(
        Arc::new(jev),
        &state.assist.memory_checks,
        candidates,
        approved,
        thresholds,
    )
    .await
    .map_err(|e| IpcError::new(e.code(), e.to_string()))
}
