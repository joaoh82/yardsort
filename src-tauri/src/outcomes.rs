//! The Outcomes view's side of IPC: a project's attempts with their outcome and its evidence,
//! the user's label, and per-agent history for the composer. The rules live in the core
//! (`yardsort_core::outcomes`).

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;
use yardsort_core::forge::PullRequestState;
use yardsort_core::outcomes::{self, AgentHistory, Outcome};
use yardsort_core::store::OutcomeRow;

use crate::error::{IpcError, IpcResult};
use crate::state::{blocking, AppState};

/// What an attempt's outcome rests on, beside the outcome itself.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    /// The branch was seen with commits of its own.
    pub ahead: bool,
    /// After that, its work was seen in the base branch: a merge or a fast-forward.
    pub merged_into_base: bool,
    pub pr_number: Option<u32>,
    /// `open`, `merged` or `closed`.
    pub pr_state: Option<String>,
    /// `archived` or `deleted`, when the workspace ended.
    pub ended: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    /// The workspace's id when the attempt began; the workspace itself may be gone.
    pub id: String,
    /// Set while the workspace still exists.
    pub workspace_id: Option<String>,
    pub workspace_name: String,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub task: Option<String>,
    pub harnesses: Vec<String>,
    /// What the user said, if anything.
    pub label: Option<String>,
    /// The outcome that counts: the label, or `kept` from a merge; `None` when unknown.
    pub outcome: Option<String>,
    /// `you` or `merge`, beside `outcome`.
    pub outcome_source: Option<String>,
    pub evidence: Evidence,
    pub created_at: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentOutcomes {
    pub harness: String,
    pub attempts: u32,
    pub kept: u32,
    pub kept_by_merge: u32,
    pub partly: u32,
    pub discarded: u32,
    /// Attempts with an outcome.
    pub known: u32,
    /// Whether `known` reaches the sample size below which nothing is said.
    pub enough: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOutcomes {
    pub project_id: String,
    /// Newest first.
    pub attempts: Vec<Attempt>,
    pub agents: Vec<AgentOutcomes>,
    /// How many known outcomes an agent needs before its history says anything.
    pub min_sample: u32,
}

fn attempt(row: OutcomeRow) -> Attempt {
    let (outcome, source) = match outcomes::outcome(&row) {
        Outcome::Labeled(label) => (Some(label), Some("you".to_owned())),
        Outcome::Merged => (Some("kept".to_owned()), Some("merge".to_owned())),
        Outcome::Unknown => (None, None),
    };
    Attempt {
        harnesses: row
            .harnesses
            .split(',')
            .filter(|h| !h.is_empty())
            .map(str::to_owned)
            .collect(),
        evidence: Evidence {
            ahead: row.ahead_at.is_some(),
            merged_into_base: row.merged_at.is_some(),
            pr_number: row.pr_number.and_then(|n| u32::try_from(n).ok()),
            pr_state: row.pr_state,
            ended: row.ended,
        },
        id: row.id,
        workspace_id: row.workspace_id,
        workspace_name: row.workspace_name,
        branch: row.branch,
        base_branch: row.base_branch,
        task: row.task,
        label: row.label,
        outcome,
        outcome_source: source,
        created_at: row.created_at as f64,
    }
}

fn agents(rows: &[OutcomeRow]) -> Vec<AgentOutcomes> {
    outcomes::history(rows)
        .into_iter()
        .map(|h: AgentHistory| AgentOutcomes {
            known: h.known(),
            enough: h.enough(),
            harness: h.harness,
            attempts: h.attempts,
            kept: h.kept,
            kept_by_merge: h.kept_by_merge,
            partly: h.partly,
            discarded: h.discarded,
        })
        .collect()
}

fn project_outcomes(state: &AppState, project_id: &str) -> IpcResult<ProjectOutcomes> {
    let rows = state.store.outcomes(Some(project_id))?;
    Ok(ProjectOutcomes {
        project_id: project_id.to_owned(),
        agents: agents(&rows),
        attempts: rows.into_iter().map(attempt).collect(),
        min_sample: outcomes::MIN_SAMPLE,
    })
}

/// A project's attempts, brought up to date first: every live workspace snapshotted, git asked
/// whether each branch was ahead or merged, and the pull requests the app already knows —
/// cached, never fetched for this — read for their state.
#[tauri::command]
#[specta::specta]
pub async fn outcomes_get(app: AppHandle, project_id: String) -> IpcResult<ProjectOutcomes> {
    blocking(app, move |state| {
        let project = state
            .store
            .project(&project_id)?
            .ok_or_else(|| IpcError::new("unknown_project", "That project no longer exists."))?;
        let root = std::path::PathBuf::from(&project.root_path);
        if root.is_dir() {
            let git = crate::git::Git::new(&state.env())?;
            outcomes::refresh(&state.store, &git, &project_id, &root)?;
        }
        let known = crate::publish::commands::found(state, &project_id, false);
        let pull_requests: Vec<(String, i64, String)> = known
            .pull_requests
            .iter()
            .map(|pr| {
                let state = match pr.state {
                    PullRequestState::Open => "open",
                    PullRequestState::Merged => "merged",
                    PullRequestState::Closed => "closed",
                };
                (pr.branch.clone(), i64::from(pr.number), state.to_owned())
            })
            .collect();
        let rows = state.store.outcomes(Some(&project_id))?;
        outcomes::observe_pull_requests(&state.store, &rows, &pull_requests)?;
        project_outcomes(state, &project_id)
    })
    .await
}

/// The user's word on an attempt — `kept`, `partly`, `discarded` — or `None` to take it back.
/// A workspace not yet recorded is snapshotted first, so any live one can be labelled.
#[tauri::command]
#[specta::specta]
pub async fn outcome_label(
    app: AppHandle,
    id: String,
    label: Option<String>,
) -> IpcResult<ProjectOutcomes> {
    blocking(app, move |state| {
        if let Some(label) = &label {
            if !outcomes::LABELS.contains(&label.as_str()) {
                return Err(IpcError::new(
                    "bad_label",
                    "An outcome is kept, partly or discarded.",
                ));
            }
        }
        outcomes::snapshot(&state.store, &id)?;
        let row = state
            .store
            .outcome(&id)?
            .ok_or_else(|| IpcError::new("unknown_attempt", "There is no such attempt."))?;
        state.store.label_outcome(&id, label.as_deref())?;
        project_outcomes(state, &row.project_id)
    })
    .await
}

/// Per-agent history across every project, for the composer: counted from outcomes only.
#[tauri::command]
#[specta::specta]
pub async fn outcomes_agents(app: AppHandle) -> IpcResult<Vec<AgentOutcomes>> {
    blocking(app, |state| Ok(agents(&state.store.outcomes(None)?))).await
}
