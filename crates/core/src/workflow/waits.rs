//! A human wait: the row that says why a run is blocked and what the person must do.
//!
//! An approval opens one when its step starts waiting. A clarification question uses the
//! same row, with `kind` `question`, the question in `prompt` and the reply in `answer`,
//! so that later change does not add a table or a second resume path. The engine applies
//! a recorded approval through the decision row it already reads. `applied_at` is set in
//! that same change, and a restart applies a wait that has not been applied exactly once.
//!
//! [`list_open`] and [`decide`] are what a phone or a remote crate calls. They read and
//! write this database. This build adds no remote route, no protocol message and no phone
//! screen. Those stay with the later notification and mobile-control work.

use crate::store::{ApprovalWrite, Store, StoreResult};

/// What the run is waiting for a person to do. `approval` today. `question` is reserved
/// for a clarification and uses this same struct.
pub const KIND_APPROVAL: &str = "approval";
pub const KIND_QUESTION: &str = "question";

/// The words after "Blocked: waiting for ".
pub const BLOCKED_APPROVAL: &str = "your approval";
pub const BLOCKED_QUESTION: &str = "your answer";

/// What an approval asks the person to do.
pub const ACTION_APPROVAL: &str = "Approve, reject, or request changes.";
/// What a clarification asks the person to do.
pub const ACTION_QUESTION: &str = "Answer the question.";

/// Where the run is, without the workflow file. A later phone shows this as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase {
    pub workflow_id: String,
    pub workflow_name: String,
    pub workspace_name: String,
    pub step_id: String,
    pub action: String,
}

impl Phase {
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "workflowId": self.workflow_id,
            "workflowName": self.workflow_name,
            "workspaceName": self.workspace_name,
            "stepId": self.step_id,
            "action": self.action,
        })
        .to_string()
    }

    pub fn parse(raw: &str) -> Self {
        let value: serde_json::Value = serde_json::from_str(raw).unwrap_or_default();
        let text = |key: &str| {
            value
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned()
        };
        Self {
            workflow_id: text("workflowId"),
            workflow_name: text("workflowName"),
            workspace_name: text("workspaceName"),
            step_id: text("stepId"),
            action: text("action"),
        }
    }
}

/// One wait, as the run view, `ys` and a later phone read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanWait {
    pub run_id: String,
    pub step_id: String,
    pub kind: String,
    pub blocked_on: String,
    pub action_needed: String,
    pub prompt: Option<String>,
    pub artifact_refs: Vec<String>,
    pub phase: Phase,
    pub answer: Option<String>,
    pub actor: Option<String>,
    pub answered_at: Option<i64>,
    pub applied_at: Option<i64>,
    pub notified_at: Option<i64>,
    pub created_at: i64,
    pub workflow_id: String,
    pub workflow_name: String,
    pub workspace_name: String,
    pub run_status: String,
}

impl HumanWait {
    /// `Blocked: waiting for your approval.`
    pub fn blocked_line(&self) -> String {
        blocked_line(&self.blocked_on)
    }

    fn from_row(row: crate::store::WorkflowWaitRow) -> Self {
        Self {
            artifact_refs: artifact_refs(&row.artifact_refs),
            phase: Phase::parse(&row.phase),
            run_id: row.run_id,
            step_id: row.step_id,
            kind: row.kind,
            blocked_on: row.blocked_on,
            action_needed: row.action_needed,
            prompt: row.prompt,
            answer: row.answer,
            actor: row.actor,
            answered_at: row.answered_at,
            applied_at: row.applied_at,
            notified_at: row.notified_at,
            created_at: row.created_at,
            workflow_id: row.workflow_id,
            workflow_name: row.workflow_name,
            workspace_name: row.workspace_name,
            run_status: row.run_status,
        }
    }

    /// Still needs the person. A decision or an answer that the engine has not applied yet
    /// does not: the person has done their part.
    pub fn needs_person(&self, decision_recorded: bool) -> bool {
        if self.applied_at.is_some() || self.run_status != "running" {
            return false;
        }
        match self.kind.as_str() {
            KIND_QUESTION => self.answer.is_none(),
            _ => !decision_recorded,
        }
    }
}

pub fn blocked_line(blocked_on: &str) -> String {
    format!("Blocked: waiting for {blocked_on}.")
}

/// The waiting notification. `code` is included only when the setting says so; otherwise the
/// body tells the person to open Yardsort. A replaced code says so, and still carries the
/// new code or that sentence.
pub fn notice_body(
    workflow_name: &str,
    workspace_name: &str,
    step: &str,
    prompt: Option<&str>,
    code: Option<&str>,
    replaced: bool,
) -> String {
    let mut body = format!("{workflow_name} in {workspace_name} is waiting on `{step}`.");
    if replaced {
        body.push_str(" The code was replaced.");
    }
    if let Some(code) = code {
        body.push_str(" Your code is ");
        body.push_str(code);
        body.push('.');
    } else {
        body.push_str(" Open Yardsort for the code.");
    }
    if let Some(line) = prompt
        .and_then(|text| text.lines().next())
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        body.push(' ');
        body.push_str(line);
    }
    body
}

/// Titles the waiting and replacement notifications use. A schedule set to never sends neither.
pub const TITLE_WAITING: &str = "Waiting for your approval";
pub const TITLE_REPLACED: &str = "Approval code replaced";

pub fn artifact_refs(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// Waits that still need a person, across every running run. The desktop list, `ys
/// workflow runs` and a later phone all use this. A decision recorded while the app was
/// closed is not in the list: it is waiting on the engine, not on the person.
pub fn list_open(store: &Store) -> StoreResult<Vec<HumanWait>> {
    let rows = store.workflow_waits_unapplied()?;
    let mut open = Vec::new();
    for row in rows {
        let wait = HumanWait::from_row(row);
        let decided = store
            .workflow_approval(&wait.run_id, &wait.step_id)?
            .is_some();
        if wait.needs_person(decided) {
            open.push(wait);
        }
    }
    Ok(open)
}

/// The label for one run's WAITING ON cell. Empty when the run is not blocked on a person.
/// A waiting approval that has not opened a row yet, from a database written before this,
/// still says `your approval`.
pub fn waiting_on(store: &Store, run_id: &str) -> StoreResult<Option<String>> {
    let mut labels: Vec<String> = list_open(store)?
        .into_iter()
        .filter(|wait| wait.run_id == run_id)
        .map(|wait| wait.blocked_on)
        .collect();
    if labels.is_empty() {
        let steps = store.workflow_steps(run_id)?;
        let legacy = steps.iter().any(|step| {
            step.action == "approve"
                && step.status == "waiting"
                && store
                    .workflow_approval(run_id, &step.step_id)
                    .ok()
                    .flatten()
                    .is_none()
        });
        if legacy {
            labels.push(BLOCKED_APPROVAL.to_owned());
        }
    }
    if labels.is_empty() {
        Ok(None)
    } else {
        Ok(Some(labels.join(", ")))
    }
}

/// Record an approval decision. The same call the window and `ys` make, and the one a
/// later remote crate makes. Nothing here moves the step or sends a notification. The
/// engine applies the row once, including after a restart.
pub fn decide(
    store: &Store,
    run_id: &str,
    step_id: &str,
    decision: &str,
    feedback: Option<&str>,
    actor: &str,
    code: Option<&str>,
) -> StoreResult<ApprovalWrite> {
    store.record_workflow_approval(run_id, step_id, decision, feedback, actor, code)
}

/// Why an answer was not stored. The words are for a later clarification, not shown yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerWrite {
    Recorded,
    Missing,
    NotAQuestion,
    AlreadyAnswered,
}

/// Store a clarification's answer on the wait. The engine, when a question step exists,
/// applies it the way it applies an approval. This build has no question step and does
/// not call this from the window or from `ys`.
pub fn answer(
    store: &Store,
    run_id: &str,
    step_id: &str,
    text: &str,
    actor: &str,
) -> StoreResult<AnswerWrite> {
    store.answer_workflow_wait(run_id, step_id, text, actor)
}
