//! Stage 4 of agent events: the handoff packet.
//!
//! One agent has worked in a workspace; the user wants to start another there. The second
//! agent sees the same files and the same diff, but nothing of what was asked, tried or found.
//! This builds a packet from what Yardsort recorded — the task, where the branch stands, who
//! wrote which file, what each run did — as text for the new agent's opening prompt, which
//! the user reads and edits before it is sent through the ordinary prompt transport.
//!
//! The packet is deterministic prose from facts. It says what it does not have, in so many
//! words: Yardsort keeps metadata, never the conversation, so "what the agent reasoned" is not
//! here and the packet says to ask the user. Nothing in it comes from a model.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::activity::provenance::{self, Provenance};
use crate::store::{EventRow, RunRow, SessionRow, Store, StoreResult};

/// How much of one task prompt goes in. A task is a paragraph or two; a pasted spec is not.
const MAX_PROMPT_CHARS: usize = 2_000;
/// How many commit subjects, changed files and tool names are listed before "and N more".
const MAX_COMMITS: usize = 20;
const MAX_FILES: usize = 80;
const MAX_TOOLS: usize = 12;
/// A `ys activity list` line names the workspace; this is the longest name printed as-is.
const MAX_NAME_CHARS: usize = 120;

/// A changed file, as the caller's git sees it. Kind is git's word: added, modified, deleted,
/// renamed, untracked, conflicted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub kind: String,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
}

/// What one run of an agent did, from its events. Counts and names only.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunSummary {
    pub harness: String,
    pub model: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i64>,
    pub end_reason: Option<String>,
    /// How the run reported, or `None` when it did not.
    pub capture: Option<String>,
    pub turns: u32,
    /// Tool name → how many times it completed.
    pub tools: BTreeMap<String, u32>,
    /// Tool calls that failed: "Read missing.txt", or just the tool.
    pub failed: Vec<String>,
    /// Files the run reported writing, in first-write order.
    pub wrote: Vec<String>,
    pub approvals: u32,
    pub notifications: u32,
}

/// Everything the packet is written from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Facts {
    pub workspace: String,
    pub project: String,
    pub branch: Option<String>,
    pub base: Option<String>,
    /// Commit subjects on the branch since its base, newest first.
    pub commits: Vec<String>,
    pub uncommitted: Vec<ChangedFile>,
    pub committed: Vec<ChangedFile>,
    pub provenance: Provenance,
    /// What the workspace was asked: each conversation's first message, in order.
    pub prompts: Vec<String>,
    pub runs: Vec<RunSummary>,
    /// How many events stand behind the summary.
    pub events: usize,
}

/// The parts of the facts the store holds: the task, the runs and what they did, the join.
/// The git parts — branch, commits, changed files — are the caller's to add, with the
/// modification times of the changed files for the observed join.
pub fn from_store(
    store: &Store,
    workspace_id: &str,
    files: &[(String, i64)],
) -> StoreResult<Facts> {
    let row = store.workspace(workspace_id)?;
    let project = row
        .as_ref()
        .and_then(|row| store.project(&row.project_id).ok().flatten())
        .map(|project| project.name)
        .unwrap_or_default();
    let sessions = store.sessions(workspace_id)?;
    let runs = store.runs(workspace_id)?;
    let events = store.all_events(Some(workspace_id))?;
    Ok(Facts {
        workspace: row.as_ref().map(|row| row.name.clone()).unwrap_or_default(),
        project,
        branch: row.as_ref().and_then(|row| row.branch.clone()),
        base: row.as_ref().and_then(|row| row.base_branch.clone()),
        commits: vec![],
        uncommitted: vec![],
        committed: vec![],
        provenance: provenance::of(store, workspace_id, files)?,
        prompts: store.session_prompts(workspace_id)?,
        runs: summarize(&runs, &sessions, &events),
        events: events.len(),
    })
}

/// One summary per agent run that was spawned, oldest first, from that run's events.
pub fn summarize(runs: &[RunRow], sessions: &[SessionRow], events: &[EventRow]) -> Vec<RunSummary> {
    let mut summaries: Vec<(String, RunSummary)> = runs
        .iter()
        .filter(|run| run.kind == "harness" && run.pty_session_id.is_some())
        .map(|run| {
            let model = run
                .session_id
                .as_deref()
                .and_then(|id| sessions.iter().find(|session| session.id == id))
                .and_then(|session| session.model.clone());
            (
                run.id.clone(),
                RunSummary {
                    harness: run.harness_id.clone().unwrap_or_else(|| "agent".to_owned()),
                    model,
                    started_at: run.started_at,
                    ended_at: run.ended_at,
                    exit_code: run.exit_code,
                    end_reason: run.end_reason.clone(),
                    ..RunSummary::default()
                },
            )
        })
        .collect();
    summaries.sort_by_key(|(_, summary)| summary.started_at);

    for event in events {
        let Some(run_id) = event.run_id.as_deref() else {
            continue;
        };
        let Some((_, summary)) = summaries.iter_mut().find(|(id, _)| id == run_id) else {
            continue;
        };
        let payload: Value = serde_json::from_str(&event.payload).unwrap_or(Value::Null);
        let text = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_owned);
        match event.kind.as_str() {
            "process.started" => summary.capture = text("capture"),
            "turn.completed" | "turn.failed" => summary.turns += 1,
            "tool.completed" => {
                let tool = text("tool").unwrap_or_else(|| "tool".to_owned());
                *summary.tools.entry(tool).or_default() += 1;
            }
            "tool.failed" => {
                let tool = text("tool").unwrap_or_else(|| "tool".to_owned());
                summary.failed.push(match text("path") {
                    Some(path) => format!("{tool} {path}"),
                    None => tool,
                });
            }
            "file.reported_write" => {
                if let Some(path) = text("path") {
                    if !summary.wrote.contains(&path) {
                        summary.wrote.push(path);
                    }
                }
            }
            "approval.requested" => summary.approvals += 1,
            "agent.notified" => summary.notifications += 1,
            _ => {}
        }
    }
    // A file tool that reported its path is a write too, for the adapters that report tool
    // calls rather than file events; the provenance join knows which tools those are, and so
    // does the reader of "Write ×2 (a.rs, b.rs)". Here, the report kinds are enough.
    summaries.into_iter().map(|(_, summary)| summary).collect()
}

/// The packet: Markdown a person can read and an agent can act on.
pub fn render(facts: &Facts) -> String {
    let mut out = String::new();
    let name = truncate(&facts.workspace, MAX_NAME_CHARS);
    out.push_str(&format!(
        "# Handoff from Yardsort: workspace \"{name}\" in {}\n\n",
        facts.project
    ));
    out.push_str(
        "You are taking over work in this workspace. What follows was assembled by Yardsort \
         from what it recorded — never from another agent's conversation, which it does not \
         keep. Where it says nothing, ask the user or read the files.\n\n",
    );

    out.push_str("## What this workspace was asked\n\n");
    if facts.prompts.is_empty() {
        out.push_str(
            "Not recorded: this workspace was opened without a first message, or its agents \
             were started without one.\n\n",
        );
    } else {
        for (index, prompt) in facts.prompts.iter().enumerate() {
            if facts.prompts.len() > 1 {
                out.push_str(&format!("Conversation {}:\n\n", index + 1));
            }
            for line in truncate(prompt.trim(), MAX_PROMPT_CHARS).lines() {
                out.push_str(&format!("> {line}\n"));
            }
            out.push('\n');
        }
    }

    out.push_str("## Where the work stands\n\n");
    match (&facts.branch, &facts.base) {
        (Some(branch), Some(base)) => out.push_str(&format!(
            "- Branch `{branch}`, started from `{base}`: {} on it since.\n",
            plural(facts.commits.len(), "commit")
        )),
        (Some(branch), None) => out.push_str(&format!("- Branch `{branch}`.\n")),
        _ => out.push_str("- Not on a branch Yardsort knows.\n"),
    }
    if !facts.commits.is_empty() {
        out.push_str("- Commits on this branch, newest first:\n");
        for subject in facts.commits.iter().take(MAX_COMMITS) {
            out.push_str(&format!("  - {subject}\n"));
        }
        if facts.commits.len() > MAX_COMMITS {
            out.push_str(&format!(
                "  - … and {} more\n",
                facts.commits.len() - MAX_COMMITS
            ));
        }
    }
    let reporting = facts.provenance.reporting_runs() > 0;
    for (title, files) in [
        ("Uncommitted", &facts.uncommitted),
        ("Committed on this branch", &facts.committed),
    ] {
        if files.is_empty() {
            continue;
        }
        out.push_str(&format!("- {title}: {}\n", plural(files.len(), "file")));
        for file in files.iter().take(MAX_FILES) {
            let stat = match (file.additions, file.deletions) {
                (Some(a), Some(d)) => format!(", +{a} −{d}"),
                _ => String::new(),
            };
            let who = if reporting {
                format!(" — {}", written_by(&facts.provenance, &file.path))
            } else {
                String::new()
            };
            out.push_str(&format!("  - `{}` ({}{stat}){who}\n", file.path, file.kind));
        }
        if files.len() > MAX_FILES {
            out.push_str(&format!("  - … and {} more\n", files.len() - MAX_FILES));
        }
    }
    if facts.uncommitted.is_empty() && facts.committed.is_empty() {
        out.push_str("- No changed files.\n");
    }
    out.push('\n');

    out.push_str("## What the agents did here\n\n");
    if facts.runs.is_empty() {
        out.push_str("No agent has run in this workspace yet.\n\n");
    }
    for run in &facts.runs {
        out.push_str(&format!("- **{}**", run.harness));
        if let Some(model) = &run.model {
            out.push_str(&format!(" ({model})"));
        }
        out.push_str(&format!(", started {}", when(run.started_at)));
        match (run.ended_at, run.exit_code, run.end_reason.as_deref()) {
            (Some(ended), Some(0), _) => out.push_str(&format!(
                ", ran {} and exited cleanly",
                lasted(run.started_at, ended)
            )),
            (Some(ended), Some(code), _) => out.push_str(&format!(
                ", ran {} and exited with {code}",
                lasted(run.started_at, ended)
            )),
            (Some(ended), None, Some("interrupted")) => out.push_str(&format!(
                ", ran {} and was interrupted (no exit status)",
                lasted(run.started_at, ended)
            )),
            (Some(ended), None, _) => out.push_str(&format!(
                ", ran {} and ended",
                lasted(run.started_at, ended)
            )),
            (None, _, _) => out.push_str(", **still running**"),
        }
        out.push_str(".\n");
        if run.capture.is_none() {
            out.push_str(
                "  - It was not reporting what it did (capture was off), so only that it ran \
                 and how it ended is known.\n",
            );
            continue;
        }
        if run.turns > 0 {
            out.push_str(&format!("  - {}.\n", plural(run.turns as usize, "turn")));
        }
        if !run.tools.is_empty() {
            let mut tools: Vec<(&String, &u32)> = run.tools.iter().collect();
            tools.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let listed: Vec<String> = tools
                .iter()
                .take(MAX_TOOLS)
                .map(|(tool, n)| {
                    if **n == 1 {
                        (*tool).clone()
                    } else {
                        format!("{tool} ×{n}")
                    }
                })
                .collect();
            out.push_str(&format!("  - Tools: {}", listed.join(", ")));
            if tools.len() > MAX_TOOLS {
                out.push_str(&format!(", and {} more", tools.len() - MAX_TOOLS));
            }
            out.push_str(".\n");
        }
        if !run.failed.is_empty() {
            out.push_str(&format!(
                "  - Failed: {}.\n",
                run.failed
                    .iter()
                    .take(MAX_TOOLS)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !run.wrote.is_empty() {
            out.push_str(&format!(
                "  - Reported writing: {}.\n",
                run.wrote
                    .iter()
                    .take(MAX_FILES)
                    .map(|path| format!("`{path}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if run.approvals > 0 {
            out.push_str(&format!(
                "  - Asked the user for permission {}.\n",
                times(run.approvals as usize)
            ));
        }
        if run.notifications > 0 {
            out.push_str(&format!(
                "  - Raised a notification {} — it was waiting on the user.\n",
                times(run.notifications as usize)
            ));
        }
    }
    out.push('\n');

    out.push_str("## What is not here\n\n");
    out.push_str(
        "- The conversations themselves: what the agents said, reasoned, tried and rejected. \
         Yardsort records metadata only. If that matters, ask the user before assuming.\n",
    );
    out.push_str(
        "- The commands the agents ran and the contents of files as they were; read the files \
         as they are now.\n",
    );
    if !reporting && !facts.runs.is_empty() {
        out.push_str("- Which files the agents wrote: no run here was reporting what it did.\n");
    }
    out.push_str(&format!(
        "- The record behind this: {} in Yardsort's activity timeline for this workspace \
         (`ys activity list --workspace \"{name}\"`).\n",
        plural(facts.events, "event")
    ));
    out
}

/// The Changes list's word on a file, for the packet.
fn written_by(provenance: &Provenance, path: &str) -> String {
    let Some(file) = provenance.files.iter().find(|file| file.path == path) else {
        return "no agent reported writing it".to_owned();
    };
    if !file.reports.is_empty() {
        let mut agents: Vec<String> = file
            .reports
            .iter()
            .map(|report| {
                report
                    .harness_id
                    .clone()
                    .unwrap_or_else(|| report.producer.clone())
            })
            .collect();
        agents.dedup();
        return format!("reported written by {}", agents.join(" and "));
    }
    match &file.observed {
        Some(observed) => {
            let mut agents: Vec<String> = observed
                .matches
                .iter()
                .map(|m| {
                    m.harness_id
                        .clone()
                        .unwrap_or_else(|| "an agent".to_owned())
                })
                .collect();
            agents.dedup();
            format!(
                "last written while {} ran a command; not reported",
                agents.join(" and ")
            )
        }
        None => "no agent reported writing it".to_owned(),
    }
}

fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn times(n: usize) -> String {
    match n {
        1 => "once".to_owned(),
        2 => "twice".to_owned(),
        n => format!("{n} times"),
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}…")
}

/// An instant, as a UTC date and time: the packet may be read on another machine, and a
/// date without a zone is a guess.
fn when(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (hh, mm) = (rem / 3600, (rem % 3600) / 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02} UTC")
}

fn lasted(from: i64, to: i64) -> String {
    let secs = (to - from).max(0) / 1000;
    if secs < 60 {
        format!("{secs} s")
    } else if secs < 3600 {
        format!("{} min", secs / 60)
    } else {
        format!("{} h {} min", secs / 3600, (secs % 3600) / 60)
    }
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{NewEvent, NewRun, NewSession};
    use serde_json::json;

    fn workspace(store: &Store) -> String {
        let project = store.add_project("app", "/code/app").unwrap();
        store
            .add_worktree(
                &project.id,
                "fix-login",
                "/wt/fix",
                Some("ys/fix"),
                Some("main"),
            )
            .unwrap()
            .id
    }

    fn event(store: &Store, ws: &str, run: &str, kind: &str, at: i64, payload: Value) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("h-{n}");
        let payload = payload.to_string();
        store
            .add_event(&NewEvent {
                id: &id,
                schema_version: 1,
                workspace_id: ws,
                session_id: None,
                run_id: Some(run),
                occurred_at: at,
                kind,
                producer: "claude",
                method: "hook",
                fidelity: "reported",
                source_key: None,
                privacy_class: "metadata",
                payload: &payload,
            })
            .unwrap();
    }

    /// A workspace with one recorded Claude run: the packet carries the task, the branch, the
    /// files with who wrote them, what the run did, and what is not there — and nothing that
    /// is not metadata.
    #[test]
    fn the_packet_says_what_was_asked_what_stands_what_was_done_and_what_is_missing() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        store
            .add_session(&NewSession {
                id: "rec-1",
                workspace_id: &ws,
                harness_id: "claude",
                model: Some("opus"),
                title: "Fix the login redirect",
                pty_session_id: "pty-1",
                prompt: Some("Fix the login redirect so that a user who signs in lands on the page they asked for.\n\nThe secret is hunter2."),
                ..Default::default()
            })
            .unwrap();
        store
            .add_run(&NewRun {
                id: "r1",
                workspace_id: &ws,
                session_id: Some("rec-1"),
                kind: "harness",
                harness_id: Some("claude"),
                harness_session_id: None,
                launched_by: "app",
            })
            .unwrap();
        store.run_spawned("r1", "pty-1").unwrap();
        let t0 = 1_790_000_000_000;
        event(
            &store,
            &ws,
            "r1",
            "process.started",
            t0,
            json!({ "capture": "hook" }),
        );
        event(
            &store,
            &ws,
            "r1",
            "tool.started",
            t0 + 1000,
            json!({ "tool": "Write", "toolUseId": "a", "path": "src/login.rs" }),
        );
        event(
            &store,
            &ws,
            "r1",
            "tool.completed",
            t0 + 2000,
            json!({ "tool": "Write", "toolUseId": "a", "path": "src/login.rs" }),
        );
        event(
            &store,
            &ws,
            "r1",
            "tool.started",
            t0 + 3000,
            json!({ "tool": "Bash", "toolUseId": "b" }),
        );
        event(
            &store,
            &ws,
            "r1",
            "tool.completed",
            t0 + 4000,
            json!({ "tool": "Bash", "toolUseId": "b" }),
        );
        event(
            &store,
            &ws,
            "r1",
            "tool.completed",
            t0 + 4500,
            json!({ "tool": "Bash" }),
        );
        event(
            &store,
            &ws,
            "r1",
            "tool.failed",
            t0 + 5000,
            json!({ "tool": "Read", "path": "missing.txt" }),
        );
        event(
            &store,
            &ws,
            "r1",
            "approval.requested",
            t0 + 6000,
            json!({ "tool": "Bash" }),
        );
        event(&store, &ws, "r1", "turn.completed", t0 + 7000, json!({}));
        store.end_run("r1", Some(0), "exited").unwrap();

        let mut facts = from_store(&store, &ws, &[("notes.md".into(), t0 + 3500)]).unwrap();
        facts.commits = vec!["Redirect after sign-in".into()];
        facts.uncommitted = vec![
            ChangedFile {
                path: "src/login.rs".into(),
                kind: "modified".into(),
                additions: Some(12),
                deletions: Some(3),
            },
            ChangedFile {
                path: "notes.md".into(),
                kind: "untracked".into(),
                additions: None,
                deletions: None,
            },
            ChangedFile {
                path: "README.md".into(),
                kind: "modified".into(),
                additions: Some(1),
                deletions: Some(1),
            },
        ];
        let text = render(&facts);

        assert!(text.starts_with("# Handoff from Yardsort: workspace \"fix-login\" in app\n"));
        assert!(text.contains("> Fix the login redirect so that"));
        assert!(
            text.contains("> The secret is hunter2."),
            "the task is the user's own words, whole"
        );
        assert!(text.contains("Branch `ys/fix`, started from `main`: 1 commit on it since."));
        assert!(text.contains("  - Redirect after sign-in\n"));
        assert!(text.contains("`src/login.rs` (modified, +12 −3) — reported written by claude"));
        assert!(text.contains(
            "`notes.md` (untracked) — last written while claude ran a command; not reported"
        ));
        assert!(text.contains("`README.md` (modified, +1 −1) — no agent reported writing it"));
        assert!(text.contains("- **claude** (opus), started 2026-09-"));
        assert!(text.contains("and exited cleanly.\n"));
        assert!(text.contains("  - 1 turn.\n"));
        assert!(text.contains("  - Tools: Bash ×2, Write.\n"));
        assert!(text.contains("  - Failed: Read missing.txt.\n"));
        assert!(text.contains("  - Asked the user for permission once.\n"));
        assert!(text.contains("The conversations themselves"));
        assert!(text.contains("9 events in Yardsort's activity timeline"));
        assert!(text.contains("`ys activity list --workspace \"fix-login\"`"));
        for never in ["toolUseId", "pty-1", "/wt/fix", "hook\n"] {
            assert!(!text.contains(never), "leaked {never}");
        }
    }

    /// Nothing recorded, nothing reporting: the packet says so at every turn rather than
    /// filling the gaps.
    #[test]
    fn an_empty_workspace_and_a_silent_run_are_described_as_such() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        let empty = render(&from_store(&store, &ws, &[]).unwrap());
        assert!(empty.contains("Not recorded: this workspace was opened without a first message"));
        assert!(empty.contains("- No changed files.\n"));
        assert!(empty.contains("No agent has run in this workspace yet."));
        assert!(!empty.contains("Which files the agents wrote"));

        store
            .add_run(&NewRun {
                id: "r1",
                workspace_id: &ws,
                session_id: None,
                kind: "harness",
                harness_id: Some("codex"),
                harness_session_id: None,
                launched_by: "cli",
            })
            .unwrap();
        store.run_spawned("r1", "pty-1").unwrap();
        event(
            &store,
            &ws,
            "r1",
            "process.started",
            1_790_000_000_000,
            json!({ "capture": null }),
        );
        let mut facts = from_store(&store, &ws, &[]).unwrap();
        facts.uncommitted = vec![ChangedFile {
            path: "a.rs".into(),
            kind: "modified".into(),
            additions: None,
            deletions: None,
        }];
        let silent = render(&facts);
        assert!(silent.contains("- **codex**, started"));
        assert!(silent.contains("**still running**"));
        assert!(silent.contains("It was not reporting what it did (capture was off)"));
        assert!(
            silent.contains("  - `a.rs` (modified)\n"),
            "no writer named when nobody was reporting"
        );
        assert!(silent.contains("Which files the agents wrote: no run here was reporting"));
    }

    #[test]
    fn dates_and_durations_read_plainly() {
        assert_eq!(when(0), "1970-01-01 00:00 UTC");
        assert_eq!(when(1_790_000_000_000), "2026-09-21 14:13 UTC");
        assert_eq!(lasted(0, 42_000), "42 s");
        assert_eq!(lasted(0, 5 * 60_000), "5 min");
        assert_eq!(lasted(0, 3_900_000), "1 h 5 min");
        assert_eq!(times(3), "3 times");
    }
}
