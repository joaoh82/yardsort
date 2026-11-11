//! A human wait: the row that says why a run is blocked and what the person must do.
//!
//! An approval opens one when its step starts waiting. A clarification question uses the
//! same row, with `kind` `question`, the question in `prompt` and the reply in `answer`.
//! The engine applies a recorded approval through the decision row it already reads.
//! `applied_at` is set in that same change, and a restart applies a wait that has not been
//! applied exactly once. A question's answer is claimed with `applied_at` before it is typed,
//! and `typed_at` on the round is set only after the keystrokes return. A terminal answer
//! takes the same one-time code as an approval. The window does not.
//!
//! [`list_open`] and [`decide_with_code`] are what a phone or a remote crate calls. They
//! read and write this database. A remote caller must use [`decide_with_code`]. The
//! window path takes a `DesktopWindow`. The `desktop-window` feature is a review guard,
//! not a hard boundary: Cargo unifies features across a build, so a crate compiled with
//! the app can see `DesktopWindow` too. This build adds no remote route, no protocol
//! message and no phone screen. Those stay with the later notification and
//! mobile-control work.

use std::collections::HashMap;

use crate::store::{ApprovalWrite, Store, StoreResult};

/// The desktop window. `desktop-window` is a review guard, not a hard boundary: Cargo
/// unifies features across a build, so this type is visible to every crate in a build
/// that enables it. Remote callers use [`decide_with_code`], which always takes the code.
#[cfg(feature = "desktop-window")]
#[derive(Debug, Clone, Copy)]
pub struct DesktopWindow {
    _private: (),
}

#[cfg(feature = "desktop-window")]
impl DesktopWindow {
    /// The Yardsort window. Do not call this from a remote crate.
    pub const fn new() -> Self {
        Self { _private: () }
    }
}

#[cfg(feature = "desktop-window")]
impl Default for DesktopWindow {
    fn default() -> Self {
        Self::new()
    }
}

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
/// A clarifying step is waiting. The notification carries the one-time code, as an approval's does.
pub const TITLE_ANSWER: &str = "Waiting for your answer";
/// The plaintext died with the last process, so a new one was issued.
pub const TITLE_ANSWER_REPLACED: &str = "Answer code replaced";
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

/// Open waits grouped by run, one read. The value is the WAITING ON label.
pub fn open_by_run(store: &Store) -> StoreResult<HashMap<String, String>> {
    let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
    for wait in list_open(store)? {
        grouped
            .entry(wait.run_id)
            .or_default()
            .push(wait.blocked_on);
    }
    Ok(grouped
        .into_iter()
        .map(|(id, labels)| (id, labels.join(", ")))
        .collect())
}

/// The label for one run's WAITING ON cell, using a list already read by [`open_by_run`].
/// A waiting approval that has not opened a row yet, from a database written before this,
/// still says `your approval`.
pub fn label_for(
    store: &Store,
    run_id: &str,
    open: &HashMap<String, String>,
) -> StoreResult<Option<String>> {
    if let Some(label) = open.get(run_id) {
        return Ok(Some(label.clone()));
    }
    legacy_approval(store, run_id)
}

/// The label for one run's WAITING ON cell. Empty when the run is not blocked on a person.
pub fn waiting_on(store: &Store, run_id: &str) -> StoreResult<Option<String>> {
    label_for(store, run_id, &open_by_run(store)?)
}

fn legacy_approval(store: &Store, run_id: &str) -> StoreResult<Option<String>> {
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
    Ok(legacy.then(|| BLOCKED_APPROVAL.to_owned()))
}

/// Record an approval from a terminal or a remote caller. The code is required. Nothing
/// here moves the step or sends a notification. The engine applies the row once,
/// including after a restart. A remote crate uses this, not the window path.
#[allow(clippy::too_many_arguments)]
pub fn decide_with_code(
    store: &Store,
    run_id: &str,
    step_id: &str,
    decision: &str,
    feedback: Option<&str>,
    actor: &str,
    code: &str,
) -> StoreResult<ApprovalWrite> {
    store.record_terminal_approval(run_id, step_id, decision, feedback, actor, code)
}

/// Record an approval from the desktop window. The person is already there, so no code
/// is asked. A remote crate cannot construct [`DesktopWindow`].
#[cfg(feature = "desktop-window")]
#[allow(clippy::too_many_arguments)]
pub fn decide_in_window(
    store: &Store,
    _window: DesktopWindow,
    run_id: &str,
    step_id: &str,
    decision: &str,
    feedback: Option<&str>,
    actor: &str,
) -> StoreResult<ApprovalWrite> {
    store.record_workflow_approval(run_id, step_id, decision, feedback, actor, None)
}

/// Why an answer was not stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerWrite {
    Recorded,
    Missing,
    NotAQuestion,
    AlreadyAnswered,
    /// The step has left `waiting` (failed, timed out, skipped) while the run is still going.
    NotWaiting,
    /// The run is not running. `status` is the word the row has.
    RunEnded {
        status: String,
    },
    WrongCode,
    CodeLocked,
    NoCode,
    CodeUnusable,
    /// This decision id was already used for a different action, step or note.
    Conflict,
}

/// Store a clarification's answer from a terminal or a remote caller. The code is required.
/// Nothing here types into the session. The engine does that once. A remote crate uses this,
/// not the window path.
pub fn answer_with_code(
    store: &Store,
    run_id: &str,
    step_id: &str,
    text: &str,
    actor: &str,
    code: &str,
) -> StoreResult<AnswerWrite> {
    store.answer_workflow_wait(run_id, step_id, text, actor, Some(code))
}

/// Store a clarification's answer from the desktop window. The person is already there, so
/// no code is asked. A remote crate cannot construct [`DesktopWindow`].
#[cfg(feature = "desktop-window")]
pub fn answer_in_window(
    store: &Store,
    _window: DesktopWindow,
    run_id: &str,
    step_id: &str,
    text: &str,
    actor: &str,
) -> StoreResult<AnswerWrite> {
    store.answer_workflow_wait(run_id, step_id, text, actor, None)
}
