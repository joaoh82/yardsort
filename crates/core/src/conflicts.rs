//! Asking a workspace's agent to resolve its pull request's merge conflicts.
//!
//! The agent asked is the one that opened the pull request, as far as Yardsort can tell: the
//! conversation whose process was running in the workspace when the forge says it was opened.
//! Failing that, the conversation the workspace's task was given to, and failing that the newest
//! one there. It already knows the work, which is what makes it the right one to merge the base
//! into it; a stranger would have to learn it first.
//!
//! Nothing here reads what an agent printed. The choice rests on the session and run records and
//! the forge's timestamp; the message is typed in like any other prompt.

use crate::forge::PullRequest;
use crate::store::{RunRow, SessionRow};

/// How far the forge's clock and this machine's may disagree, either way.
const CLOCK_SKEW_MS: i64 = 2 * 60 * 1000;

/// The conversation to ask, out of a workspace's `sessions` and `runs` (both newest first).
///
/// `task_session` is the conversation the task was given to (see
/// [`Store::task_session`](crate::store::Store::task_session)); `opened_at` is when the pull
/// request was opened, in epoch milliseconds, when the forge said.
pub fn author<'a>(
    sessions: &'a [SessionRow],
    runs: &[RunRow],
    task_session: Option<&str>,
    opened_at: Option<i64>,
) -> Option<&'a SessionRow> {
    let by_id = |id: &str| sessions.iter().find(|session| session.id == id);
    let running_then = opened_at.and_then(|at| {
        runs.iter()
            .filter(|run| run.kind == "harness")
            .filter(|run| {
                run.started_at <= at + CLOCK_SKEW_MS
                    && run.ended_at.is_none_or(|ended| ended + CLOCK_SKEW_MS >= at)
            })
            .find_map(|run| run.session_id.as_deref().and_then(by_id))
    });
    running_then
        .or_else(|| task_session.and_then(by_id))
        .or_else(|| sessions.first())
}

/// The message the agent is sent.
///
/// `checked_out` is the branch the workspace is on, when it is not the pull request's. `task` is
/// given to a conversation that has not seen it: one started fresh because the original could
/// not be continued.
pub fn prompt(
    pr: &PullRequest,
    base: &str,
    checked_out: Option<&str>,
    task: Option<&str>,
) -> String {
    let branch = &pr.branch;
    let mut text = format!(
        "Pull request #{number} ({url}), \"{title}\", has merge conflicts with `{base}`, so it \
         cannot be merged.\n\n",
        number = pr.number,
        url = pr.url,
        title = pr.title,
    );
    if let Some(task) = task.map(str::trim).filter(|task| !task.is_empty()) {
        text.push_str(&format!("The task it was opened for:\n\n{task}\n\n"));
    }
    if let Some(other) = checked_out.filter(|other| *other != branch) {
        text.push_str(&format!(
            "This worktree has `{other}` checked out, not `{branch}`. Switch to `{branch}` only \
             if nothing uncommitted would be lost; otherwise stop and ask me.\n\n"
        ));
    }
    text.push_str(&format!(
        "Fetch the latest `{base}` and merge it into `{branch}`. Resolve each conflict so that \
         what both sides meant is kept, run the project's checks, commit the merge and push. Do \
         not rebase or force-push: the pull request's history stays as it is. If a conflict \
         needs a decision the code and the task cannot settle, stop and ask me."
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, started_at: i64) -> SessionRow {
        SessionRow {
            id: id.into(),
            workspace_id: "w".into(),
            harness_id: "claude".into(),
            model: None,
            effort: None,
            harness_session_id: None,
            title: id.into(),
            forked_from: None,
            running: false,
            exit_code: Some(0),
            pty_session_id: None,
            started_at,
            ended_at: None,
        }
    }

    fn run(session: Option<&str>, kind: &str, started_at: i64, ended_at: Option<i64>) -> RunRow {
        RunRow {
            id: format!("run-{started_at}"),
            workspace_id: "w".into(),
            session_id: session.map(str::to_owned),
            kind: kind.into(),
            harness_id: Some("claude".into()),
            harness_session_id: None,
            pty_session_id: None,
            launched_by: "app".into(),
            started_at,
            ended_at,
            exit_code: None,
            end_reason: None,
            collection: "off".into(),
        }
    }

    const MIN: i64 = 60_000;

    #[test]
    fn the_agent_running_when_it_was_opened_is_asked() {
        // Newest first, as the store lists them.
        let sessions = [
            session("later", 300 * MIN),
            session("opener", 100 * MIN),
            session("task", 10 * MIN),
        ];
        let runs = [
            run(Some("later"), "harness", 300 * MIN, None),
            run(None, "shell", 150 * MIN, None),
            run(Some("opener"), "harness", 100 * MIN, Some(210 * MIN)),
            run(Some("task"), "harness", 10 * MIN, Some(50 * MIN)),
        ];
        let at = |opened| author(&sessions, &runs, Some("task"), opened).map(|s| s.id.as_str());
        assert_eq!(at(Some(200 * MIN)), Some("opener"));
        // Its process ended a moment before the forge's clock says the pull request was opened.
        assert_eq!(at(Some(211 * MIN)), Some("opener"));
        // Nobody was running then: the conversation that was given the task.
        assert_eq!(at(Some(250 * MIN)), Some("task"));
        assert_eq!(at(None), Some("task"));
        // No task either: the newest conversation.
        assert_eq!(
            author(&sessions, &runs, None, Some(250 * MIN)).map(|s| s.id.as_str()),
            Some("later")
        );
        assert!(author(&[], &runs, None, Some(200 * MIN)).is_none());
    }

    fn pr() -> PullRequest {
        let row = serde_json::json!({
            "number": 9, "url": "https://github.com/o/r/pull/9", "title": "Fix login",
            "headRefName": "ys/fix", "state": "OPEN",
        });
        crate::forge::pull_request(&row).unwrap()
    }

    #[test]
    fn the_message_names_the_pull_request_and_says_never_to_force_push() {
        let text = prompt(&pr(), "main", Some("ys/fix"), None);
        assert!(text.starts_with("Pull request #9 (https://github.com/o/r/pull/9), \"Fix login\""));
        assert!(text.contains("merge it into `ys/fix`"));
        assert!(text.contains("Do not rebase or force-push"));
        assert!(!text.contains("task it was opened for"));
        assert!(!text.contains("checked out"), "on its own branch already");

        let fresh = prompt(&pr(), "main", Some("ys/other"), Some("  Make login work  "));
        assert!(fresh.contains("The task it was opened for:\n\nMake login work\n\n"));
        assert!(fresh.contains("This worktree has `ys/other` checked out, not `ys/fix`"));
    }
}
