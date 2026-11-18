//! A host's Notifications list: what waits for its person, and what finished while they were
//! not looking. Since minor 15.
//!
//! The host keeps the list (`docs/design/48-notifications.md`). `notification.list` gives it to
//! a device, `notification.seen` says its person has looked, and `notification.follow` has the
//! host say when the list changes (the [`crate::Event::NotificationsChanged`] event), so a
//! device reads it again then and not on a clock.
//!
//! A row carries the start of what an agent said last and the command it wants to run: what
//! watching its terminal shows. So all four need `terminal:read`, and a row is given only for a
//! project the grant reaches and a workspace on show. Having read a row is what seeing it
//! means, so marking it seen needs no more than reading it. Nothing here answers what waits: a
//! permission is answered with `approval.answer`, a workflow with `workflow.decide`, a smoke
//! test with `test.decide`, each under its own scope.
//!
//! What a workflow asks is given only to a grant that may read workflows (`workflow:read`).

use serde::{Deserialize, Serialize};

use crate::ids::{ProjectId, SessionRecordId, TerminalId, WorkspaceId};

/// The most rows one reply carries. A list with more says so ([`NotificationPage::more`]).
pub const MAX_NOTIFICATIONS: usize = 100;
/// Longest line of substance on a row, in characters: what an agent said, what a workflow
/// asks, a command, a reason. A longer one is clipped.
pub const MAX_LINE: usize = 240;
/// The most rows one `notification.seen` names.
pub const MAX_SEEN: usize = 200;

/// What a row is about. The names are the host's own for its list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationKind {
    /// An agent asked for a permission.
    #[serde(rename = "agent.asks")]
    AgentAsks,
    /// An agent finished a turn.
    #[serde(rename = "agent.finished")]
    AgentFinished,
    /// An agent's turn failed.
    #[serde(rename = "agent.failed")]
    AgentFailed,
    /// An agent's process exited with an error.
    #[serde(rename = "agent.stopped")]
    AgentStopped,
    /// A workflow run waits for an approval.
    #[serde(rename = "workflow.approval")]
    WorkflowApproval,
    /// A workflow run waits for the answer to a question.
    #[serde(rename = "workflow.question")]
    WorkflowQuestion,
    #[serde(rename = "workflow.succeeded")]
    WorkflowSucceeded,
    #[serde(rename = "workflow.failed")]
    WorkflowFailed,
    /// A smoke test's run waits for its file to be approved.
    #[serde(rename = "test.waits")]
    TestWaits,
    /// A schedule's time came and its run could not be started.
    #[serde(rename = "schedule.refused")]
    ScheduleRefused,
    #[serde(rename = "schedule.noWorkspace")]
    ScheduleNoWorkspace,
    /// A trigger fired and its run could not be started.
    #[serde(rename = "trigger.refused")]
    TriggerRefused,
    /// A trigger has had the runs its limit allows.
    #[serde(rename = "trigger.capped")]
    TriggerCapped,
    #[serde(rename = "trigger.noWorkspace")]
    TriggerNoWorkspace,
    /// A kind from a newer host. A device shows nothing for it.
    #[serde(other)]
    Other,
}

/// Where a row is in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationPart {
    /// Waiting for an answer, or to be put right.
    Needs,
    /// Finished, and not seen yet.
    Finished,
    /// Answered or seen. Kept a week.
    Earlier,
    /// A part from a newer host.
    #[serde(other)]
    Other,
}

/// `notification.list`. Nothing to say today; an object, so that something can be said later.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationListParams {}

/// One row of a host's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSummary {
    /// The host's id for the row: what `notification.seen` names.
    pub id: String,
    pub kind: NotificationKind,
    pub part: NotificationPart,
    pub project: ProjectId,
    pub project_name: String,
    /// The workspace it happened in. Only a schedule or a trigger can be without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
    /// What that workspace is called, or was. Empty for a schedule that makes its own.
    pub workspace_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The agent's conversation, for a row about an agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionRecordId>,
    /// What the row is about within its workspace: the conversation's name, or the workflow's,
    /// the test's, the schedule's or the trigger's.
    pub title: String,
    /// Which agent: `claude`, `codex`…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// The conversation's terminal, while its process runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalId>,
    /// The workflow's run, for a row about one: what `workflow.run.get` names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    /// The step that waits, for a workflow's question or approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    /// When it happened, in epoch milliseconds.
    pub at: i64,
    /// When it was answered or seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<i64>,
    /// How long the agent worked before it finished, when that is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worked_ms: Option<i64>,
    /// What its process exited with, for one that stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// The command an agent wants to run, while its question is open. At most [`MAX_LINE`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// The start of what the agent said last, or of what a workflow asks. At most [`MAX_LINE`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said: Option<String>,
    /// Why, for something that went wrong. At most [`MAX_LINE`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The reply to `notification.list`: what still waits, the newest first, then the newest of
/// what left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPage {
    pub notifications: Vec<NotificationSummary>,
    /// How many rows need their person, over everything the grant reaches: all of them, not
    /// only those in `notifications`.
    pub needs: u32,
    /// How many finished and were not seen, counted the same way.
    pub finished: u32,
    /// The list was cut to what one reply carries: what still waits went first.
    pub more: bool,
}

/// `notification.seen`: these finished rows were looked at, or every one was. A row that waits
/// for an answer is not marked: it leaves when it gets one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSeenParams {
    /// Rows by their ids. At most [`MAX_SEEN`].
    #[serde(default)]
    pub ids: Vec<String>,
    /// Every finished row the grant reaches.
    #[serde(default)]
    pub all: bool,
}

impl NotificationSeenParams {
    pub fn invalid(&self) -> Option<&'static str> {
        if self.ids.len() > MAX_SEEN {
            return Some("That is more rows than one request may name.");
        }
        if self.ids.iter().any(|id| crate::ids::check_id(id).is_err()) {
            return Some("That is not a row's id.");
        }
        if self.ids.is_empty() && !self.all {
            return Some("Name the rows that were seen, or say all of them.");
        }
        None
    }
}

/// `notifications.changed`: the list a device follows is not what it last read. It carries
/// nothing else: the device reads the list again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationsChanged {}
