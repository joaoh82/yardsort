//! Asking the agent that opened a pull request to resolve its merge conflicts.
//!
//! Which agent, and what it is told, is [`yardsort_core::conflicts`]'s. This is how the message
//! reaches it: typed into the conversation when it is running and quiet, typed into it once it
//! is resumed when it has ended, or given to a new conversation of the same agent when the old
//! one cannot be continued. A busy agent is never typed into.

use pty_host::{SessionId, SessionInfo, SessionState, TermSize};
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;
use yardsort_core::conflicts;

use super::commands::{failed, found, project_root};
use super::ensure_own;
use crate::changes::commands::workspace;
use crate::error::{IpcError, IpcResult};
use crate::forge::{ForgeError, Gh, Mergeable, PullRequest, PullRequestState};
use crate::git::{Git, Head};
use crate::sessions::{self, Continue, SessionRecord};
use crate::state::{blocking, AppState};
use crate::store::WorkspaceRow;
use crate::terminal::{spawn_in_workspace, HarnessRequest, Launch, RECORD_LABEL};

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
    /// The conversation's record. The request goes to this one or to none: see
    /// [`workspace_resolve_conflicts`].
    pub session_id: String,
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

pub(super) struct Plan {
    pub(super) record: SessionRecord,
    pub(super) reach: Reach,
}

/// Choose the conversation and how to reach it. An error when there is no agent, or the one
/// there is busy.
pub(super) fn plan(
    state: &AppState,
    row: &WorkspaceRow,
    opened_at: Option<i64>,
) -> IpcResult<Plan> {
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
/// confirmation, before anything is sent. Reads the project's list, cached when it is fresh.
#[tauri::command]
#[specta::specta]
pub async fn workspace_conflict_helper(
    app: AppHandle,
    workspace_id: String,
    number: u32,
) -> IpcResult<ConflictHelper> {
    blocking(app, move |state| {
        let (row, root) = workspace(state, &workspace_id)?;
        let requests = found(state, &row.project_id, false).pull_requests;
        ensure_own(&Git::new(&state.env())?, &root, &row, &requests, number)?;
        let opened_at = requests
            .iter()
            .find(|pr| pr.number == number)
            .and_then(|pr| pr.created_at);
        let Plan { record, reach } = plan(state, &row, opened_at)?;
        Ok(ConflictHelper {
            session_id: record.id,
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
/// It must be one this workspace opened, and `session_id` the conversation the user agreed to
/// ask — [`ConflictHelper::session_id`]. If another has become the one to ask since, nothing is
/// sent: the user said yes to that conversation, not to whichever.
#[tauri::command]
#[specta::specta]
pub async fn workspace_resolve_conflicts(
    app: AppHandle,
    workspace_id: String,
    number: u32,
    session_id: String,
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
        // Whose it is, out of the list as it stands now, with the answer just had for this one:
        // it may be older than the fifty the list holds.
        let mut requests = found(state, &row.project_id, true).pull_requests;
        requests.retain(|other| other.number != number);
        requests.push(pr.clone());
        let git = Git::new(&state.env())?;
        ensure_own(&git, &root, &row, &requests, number)?;
        let checked_out = match git.head(&root)? {
            Head::Branch(name) => Some(name),
            Head::Unborn(_) | Head::Detached(_) => None,
        };
        let Plan { record, reach } = plan(state, &row, pr.created_at)?;
        same_agent(&record, &session_id)?;
        let session = deliver(
            state,
            &row,
            &record,
            reach,
            |task| conflicts::prompt(&pr, &base, checked_out.as_deref(), task),
            &format!("Resolve conflicts in #{number}"),
            size,
        )?;
        Ok(ConflictsAsked { reach, session })
    })
    .await
}

/// Get a message to the conversation `plan` chose, the way it said: typed into it, typed into
/// it once resumed, or as the first message of a new conversation of the same agent. `text` is
/// asked for the words, with the workspace's task for a conversation that has not seen it;
/// `title` names a new conversation in the history.
pub(super) fn deliver(
    state: &AppState,
    row: &WorkspaceRow,
    record: &SessionRecord,
    reach: Reach,
    text: impl Fn(Option<&str>) -> String,
    title: &str,
    size: TermSize,
) -> IpcResult<SessionInfo> {
    Ok(match reach {
        Reach::Type => {
            let id = SessionId(record.pty_session_id.clone().unwrap_or_default());
            pty_host::paste_and_submit(state.host.as_ref(), &id, &text(None))?;
            state.host.info(&id)?
        }
        Reach::Resume => {
            sessions::continue_session(state, &record.id, Continue::Resume, size, Some(text(None)))?
        }
        Reach::Start => {
            // A new conversation has not seen the task, so it is told.
            let task = state.store.session_prompts(&row.id)?.into_iter().next();
            let session = spawn_in_workspace(
                state,
                &row.id,
                // Sent as a handoff: the record keeps no first message, so the workspace's
                // task stays what the user first asked.
                Launch::Harness(HarnessRequest {
                    id: record.harness_id.clone(),
                    model: record.model.clone(),
                    effort: record.effort.clone(),
                    prompt: Some(text(task.as_deref())),
                    handoff: true,
                    skip_memory: false,
                }),
                size,
            )?;
            if let Some(id) = session.labels.get(RECORD_LABEL) {
                state.store.set_session_title(id, title)?;
            }
            session
        }
    })
}

/// Refuse when the conversation chosen now is not the one the user agreed to ask.
pub(super) fn same_agent(chosen: &SessionRecord, confirmed: &str) -> IpcResult<()> {
    if chosen.id == confirmed {
        return Ok(());
    }
    Err(IpcError::new(
        "agent_changed",
        format!(
            "Since you confirmed, {} in “{}” has become the one to ask. Nothing was sent; ask \
             again to confirm it.",
            chosen.harness_label, chosen.title
        ),
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str) -> SessionRecord {
        SessionRecord {
            id: id.into(),
            workspace_id: "w".into(),
            harness_id: "claude".into(),
            harness_label: "Claude Code".into(),
            model: None,
            effort: None,
            title: "Fix the login".into(),
            forked_from: None,
            running: false,
            pty_session_id: None,
            exit_code: Some(0),
            interrupted: false,
            started_at: 0.0,
            ended_at: None,
            resumable: true,
            forkable: true,
            unavailable_reason: None,
        }
    }

    #[test]
    fn only_the_conversation_confirmed_is_asked() {
        assert!(same_agent(&record("r1"), "r1").is_ok());
        let refused = same_agent(&record("r2"), "r1").unwrap_err();
        assert_eq!(refused.code, "agent_changed");
        assert!(refused.message.contains("Claude Code in “Fix the login”"));
    }
}
