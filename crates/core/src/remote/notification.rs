//! `notification.list` and `notification.seen`: this host's Notifications list
//! ([`crate::notifications`]), for a device.
//!
//! The list is kept by whatever runs this host's terminals, with a
//! [`Keeper`](crate::notifications::Keeper), which is lent here. Without it the operations are
//! not answered, and a device shows nothing of the list, as for a host from before it.
//!
//! What is said keeps to the grant: only rows of projects it reaches, and of a workspace only
//! one on show. A row's line is what watching its agent's terminal shows, which the scope these
//! need already allows. What a workflow asks is said only to a grant that may read workflows.
//! `notification.follow` is the channel's own to answer ([`super::serve::Visitor`]).

use std::collections::HashSet;

use remote_proto::notification::{
    NotificationKind, NotificationPage, NotificationPart, NotificationSeenParams,
    NotificationSummary, MAX_LINE, MAX_NOTIFICATIONS,
};
use remote_proto::{clip, ErrorCode, Grant, ProjectId, RemoteError, Scope, MAX_LABEL, MAX_TEXT};

use super::{encoded_len, fitting, local_id, unreadable, Answer, HostFacade};
use crate::notifications::{self, Notification};

impl HostFacade<'_> {
    pub(super) fn notification_list(&self, grant: &Grant) -> Answer<NotificationPage> {
        let rows = self.reachable_notifications(grant)?;
        let count = |part: notifications::NotificationPart| {
            u32::try_from(rows.iter().filter(|row| row.part == part).count()).unwrap_or(u32::MAX)
        };
        let needs = count(notifications::NotificationPart::Needs);
        let finished = count(notifications::NotificationPart::Finished);
        let workflows = grant.allows(Scope::WorkflowRead);
        let listed = rows.len();
        let mut summaries = rows
            .into_iter()
            .take(MAX_NOTIFICATIONS)
            .map(|row| summary(row, workflows))
            .collect::<Answer<Vec<_>>>()?;
        let page = |notifications: Vec<NotificationSummary>, more: bool| NotificationPage {
            notifications,
            needs,
            finished,
            more,
        };
        // What still waits is first in the list, so what a frame cannot carry is the oldest of
        // what left.
        let fits = fitting(summaries.len(), |n| {
            encoded_len(&page(summaries[..n].to_vec(), true))
        })?;
        summaries.truncate(fits);
        let more = summaries.len() < listed;
        Ok(page(summaries, more))
    }

    pub(super) fn notifications_seen(
        &self,
        grant: &Grant,
        params: &NotificationSeenParams,
    ) -> Answer<()> {
        let keeper = self.notifications.ok_or_else(not_here)?;
        // Only what this grant is shown can be marked by it. An id it is not shown is passed
        // over without a word, as one that is gone: ids cannot be probed.
        let reachable: Vec<String> = self
            .reachable_notifications(grant)?
            .into_iter()
            .filter(|row| row.part == notifications::NotificationPart::Finished)
            .map(|row| row.id)
            .collect();
        let ids: Vec<String> = if params.all {
            reachable
        } else {
            let asked: HashSet<&str> = params.ids.iter().map(String::as_str).collect();
            reachable
                .into_iter()
                .filter(|id| asked.contains(id.as_str()))
                .collect()
        };
        keeper.seen(self.store, &ids).map_err(|_| unreadable())
    }

    /// The list, as much of it as `grant` is shown: the rows of the projects it reaches.
    fn reachable_notifications(&self, grant: &Grant) -> Answer<Vec<Notification>> {
        let keeper = self.notifications.ok_or_else(not_here)?;
        let conversations = self
            .conversations
            .map(|conversations| (&conversations.sources, conversations.data_dir.as_path()));
        let rows = keeper
            .list(self.store, conversations, crate::store::now_ms())
            .map_err(|_| unreadable())?;
        let mut shown = Vec::with_capacity(rows.len());
        for row in rows {
            let project: ProjectId = local_id(&row.project_id)?;
            if grant.reaches(&project) {
                shown.push(row);
            }
        }
        Ok(shown)
    }
}

fn summary(row: Notification, workflows: bool) -> Answer<NotificationSummary> {
    use notifications::NotificationKind as Kind;
    let kind = match row.kind {
        Kind::AgentAsks => NotificationKind::AgentAsks,
        Kind::AgentFinished => NotificationKind::AgentFinished,
        Kind::AgentFailed => NotificationKind::AgentFailed,
        Kind::AgentStopped => NotificationKind::AgentStopped,
        Kind::WorkflowApproval => NotificationKind::WorkflowApproval,
        Kind::WorkflowQuestion => NotificationKind::WorkflowQuestion,
        Kind::WorkflowSucceeded => NotificationKind::WorkflowSucceeded,
        Kind::WorkflowFailed => NotificationKind::WorkflowFailed,
        Kind::TestWaits => NotificationKind::TestWaits,
        Kind::ScheduleRefused => NotificationKind::ScheduleRefused,
        Kind::ScheduleNoWorkspace => NotificationKind::ScheduleNoWorkspace,
        Kind::TriggerRefused => NotificationKind::TriggerRefused,
        Kind::TriggerCapped => NotificationKind::TriggerCapped,
        Kind::TriggerNoWorkspace => NotificationKind::TriggerNoWorkspace,
    };
    let of_workflow = matches!(
        kind,
        NotificationKind::WorkflowApproval
            | NotificationKind::WorkflowQuestion
            | NotificationKind::WorkflowSucceeded
            | NotificationKind::WorkflowFailed
    );
    let line = |text: Option<String>| text.map(|text| clip(&text, MAX_LINE));
    Ok(NotificationSummary {
        id: row.id,
        kind,
        part: match row.part {
            notifications::NotificationPart::Needs => NotificationPart::Needs,
            notifications::NotificationPart::Finished => NotificationPart::Finished,
            notifications::NotificationPart::Earlier => NotificationPart::Earlier,
        },
        project: local_id(&row.project_id)?,
        project_name: clip(&row.project_name, MAX_TEXT),
        workspace: row.workspace_id.as_deref().map(local_id).transpose()?,
        workspace_name: clip(&row.workspace_name, MAX_TEXT),
        branch: row.branch.map(|branch| clip(&branch, MAX_TEXT)),
        session: row.session_id.as_deref().map(local_id).transpose()?,
        title: clip(&row.title, MAX_TEXT),
        harness: row.harness_id.map(|harness| clip(&harness, MAX_LABEL)),
        terminal: row.terminal_id.as_deref().map(local_id).transpose()?,
        run: row.subject_id.filter(|_| of_workflow),
        step: row.step_id.filter(|_| of_workflow),
        at: row.at,
        settled_at: row.settled_at,
        worked_ms: row.worked_ms,
        exit_code: row.exit_code.and_then(|code| i32::try_from(code).ok()),
        command: line(row.command),
        // What a workflow asks is the workflow's to show: only to a grant that may read them.
        said: line(row.said).filter(|_| !of_workflow || workflows),
        note: line(row.note).filter(|_| !of_workflow || workflows),
    })
}

fn not_here() -> RemoteError {
    RemoteError::new(
        ErrorCode::Unsupported,
        "The Notifications list is not kept on this computer.",
    )
}
