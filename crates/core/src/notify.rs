//! Notices this computer will hand to the server for the phone.
//!
//! Each one is a category, a kind and a stable event key. The words the phone shows are chosen
//! on the server. A one-time approval or answer code is never put in a key or stored here.
//! The first look at a task list or a review list records what is already there and sends nothing.

use remote_proto::notify::{NoticeCategory, NoticeSubmission, FLUSH_CAP, NOTICE_TTL_MS};
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

/// Sent, dropped and expired outbox rows, and seen items, older than this are deleted.
const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;

use crate::forge::{Checks, Mergeable, PullRequest, ReviewState};
use crate::store::{Store, StoreResult};
use crate::tasks::{ClosedAs, Task, TaskState};

/// One notice waiting in the local outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeDraft {
    pub event_key: String,
    pub category: NoticeCategory,
    pub kind: &'static str,
    /// The run and step a workflow notice opens. Never a one-time code.
    pub run_id: Option<String>,
    pub step_id: Option<String>,
    /// The workspace and terminal an agent notice opens.
    pub workspace_id: Option<String>,
    pub terminal_id: Option<String>,
}

impl NoticeDraft {
    pub fn submission(&self) -> NoticeSubmission {
        NoticeSubmission {
            event_key: self.event_key.clone(),
            category: self.category,
            kind: self.kind.to_owned(),
            run_id: self.run_id.clone(),
            step_id: self.step_id.clone(),
            workspace_id: self.workspace_id.clone(),
            terminal_id: self.terminal_id.clone(),
        }
    }
}

/// A row the host can submit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedNotice {
    pub event_key: String,
    pub category: String,
    pub kind: String,
    pub run_id: Option<String>,
    pub step_id: Option<String>,
    pub workspace_id: Option<String>,
    pub terminal_id: Option<String>,
}

/// Keep `draft` until the server has accepted it. A key already waiting, sent, or dropped is
/// left as it is, so a retry does not queue a second copy.
pub fn enqueue(store: &Store, draft: &NoticeDraft, now: i64) -> StoreResult<bool> {
    enqueue_on(&store.conn(), draft, now)
}

fn enqueue_on(conn: &rusqlite::Connection, draft: &NoticeDraft, now: i64) -> StoreResult<bool> {
    let written = conn.execute(
        "INSERT OR IGNORE INTO mobile_notice_outbox
         (event_key, category, kind, created_at, expires_at, state, run_id, step_id,
          workspace_id, terminal_id)
         VALUES (?1, ?2, ?3, ?4, ?5, 'queued', ?6, ?7, ?8, ?9)",
        params![
            draft.event_key,
            category_name(draft.category),
            draft.kind,
            now,
            now.saturating_add(NOTICE_TTL_MS),
            draft.run_id,
            draft.step_id,
            draft.workspace_id,
            draft.terminal_id,
        ],
    )?;
    Ok(written > 0)
}

fn category_name(category: NoticeCategory) -> &'static str {
    match category {
        NoticeCategory::Task => "task",
        NoticeCategory::Review => "review",
        NoticeCategory::Failure => "failure",
        NoticeCategory::DesktopUpdate => "desktopUpdate",
        NoticeCategory::Workflow => "workflow",
        NoticeCategory::Agent => "agent",
    }
}

/// Queued notices that are still inside their day, at most [`FLUSH_CAP`], oldest first.
/// Older ones are marked expired. Anything past the cap is marked dropped, so a reconnect
/// does not hand the server the whole backlog.
pub fn pending(store: &Store, now: i64) -> StoreResult<Vec<QueuedNotice>> {
    let conn = store.conn();
    prune_outbox(&conn, now)?;
    conn.execute(
        "UPDATE mobile_notice_outbox SET state = 'expired'
         WHERE state = 'queued' AND expires_at <= ?1",
        [now],
    )?;
    let mut stmt = conn.prepare(
        "SELECT event_key FROM mobile_notice_outbox
         WHERE state = 'queued' ORDER BY created_at DESC, event_key DESC",
    )?;
    let keys: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    drop(stmt);
    if keys.len() > FLUSH_CAP {
        tracing::warn!(
            count = keys.len() - FLUSH_CAP,
            "the local notice queue dropped notices past the flush cap"
        );
        for key in &keys[FLUSH_CAP..] {
            conn.execute(
                "UPDATE mobile_notice_outbox SET state = 'dropped'
                 WHERE event_key = ?1 AND state = 'queued'",
                [key],
            )?;
        }
    }
    let keep: Vec<&String> = keys.iter().take(FLUSH_CAP).rev().collect();
    let mut ready = Vec::with_capacity(keep.len());
    for key in keep {
        if let Some(row) = conn
            .query_row(
                "SELECT event_key, category, kind, run_id, step_id, workspace_id, terminal_id
                 FROM mobile_notice_outbox
                 WHERE event_key = ?1 AND state = 'queued'",
                [key],
                |row| {
                    Ok(QueuedNotice {
                        event_key: row.get(0)?,
                        category: row.get(1)?,
                        kind: row.get(2)?,
                        run_id: row.get(3)?,
                        step_id: row.get(4)?,
                        workspace_id: row.get(5)?,
                        terminal_id: row.get(6)?,
                    })
                },
            )
            .optional()?
        {
            ready.push(row);
        }
    }
    Ok(ready)
}

/// `sent` once the server has accepted it, or `dropped` when the server refused it for good.
pub fn settle(store: &Store, event_key: &str, sent: bool) -> StoreResult<()> {
    let state = if sent { "sent" } else { "dropped" };
    store.conn().execute(
        "UPDATE mobile_notice_outbox SET state = ?2
         WHERE event_key = ?1 AND state = 'queued'",
        params![event_key, state],
    )?;
    Ok(())
}

fn seen(conn: &rusqlite::Connection, key: &str) -> StoreResult<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT stamp FROM mobile_notice_seen WHERE item_key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?)
}

fn remember(conn: &rusqlite::Connection, key: &str, stamp: &str, now: i64) -> StoreResult<()> {
    conn.execute(
        "INSERT INTO mobile_notice_seen (item_key, stamp, seen_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(item_key) DO UPDATE SET stamp = excluded.stamp, seen_at = excluded.seen_at",
        params![key, stamp, now],
    )?;
    Ok(())
}

/// Sent, dropped and expired outbox rows older than a week. A desktop update that was sent is
/// removed here, so the same version can be queued again: that later notice is a reminder.
fn prune_outbox(conn: &rusqlite::Connection, now: i64) -> StoreResult<()> {
    let before = now.saturating_sub(WEEK_MS);
    conn.execute(
        "DELETE FROM mobile_notice_outbox
         WHERE state IN ('sent', 'dropped', 'expired') AND created_at < ?1",
        [before],
    )?;
    Ok(())
}

/// Seen rows for `prefix` that this observation did not refresh. Items still in the list were
/// just given a new `seen_at`, so only ones no longer listed are removed.
fn prune_seen(conn: &rusqlite::Connection, prefix: &str, now: i64) -> StoreResult<()> {
    let before = now.saturating_sub(WEEK_MS);
    conn.execute(
        "DELETE FROM mobile_notice_seen WHERE item_key LIKE ?1 AND seen_at < ?2",
        params![format!("{prefix}%"), before],
    )?;
    Ok(())
}

/// An item this computer has not seen is recorded and sends nothing. A later change of stamp is
/// progress, and a task that was already seen and then closed as completed is completion.
pub fn observe_tasks(store: &Store, tasks: &[Task], now: i64) -> StoreResult<Vec<NoticeDraft>> {
    let mut out = Vec::new();
    let conn = store.conn();
    prune_outbox(&conn, now)?;
    for task in tasks {
        let Some((slug, number)) = github_slug(&task.url) else {
            continue;
        };
        if number == 0 {
            continue;
        }
        let item = format!("task:{slug}:{number}");
        let stamp = piece(&task.updated_at, 40);
        let known = seen(&conn, &item)?;
        let completed =
            task.state == TaskState::Closed && task.closed_as == Some(ClosedAs::Completed);
        if known.is_none() {
            remember(&conn, &item, &stamp, now)?;
            if completed {
                remember(&conn, &format!("{item}:complete"), "1", now)?;
            }
            continue;
        }
        // Refresh seen_at even when nothing changed, so a week of quiet does not look like a
        // first sighting the next time the stamp moves or the task finishes.
        if known.as_deref() != Some(stamp.as_str()) {
            if let Some(draft) = task_progress(&slug, number, &stamp) {
                if enqueue_on(&conn, &draft, now)? {
                    out.push(draft);
                }
            }
        }
        remember(&conn, &item, &stamp, now)?;
        if completed {
            let marker = format!("{item}:complete");
            if seen(&conn, &marker)?.is_none() {
                if let Some(draft) = task_complete(&slug, number) {
                    if enqueue_on(&conn, &draft, now)? {
                        out.push(draft);
                    }
                }
            }
            remember(&conn, &marker, "1", now)?;
        }
    }
    prune_seen(&conn, "task:", now)?;
    Ok(out)
}

/// A pull request this computer has not seen is recorded and sends nothing. A reason that
/// appears on one it has already seen is one notice.
pub fn observe_reviews(
    store: &Store,
    requests: &[PullRequest],
    now: i64,
) -> StoreResult<Vec<NoticeDraft>> {
    let mut out = Vec::new();
    let conn = store.conn();
    prune_outbox(&conn, now)?;
    for pr in requests {
        if pr.state != crate::forge::PullRequestState::Open {
            continue;
        }
        let Some((slug, number)) = github_slug(&pr.url) else {
            continue;
        };
        let marker = format!("review:{slug}:{number}");
        let known = seen(&conn, &marker)?.is_some();
        if !known {
            remember(&conn, &marker, "1", now)?;
        }
        for reason in attention(pr) {
            let item = format!("{marker}:{reason}");
            let already = seen(&conn, &item)?.is_some();
            if known && !already {
                if let Some(draft) = review_attention(&slug, number, reason) {
                    if enqueue_on(&conn, &draft, now)? {
                        out.push(draft);
                    }
                }
            }
            remember(&conn, &item, "1", now)?;
        }
        if known {
            remember(&conn, &marker, "1", now)?;
        }
    }
    prune_seen(&conn, "review:", now)?;
    Ok(out)
}

/// A desktop update of `version`. The same version from another computer shares the key.
pub fn observe_desktop_update(store: &Store, version: &str, now: i64) -> StoreResult<bool> {
    let Some(draft) = desktop_update(version) else {
        return Ok(false);
    };
    enqueue(store, &draft, now)
}

pub fn task_progress(slug: &str, number: u64, stamp: &str) -> Option<NoticeDraft> {
    draft(
        NoticeCategory::Task,
        "progress",
        &format!("{slug}:{number}:progress:{stamp}"),
    )
}

pub fn task_complete(slug: &str, number: u64) -> Option<NoticeDraft> {
    draft(
        NoticeCategory::Task,
        "complete",
        &format!("{slug}:{number}:complete"),
    )
}

pub fn review_attention(slug: &str, number: u64, reason: &str) -> Option<NoticeDraft> {
    draft(
        NoticeCategory::Review,
        "attention",
        &format!("{slug}:{number}:{reason}"),
    )
}

/// A run that ended failed. `ended_at` is when, as the run's row has it: a run continued after
/// a failure keeps its id, and one that fails again is a new failure to be told about, not a
/// retry of the first notice. The same failure asked about twice still has one key.
pub fn workflow_failure(run_id: &str, status: &str, ended_at: i64) -> Option<NoticeDraft> {
    if status != "failed" {
        return None;
    }
    draft(
        NoticeCategory::Failure,
        "failed",
        &format!("workflow:{}:{ended_at}", piece(run_id, 80)),
    )
}

pub fn desktop_update(version: &str) -> Option<NoticeDraft> {
    draft(
        NoticeCategory::DesktopUpdate,
        "available",
        &piece(version, 40),
    )
}

pub fn workflow_question(run_id: &str, step_id: &str, opened_at: i64) -> Option<NoticeDraft> {
    about(
        draft(
            NoticeCategory::Workflow,
            "question",
            &format!(
                "question:{}:{}:{opened_at}",
                piece(run_id, 80),
                piece(step_id, 80)
            ),
        ),
        run_id,
        step_id,
    )
}

pub fn workflow_approval(run_id: &str, step_id: &str, opened_at: i64) -> Option<NoticeDraft> {
    about(
        draft(
            NoticeCategory::Workflow,
            "approval",
            &format!(
                "approval:{}:{}:{opened_at}",
                piece(run_id, 80),
                piece(step_id, 80)
            ),
        ),
        run_id,
        step_id,
    )
}

/// What ends a wait for the whole run: a person typed, or the turn or session moved on.
const RUN_ANSWERED: [&str; 4] = [
    "prompt.submitted",
    "turn.completed",
    "turn.failed",
    "session.ended",
];

/// What ends a wait for one request: the tool it asked about ran or failed, or the request was
/// settled. With tools running side by side, another tool finishing is not an answer, so these
/// count only when they name the request's tool call or permission — or when one side names
/// neither, and there is nothing to match them by.
const REQUEST_ANSWERED: [&str; 3] = ["tool.completed", "tool.failed", "approval.resolved"];

/// An agent in `run` asked for a permission at `at` (the event `approval.requested`, which
/// every harness that can ask reports from its own hooks, never from what its terminal shows).
/// One notice per wait: a request while an earlier one in the same run is still unanswered is
/// coalesced into it. Only a computer signed in to a server queues one, and only for a run whose
/// terminal is still there to open.
pub fn agent_approval(
    store: &Store,
    run: &crate::store::RunRow,
    at: i64,
    now: i64,
) -> StoreResult<Option<NoticeDraft>> {
    if run.ended_at.is_some() || store.remote_host()?.is_none() {
        return Ok(None);
    }
    let Some(terminal) = run.pty_session_id.as_deref() else {
        return Ok(None);
    };
    let conn = store.conn();
    prune_outbox(&conn, now)?;
    let item = format!("agent:{}", piece(&run.id, 80));
    if let Some(since) = seen(&conn, &item)?.and_then(|stamp| stamp.parse::<i64>().ok()) {
        let answered = wait_answered(&conn, &run.id, since)?;
        if !answered {
            return Ok(None);
        }
    }
    let Some(mut draft) = draft(
        NoticeCategory::Agent,
        "approval",
        &format!("{}:approval:{at}", piece(&run.id, 80)),
    ) else {
        return Ok(None);
    };
    if subject_ok(&run.workspace_id) && subject_ok(terminal) {
        draft.workspace_id = Some(run.workspace_id.clone());
        draft.terminal_id = Some(terminal.to_owned());
    }
    remember(&conn, &item, &at.to_string(), now)?;
    prune_seen(&conn, "agent:", now)?;
    Ok(enqueue_on(&conn, &draft, now)?.then_some(draft))
}

/// Whether anything since `since` — when the run's current wait began — answered it. The wait's
/// requests are the `approval.requested` events from then on; see [`REQUEST_ANSWERED`].
fn wait_answered(conn: &rusqlite::Connection, run_id: &str, since: i64) -> StoreResult<bool> {
    let ids = |payload: &str| {
        let payload: serde_json::Value = serde_json::from_str(payload).unwrap_or_default();
        let id = |key: &str| {
            payload
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        (id("toolUseId"), id("permissionId"))
    };
    let mut statement = conn.prepare(&format!(
        "SELECT kind, payload FROM agent_events
         WHERE run_id = ?1 AND occurred_at >= ?2 AND kind IN ('approval.requested', {})
         ORDER BY occurred_at, seq",
        RUN_ANSWERED
            .iter()
            .chain(&REQUEST_ANSWERED)
            .map(|kind| format!("'{kind}'"))
            .collect::<Vec<_>>()
            .join(", ")
    ))?;
    let events = statement
        .query_map(params![run_id, since], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut asked: Vec<(Option<String>, Option<String>)> = Vec::new();
    for (kind, payload) in events {
        if kind == "approval.requested" {
            asked.push(ids(&payload));
        } else if RUN_ANSWERED.contains(&kind.as_str()) {
            return Ok(true);
        } else {
            let (tool, permission) = ids(&payload);
            let matches = |(asked_tool, asked_permission): &(Option<String>, Option<String>)| {
                let unnamed = asked_tool.is_none() && asked_permission.is_none();
                unnamed
                    || (tool.is_some() && tool == *asked_tool)
                    || (permission.is_some() && permission == *asked_permission)
            };
            if (tool.is_none() && permission.is_none()) || asked.iter().any(matches) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Attach the run and step when both are ids and neither is a one-time code. Otherwise the
/// notice still goes out, without a subject the phone could open.
fn about(draft: Option<NoticeDraft>, run_id: &str, step_id: &str) -> Option<NoticeDraft> {
    draft.map(|mut draft| {
        if subject_ok(run_id) && subject_ok(step_id) {
            draft.run_id = Some(run_id.to_owned());
            draft.step_id = Some(step_id.to_owned());
        }
        draft
    })
}

fn subject_ok(text: &str) -> bool {
    remote_proto::ids::check_id(text).is_ok() && !looks_like_code(text)
}

fn looks_like_code(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 8
        && bytes
            .iter()
            .all(|byte| b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".contains(byte))
}

/// The category prefix plus the hex SHA-256 of `rest`. The repository, the number and the run
/// stay out of the key the server stores. Two computers hash the same rest to the same key.
fn event_key(category: NoticeCategory, rest: &str) -> String {
    let digest = Sha256::digest(rest.as_bytes());
    let mut key = category.prefix().to_owned();
    for byte in digest {
        key.push_str(&format!("{byte:02x}"));
    }
    key
}

fn draft(category: NoticeCategory, kind: &'static str, rest: &str) -> Option<NoticeDraft> {
    let event_key = event_key(category, rest);
    remote_proto::notify::valid_event_key(category, &event_key).then_some(NoticeDraft {
        event_key,
        category,
        kind,
        run_id: None,
        step_id: None,
        workspace_id: None,
        terminal_id: None,
    })
}

fn attention(pr: &PullRequest) -> Vec<&'static str> {
    let mut reasons = Vec::new();
    if pr.checks == Checks::Failing {
        reasons.push("checks");
    }
    if pr
        .details
        .as_ref()
        .is_some_and(|d| d.mergeable == Mergeable::Conflicting)
    {
        reasons.push("conflicts");
    }
    let changes = pr.details.as_ref().is_some_and(|d| {
        d.review == "CHANGES_REQUESTED"
            || d.reviews
                .iter()
                .any(|r| r.state == ReviewState::ChangesRequested)
    });
    if changes {
        reasons.push("changes");
    }
    if !pr.draft && pr.details.as_ref().is_some_and(|d| d.reviews.is_empty()) {
        reasons.push("unreviewed");
    }
    reasons
}

/// `owner.name` and the issue or pull request number from a GitHub URL.
pub fn github_slug(url: &str) -> Option<(String, u64)> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut parts = rest.split('/');
    let owner = piece(parts.next()?, 80);
    let name = piece(parts.next()?, 80);
    let kind = parts.next()?;
    if kind != "issues" && kind != "pull" && kind != "pulls" {
        return None;
    }
    let number = parts.next()?.split(['#', '?']).next()?.parse().ok()?;
    if owner.is_empty() || name.is_empty() || number == 0 {
        return None;
    }
    Some((format!("{owner}.{name}"), number))
}

/// Lower case letters, digits and `.:_-`. Anything else becomes a single hyphen, so a key
/// stays inside the server's alphabet.
fn piece(raw: &str, max: usize) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '_' | '-') {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.len() >= max {
            break;
        }
    }
    out.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::{
        CheckCounts, Mergeable, PullRequest, PullRequestDetails, PullRequestReview,
        PullRequestState, ReviewState,
    };
    use crate::tasks::{Task, TaskLabel, TaskSourceKind, TaskState};

    fn task(url: &str, updated: &str, closed: Option<ClosedAs>) -> Task {
        Task {
            source: TaskSourceKind::GitHub,
            key: "#91".into(),
            url: url.into(),
            title: "Worktrees on a network drive are slow".into(),
            state: if closed.is_some() {
                TaskState::Closed
            } else {
                TaskState::Open
            },
            closed_as: closed,
            author: Some("grace".into()),
            labels: vec![TaskLabel {
                name: "bug".into(),
                color: "d73a4a".into(),
            }],
            assignees: vec![],
            comments: 2,
            created_at: "2026-10-02T17:43:03Z".into(),
            updated_at: updated.into(),
            needs_answer: true,
            linked_pull_requests: vec![],
        }
    }

    fn details(
        mergeable: Mergeable,
        reviews: Vec<PullRequestReview>,
        review: &str,
    ) -> PullRequestDetails {
        PullRequestDetails {
            base: "main".into(),
            head_oid: "abc".into(),
            base_oid: "def".into(),
            additions: 1,
            deletions: 0,
            review: review.into(),
            updated_at: "2026-10-02T18:00:00Z".into(),
            checks: vec![],
            check_counts: CheckCounts::default(),
            mergeable,
            review_requests: vec![],
            reviews,
            cross_repository: false,
        }
    }

    fn pr(checks: Checks, details: Option<PullRequestDetails>) -> PullRequest {
        PullRequest {
            number: 41,
            url: "https://github.com/Example/widgets/pull/41".into(),
            title: "A secret title must stay off the phone".into(),
            branch: "fix".into(),
            state: PullRequestState::Open,
            draft: false,
            checks,
            details,
            author: Some("grace".into()),
            created_at: Some(0),
        }
    }

    #[test]
    fn a_run_that_fails_again_after_being_continued_is_a_new_notice() {
        let store = Store::in_memory();
        let first = workflow_failure("run-1", "failed", 100).unwrap();
        let again = workflow_failure("run-1", "failed", 250).unwrap();
        assert_ne!(first.event_key, again.event_key);
        assert!(enqueue(&store, &first, 100).unwrap());
        assert!(
            !enqueue(&store, &first, 101).unwrap(),
            "the same failure is still queued once"
        );
        assert!(enqueue(&store, &again, 250).unwrap());
    }

    #[test]
    fn each_category_has_a_stable_key_and_no_code() {
        let (code, _, _) = crate::workflow::code::issue();
        let drafts = [
            task_progress("example.widgets", 91, "2026-10-02t18-00-00z").unwrap(),
            task_complete("example.widgets", 91).unwrap(),
            review_attention("example.widgets", 41, "checks").unwrap(),
            workflow_failure("run-1", "failed", 9).unwrap(),
            desktop_update("1.2.3").unwrap(),
            workflow_question("run-1", "ask", 10).unwrap(),
            workflow_approval("run-1", "approve", 11).unwrap(),
        ];
        let categories = [
            NoticeCategory::Task,
            NoticeCategory::Task,
            NoticeCategory::Review,
            NoticeCategory::Failure,
            NoticeCategory::DesktopUpdate,
            NoticeCategory::Workflow,
            NoticeCategory::Workflow,
        ];
        assert!(workflow_failure("run-1", "succeeded", 9).is_none());
        for (draft, category) in drafts.iter().zip(categories) {
            assert_eq!(draft.category, category);
            assert!(remote_proto::notify::valid_event_key(
                category,
                &draft.event_key
            ));
            let rest = draft.event_key.strip_prefix(category.prefix()).unwrap();
            assert_eq!(rest.len(), 64, "{}", draft.event_key);
            assert!(rest.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(!draft.event_key.contains("example"));
            assert!(!draft.event_key.contains("widgets"));
            let text = format!("{draft:?}{}", draft.event_key);
            assert!(!text.contains(&code), "{text}");
            assert!(!text.contains("secret title"));
            assert!(!text.contains("network drive"));
        }
    }

    #[test]
    fn the_first_look_is_quiet_and_a_later_change_is_one_notice() {
        let store = Store::in_memory();
        let url = "https://github.com/Example/widgets/issues/91";
        let first = task(url, "2026-10-02T18:00:00Z", None);
        assert!(observe_tasks(&store, std::slice::from_ref(&first), 1)
            .unwrap()
            .is_empty());
        assert!(observe_tasks(&store, std::slice::from_ref(&first), 2)
            .unwrap()
            .is_empty());
        let moved = task(url, "2026-10-03T18:00:00Z", None);
        let progress = observe_tasks(&store, std::slice::from_ref(&moved), 3).unwrap();
        assert_eq!(progress.len(), 1);
        assert_eq!(progress[0].kind, "progress");
        assert!(observe_tasks(&store, std::slice::from_ref(&moved), 4)
            .unwrap()
            .is_empty());
        let done = task(url, "2026-10-03T18:00:00Z", Some(ClosedAs::Completed));
        let complete = observe_tasks(&store, &[done], 5).unwrap();
        assert_eq!(complete.len(), 1);
        assert_eq!(complete[0].kind, "complete");
        assert!(observe_tasks(
            &store,
            &[task(url, "2026-10-03T18:00:00Z", Some(ClosedAs::Completed))],
            6
        )
        .unwrap()
        .is_empty());
        let not_planned = task(url, "2026-10-04T18:00:00Z", Some(ClosedAs::NotPlanned));
        let only_progress = observe_tasks(&store, &[not_planned], 7).unwrap();
        assert!(only_progress.iter().all(|d| d.kind != "complete"));
    }

    #[test]
    fn an_open_list_then_old_closed_tasks_sends_nothing() {
        let store = Store::in_memory();
        let open = task(
            "https://github.com/Example/widgets/issues/1",
            "2026-10-02T18:00:00Z",
            None,
        );
        assert!(observe_tasks(&store, std::slice::from_ref(&open), 1)
            .unwrap()
            .is_empty());
        let old = task(
            "https://github.com/Example/widgets/issues/2",
            "2020-01-01T00:00:00Z",
            Some(ClosedAs::Completed),
        );
        assert!(
            observe_tasks(&store, std::slice::from_ref(&old), 2)
                .unwrap()
                .is_empty(),
            "a completed task seen for the first time is recorded quietly"
        );
        assert!(observe_tasks(&store, std::slice::from_ref(&old), 3)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_review_reason_is_notified_once() {
        let store = Store::in_memory();
        let quiet = pr(
            Checks::Passing,
            Some(details(Mergeable::Mergeable, vec![], "")),
        );
        // An empty review list on a pull request that is not a draft is unreviewed, so the
        // first look records that and a second look adds nothing.
        assert!(observe_reviews(&store, std::slice::from_ref(&quiet), 1)
            .unwrap()
            .is_empty());
        assert!(observe_reviews(&store, std::slice::from_ref(&quiet), 2)
            .unwrap()
            .is_empty());
        let failing = pr(
            Checks::Failing,
            Some(details(
                Mergeable::Conflicting,
                vec![PullRequestReview {
                    login: "ada".into(),
                    state: ReviewState::ChangesRequested,
                }],
                "CHANGES_REQUESTED",
            )),
        );
        let found = observe_reviews(&store, std::slice::from_ref(&failing), 3).unwrap();
        let mut keys: Vec<_> = found.iter().map(|d| d.event_key.clone()).collect();
        keys.sort();
        let mut expected = ["checks", "conflicts", "changes"].map(|reason| {
            review_attention("example.widgets", 41, reason)
                .unwrap()
                .event_key
        });
        expected.sort();
        assert_eq!(keys, expected);
        assert!(observe_reviews(&store, std::slice::from_ref(&failing), 4)
            .unwrap()
            .is_empty());
        for row in &found {
            assert!(!row.event_key.contains("secret"));
        }
        let mut later = failing.clone();
        later.number = 99;
        later.url = "https://github.com/Example/widgets/pull/99".into();
        assert!(
            observe_reviews(&store, std::slice::from_ref(&later), 5)
                .unwrap()
                .is_empty(),
            "a pull request seen for the first time is recorded quietly"
        );
    }

    #[test]
    fn the_outbox_keeps_one_row_across_retries_and_drops_what_expired() {
        let store = Store::in_memory();
        let draft = desktop_update("1.2.3").unwrap();
        assert!(enqueue(&store, &draft, 1).unwrap());
        assert!(!enqueue(&store, &draft, 2).unwrap());
        let ready = pending(&store, 3).unwrap();
        assert_eq!(ready.len(), 1);
        for n in 0..20u64 {
            let one = task_progress("example.widgets", n, "stamp").unwrap();
            assert!(enqueue(&store, &one, 10 + i64::try_from(n).unwrap()).unwrap());
        }
        let capped = pending(&store, 40).unwrap();
        assert_eq!(capped.len(), FLUSH_CAP);
        let later = pending(&store, 40 + NOTICE_TTL_MS).unwrap();
        assert!(later.is_empty());
        assert!(pending(&store, 40 + NOTICE_TTL_MS).unwrap().is_empty());
        let kept = desktop_update("9.9.9").unwrap();
        assert!(enqueue(&store, &kept, 1).unwrap());
        settle(&store, &kept.event_key, true).unwrap();
        pending(&store, 1 + WEEK_MS + 1).unwrap();
        assert!(
            enqueue(&store, &kept, 2).unwrap(),
            "a sent desktop update older than a week is queued again, as a reminder"
        );
    }

    #[test]
    fn an_idle_week_does_not_hide_the_next_change() {
        let store = Store::in_memory();
        let url = "https://github.com/Example/widgets/issues/91";
        let first = task(url, "2026-10-02T18:00:00Z", None);
        assert!(observe_tasks(&store, std::slice::from_ref(&first), 1)
            .unwrap()
            .is_empty());
        let week = 1 + 8 * 24 * 60 * 60 * 1000;
        assert!(
            observe_tasks(&store, std::slice::from_ref(&first), week)
                .unwrap()
                .is_empty(),
            "an unchanged task is still known after a week"
        );
        let done = task(url, "2026-10-02T18:00:00Z", Some(ClosedAs::Completed));
        let complete = observe_tasks(&store, std::slice::from_ref(&done), week + 1).unwrap();
        assert_eq!(complete.len(), 1);
        assert_eq!(complete[0].kind, "complete");
    }
}
