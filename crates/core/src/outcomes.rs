//! Stage 6 of agent events: what became of each attempt.
//!
//! A workspace is an attempt at its task — its first message — and the agents that worked in it
//! are its sessions' harnesses. What became of it is the user's to say: kept, partly, or
//! discarded. A merge backs that up as strong evidence of its own: a merged pull request, or a
//! branch whose commits were later seen in its base (a merge or a fast-forward; a squash merge
//! leaves no trace in git, which is why the pull request's state is read too). Nothing else is
//! an outcome: a deleted workspace, a closed pull request or a failed run is evidence shown
//! beside it, never counted.
//!
//! Per-agent history counts outcomes only, and says "too few to say" below a sample size, so a
//! handful of attempts never crowns a winner. It is shown beside the composer's agent picker and
//! never used to pick for the user. See `docs/design/20-agent-events-stage-6-outcomes.md`.

use std::collections::BTreeMap;
use std::path::Path;

use crate::git::Git;
use crate::store::{OutcomeRow, OutcomeSnapshot, Store, StoreResult};

/// Below this many known outcomes, an agent's history is "too few to say".
pub const MIN_SAMPLE: u32 = 5;
/// How much of the task an outcome keeps.
const MAX_TASK_CHARS: usize = 300;

/// What the user can say about an attempt.
pub const LABELS: [&str; 3] = ["kept", "partly", "discarded"];

/// An attempt's outcome, and what it rests on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The user said so.
    Labeled(String),
    /// Nobody said, but it was merged: counted as kept, marked as coming from the merge.
    Merged,
    /// Neither: not counted.
    Unknown,
}

pub fn outcome(row: &OutcomeRow) -> Outcome {
    if let Some(label) = &row.label {
        return Outcome::Labeled(label.clone());
    }
    if row.pr_state.as_deref() == Some("merged") || row.merged_at.is_some() {
        return Outcome::Merged;
    }
    Outcome::Unknown
}

/// Copy what an outcome describes from its workspace while the workspace is still there: its
/// name and branch, its task, the agents that worked in it. `local` is the project itself, not
/// an attempt, and is skipped. Safe to call again: it only refreshes what it describes.
pub fn snapshot(store: &Store, workspace_id: &str) -> StoreResult<bool> {
    let Some(workspace) = store.workspace(workspace_id)? else {
        return Ok(false);
    };
    if workspace.kind == "local" {
        return Ok(false);
    }
    let mut sessions = store.sessions(workspace_id)?;
    sessions.reverse();
    let mut harnesses: Vec<String> = vec![];
    for session in sessions {
        if !harnesses.contains(&session.harness_id) {
            harnesses.push(session.harness_id);
        }
    }
    let task = store
        .session_prompts(workspace_id)?
        .into_iter()
        .next()
        .map(|task| {
            let line = task.split_whitespace().collect::<Vec<_>>().join(" ");
            let mut short: String = line.chars().take(MAX_TASK_CHARS).collect();
            if line.chars().count() > MAX_TASK_CHARS {
                short.push('…');
            }
            short
        });
    store.snapshot_outcome(&OutcomeSnapshot {
        id: &workspace.id,
        project_id: &workspace.project_id,
        workspace_name: &workspace.name,
        branch: workspace.branch.as_deref(),
        base_branch: workspace.base_branch.as_deref(),
        task: task.as_deref(),
        harnesses: &harnesses.join(","),
    })?;
    Ok(true)
}

/// Git's evidence for one attempt, read in the project's repository: is the branch ahead of its
/// base, and — once it has been — is its work now in the base? A branch that is gone, or a base
/// git does not know, says nothing.
pub fn observe_git(store: &Store, git: &Git, root: &Path, row: &OutcomeRow) -> StoreResult<()> {
    let (Some(branch), Some(base)) = (row.branch.as_deref(), row.base_branch.as_deref()) else {
        return Ok(());
    };
    let range = format!("refs/heads/{base}..refs/heads/{branch}");
    let Ok(count) = git.run(root, &["rev-list", "--count", &range]) else {
        return Ok(());
    };
    let ahead = count.trim().parse::<u64>().unwrap_or(0) > 0;
    store.outcome_git(&row.id, ahead, !ahead)
}

/// The forge's evidence: the state of the pull request whose head is each attempt's branch.
/// `pull_requests` is `(branch, number, state)`, newest first; the newest for a branch wins.
pub fn observe_pull_requests(
    store: &Store,
    rows: &[OutcomeRow],
    pull_requests: &[(String, i64, String)],
) -> StoreResult<()> {
    for row in rows {
        let Some(branch) = row.branch.as_deref() else {
            continue;
        };
        if let Some((_, number, state)) = pull_requests.iter().find(|(b, _, _)| b == branch) {
            if row.pr_number != Some(*number) || row.pr_state.as_deref() != Some(state) {
                store.outcome_pull_request(&row.id, *number, state)?;
            }
        }
    }
    Ok(())
}

/// Bring a project's outcomes up to date: snapshot every live attempt, and read git's evidence
/// for all of them, ended ones included — their branches are kept.
pub fn refresh(store: &Store, git: &Git, project_id: &str, root: &Path) -> StoreResult<()> {
    for workspace in store.workspaces()? {
        if workspace.project_id == project_id && !workspace.forgotten {
            snapshot(store, &workspace.id)?;
        }
    }
    for row in store.outcomes(Some(project_id))? {
        observe_git(store, git, root, &row)?;
    }
    Ok(())
}

/// One agent's history: the outcomes of the attempts it worked in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AgentHistory {
    pub harness: String,
    /// Attempts it worked in, known outcome or not.
    pub attempts: u32,
    pub kept: u32,
    /// Of `kept`, how many rest on a merge rather than the user's word.
    pub kept_by_merge: u32,
    pub partly: u32,
    pub discarded: u32,
}

impl AgentHistory {
    /// Attempts with an outcome.
    pub fn known(&self) -> u32 {
        self.kept + self.partly + self.discarded
    }

    /// Whether there are enough outcomes to say anything.
    pub fn enough(&self) -> bool {
        self.known() >= MIN_SAMPLE
    }
}

/// Per-agent history over these attempts, most-used agent first. An attempt two agents worked in
/// counts for both: a handoff is shared work.
pub fn history(rows: &[OutcomeRow]) -> Vec<AgentHistory> {
    let mut by: BTreeMap<String, AgentHistory> = BTreeMap::new();
    for row in rows {
        let outcome = outcome(row);
        for harness in row.harnesses.split(',').filter(|h| !h.is_empty()) {
            let entry = by
                .entry(harness.to_owned())
                .or_insert_with(|| AgentHistory {
                    harness: harness.to_owned(),
                    ..AgentHistory::default()
                });
            entry.attempts += 1;
            match &outcome {
                Outcome::Labeled(label) if label == "kept" => entry.kept += 1,
                Outcome::Labeled(label) if label == "partly" => entry.partly += 1,
                Outcome::Labeled(_) => entry.discarded += 1,
                Outcome::Merged => {
                    entry.kept += 1;
                    entry.kept_by_merge += 1;
                }
                Outcome::Unknown => {}
            }
        }
    }
    let mut list: Vec<AgentHistory> = by.into_values().collect();
    list.sort_by(|a, b| b.attempts.cmp(&a.attempts).then(a.harness.cmp(&b.harness)));
    list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::git;
    use crate::store::NewSession;

    /// A repository on `main` with one commit, a project for it, and a worktree workspace row on
    /// `ys/fix` from `main` — the branch made, the folder not needed for git's evidence.
    fn repo() -> (tempfile::TempDir, Store, Git, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let git = git();
        let root = dir.path();
        git.init(root).unwrap();
        let run = |args: &[&str]| git.run(root, args).unwrap();
        run(&["checkout", "-q", "-b", "main"]);
        std::fs::write(root.join("README.md"), "hi\n").unwrap();
        run(&["add", "-A"]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "one",
        ]);
        run(&["branch", "ys/fix"]);
        let store = Store::in_memory();
        let project = store.add_project("app", &root.to_string_lossy()).unwrap();
        let ws = store
            .add_worktree(
                &project.id,
                "fix-login",
                "/wt/fix",
                Some("ys/fix"),
                Some("main"),
            )
            .unwrap();
        (dir, store, git, project.id, ws.id)
    }

    fn session(store: &Store, ws: &str, id: &str, harness: &str, prompt: Option<&str>) {
        store
            .add_session(&NewSession {
                id,
                workspace_id: ws,
                harness_id: harness,
                title: "t",
                pty_session_id: &format!("pty-{id}"),
                prompt,
                ..Default::default()
            })
            .unwrap();
    }

    /// The snapshot outlives the workspace: its name, branch, task and agents are still there
    /// once the row and its sessions are gone, and a label can be given afterwards.
    #[test]
    fn an_outcome_keeps_what_it_describes_after_the_workspace_is_deleted() {
        let (_dir, store, _git, project, ws) = repo();
        session(
            &store,
            &ws,
            "s1",
            "claude",
            Some("Fix the login\n\nredirect."),
        );
        session(&store, &ws, "s2", "codex", None);
        session(&store, &ws, "s3", "claude", None);
        assert!(snapshot(&store, &ws).unwrap());
        store.remove_worktree(&ws).unwrap();

        let row = store.outcome(&ws).unwrap().unwrap();
        assert_eq!(row.workspace_id, None, "the workspace is gone");
        assert_eq!(
            (
                row.workspace_name.as_str(),
                row.branch.as_deref(),
                row.task.as_deref()
            ),
            ("fix-login", Some("ys/fix"), Some("Fix the login redirect."))
        );
        assert_eq!(row.harnesses, "claude,codex", "first-seen order, once each");
        assert_eq!(row.project_id, project);
        store.label_outcome(&ws, Some("discarded")).unwrap();
        assert_eq!(
            outcome(&store.outcome(&ws).unwrap().unwrap()),
            Outcome::Labeled("discarded".into())
        );
    }

    /// Git's evidence: a branch with nothing of its own is not "merged" — it has to have been
    /// seen ahead first — and one whose commits later reach the base is.
    #[test]
    fn a_merge_is_seen_only_after_the_branch_was_ahead() {
        let (dir, store, git, project, ws) = repo();
        let root = dir.path();
        snapshot(&store, &ws).unwrap();
        refresh(&store, &git, &project, root).unwrap();
        let row = store.outcome(&ws).unwrap().unwrap();
        assert_eq!(
            (row.ahead_at, row.merged_at),
            (None, None),
            "empty branch: nothing"
        );
        assert_eq!(outcome(&row), Outcome::Unknown);

        let run = |args: &[&str]| git.run(root, args).unwrap();
        run(&["checkout", "-q", "ys/fix"]);
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        run(&["add", "-A"]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "work",
        ]);
        refresh(&store, &git, &project, root).unwrap();
        let row = store.outcome(&ws).unwrap().unwrap();
        assert!(row.ahead_at.is_some() && row.merged_at.is_none());

        run(&["checkout", "-q", "main"]);
        run(&["merge", "-q", "--ff-only", "ys/fix"]);
        refresh(&store, &git, &project, root).unwrap();
        let row = store.outcome(&ws).unwrap().unwrap();
        assert!(row.merged_at.is_some());
        assert_eq!(outcome(&row), Outcome::Merged);
    }

    /// The forge's evidence covers the squash merge git cannot see; a closed pull request is
    /// evidence, not an outcome; and the user's label always wins.
    #[test]
    fn a_merged_pull_request_counts_as_kept_until_the_user_says_otherwise() {
        let (_dir, store, _git, _project, ws) = repo();
        snapshot(&store, &ws).unwrap();
        let rows = store.outcomes(None).unwrap();
        observe_pull_requests(&store, &rows, &[("ys/fix".into(), 7, "closed".into())]).unwrap();
        assert_eq!(
            outcome(&store.outcome(&ws).unwrap().unwrap()),
            Outcome::Unknown
        );
        observe_pull_requests(&store, &rows, &[("ys/fix".into(), 8, "merged".into())]).unwrap();
        let row = store.outcome(&ws).unwrap().unwrap();
        assert_eq!(
            (row.pr_number, row.pr_state.as_deref()),
            (Some(8), Some("merged"))
        );
        assert_eq!(outcome(&row), Outcome::Merged);
        store.label_outcome(&ws, Some("partly")).unwrap();
        assert_eq!(
            outcome(&store.outcome(&ws).unwrap().unwrap()),
            Outcome::Labeled("partly".into())
        );
        store.label_outcome(&ws, None).unwrap();
        assert_eq!(
            outcome(&store.outcome(&ws).unwrap().unwrap()),
            Outcome::Merged,
            "taken back"
        );
    }

    /// History counts outcomes only, credits every agent that worked in an attempt, says how many
    /// of the kept rest on a merge, and is "too few" below the sample size.
    #[test]
    fn history_counts_outcomes_only_and_knows_when_it_is_too_few() {
        let row = |harnesses: &str, label: Option<&str>, merged: bool| OutcomeRow {
            harnesses: harnesses.into(),
            label: label.map(str::to_owned),
            pr_state: merged.then(|| "merged".to_owned()),
            ..OutcomeRow::default()
        };
        let rows = vec![
            row("claude", Some("kept"), false),
            row("claude,codex", None, true),
            row("claude", Some("discarded"), false),
            row("claude", None, false),
            row("claude", Some("partly"), false),
            row("claude", Some("kept"), false),
            row("codex", Some("kept"), false),
        ];
        let list = history(&rows);
        let claude = &list[0];
        assert_eq!(
            (
                claude.harness.as_str(),
                claude.attempts,
                claude.kept,
                claude.kept_by_merge
            ),
            ("claude", 6, 3, 1)
        );
        assert_eq!((claude.partly, claude.discarded, claude.known()), (1, 1, 5));
        assert!(claude.enough());
        let codex = &list[1];
        assert_eq!((codex.attempts, codex.kept, codex.known()), (2, 2, 2));
        assert!(!codex.enough(), "two outcomes are too few to say anything");
    }
}
