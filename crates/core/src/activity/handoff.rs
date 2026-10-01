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

use std::path::Path;

use crate::activity::provenance::{self, Provenance};
use crate::changes::{Changes, FileChange, Scope};
use crate::error::IpcResult;
use crate::git::{Git, Head};
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
    /// How the run reported, or `None` when it did not — or when that is not known, which
    /// `capture_known` tells apart (see `provenance::RunCoverage`).
    pub capture: Option<String>,
    pub capture_known: bool,
    pub turns: u32,
    /// Tool name → how many times it completed.
    pub tools: BTreeMap<String, u32>,
    /// Tool calls that failed: "Read missing.txt", or just the tool.
    pub failed: Vec<String>,
    /// Files the run reported writing, in first-write order: from the provenance join, so a
    /// tool-based write (Claude's `Write`, pi's `write`) counts and a failed patch does not.
    pub wrote: Vec<String>,
    pub approvals: u32,
    pub notifications: u32,
}

/// Assist's judgment of one changed file, from the review the Changes list already shows —
/// a ranking of what deserves the next agent's attention, not a fact about the file. Only
/// when the user has Assist reviewing changes; nothing new is sent for the packet.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ranking {
    /// `direct`, `supporting`, `unrelated` or `unsure`, as the review names them; `None`
    /// when the task was not known or the file was not checked.
    pub relevance: Option<String>,
    /// The review's flags, as it names them: `secret`, `weakensTests`, `disablesChecks`,
    /// `credentialsFile`, `unaccounted`.
    pub flags: Vec<String>,
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
    /// The project's memory section, when the project shares it (see `crate::memory`). A
    /// handoff launch gets no memory added at launch, so the packet carries it.
    pub memory: Option<String>,
    /// Assist's judgment per changed file, when the caller has one; empty otherwise. Keyed by
    /// scope and path: a file changed both on the branch and since the last commit has two
    /// diffs, and two reviews.
    pub rankings: BTreeMap<(Scope, String), Ranking>,
}

/// The whole of the facts: the store's half and git's — head, base, the commits since, the
/// change list with each file's clock for the observed join. What `ys` and the app both call.
pub fn facts(
    store: &Store,
    git: &Git,
    root: &Path,
    workspace_id: &str,
    base_branch: Option<&str>,
) -> IpcResult<Facts> {
    let set = Changes {
        git,
        root,
        base_branch,
    }
    .list()?;
    let file = |change: &FileChange| ChangedFile {
        path: change.path.clone(),
        kind: serde_json::to_value(change.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        additions: change.additions,
        deletions: change.deletions,
    };
    let paths: Vec<String> = set
        .uncommitted
        .iter()
        .chain(&set.committed)
        .map(|change| change.path.clone())
        .collect();
    let mut facts = from_store(store, workspace_id, &provenance::last_written(root, &paths))?;
    facts.branch = match git.head(root) {
        Ok(Head::Branch(name) | Head::Unborn(name)) => Some(name),
        _ => None,
    };
    facts.base = set.base.clone();
    if let Some(base) = &set.base {
        facts.commits = git
            .commits_since(root, base)
            .map(|commits| commits.into_iter().map(|c| c.subject).collect())
            .unwrap_or_default();
    }
    facts.uncommitted = set.uncommitted.iter().map(file).collect();
    facts.committed = set.committed.iter().map(file).collect();
    Ok(facts)
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
    let provenance = provenance::of(store, workspace_id, files)?;
    Ok(Facts {
        workspace: row.as_ref().map(|row| row.name.clone()).unwrap_or_default(),
        project,
        branch: row.as_ref().and_then(|row| row.branch.clone()),
        base: row.as_ref().and_then(|row| row.base_branch.clone()),
        commits: vec![],
        uncommitted: vec![],
        committed: vec![],
        prompts: store.session_prompts(workspace_id)?,
        memory: match &row {
            Some(row) => crate::memory::prompt_section(store, &row.project_id).unwrap_or(None),
            None => None,
        },
        runs: summarize(&runs, &sessions, &events, &provenance),
        events: events.len(),
        provenance,
        rankings: BTreeMap::new(),
    })
}

/// One summary per agent run that was spawned, oldest first, from that run's events. What a
/// run wrote and whether it was reporting come from the provenance join, which already knows
/// which events are writes (a tool-based `Write` is, a failed patch is not) and which runs
/// were reporting even after their start event is gone.
pub fn summarize(
    runs: &[RunRow],
    sessions: &[SessionRow],
    events: &[EventRow],
    provenance: &Provenance,
) -> Vec<RunSummary> {
    let mut summaries: Vec<(String, RunSummary)> = runs
        .iter()
        .filter(|run| run.kind == "harness" && run.pty_session_id.is_some())
        .map(|run| {
            let model = run
                .session_id
                .as_deref()
                .and_then(|id| sessions.iter().find(|session| session.id == id))
                .and_then(|session| session.model.clone());
            let coverage = provenance
                .runs
                .iter()
                .find(|coverage| coverage.run_id == run.id);
            let mut wrote: Vec<(i64, String)> = provenance
                .files
                .iter()
                .flat_map(|file| {
                    file.reports
                        .iter()
                        .filter(|report| report.run_id.as_deref() == Some(run.id.as_str()))
                        .map(|report| (report.first_at, file.path.clone()))
                })
                .collect();
            wrote.sort();
            (
                run.id.clone(),
                RunSummary {
                    harness: run.harness_id.clone().unwrap_or_else(|| "agent".to_owned()),
                    model,
                    started_at: run.started_at,
                    ended_at: run.ended_at,
                    exit_code: run.exit_code,
                    end_reason: run.end_reason.clone(),
                    capture: coverage.and_then(|c| c.capture.clone()),
                    capture_known: coverage.is_some_and(|c| c.capture_known),
                    wrote: wrote.into_iter().map(|(_, path)| path).collect(),
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
            "approval.requested" => summary.approvals += 1,
            "agent.notified" => summary.notifications += 1,
            _ => {}
        }
    }
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
    for (title, scope, files) in [
        ("Uncommitted", Scope::Uncommitted, &facts.uncommitted),
        (
            "Committed on this branch",
            Scope::Committed,
            &facts.committed,
        ),
    ] {
        if files.is_empty() {
            continue;
        }
        out.push_str(&format!("- {title}: {}\n", plural(files.len(), "file")));
        // With Assist's judgment in hand, the files it would look at first come first.
        let mut ordered: Vec<&ChangedFile> = files.iter().collect();
        if !facts.rankings.is_empty() {
            ordered.sort_by_key(|file| rank(facts.rankings.get(&(scope, file.path.clone()))));
        }
        for file in ordered.iter().take(MAX_FILES) {
            let stat = match (file.additions, file.deletions) {
                (Some(a), Some(d)) => format!(", +{a} −{d}"),
                _ => String::new(),
            };
            let who = if reporting {
                format!(" — {}", written_by(&facts.provenance, &file.path))
            } else {
                String::new()
            };
            let judged = facts
                .rankings
                .get(&(scope, file.path.clone()))
                .map(judgment)
                .filter(|j| !j.is_empty())
                .map(|j| format!(" — {j}"))
                .unwrap_or_default();
            out.push_str(&format!(
                "  - `{}` ({}{stat}){who}{judged}\n",
                file.path, file.kind
            ));
        }
        if files.len() > MAX_FILES {
            out.push_str(&format!("  - … and {} more\n", files.len() - MAX_FILES));
        }
    }
    if facts.uncommitted.is_empty() && facts.committed.is_empty() {
        out.push_str("- No changed files.\n");
    }
    if !facts.rankings.is_empty() {
        out.push_str(
            "- The files are in the order Assist would look at them, with its word on each, \
             from the review the Changes list already shows: a judgment about relevance to the \
             task and risk, not a fact about the file.\n",
        );
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
        if run.capture.is_none() && run.capture_known {
            out.push_str(
                "  - It was not reporting what it did (capture was off), so only that it ran \
                 and how it ended is known.\n",
            );
            continue;
        }
        if run.capture.is_none() {
            out.push_str(
                "  - Whether it was reporting is not known: its start was cleared or has aged \
                 out of the record. What follows is what remains.\n",
            );
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

    if let Some(memory) = &facts.memory {
        out.push_str(memory);
        out.push('\n');
    }

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

/// Where a file goes in the list: what the task asks for first, then what supports it, then
/// what Assist could not place or was not asked about, then what looks unrelated.
fn rank(ranking: Option<&Ranking>) -> u8 {
    match ranking.and_then(|r| r.relevance.as_deref()) {
        Some("direct") => 0,
        Some("supporting") => 1,
        Some("unrelated") => 3,
        _ => 2,
    }
}

/// Assist's word on a file, in the words the badges use.
fn judgment(ranking: &Ranking) -> String {
    let mut words: Vec<String> = vec![];
    match ranking.relevance.as_deref() {
        Some("direct") => words.push("on task".to_owned()),
        Some("supporting") => words.push("supports the task".to_owned()),
        Some("unrelated") => words.push("looks unrelated to the task".to_owned()),
        Some("unsure") => words.push("Assist unsure of its relevance".to_owned()),
        _ => {}
    }
    let flags: Vec<&str> = ranking
        .flags
        .iter()
        .map(|flag| match flag.as_str() {
            "secret" => "may add a secret",
            "weakensTests" => "weakens a test",
            "disablesChecks" => "switches a check off",
            "credentialsFile" => "a credentials file",
            "unaccounted" => "a substantive change no agent accounted for",
            other => other,
        })
        .collect();
    if !flags.is_empty() {
        words.push(format!("Assist flags: {}", flags.join(", ")));
    }
    words.join("; ")
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
        // The producer is the run's harness, and the method names its capture, as the
        // adapters do; a Codex event says so even in a test.
        let (producer, method) = if run == "codex" {
            ("codex", "notify")
        } else {
            ("claude", "hook")
        };
        store
            .add_event(&NewEvent {
                id: &id,
                schema_version: 1,
                workspace_id: ws,
                session_id: None,
                run_id: Some(run),
                occurred_at: at,
                kind,
                producer,
                method,
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
        // The session is stamped with the time the test runs; only the shape of the date is fixed.
        let started = text
            .split_once("- **claude** (opus), started ")
            .expect("the agent's line")
            .1;
        assert!(
            started.as_bytes()[..10]
                .iter()
                .enumerate()
                .all(|(i, b)| if i == 4 || i == 7 {
                    *b == b'-'
                } else {
                    b.is_ascii_digit()
                }),
            "{started}"
        );
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

    /// Writes come from the provenance join, not a second reading of the events: Claude's
    /// `Write` (a tool call, no file event) is a write; Codex's failed patch (a file event
    /// with `status: "failed"`) is not; and a run whose start event was cleared while it was
    /// live is still known to be reporting by what it reported, so its later failure shows.
    #[test]
    fn writes_and_coverage_agree_with_the_provenance_join() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        for (id, harness) in [
            ("claude", "claude"),
            ("codex", "codex"),
            ("cleared", "claude"),
        ] {
            store
                .add_run(&NewRun {
                    id,
                    workspace_id: &ws,
                    session_id: None,
                    kind: "harness",
                    harness_id: Some(harness),
                    harness_session_id: None,
                    launched_by: "app",
                })
                .unwrap();
            store.run_spawned(id, &format!("pty-{id}")).unwrap();
        }
        let t0 = 1_790_000_000_000;
        event(
            &store,
            &ws,
            "claude",
            "process.started",
            t0,
            json!({ "capture": "hook" }),
        );
        event(
            &store,
            &ws,
            "claude",
            "tool.completed",
            t0 + 1000,
            json!({ "tool": "Write", "toolUseId": "w", "path": "written.rs" }),
        );
        event(
            &store,
            &ws,
            "codex",
            "process.started",
            t0,
            json!({ "capture": "notify" }),
        );
        event(
            &store,
            &ws,
            "codex",
            "file.reported_write",
            t0 + 2000,
            json!({ "path": "failed.rs", "kind": "update", "status": "failed" }),
        );
        event(
            &store,
            &ws,
            "codex",
            "file.reported_write",
            t0 + 2500,
            json!({ "path": "landed.rs", "kind": "update", "status": "completed" }),
        );
        event(
            &store,
            &ws,
            "cleared",
            "process.started",
            t0,
            json!({ "capture": "hook" }),
        );
        store.clear_activity(Some(&ws)).unwrap();
        // After the clear, only what the runs report from here on exists.
        event(
            &store,
            &ws,
            "claude",
            "tool.completed",
            t0 + 3000,
            json!({ "tool": "Edit", "toolUseId": "e", "path": "written.rs" }),
        );
        event(
            &store,
            &ws,
            "codex",
            "file.reported_write",
            t0 + 3500,
            json!({ "path": "later.rs", "kind": "add", "status": "completed" }),
        );
        event(
            &store,
            &ws,
            "cleared",
            "tool.failed",
            t0 + 4000,
            json!({ "tool": "Read", "path": "missing.txt" }),
        );

        let facts = from_store(&store, &ws, &[]).unwrap();
        let by = |id: &str| {
            facts
                .runs
                .iter()
                .find(|r| r.harness == id || (id == "cleared" && r.failed.len() == 1))
                .unwrap()
        };
        let claude = facts
            .runs
            .iter()
            .find(|r| r.wrote.contains(&"written.rs".to_owned()))
            .unwrap();
        assert_eq!(claude.wrote, ["written.rs"], "a tool-based write, once");
        let codex = by("codex");
        assert_eq!(
            codex.wrote,
            ["later.rs"],
            "after the clear only later.rs remains, and never failed.rs"
        );
        assert_eq!(
            (codex.capture.as_deref(), codex.capture_known),
            (Some("notify"), true)
        );
        let cleared = by("cleared");
        assert_eq!(
            (cleared.capture.as_deref(), cleared.capture_known),
            (Some("hook"), true)
        );
        assert_eq!(cleared.failed, ["Read missing.txt"]);
        let text = render(&facts);
        assert!(!text.contains("failed.rs"));
        assert!(text.contains("  - Failed: Read missing.txt.\n"));
        assert!(
            !text.contains("capture was off"),
            "none of these runs is known silent"
        );
    }

    /// With Assist's review in hand, the changed files come in the order it would look at
    /// them, each with its word — and the packet says that is a judgment. Without one, the
    /// list is git's order and says nothing of the kind.
    #[test]
    fn assists_judgment_orders_the_files_and_is_named_as_a_judgment() {
        let mut facts = Facts {
            workspace: "w".into(),
            project: "p".into(),
            uncommitted: vec![
                ChangedFile {
                    path: "ci.yml".into(),
                    kind: "modified".into(),
                    additions: None,
                    deletions: None,
                },
                ChangedFile {
                    path: "src/login.rs".into(),
                    kind: "modified".into(),
                    additions: None,
                    deletions: None,
                },
                ChangedFile {
                    path: "notes.md".into(),
                    kind: "untracked".into(),
                    additions: None,
                    deletions: None,
                },
                ChangedFile {
                    path: "src/login_test.rs".into(),
                    kind: "modified".into(),
                    additions: None,
                    deletions: None,
                },
            ],
            ..Facts::default()
        };
        let plain = render(&facts);
        let at = |text: &str, needle: &str| text.find(needle).unwrap();
        assert!(
            at(&plain, "`ci.yml`") < at(&plain, "`src/login.rs`"),
            "git's order"
        );
        assert!(!plain.contains("Assist"));

        facts.rankings = [
            ("ci.yml", Some("unrelated"), vec!["disablesChecks"]),
            ("src/login.rs", Some("direct"), vec![]),
            (
                "src/login_test.rs",
                Some("supporting"),
                vec!["weakensTests"],
            ),
        ]
        .into_iter()
        .map(|(path, relevance, flags)| {
            (
                (Scope::Uncommitted, path.to_owned()),
                Ranking {
                    relevance: relevance.map(str::to_owned),
                    flags: flags.into_iter().map(str::to_owned).collect(),
                },
            )
        })
        .collect();
        let ranked = render(&facts);
        let login = at(&ranked, "`src/login.rs`");
        let test = at(&ranked, "`src/login_test.rs`");
        let notes = at(&ranked, "`notes.md`");
        let ci = at(&ranked, "`ci.yml`");
        assert!(
            login < test && test < notes && notes < ci,
            "direct, supporting, unjudged, unrelated"
        );
        assert!(ranked.contains("`src/login.rs` (modified) — on task\n"));
        assert!(ranked.contains(
            "`src/login_test.rs` (modified) — supports the task; Assist flags: weakens a test\n"
        ));
        assert!(ranked.contains("`ci.yml` (modified) — looks unrelated to the task; Assist flags: switches a check off\n"));
        assert!(ranked.contains("`notes.md` (untracked)\n"));
        assert!(ranked.contains(
            "a judgment about relevance to the task and risk, not a fact about the file"
        ));
    }

    /// One path, two scopes, two diffs, two reviews: the uncommitted edit that adds a secret
    /// keeps its flag though the committed change to the same file was judged clean.
    #[test]
    fn a_file_changed_in_both_scopes_keeps_each_scopes_judgment() {
        let file = |path: &str| ChangedFile {
            path: path.into(),
            kind: "modified".into(),
            additions: None,
            deletions: None,
        };
        let mut facts = Facts {
            workspace: "w".into(),
            project: "p".into(),
            uncommitted: vec![file("src/login.rs")],
            committed: vec![file("src/login.rs")],
            ..Facts::default()
        };
        facts.rankings.insert(
            (Scope::Uncommitted, "src/login.rs".into()),
            Ranking {
                relevance: Some("direct".into()),
                flags: vec!["secret".into()],
            },
        );
        facts.rankings.insert(
            (Scope::Committed, "src/login.rs".into()),
            Ranking {
                relevance: Some("direct".into()),
                flags: vec![],
            },
        );
        let text = render(&facts);
        let uncommitted = text.find("- Uncommitted").unwrap();
        let committed = text.find("- Committed on this branch").unwrap();
        let flagged = text
            .find("`src/login.rs` (modified) — on task; Assist flags: may add a secret")
            .unwrap();
        let clean = text.rfind("`src/login.rs` (modified) — on task\n").unwrap();
        assert!(
            uncommitted < flagged && flagged < committed,
            "the flag stays uncommitted"
        );
        assert!(
            clean > committed,
            "the committed entry is judged on its own diff"
        );
    }

    /// A project that shares its memory has it in the packet, before what is not here; one that
    /// does not, has none.
    #[test]
    fn the_packet_carries_the_projects_memory_when_it_is_shared() {
        let store = Store::in_memory();
        let ws = workspace(&store);
        let project = store.workspace(&ws).unwrap().unwrap().project_id;
        crate::memory::write(&store, &project, "The tests need TZ=UTC.").unwrap();
        let without = render(&from_store(&store, &ws, &[]).unwrap());
        assert!(!without.contains("Project memory"));
        store.set_memory_shared(&project, true).unwrap();
        let with = render(&from_store(&store, &ws, &[]).unwrap());
        let memory = with.find("## Project memory").unwrap();
        assert!(memory < with.find("## What is not here").unwrap());
        assert!(with.contains("- The tests need TZ=UTC. (memory "));
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
