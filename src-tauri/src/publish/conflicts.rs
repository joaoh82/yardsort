//! Asking the agent that opened a pull request to resolve its merge conflicts.
//!
//! Which agent, and what it is told, is [`yardsort_core::conflicts`]'s. This is how the message
//! reaches it: typed into the conversation when it is running and quiet, typed into it once it
//! is resumed when it has ended, or given to a new conversation of the same agent when the old
//! one cannot be continued. A busy agent is never typed into.

use std::time::Duration;

use pty_host::{SessionId, SessionInfo, SessionState, TermSize};
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;
use yardsort_core::conflicts;

use super::commands::{failed, project_root};
use crate::changes::commands::workspace;
use crate::error::{IpcError, IpcResult};
use crate::forge::{ForgeError, Gh, Mergeable, PullRequest, PullRequestState};
use crate::git::{Git, Head};
use crate::sessions::{self, Continue, SessionRecord};
use crate::state::{blocking, AppState};
use crate::store::WorkspaceRow;
use crate::terminal::{spawn_in_workspace, HarnessRequest, Launch, RECORD_LABEL};

/// How long after the text the Enter is sent, so the agent takes it as one paste.
const SUBMIT_DELAY: Duration = Duration::from_millis(150);

/// How the message reaches the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Reach {
    /// It is running and quiet: the message is typed into it.
    Type,
    /// Its process has ended: the conversation is resumed and the message typed in once ready.
    Resume,
    /// The conversation cannot be continued: a new one of the same agent is started with it.
    Start,
}

/// Who would be asked, for the confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConflictHelper {
    pub harness_label: String,
    /// The conversation's title, as its history lists it.
    pub title: String,
    pub reach: Reach,
}

/// The agent was asked.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConflictsAsked {
    pub reach: Reach,
    /// The terminal it is in: an existing tab for [`Reach::Type`], a new one otherwise.
    pub session: SessionInfo,
}

struct Plan {
    record: SessionRecord,
    reach: Reach,
}

/// Choose the conversation and how to reach it. An error when there is no agent, or the one
/// there is busy.
fn plan(state: &AppState, row: &WorkspaceRow, opened_at: Option<i64>) -> IpcResult<Plan> {
    let sessions = state.store.sessions(&row.id)?;
    let runs = state.store.runs(&row.id)?;
    let task = state.store.task_session(&row.id)?;
    let chosen =
        conflicts::author(&sessions, &runs, task.as_deref(), opened_at).ok_or_else(|| {
            IpcError::new(
                "no_agent",
                "No agent has worked in this workspace, so there is nobody to ask.",
            )
        })?;
    let record = sessions::list(state, &row.id)?
        .into_iter()
        .find(|record| record.id == chosen.id)
        .ok_or_else(|| IpcError::internal("session vanished while listing"))?;
    let live = record
        .pty_session_id
        .as_ref()
        .filter(|_| record.running)
        .and_then(|id| state.host.info(&SessionId(id.clone())).ok())
        .filter(|info| matches!(info.state, SessionState::Running));
    let reach = match &live {
        Some(info) if info.busy => {
            return Err(IpcError::new(
                "agent_busy",
                format!(
                    "{} is working right now. Ask again once it is quiet: Yardsort never types \
                     into a busy agent.",
                    record.harness_label
                ),
            ))
        }
        Some(_) => Reach::Type,
        None if record.resumable => Reach::Resume,
        None => {
            // A new conversation only needs the agent itself to be there.
            let available =
                crate::harness::find(&record.harness_id, &state.settings.get().harnesses)
                    .is_some_and(|def| def.enabled);
            if !available {
                return Err(IpcError::new(
                    "cannot_continue",
                    format!(
                        "{} is not configured, or is disabled in settings, so it cannot be asked.",
                        record.harness_label
                    ),
                ));
            }
            Reach::Start
        }
    };
    Ok(Plan { record, reach })
}

/// Who would be asked to resolve pull request `number`'s conflicts, and how — for the
/// confirmation, before anything is sent. Asks nothing of the forge.
#[tauri::command]
#[specta::specta]
pub async fn workspace_conflict_helper(
    app: AppHandle,
    workspace_id: String,
    number: u32,
) -> IpcResult<ConflictHelper> {
    blocking(app, move |state| {
        let (row, _root) = workspace(state, &workspace_id)?;
        let opened_at = state
            .forge
            .cached(&row.project_id)
            .and_then(|found| {
                found
                    .pull_requests
                    .into_iter()
                    .find(|pr| pr.number == number)
            })
            .and_then(|pr| pr.created_at);
        let Plan { record, reach } = plan(state, &row, opened_at)?;
        Ok(ConflictHelper {
            harness_label: record.harness_label,
            title: record.title,
            reach,
        })
    })
    .await
}

/// Ask the agent that opened pull request `number` to resolve its merge conflicts.
///
/// The forge is asked first, not the list from a moment ago: a pull request that has stopped
/// conflicting, or whose conflicts GitHub has not worked out yet, is not worth an agent's turn.
#[tauri::command]
#[specta::specta]
pub async fn workspace_resolve_conflicts(
    app: AppHandle,
    workspace_id: String,
    number: u32,
    size: TermSize,
) -> IpcResult<ConflictsAsked> {
    blocking(app, move |state| {
        let (row, root) = workspace(state, &workspace_id)?;
        let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
        let pr = gh
            .pull_request(&project_root(state, &row.project_id)?, number)
            .map_err(failed)?;
        state.forge.forget(&row.project_id);
        let base = still_conflicting(&pr)?;
        let checked_out = match Git::new(&state.env())?.head(&root)? {
            Head::Branch(name) => Some(name),
            Head::Unborn(_) | Head::Detached(_) => None,
        };
        let Plan { record, reach } = plan(state, &row, pr.created_at)?;
        let session = match reach {
            Reach::Type => {
                let text = conflicts::prompt(&pr, &base, checked_out.as_deref(), None);
                let id = SessionId(record.pty_session_id.clone().unwrap_or_default());
                state.host.paste(&id, &text)?;
                std::thread::sleep(SUBMIT_DELAY);
                state.host.write(&id, b"\r")?;
                state.host.info(&id)?
            }
            Reach::Resume => {
                let text = conflicts::prompt(&pr, &base, checked_out.as_deref(), None);
                sessions::continue_session(state, &record.id, Continue::Resume, size, Some(text))?
            }
            Reach::Start => {
                // A new conversation has not seen the task, so it is told.
                let task = state.store.session_prompts(&row.id)?.into_iter().next();
                let text = conflicts::prompt(&pr, &base, checked_out.as_deref(), task.as_deref());
                let session = spawn_in_workspace(
                    state,
                    &row.id,
                    // Sent as a handoff: the record keeps no first message, so the workspace's
                    // task stays what the user first asked.
                    Launch::Harness(HarnessRequest {
                        id: record.harness_id.clone(),
                        model: record.model.clone(),
                        effort: record.effort.clone(),
                        prompt: Some(text),
                        handoff: true,
                        skip_memory: false,
                    }),
                    size,
                )?;
                if let Some(id) = session.labels.get(RECORD_LABEL) {
                    state
                        .store
                        .set_session_title(id, &format!("Resolve conflicts in #{number}"))?;
                }
                session
            }
        };
        Ok(ConflictsAsked { reach, session })
    })
    .await
}

/// The base the pull request conflicts with, or why it is not one to ask about.
fn still_conflicting(pr: &PullRequest) -> IpcResult<String> {
    if pr.state != PullRequestState::Open {
        return Err(IpcError::new(
            "pull_request_not_open",
            format!("Pull request #{} is no longer open.", pr.number),
        ));
    }
    let details = pr
        .details
        .as_ref()
        .ok_or_else(|| IpcError::internal("gh answered without the pull request's details"))?;
    match details.mergeable {
        Mergeable::Conflicting => Ok(details.base.clone()),
        Mergeable::Mergeable => Err(IpcError::new(
            "no_conflicts",
            format!(
                "Pull request #{} no longer conflicts with {}.",
                pr.number, details.base
            ),
        )),
        Mergeable::Unknown => Err(IpcError::new(
            "conflicts_unknown",
            format!(
                "GitHub has not finished checking pull request #{} for conflicts. Try again in \
                 a moment.",
                pr.number
            ),
        )),
    }
}
