//! The conversation digest: what was said in a workspace's other conversations, for the agent
//! that asks. See `docs/design/29-conversation-context.md`.
//!
//! [`gather`] reads each conversation's transcript where its agent keeps it, with the readers in
//! [`super::transcript`]; [`select`] fits the turns into a budget, deterministically, newest
//! first; [`render`] writes the text an agent reads. Nothing is stored: the digest is built when
//! it is asked for, from the session records that tie a workspace to its conversations, and those
//! go with the workspace.
//!
//! What is left out, on purpose: tool calls and their output, the agents' reasoning, what the
//! harness injected, and what Yardsort itself added to a first message — the project's memory, a
//! handoff packet — which the reader is given anyway, and which would otherwise quote itself.

use std::path::Path;

use serde::Serialize;

use super::handoff::when;
use super::transcript::{self, Turn};
use crate::memory::short_id;
use crate::store::{SessionRow, Store, StoreResult, WorkspaceRow};
use crate::usage::logs::Sources;

/// How much text a digest may hold, in characters: about 8k tokens.
pub const DEFAULT_BUDGET: usize = 32_000;
/// The newest turns of each conversation, which are given whole while they fit.
const RECENT_TURNS: usize = 5;
/// A conversation's first message is its task: kept, shortened to this, whatever its age.
const FIRST_CHARS: usize = 2_000;
/// An older turn, when there is room for it: the user's message and the reply, shortened.
const OLDER_USER_CHARS: usize = 1_000;
const OLDER_REPLY_CHARS: usize = 600;
/// Where a first message's own words end and Yardsort's additions begin.
const MEMORY_MARKER: &str = "\n\n## Project memory\n";
const HANDOFF_MARKER: &str = "# Handoff from Yardsort";
/// How long before its record Codex may have started a session file: the file is the agent's,
/// the record Yardsort's, and they are not written at the same instant.
const CODEX_SLACK_MS: i64 = 60_000;

/// One conversation in the workspace, as read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    /// The session record's id.
    pub id: String,
    pub harness: String,
    pub model: Option<String>,
    pub started_at: i64,
    pub running: bool,
    /// The conversation this one was forked from, by record id. A fork's transcript repeats the
    /// turns before it; those are left to the conversation they belong to.
    pub forked_from: Option<String>,
    pub turns: Vec<Turn>,
    /// Why its transcript could not be read, when it could not.
    pub unreadable: Option<String>,
}

impl Conversation {
    fn last_at(&self) -> i64 {
        self.turns.last().map_or(self.started_at, |t| t.at)
    }
}

/// Read every conversation in `workspace` but `except` — the asking agent's own — newest activity
/// first. A conversation whose transcript cannot be found or read is still listed, with why.
pub fn gather(
    store: &Store,
    workspace: &WorkspaceRow,
    except: Option<&str>,
    sources: &Sources,
    now: i64,
) -> StoreResult<Vec<Conversation>> {
    let mut out = Vec::new();
    for session in store.sessions(&workspace.id)? {
        if Some(session.id.as_str()) == except {
            continue;
        }
        let read = read_session(store, workspace, &session, sources, now)?;
        let (mut turns, unreadable) = match read {
            Ok(turns) => (turns, None),
            Err(why) => (Vec::new(), Some(why)),
        };
        if let Some(first) = turns.first_mut() {
            first.user = own_words(&first.user);
        }
        if session.forked_from.is_some() {
            turns.retain(|turn| turn.at >= session.started_at);
        }
        // A conversation that has not said anything yet has nothing to tell.
        if turns.is_empty() && unreadable.is_none() {
            continue;
        }
        out.push(Conversation {
            id: session.id,
            harness: session.harness_id,
            model: session.model,
            started_at: session.started_at,
            running: session.running,
            forked_from: session.forked_from,
            turns,
            unreadable,
        });
    }
    // Newest activity first; those that cannot be read after the rest.
    out.sort_by(|a, b| {
        (a.unreadable.is_some(), b.last_at(), &a.id).cmp(&(
            b.unreadable.is_some(),
            a.last_at(),
            &b.id,
        ))
    });
    Ok(out)
}

/// A first message without what Yardsort added after the user's words.
fn own_words(text: &str) -> String {
    if text.starts_with(HANDOFF_MARKER) {
        return "[a handoff packet from Yardsort]".to_owned();
    }
    match text.find(MEMORY_MARKER) {
        Some(at) => text[..at].trim_end().to_owned(),
        None => text.to_owned(),
    }
}

fn read_session(
    store: &Store,
    workspace: &WorkspaceRow,
    session: &SessionRow,
    sources: &Sources,
    now: i64,
) -> StoreResult<Result<Vec<Turn>, String>> {
    let read = |path: &Path, reader: fn(std::io::BufReader<std::fs::File>) -> _| {
        std::fs::File::open(path)
            .map(std::io::BufReader::new)
            .and_then(reader)
            .map_err(|e| format!("its transcript could not be read: {e}"))
    };
    Ok(match session.harness_id.as_str() {
        "claude" => {
            let Some(id) = session.harness_session_id.as_deref() else {
                return Ok(Err(
                    "Yardsort did not choose its id, so its transcript cannot be found".to_owned(),
                ));
            };
            match transcript::claude_file(&sources.claude, id) {
                Some(path) => read(&path, transcript::read_claude),
                None => Err("its transcript was not found".to_owned()),
            }
        }
        "codex" => {
            let Some(home) = sources.codex.as_deref() else {
                return Ok(Err("Codex's home folder is not known".to_owned()));
            };
            let mut files: Vec<_> = store
                .reported_threads(&session.id)?
                .iter()
                .filter_map(|thread| super::codex::find_rollout(home, thread))
                .collect();
            if files.is_empty() {
                let to = session.ended_at.unwrap_or(now);
                files.extend(transcript::codex_file_in(
                    home,
                    Path::new(&workspace.path),
                    session.started_at - CODEX_SLACK_MS,
                    to,
                ));
            }
            if files.is_empty() {
                return Ok(Err("its session file was not found".to_owned()));
            }
            let mut turns = Vec::new();
            for file in files {
                match read(&file, transcript::read_codex) {
                    Ok(more) => turns.extend(more),
                    Err(why) => return Ok(Err(why)),
                }
            }
            turns.sort_by_key(|t| t.at);
            turns.dedup();
            Ok(turns)
        }
        other => Err(format!("Yardsort cannot read {other}'s conversations yet")),
    })
}

/// What to show.
#[derive(Debug, Clone)]
pub struct Options {
    /// Characters, across every conversation.
    pub budget: usize,
    /// Only the conversation whose id starts with this.
    pub conversation: Option<String>,
    /// Only turns from this instant on, epoch milliseconds.
    pub since: Option<i64>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            budget: DEFAULT_BUDGET,
            conversation: None,
            since: None,
        }
    }
}

/// The digest: what fitted, and what did not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Digest {
    pub conversations: Vec<Shown>,
}

/// One conversation in the digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    /// The record's short id, as `--conversation` takes it.
    pub id: String,
    pub harness: String,
    pub model: Option<String>,
    pub started_at: i64,
    pub last_turn_at: Option<i64>,
    pub running: bool,
    pub forked_from: Option<String>,
    /// How many turns the conversation has.
    pub turn_count: u32,
    /// What is shown, newest first.
    pub turns: Vec<ShownTurn>,
    /// The turns left out for the budget, by number, oldest first.
    pub omitted: Vec<u32>,
    pub unreadable: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShownTurn {
    /// Counted from 1, in the conversation.
    pub n: u32,
    pub at: i64,
    pub user: String,
    pub reply: Option<String>,
    /// Whether either text was cut to fit.
    pub shortened: bool,
}

/// How much of a turn is shown. Ordered from least to most.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Form {
    /// The user's message, cut to `n` characters.
    User(usize),
    /// Both, cut to these.
    Both(usize, usize),
    Whole,
}

fn shown_turn(n: usize, turn: &Turn, form: Form) -> ShownTurn {
    let reply = turn.reply();
    let (user, reply) = match form {
        Form::Whole => (turn.user.clone(), reply.map(str::to_owned)),
        Form::Both(u, r) => (cut(&turn.user, u), reply.map(|text| cut(text, r))),
        Form::User(u) => (cut(&turn.user, u), None),
    };
    let shortened = user != turn.user || reply.as_deref() != turn.reply();
    ShownTurn {
        n: n as u32,
        at: turn.at,
        user,
        reply,
        shortened,
    }
}

/// Fit the conversations into the budget. The same input gives the same digest: first each
/// conversation's first message, its task; then each one's newest turns, whole if they fit and
/// shortened if not; then the older turns, shortened, newest first. A turn that does not fit is
/// counted as left out, and a smaller one after it may still fit.
pub fn select(conversations: &[Conversation], options: &Options) -> Digest {
    let wanted: Vec<&Conversation> = conversations
        .iter()
        .filter(|c| {
            options
                .conversation
                .as_deref()
                .is_none_or(|prefix| c.id.starts_with(prefix))
        })
        .collect();
    // Turn numbers are the conversation's; `since` only hides the earlier ones.
    let eligible = |c: &Conversation, i: usize| options.since.is_none_or(|t| c.turns[i].at >= t);
    let mut chosen: Vec<Vec<Option<Form>>> =
        wanted.iter().map(|c| vec![None; c.turns.len()]).collect();
    let mut used: usize = wanted
        .iter()
        .map(|c| render_header(&header(c, &[])).chars().count())
        .sum();
    let cost = |c: &Conversation, i: usize, form: Form| {
        render_turn(&shown_turn(i + 1, &c.turns[i], form))
            .chars()
            .count()
    };
    let mut offer = |chosen: &mut Vec<Vec<Option<Form>>>, ci: usize, i: usize, forms: &[Form]| {
        let c = wanted[ci];
        let had = chosen[ci][i].map_or(0, |form| cost(c, i, form));
        for &form in forms {
            if chosen[ci][i].is_some_and(|current| current >= form) {
                return;
            }
            let more = cost(c, i, form);
            if used - had + more <= options.budget {
                used = used - had + more;
                chosen[ci][i] = Some(form);
                return;
            }
        }
    };
    let older = [
        Form::Both(OLDER_USER_CHARS, OLDER_REPLY_CHARS),
        Form::User(OLDER_USER_CHARS),
    ];
    let recent = [Form::Whole, older[0], older[1]];
    for (ci, c) in wanted.iter().enumerate() {
        if !c.turns.is_empty() && eligible(c, 0) && c.turns.len() > RECENT_TURNS {
            offer(&mut chosen, ci, 0, &[Form::User(FIRST_CHARS)]);
        }
    }
    for (ci, c) in wanted.iter().enumerate() {
        for i in (c.turns.len().saturating_sub(RECENT_TURNS)..c.turns.len()).rev() {
            if eligible(c, i) {
                offer(&mut chosen, ci, i, &recent);
            }
        }
    }
    // The first message keeps its longer cut when it gains its reply.
    let first = [
        Form::Both(FIRST_CHARS, OLDER_REPLY_CHARS),
        Form::User(FIRST_CHARS),
    ];
    for (ci, c) in wanted.iter().enumerate() {
        for i in (0..c.turns.len().saturating_sub(RECENT_TURNS)).rev() {
            if eligible(c, i) {
                offer(&mut chosen, ci, i, if i == 0 { &first } else { &older });
            }
        }
    }
    let conversations = wanted
        .iter()
        .zip(chosen)
        .map(|(c, forms)| {
            let turns: Vec<ShownTurn> = forms
                .iter()
                .enumerate()
                .rev()
                .filter_map(|(i, form)| form.map(|form| shown_turn(i + 1, &c.turns[i], form)))
                .collect();
            let omitted: Vec<u32> = forms
                .iter()
                .enumerate()
                .filter(|(i, form)| form.is_none() && eligible(c, *i))
                .map(|(i, _)| i as u32 + 1)
                .collect();
            let mut shown = header(c, &omitted);
            shown.turns = turns;
            shown
        })
        .collect();
    Digest { conversations }
}

fn header(c: &Conversation, omitted: &[u32]) -> Shown {
    Shown {
        id: short_id(&c.id).to_owned(),
        harness: c.harness.clone(),
        model: c.model.clone(),
        started_at: c.started_at,
        last_turn_at: c.turns.last().map(|t| t.at),
        running: c.running,
        forked_from: c.forked_from.as_deref().map(|id| short_id(id).to_owned()),
        turn_count: c.turns.len() as u32,
        turns: Vec::new(),
        omitted: omitted.to_vec(),
        unreadable: c.unreadable.clone(),
    }
}

/// One turn of one conversation, whole: the user's message and everything the agent wrote in
/// reply — its commentary between tool calls too — still without the tool calls.
pub fn render_whole_turn(c: &Conversation, n: usize) -> Option<String> {
    let turn = c.turns.get(n.checked_sub(1)?)?;
    let mut out = format!(
        "## {} · conversation {} · turn {n} of {} · {}\n\n",
        agent(&c.harness, c.model.as_deref()),
        short_id(&c.id),
        c.turns.len(),
        when(turn.at)
    );
    out.push_str("User:\n");
    out.push_str(&quote(&turn.user));
    for said in &turn.said {
        out.push_str("\nAgent:\n");
        out.push_str(&quote(said));
    }
    if turn.said.is_empty() {
        out.push_str("\nThe agent wrote no reply in this turn.\n");
    }
    Some(out)
}

/// The digest as an agent reads it.
pub fn render(digest: &Digest) -> String {
    let mut out = String::from("# What was said in this workspace\n\n");
    out.push_str(
        "Yardsort read this from each agent's own transcript: every message the user typed, and \
         the agent's reply at the end of each turn, newest turn first. Tool calls, their output \
         and the agents' reasoning are left out; read the files for the state of the work. \
         Times are UTC.\n",
    );
    if digest.conversations.is_empty() {
        out.push_str("\nNo other conversation in this workspace has said anything yet.\n");
        return out;
    }
    for shown in &digest.conversations {
        if !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(&render_header(shown));
        for turn in &shown.turns {
            out.push_str(&render_turn(turn));
        }
        if !shown.omitted.is_empty() {
            out.push_str(&format!(
                "\nNot shown, to keep this short: {} {}. `ys workspace context --conversation {} \
                 --turn N` shows one whole.\n",
                if shown.omitted.len() == 1 {
                    "turn"
                } else {
                    "turns"
                },
                ranges(&shown.omitted),
                shown.id
            ));
        }
    }
    out
}

fn agent(harness: &str, model: Option<&str>) -> String {
    match model {
        Some(model) => format!("{harness} ({model})"),
        None => harness.to_owned(),
    }
}

fn render_header(shown: &Shown) -> String {
    let mut line = format!(
        "## {} · conversation {}",
        agent(&shown.harness, shown.model.as_deref()),
        shown.id
    );
    // A conversation that could not be read has no count to give.
    match (shown.turn_count, &shown.unreadable) {
        (0, Some(_)) => {}
        (1, _) => line.push_str(" · 1 turn"),
        (n, _) => line.push_str(&format!(" · {n} turns")),
    }
    line.push_str(&format!(" · started {}", when(shown.started_at)));
    if let Some(last) = shown.last_turn_at {
        line.push_str(&format!(" · last turn {}", when(last)));
    }
    line.push_str(if shown.running {
        " · running"
    } else {
        " · ended"
    });
    line.push_str("\n\n");
    if let Some(from) = &shown.forked_from {
        line.push_str(&format!(
            "Forked from conversation {from}: the turns before the fork are that one's.\n\n"
        ));
    }
    if let Some(why) = &shown.unreadable {
        line.push_str(&format!("Its conversation is not here: {why}.\n\n"));
    }
    line
}

fn render_turn(turn: &ShownTurn) -> String {
    let mut out = format!("### Turn {} · {}\n\nUser:\n", turn.n, when(turn.at));
    out.push_str(&quote(&turn.user));
    match &turn.reply {
        Some(reply) => {
            out.push_str("\nAgent:\n");
            out.push_str(&quote(reply));
        }
        None if turn.shortened => out.push_str("\nThe reply is left out, to keep this short.\n"),
        None => out.push_str("\nNo reply yet.\n"),
    }
    out.push('\n');
    out
}

/// Text as a Markdown quote, so a heading in a message is not taken for one of the digest's.
fn quote(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        if line.is_empty() {
            out.push_str(">\n");
        } else {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// At most `max` characters, cut at a space near the end when there is one, marked with `…`, and
/// with a code fence closed if the cut left one open.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut kept: String = text.chars().take(max).collect();
    let near = kept
        .char_indices()
        .rev()
        .take(80)
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, _)| i);
    if let Some(at) = near {
        kept.truncate(at);
    }
    let mut kept = kept.trim_end().to_owned();
    kept.push('…');
    let fences = kept
        .lines()
        .filter(|line| line.trim_start().starts_with("```"))
        .count();
    if fences % 2 == 1 {
        kept.push_str("\n```");
    }
    kept
}

/// `1–3, 5, 7–9`.
fn ranges(numbers: &[u32]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < numbers.len() {
        let start = numbers[i];
        let mut end = start;
        while i + 1 < numbers.len() && numbers[i + 1] == end + 1 {
            i += 1;
            end = numbers[i];
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}–{end}")
        });
        i += 1;
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{NewEvent, NewRun, NewSession};
    use std::path::PathBuf;

    const T0: i64 = 1_792_900_000_000;

    fn turn(at: i64, user: &str, reply: &str) -> Turn {
        Turn {
            at,
            user: user.to_owned(),
            said: if reply.is_empty() {
                vec![]
            } else {
                vec![reply.to_owned()]
            },
        }
    }

    fn conversation(id: &str, turns: Vec<Turn>) -> Conversation {
        Conversation {
            id: id.to_owned(),
            harness: "claude".to_owned(),
            model: None,
            started_at: T0,
            running: false,
            forked_from: None,
            turns,
            unreadable: None,
        }
    }

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
    }

    fn workspace(store: &Store, path: &Path) -> WorkspaceRow {
        let project = store.add_project("app", "/code/app").unwrap();
        store
            .add_worktree(
                &project.id,
                "fix-login",
                &path.to_string_lossy(),
                Some("ys/fix"),
                Some("main"),
            )
            .unwrap()
    }

    /// A Claude Code config directory holding the recorded conversation and its fork, under the
    /// ids `claude-a` and `claude-b`, and a Codex home holding the recorded thread.
    fn sources(dir: &Path) -> Sources {
        let claude = dir.join("claude");
        let folder = claude.join("projects").join("-wt-fix");
        std::fs::create_dir_all(&folder).unwrap();
        let recorded = fixtures().join("claude-transcript/2.1.302");
        std::fs::copy(
            recorded.join("conversation.jsonl"),
            folder.join("claude-a.jsonl"),
        )
        .unwrap();
        std::fs::copy(recorded.join("fork.jsonl"), folder.join("claude-b.jsonl")).unwrap();
        let codex = dir.join("codex");
        let day = codex.join("sessions/2026/10/25");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::copy(
            fixtures().join("codex/0.163.0/conversation.jsonl"),
            day.join("rollout-2026-10-25T13-35-00-01a0d83f-0000-7000-8000-000000000001.jsonl"),
        )
        .unwrap();
        Sources {
            claude: vec![claude],
            codex: Some(codex),
            grok: None,
        }
    }

    fn session(store: &Store, ws: &str, id: &str, harness: &str, native: Option<&str>) {
        store
            .add_session(&NewSession {
                id,
                workspace_id: ws,
                harness_id: harness,
                harness_session_id: native,
                title: id,
                pty_session_id: &format!("pty-{id}"),
                ..Default::default()
            })
            .unwrap();
    }

    #[test]
    fn every_other_conversation_is_read_from_its_own_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace(&store, Path::new("/tmp/yardsort-fixture/repo"));
        let sources = sources(dir.path());
        session(&store, &ws.id, "rec-claude", "claude", Some("claude-a"));
        session(&store, &ws.id, "rec-codex", "codex", None);
        session(&store, &ws.id, "rec-asking", "claude", Some("claude-b"));
        session(&store, &ws.id, "rec-grok", "grok", None);
        // The Codex record starts as the recorded session file did.
        let codex_start =
            transcript::codex_meta(&fixtures().join("codex/0.163.0/conversation.jsonl"))
                .unwrap()
                .started_at
                .unwrap();
        store.set_session_started("rec-codex", codex_start);
        let now = codex_start + 3_600_000;

        let found = gather(&store, &ws, Some("rec-asking"), &sources, now).unwrap();
        let ids: Vec<&str> = found.iter().map(|c| c.id.as_str()).collect();
        assert!(
            !ids.contains(&"rec-asking"),
            "the asking agent's own is left out"
        );
        let by = |id: &str| found.iter().find(|c| c.id == id).unwrap();
        assert_eq!(by("rec-claude").turns.len(), 2);
        assert!(by("rec-claude").unreadable.is_none());
        assert_eq!(
            by("rec-codex").turns.len(),
            2,
            "found by where and when it began"
        );
        assert_eq!(
            by("rec-grok").unreadable.as_deref(),
            Some("Yardsort cannot read grok's conversations yet")
        );
        // Newest activity first: Codex was recorded after Claude.
        assert_eq!(ids[0], "rec-codex");

        let text = render(&select(&found, &Options::default()));
        assert!(
            text.contains("> Thanks. In one sentence, what is left to do?"),
            "{text}"
        );
        assert!(text.contains("> Nothing is left"), "{text}");
        assert!(text.contains("Its conversation is not here: Yardsort cannot read grok's"));
        assert!(
            text.contains("## grok · conversation rec-grok · started"),
            "{text}"
        );
        assert!(!text.contains("\n\n\n"), "{text}");
        assert!(!text.contains("File created successfully"), "{text}");
        assert!(!text.contains("tool_use"), "{text}");
    }

    #[test]
    fn a_reported_thread_names_the_codex_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        // Not where the file says it began: the thread id alone finds it.
        let ws = workspace(&store, Path::new("/somewhere/else"));
        let sources = sources(dir.path());
        session(&store, &ws.id, "rec-codex", "codex", None);
        store
            .add_run(&NewRun {
                id: "run-codex",
                workspace_id: &ws.id,
                session_id: Some("rec-codex"),
                kind: "harness",
                harness_id: Some("codex"),
                harness_session_id: None,
                launched_by: "app",
            })
            .unwrap();
        store
            .add_event(&NewEvent {
                id: "e1",
                schema_version: 1,
                workspace_id: &ws.id,
                session_id: None,
                run_id: Some("run-codex"),
                occurred_at: T0,
                kind: "turn.completed",
                producer: "codex",
                method: "session_file",
                fidelity: "reported",
                source_key: None,
                privacy_class: "metadata",
                payload: r#"{"threadId":"01a0d83f-0000-7000-8000-000000000001"}"#,
            })
            .unwrap();
        let found = gather(&store, &ws, None, &sources, T0).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].turns.len(), 2, "{:?}", found[0].unreadable);
    }

    #[test]
    fn a_fork_shows_only_its_own_turns() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace(&store, Path::new("/wt/fix"));
        let sources = sources(dir.path());
        session(&store, &ws.id, "rec-a", "claude", Some("claude-a"));
        store
            .add_session(&NewSession {
                id: "rec-b",
                workspace_id: &ws.id,
                harness_id: "claude",
                harness_session_id: Some("claude-b"),
                title: "fork",
                forked_from: Some("rec-a"),
                pty_session_id: "pty-rec-b",
                ..Default::default()
            })
            .unwrap();
        let fork_at = transcript::read_claude(std::io::BufReader::new(
            std::fs::File::open(dir.path().join("claude/projects/-wt-fix/claude-b.jsonl")).unwrap(),
        ))
        .unwrap()[2]
            .at
            - 1_000;
        store.set_session_started("rec-b", fork_at);
        let found = gather(&store, &ws, Some("rec-a"), &sources, T0).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].turns.len(), 1);
        assert_eq!(
            found[0].turns[0].user,
            "In one sentence, say which file you created."
        );
        let text = render(&select(&found, &Options::default()));
        assert!(text.contains("Forked from conversation rec-a: "), "{text}");
    }

    #[test]
    fn what_yardsort_added_to_a_first_message_is_not_the_users() {
        assert_eq!(
            own_words("fix the login\n\n## Project memory\n\n- (memory ab12) the tests need TZ"),
            "fix the login"
        );
        assert_eq!(
            own_words("# Handoff from Yardsort: workspace \"x\" in app\n\nYou are taking over"),
            "[a handoff packet from Yardsort]"
        );
        assert_eq!(
            own_words("## Project memory is a heading here"),
            "## Project memory is a heading here"
        );
    }

    #[test]
    fn the_budget_keeps_the_task_and_the_newest_turns_and_counts_the_rest() {
        let long = "word ".repeat(400); // 2,000 characters
        let turns: Vec<Turn> = (0..20)
            .map(|i| {
                turn(
                    T0 + i * 60_000,
                    &format!("message {} {long}", i + 1),
                    &format!("reply {} {long}", i + 1),
                )
            })
            .collect();
        let c = conversation("conv-0001-aaaa", turns);
        let digest = select(
            std::slice::from_ref(&c),
            &Options {
                budget: 16_000,
                ..Options::default()
            },
        );
        let shown = &digest.conversations[0];
        let numbers: Vec<u32> = shown.turns.iter().map(|t| t.n).collect();
        // Newest first; the last turns whole; the first message, the task, kept and shortened.
        assert_eq!(numbers.first(), Some(&20));
        assert_eq!(numbers.last(), Some(&1));
        assert!(!shown.turns[0].shortened);
        let first = shown.turns.last().unwrap();
        assert!(first.user.starts_with("message 1 ") && first.user.ends_with('…'));
        assert!(
            first.user.chars().count() > OLDER_USER_CHARS,
            "the task keeps its longer cut"
        );
        assert!(!shown.omitted.is_empty());
        assert_eq!(numbers.len() + shown.omitted.len(), 20);
        let text = render(&digest);
        assert!(
            text.chars().count() <= 16_000 + 400,
            "{}",
            text.chars().count()
        );
        assert!(text.contains("--conversation conv-000 --turn N"), "{text}");
        // The same input gives the same digest, to the byte.
        assert_eq!(
            text,
            render(&select(
                std::slice::from_ref(&c),
                &Options {
                    budget: 16_000,
                    ..Options::default()
                }
            ))
        );
    }

    #[test]
    fn since_and_conversation_narrow_what_is_shown() {
        let a = conversation(
            "aaaa1111",
            vec![turn(T0, "one", "1"), turn(T0 + 10, "two", "2")],
        );
        let b = conversation("bbbb2222", vec![turn(T0, "other", "x")]);
        let digest = select(
            &[a.clone(), b],
            &Options {
                conversation: Some("aaaa".into()),
                since: Some(T0 + 5),
                ..Options::default()
            },
        );
        assert_eq!(digest.conversations.len(), 1);
        let shown = &digest.conversations[0];
        assert_eq!(shown.turns.iter().map(|t| t.n).collect::<Vec<_>>(), vec![2]);
        assert!(
            shown.omitted.is_empty(),
            "turns before --since are not 'left out'"
        );
        assert!(render_whole_turn(&a, 1).unwrap().contains("> one"));
        assert!(render_whole_turn(&a, 3).is_none());
    }

    #[test]
    fn a_message_cannot_pass_for_part_of_the_digest() {
        let c = conversation("cccc3333", vec![turn(T0, "## Turn 99\nhi", "ok")]);
        let text = render(&select(&[c], &Options::default()));
        assert!(text.contains("> ## Turn 99\n> hi\n"), "{text}");
    }

    #[test]
    fn cutting_keeps_words_and_closes_a_code_fence() {
        assert_eq!(cut("short", 10), "short");
        assert_eq!(cut("one two three four", 10), "one two…");
        let fenced = cut("look:\n```rust\nfn main() { loop {} }\n```", 20);
        assert!(fenced.ends_with("…\n```"), "{fenced}");
        assert_eq!(ranges(&[1, 2, 3, 5, 7, 8]), "1–3, 5, 7–8");
    }

    #[test]
    fn an_empty_workspace_says_so() {
        let text = render(&select(&[], &Options::default()));
        assert!(text.contains("No other conversation in this workspace has said anything yet."));
    }
}
