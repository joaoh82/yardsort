//! What a run made: the pull requests opened while it was going, and what its `report` steps
//! said it came to.
//!
//! A run made a pull request when its workspace has one that was opened during the run's life.
//! Two kinds, both claimed by the workspace and never by reading what an agent printed:
//!
//! - **In the run's own repository**, on the workspace's branch. The app records these each time
//!   it reads the project's pull requests ([`observe_pull_requests`], about once a minute and on
//!   focus), so `ys` and the phone can say what a run made without asking GitHub.
//! - **In another repository**, the ones the workspace tracks (doc 38): an agent's
//!   `gh pr create` there is tracked by the hook, or a person tracks one. They are read from the
//!   tracked list at the time of asking, by when each was tracked.
//!
//! A run that is continued keeps one life from when it was asked for to when it last ended, so a
//! pull request opened between a failure and the continue counts as the run's too.

use serde::Serialize;
use specta::Type;

use crate::forge::{PullRequest, PullRequestState};
use crate::store::{RunPullRequestRow, Store, StoreResult, WorkflowRunRow};

/// How much earlier than the run was asked for a pull request may have been opened and still be
/// the run's: the forge's clock and this machine's are not the same clock.
pub const OPENED_BEFORE_SLACK_MS: i64 = 2 * 60 * 1000;

/// How long after a run ended a pull request opened on its branch is still its own. An agent
/// asked to open one in the last step is often still pushing when the run's last wait ends.
pub const OPENED_AFTER_MS: i64 = 15 * 60 * 1000;

/// How far back the app looks for runs to match a project's pull requests to. A run that ended
/// before this can gain nothing new, and the pull requests it has keep their state as last seen.
pub const OBSERVE_RUNS_FOR_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// A pull request a run made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MadePullRequest {
    /// `owner/name` for one in another repository. `None` for the run's own repository.
    pub repo: Option<String>,
    pub number: u32,
    pub url: String,
    /// As the forge said. `None` for one in another repository, which `ys pr tracked` and the
    /// Pull requests view look up.
    pub title: Option<String>,
    /// `open`, `merged` or `closed`, as the forge said at the app's last look. `None` for one in
    /// another repository.
    pub state: Option<String>,
    pub draft: bool,
    /// When it was opened (the run's own repository) or tracked (another one), in epoch ms.
    #[specta(type = f64)]
    pub opened_at: i64,
}

impl MadePullRequest {
    /// `#12` for the run's own repository, `owner/name#12` for another.
    pub fn reference(&self) -> String {
        match &self.repo {
            Some(repo) => format!("{repo}#{}", self.number),
            None => format!("#{}", self.number),
        }
    }

    /// One line for a person to read: `owner/name#12 Title — https://…`.
    pub fn line(&self) -> String {
        let mut line = self.reference();
        if let Some(title) = self.title.as_deref().filter(|t| !t.is_empty()) {
            line.push(' ');
            line.push_str(title);
        }
        if let Some(state) = self.state.as_deref().filter(|s| *s != "open") {
            line.push_str(&format!(" ({state})"));
        } else if self.draft {
            line.push_str(" (draft)");
        }
        line.push_str(" — ");
        line.push_str(&self.url);
        line
    }
}

/// Whether `at` falls within `run`'s life, give or take the slack either side. A run still going
/// has no end yet.
fn during(run: &WorkflowRunRow, at: i64) -> bool {
    at >= run.created_at - OPENED_BEFORE_SLACK_MS
        && run
            .ended_at
            .is_none_or(|ended| at <= ended + OPENED_AFTER_MS)
}

/// The pull requests `run` made, its own repository's first, each oldest first.
pub fn pull_requests_made(
    store: &Store,
    run: &WorkflowRunRow,
) -> StoreResult<Vec<MadePullRequest>> {
    let mut made: Vec<MadePullRequest> = store
        .run_pull_requests(&run.id)?
        .into_iter()
        .map(|row| MadePullRequest {
            repo: None,
            number: row.number,
            url: row.url,
            title: Some(row.title),
            state: Some(row.state),
            draft: row.draft,
            opened_at: row.opened_at,
        })
        .collect();
    if let Some(workspace_id) = run.workspace_id.as_deref() {
        let mut elsewhere: Vec<MadePullRequest> = store
            .tracked_pull_requests(workspace_id)?
            .into_iter()
            .filter(|row| during(run, row.created_at))
            .map(|row| MadePullRequest {
                repo: Some(short_repo(&row.repo)),
                number: row.number,
                url: row.url,
                title: None,
                state: None,
                draft: false,
                opened_at: row.created_at,
            })
            .collect();
        elsewhere.sort_by_key(|pr| (pr.opened_at, pr.number));
        made.extend(elsewhere);
    }
    Ok(made)
}

/// `owner/name` from a tracked row's `host/owner/name`.
fn short_repo(repo: &str) -> String {
    match repo.split_once('/') {
        Some((_host, rest)) if rest.contains('/') => rest.to_owned(),
        _ => repo.to_owned(),
    }
}

/// The text `{{ run.pull_requests }}` renders to: one line per pull request, or a sentence that
/// says there are none, so a report never has a hole in it.
pub fn pull_requests_text(made: &[MadePullRequest]) -> String {
    if made.is_empty() {
        return "This run opened no pull requests.".to_owned();
    }
    made.iter()
        .map(|pr| format!("- {}", pr.line()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Match a project's pull requests, as just read from the forge, to the runs in its workspaces
/// that opened them: on the workspace's branch, and opened during the run's life. A pull request
/// already matched keeps its row, and its title and state follow what the forge says now.
/// Never asks the forge itself.
pub fn observe_pull_requests(
    store: &Store,
    project_id: &str,
    pull_requests: &[PullRequest],
    now: i64,
) -> StoreResult<()> {
    if pull_requests.is_empty() {
        return Ok(());
    }
    let runs = store.workflow_runs_in_project_since(project_id, now - OBSERVE_RUNS_FOR_MS)?;
    let mut branches = std::collections::BTreeMap::<String, Option<String>>::new();
    for run in &runs {
        let Some(workspace_id) = run.workspace_id.as_deref() else {
            continue;
        };
        if !branches.contains_key(workspace_id) {
            let branch = store.workspace(workspace_id)?.and_then(|ws| ws.branch);
            branches.insert(workspace_id.to_owned(), branch);
        }
        let Some(branch) = branches.get(workspace_id).cloned().flatten() else {
            continue;
        };
        let known: Vec<u32> = store
            .run_pull_requests(&run.id)?
            .iter()
            .map(|row| row.number)
            .collect();
        for pr in pull_requests {
            let mine = known.contains(&pr.number)
                || (pr.branch == branch && pr.created_at.is_some_and(|at| during(run, at)));
            if !mine {
                continue;
            }
            store.record_run_pull_request(&RunPullRequestRow {
                run_id: run.id.clone(),
                number: pr.number,
                url: pr.url.clone(),
                title: pr.title.clone(),
                state: match pr.state {
                    PullRequestState::Open => "open",
                    PullRequestState::Merged => "merged",
                    PullRequestState::Closed => "closed",
                }
                .to_owned(),
                draft: pr.draft,
                opened_at: pr.created_at.unwrap_or(now),
            })?;
        }
    }
    Ok(())
}

/// What one `report` step said, once it succeeded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The step's id.
    pub step: String,
    pub title: Option<String>,
    /// Markdown.
    pub body: String,
}

/// A run's result: what each of its succeeded `report` steps said, in file order. Empty when
/// it has none, or none has run yet. A report step that ran in an earlier attempt and is waiting
/// to run again after a continue is not counted until it succeeds again.
pub fn reports(store: &Store, run_id: &str) -> StoreResult<Vec<Report>> {
    Ok(store
        .workflow_steps(run_id)?
        .into_iter()
        .filter(|step| step.action == "report" && step.status == "succeeded")
        .filter_map(|step| {
            let outputs = super::engine::parse_outputs(&step.outputs);
            let body = outputs.get("body")?.clone();
            Some(Report {
                step: step.step_id,
                title: outputs.get("title").cloned(),
                body,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::Checks;
    use crate::store::{NewTrackedPullRequest, NewWorkflowRun};

    const HOUR: i64 = 60 * 60 * 1000;

    fn pr(number: u32, branch: &str, created_at: Option<i64>) -> PullRequest {
        PullRequest {
            number,
            url: format!("https://github.com/o/app/pull/{number}"),
            title: format!("Change {number}"),
            branch: branch.to_owned(),
            state: PullRequestState::Open,
            draft: false,
            checks: Checks::None,
            details: None,
            author: None,
            created_at,
        }
    }

    /// A project with one workspace on `ys/fix`, and a run in it asked for at `created` that
    /// ended at `ended`.
    fn with_run(created: i64, ended: Option<i64>) -> (Store, WorkflowRunRow) {
        let store = Store::in_memory();
        let project = store.add_project("app", "/src/app").unwrap();
        let ws = store
            .add_worktree(&project.id, "fix", "/wt/fix", Some("ys/fix"), Some("main"))
            .unwrap();
        store
            .queue_workflow_run(&NewWorkflowRun {
                id: "run-1",
                workflow_id: "factory",
                workflow_name: "Factory",
                definition: "",
                project_id: &project.id,
                workspace_id: &ws.id,
                workspace_name: "fix",
                inputs: "{}",
                context: "{}",
                requested_by: "app",
                steps: &[("tell", "notify")],
            })
            .unwrap();
        store
            .set_workflow_run_times("run-1", created, ended)
            .unwrap();
        let run = store.workflow_run("run-1").unwrap().unwrap();
        (store, run)
    }

    #[test]
    fn a_pull_request_opened_on_the_branch_during_the_run_is_the_runs() {
        let now = 100 * HOUR;
        let (store, run) = with_run(now - 3 * HOUR, Some(now - HOUR));
        let project = run.project_id.clone().unwrap();
        let seen = [
            pr(1, "ys/fix", Some(now - 2 * HOUR)),
            // Before the run was asked for.
            pr(2, "ys/fix", Some(now - 4 * HOUR)),
            // Another branch.
            pr(3, "ys/other", Some(now - 2 * HOUR)),
            // Long after it ended.
            pr(4, "ys/fix", Some(now)),
            // No time from the forge: nothing to place it by.
            pr(5, "ys/fix", None),
        ];
        observe_pull_requests(&store, &project, &seen, now).unwrap();
        let made = pull_requests_made(&store, &run).unwrap();
        assert_eq!(made.iter().map(|p| p.number).collect::<Vec<_>>(), [1]);
        assert_eq!(made[0].title.as_deref(), Some("Change 1"));
        assert_eq!(made[0].repo, None);
    }

    #[test]
    fn a_pull_request_opened_just_after_the_last_step_still_counts() {
        let now = 100 * HOUR;
        let ended = now - HOUR;
        let (store, run) = with_run(now - 3 * HOUR, Some(ended));
        let project = run.project_id.clone().unwrap();
        let seen = [pr(7, "ys/fix", Some(ended + 5 * 60 * 1000))];
        observe_pull_requests(&store, &project, &seen, now).unwrap();
        assert_eq!(pull_requests_made(&store, &run).unwrap().len(), 1);
    }

    #[test]
    fn a_matched_pull_request_follows_the_forge_and_is_kept_once() {
        let now = 100 * HOUR;
        let (store, run) = with_run(now - 3 * HOUR, None);
        let project = run.project_id.clone().unwrap();
        observe_pull_requests(&store, &project, &[pr(1, "ys/fix", Some(now - HOUR))], now).unwrap();
        let mut merged = pr(1, "ys/fix", Some(now - HOUR));
        merged.state = PullRequestState::Merged;
        merged.title = "Renamed".to_owned();
        observe_pull_requests(&store, &project, &[merged], now).unwrap();
        let made = pull_requests_made(&store, &run).unwrap();
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].state.as_deref(), Some("merged"));
        assert_eq!(made[0].title.as_deref(), Some("Renamed"));
    }

    #[test]
    fn one_tracked_elsewhere_during_the_run_is_listed_after_its_own() {
        let now = 100 * HOUR;
        let (store, run) = with_run(now - 3 * HOUR, Some(now - HOUR));
        let ws = run.workspace_id.clone().unwrap();
        let track = |number: u32, at: i64| {
            let (row, _) = store
                .track_pull_request(&NewTrackedPullRequest {
                    workspace_id: &ws,
                    session_id: None,
                    repo: "github.com/o/cloud",
                    number,
                    url: &format!("https://github.com/o/cloud/pull/{number}"),
                    tracked_by: "hook",
                })
                .unwrap();
            store.set_tracked_at(&row.id, at).unwrap();
        };
        track(41, now - 2 * HOUR);
        // Tracked a day before the run: an earlier piece of work.
        track(12, now - 30 * HOUR);
        let project = run.project_id.clone().unwrap();
        observe_pull_requests(
            &store,
            &project,
            &[pr(1, "ys/fix", Some(now - 2 * HOUR))],
            now,
        )
        .unwrap();
        let made = pull_requests_made(&store, &run).unwrap();
        assert_eq!(
            made.iter()
                .map(MadePullRequest::reference)
                .collect::<Vec<_>>(),
            ["#1", "o/cloud#41"]
        );
        assert_eq!(
            pull_requests_text(&made),
            "- #1 Change 1 — https://github.com/o/app/pull/1\n\
             - o/cloud#41 — https://github.com/o/cloud/pull/41"
        );
    }

    #[test]
    fn no_pull_requests_reads_as_a_sentence() {
        assert_eq!(pull_requests_text(&[]), "This run opened no pull requests.");
    }
}
