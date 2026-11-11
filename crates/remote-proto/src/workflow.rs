//! Workflow runs, as a signed-in phone may see them, and the decision it may send.
//!
//! The desktop owns the run. This crate only names what crosses the relay. A one-time code
//! is a field of `workflow.decide` and of the challenge reply. It is not a field of a run,
//! a step, an artifact, or a notification.

use serde::{Deserialize, Serialize};

use crate::model::{clip, Cursor, MAX_NOTE, MAX_TEXT};

/// Longest one-time code a request may carry. The codes Yardsort issues are 8 characters.
pub const MAX_CODE_LEN: usize = 64;
/// Longest artifact body a run detail includes, in characters.
pub const MAX_ARTIFACT_PREVIEW: usize = 2_000;

/// What a phone asks to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkflowAction {
    Approve,
    Reject,
    Changes,
    Answer,
    Resume,
}

/// A page of runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunListParams {
    #[serde(default)]
    pub cursor: Option<Cursor>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// One run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunRef {
    pub run_id: String,
}

/// One step of a run. The challenge asks for the code of the attempt and round the phone is showing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepRef {
    pub run_id: String,
    pub step_id: String,
    /// The waiting step's attempt, as the run detail showed it.
    pub attempt: u32,
    /// The open clarification round, or 0 when the step is not asking a question.
    pub round: u32,
}

/// What the phone shows when the attempt or the round moved on before the tap.
pub const STALE_MESSAGE: &str = "This changed. Refresh and read it again.";

/// One decision. `decision_id` makes a retry apply once.
/// `attempt` and `round` are the ones the phone was showing. A later one is refused as `stale`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDecideParams {
    pub run_id: String,
    #[serde(default)]
    pub step_id: Option<String>,
    pub action: WorkflowAction,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    pub decision_id: String,
    /// The waiting step's attempt, as the run detail showed it.
    pub attempt: u32,
    /// The open clarification round, or 0 when the step is not asking a question.
    pub round: u32,
}

impl std::fmt::Debug for WorkflowDecideParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowDecideParams")
            .field("run_id", &self.run_id)
            .field("step_id", &self.step_id)
            .field("action", &self.action)
            .field("note", &self.note)
            .field("code", &self.code.as_ref().map(|_| "[redacted]"))
            .field("decision_id", &self.decision_id)
            .field("attempt", &self.attempt)
            .field("round", &self.round)
            .finish()
    }
}

impl WorkflowDecideParams {
    /// Bounds a well-formed decision must also meet. `None` when it does.
    pub fn invalid(&self) -> Option<&'static str> {
        if crate::ids::check_id(&self.run_id).is_err() {
            return Some("That run id is not one Yardsort uses.");
        }
        if crate::ids::check_id(&self.decision_id).is_err() {
            return Some("That decision id is not one Yardsort uses.");
        }
        if let Some(step) = &self.step_id {
            if crate::ids::check_id(step).is_err() {
                return Some("That step id is not one Yardsort uses.");
            }
        }
        if let Some(note) = &self.note {
            if note.chars().count() > MAX_NOTE {
                return Some("That note is longer than one decision may carry.");
            }
        }
        match self.action {
            WorkflowAction::Resume => {}
            WorkflowAction::Approve
            | WorkflowAction::Reject
            | WorkflowAction::Changes
            | WorkflowAction::Answer => {
                let Some(code) = &self.code else {
                    return Some("A decision needs the one-time code.");
                };
                if !code_ok(code) {
                    return Some("That code is not one Yardsort shows.");
                }
                if self.step_id.is_none() {
                    return Some("Name the step this decision is for.");
                }
            }
        }
        if matches!(
            self.action,
            WorkflowAction::Changes | WorkflowAction::Answer
        ) && self
            .note
            .as_deref()
            .is_none_or(|note| note.trim().is_empty())
        {
            return Some("Say what should change, or give the answer.");
        }
        None
    }
}

fn code_ok(code: &str) -> bool {
    (1..=MAX_CODE_LEN).contains(&code.len())
        && code.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// One run in a list. No code, no path, no artifact body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunSummary {
    pub run_id: String,
    pub name: String,
    pub status: String,
    pub workspace: String,
    pub waiting_on: Option<String>,
    pub blocking: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunPage {
    pub runs: Vec<WorkflowRunSummary>,
    pub next: Option<Cursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepView {
    pub step_id: String,
    pub action: String,
    pub status: String,
    pub attempts: u32,
    pub note: Option<String>,
}

/// An artifact the phone may see. `body` is set only for a relevant document, and it is
/// capped and redacted. A secret file is left out entirely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowArtifactView {
    pub name: String,
    pub source: String,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunDetail {
    pub run_id: String,
    pub name: String,
    pub status: String,
    pub workspace: String,
    pub error: Option<String>,
    pub waiting_on: Option<String>,
    pub blocking: Option<String>,
    /// `approval` or `question` when a person still has to act, otherwise absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_kind: Option<String>,
    pub attempts: u32,
    pub locked: bool,
    /// The waiting step's attempt. A decision for an earlier one is stale. 0 when none is waiting.
    pub attempt: u32,
    /// The open clarification round, or 0 when this step is not asking a question.
    pub round: u32,
    /// The person has decided or answered, and the computer has not applied it yet,
    /// or the run is queued.
    pub resume_eligible: bool,
    pub steps: Vec<WorkflowStepView>,
    pub artifacts: Vec<WorkflowArtifactView>,
}

/// The one-time code for the attempt and round the phone asked about. Sent on the relay to
/// the phone that asked. Never pushed, and never included in a run.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowChallenge {
    pub code: String,
    /// The attempt the code belongs to. The same number the request named.
    pub attempt: u32,
    /// The clarification round the code belongs to, or 0.
    pub round: u32,
}

impl std::fmt::Debug for WorkflowChallenge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowChallenge")
            .field("code", &"[redacted]")
            .field("attempt", &self.attempt)
            .field("round", &self.round)
            .finish()
    }
}

/// What the decision did. A retry of the same id returns the same outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDecision {
    pub outcome: String,
    pub message: String,
}

fn hides_local(text: &str) -> bool {
    text.contains('/') || text.contains('\\') || text.contains("://")
}

/// A run error the phone may see. A path or a URL is replaced, so a local folder is not sent.
pub fn safe_error(error: &str) -> String {
    if hides_local(error) {
        "The run ended with an error.".to_owned()
    } else {
        clip(error, MAX_TEXT)
    }
}

/// A step note the phone may see. The same path and URL rule as [`safe_error`].
pub fn safe_note(note: &str) -> String {
    if hides_local(note) {
        "This note was left out.".to_owned()
    } else {
        clip(note, MAX_NOTE)
    }
}

/// Drop a line that is carrying a secret. The rest of the body stays.
pub fn redact_preview(body: &str) -> String {
    let mut out = String::new();
    for line in body.lines() {
        if !out.is_empty() {
            out.push('\n');
        }
        if secret_line(line) {
            out.push_str("[redacted]");
        } else {
            out.push_str(line);
        }
    }
    clip(&out, MAX_ARTIFACT_PREVIEW)
}

fn secret_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower.contains("begin") && lower.contains("private key") {
        return true;
    }
    if line.contains("ghp_")
        || line.contains("github_pat_")
        || line.contains("sk-")
        || line.contains("xoxb-")
        || line.contains("AKIA")
    {
        return true;
    }
    for key in ["api_key", "secret", "token", "password"] {
        if let Some(at) = lower.find(key) {
            let rest = lower[at + key.len()..].trim_start();
            if rest.starts_with('=') || rest.starts_with(':') {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_one_time_code() {
        let decide = WorkflowDecideParams {
            run_id: "run-1".into(),
            step_id: Some("sign-off".into()),
            action: WorkflowAction::Approve,
            note: None,
            code: Some("ABCD2345".into()),
            decision_id: "d-1".into(),
            attempt: 1,
            round: 0,
        };
        let shown = format!("{decide:?}");
        assert!(!shown.contains("ABCD2345"));
        assert!(shown.contains("[redacted]"));
        let challenge = WorkflowChallenge {
            code: "ABCD2345".into(),
            attempt: 1,
            round: 2,
        };
        assert!(!format!("{challenge:?}").contains("ABCD2345"));
    }
}
