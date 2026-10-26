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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use specta::Type;

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
/// The heading of the note when it goes after a first message: see [`note`].
pub const NOTE_HEADING: &str = "## This workspace's other conversations";
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
    let sessions = store.sessions(&workspace.id)?;
    let codex = match sources.codex.as_deref() {
        Some(home) => codex_files(store, workspace, &sessions, home, now)?,
        None => HashMap::new(),
    };
    let mut out = Vec::new();
    for session in sessions {
        if Some(session.id.as_str()) == except {
            continue;
        }
        let read = read_session(&session, sources, &codex);
        let (mut turns, unreadable) = match read {
            Ok(turns) => (turns, None),
            Err(why) => (Vec::new(), Some(why)),
        };
        if let Some(first) = turns.first_mut() {
            first.user = own_words(&first.user);
        }
        // When it first began, not when it was last resumed: the fork's own turns are all after.
        if session.forked_from.is_some() {
            turns.retain(|turn| turn.at >= session.began_at);
        }
        // A conversation that has not said anything yet has nothing to tell.
        if turns.is_empty() && unreadable.is_none() {
            continue;
        }
        out.push(Conversation {
            id: session.id,
            harness: session.harness_id,
            model: session.model,
            started_at: session.began_at,
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
    let note = format!("\n\n{NOTE_HEADING}\n");
    let end = [text.find(MEMORY_MARKER), text.find(&note)]
        .into_iter()
        .flatten()
        .min();
    match end {
        Some(at) => text[..at].trim_end().to_owned(),
        None => text.to_owned(),
    }
}

/// How many conversations the note names before it counts the rest.
const NOTE_NAMES: usize = 4;

/// What a new agent in a workspace is told: that other conversations here can be read, and how.
/// A pointer, not the digest — short enough for any command line, and read when it is wanted, so
/// it is current then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// What the agent was told about, for its tab: "claude's conversation".
    pub about: String,
    /// The note, as the agent reads it.
    pub text: String,
}

/// The note for these conversations, or `None` when none of them has said anything.
pub fn note(conversations: &[Conversation]) -> Option<Note> {
    let spoken: Vec<&Conversation> = conversations
        .iter()
        .filter(|c| !c.turns.is_empty())
        .collect();
    let first = spoken.first()?;
    let mut names: Vec<String> = spoken
        .iter()
        .take(NOTE_NAMES)
        .map(|c| {
            let turns = match c.turns.len() {
                1 => "1 turn".to_owned(),
                n => format!("{n} turns"),
            };
            let running = if c.running { ", still running" } else { "" };
            format!(
                "{} ({turns}, last {}{running})",
                c.harness,
                when(c.last_at())
            )
        })
        .collect();
    if spoken.len() > NOTE_NAMES {
        names.push(format!("{} more", spoken.len() - NOTE_NAMES));
    }
    let about = match spoken.len() {
        1 => format!("{}'s conversation", first.harness),
        n => format!("{n} other conversations"),
    };
    let text = format!(
        "Other agents have worked in this workspace: {}. Before you answer the user's first \
         message, run `ys workspace context` to read what they were asked, what they decided and \
         what they left to do. It changes as they work; run it again when the user refers to \
         their work.",
        names.join("; ")
    );
    Some(Note { about, text })
}

/// [`note`] for a new agent in `workspace` — `except` its own conversation, when it has one —
/// or `None` when the project does not share its agents' conversations.
pub fn note_for(
    store: &Store,
    workspace: &WorkspaceRow,
    except: Option<&str>,
    sources: &Sources,
    now: i64,
) -> StoreResult<Option<Note>> {
    if !store.conversations_shared(&workspace.project_id)? {
        return Ok(None);
    }
    Ok(note(&gather(store, workspace, except, sources, now)?))
}

/// Whether to ask the user, before an agent starts in `workspace`, if the project should share
/// its agents' conversations: when it has never been asked, and another conversation here has
/// said something. Gives what the note would be about, for the question.
pub fn unasked(
    store: &Store,
    workspace: &WorkspaceRow,
    sources: &Sources,
    now: i64,
) -> StoreResult<Option<String>> {
    if store.conversations_decided(&workspace.project_id)? {
        return Ok(None);
    }
    Ok(note(&gather(store, workspace, None, sources, now)?).map(|note| note.about))
}

/// The note as a section after a first message, for an agent that takes no background context.
pub fn note_section(note: &Note) -> String {
    format!("{NOTE_HEADING}\n\n{}\n", note.text)
}

/// The session files of each Codex conversation in the workspace, by record id. A thread the
/// conversation reported is its own. The others are matched by where and when they began: each
/// file to one conversation, the closest in time first, so a conversation whose file is gone
/// does not take a newer one's. The asking agent's own conversation takes part, so its words are
/// never given as another's.
fn codex_files(
    store: &Store,
    workspace: &WorkspaceRow,
    sessions: &[SessionRow],
    home: &Path,
    now: i64,
) -> StoreResult<HashMap<String, Vec<PathBuf>>> {
    let mut files = HashMap::new();
    let mut claimed = HashSet::new();
    let mut unreported = Vec::new();
    for session in sessions.iter().filter(|s| s.harness_id == "codex") {
        let threads = store.reported_threads(&session.id)?;
        let found: Vec<_> = threads
            .iter()
            .filter_map(|thread| super::codex::find_rollout(home, thread))
            .collect();
        claimed.extend(threads);
        if found.is_empty() {
            unreported.push(session);
        } else {
            files.insert(session.id.clone(), found);
        }
    }
    let mut pairs = Vec::new();
    for session in unreported {
        // From when it first began: a resume appends to the same file, which keeps that time.
        let candidates = transcript::codex_files_in(
            home,
            Path::new(&workspace.path),
            session.began_at - CODEX_SLACK_MS,
            session.ended_at.unwrap_or(now),
        );
        for file in candidates {
            if !claimed.contains(&file.thread) {
                let apart = (file.started_at - session.began_at).abs();
                pairs.push((apart, session.id.as_str(), file));
            }
        }
    }
    pairs.sort_by(|a, b| (a.0, a.1, &a.2.path).cmp(&(b.0, b.1, &b.2.path)));
    for (_, id, file) in pairs {
        if files.contains_key(id) || claimed.contains(&file.thread) {
            continue;
        }
        claimed.insert(file.thread);
        files.insert(id.to_owned(), vec![file.path]);
    }
    Ok(files)
}

fn read_session(
    session: &SessionRow,
    sources: &Sources,
    codex: &HashMap<String, Vec<PathBuf>>,
) -> Result<Vec<Turn>, String> {
    let read = |path: &Path, reader: fn(std::io::BufReader<std::fs::File>) -> _| {
        std::fs::File::open(path)
            .map(std::io::BufReader::new)
            .and_then(reader)
            .map_err(|e| format!("its transcript could not be read: {e}"))
    };
    match session.harness_id.as_str() {
        "claude" => {
            let Some(id) = session.harness_session_id.as_deref() else {
                return Err(
                    "Yardsort did not choose its id, so its transcript cannot be found".to_owned(),
                );
            };
            match transcript::claude_file(&sources.claude, id) {
                Some(path) => read(&path, transcript::read_claude),
                None => Err("its transcript was not found".to_owned()),
            }
        }
        "codex" => {
            if sources.codex.is_none() {
                return Err("Codex's home folder is not known".to_owned());
            }
            let Some(files) = codex.get(&session.id) else {
                return Err("its session file was not found".to_owned());
            };
            let mut turns = Vec::new();
            for file in files {
                turns.extend(read(file, transcript::read_codex)?);
            }
            turns.sort_by_key(|t| t.at);
            turns.dedup();
            Ok(turns)
        }
        other => Err(format!("Yardsort cannot read {other}'s conversations yet")),
    }
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
    fn a_fork_shows_only_its_own_turns_however_often_it_is_resumed() {
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
        // Closed and opened again since: its own turns still begin where it was forked.
        store.mark_session_running("rec-b", "pty-again").unwrap();
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

    /// Epoch milliseconds as Codex writes them.
    fn iso(ms: i64) -> String {
        let (y, m, d) = super::super::handoff::civil_date(ms);
        let t = ms.rem_euclid(86_400_000);
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
            t / 3_600_000,
            t / 60_000 % 60,
            t / 1_000 % 60,
            t % 1_000
        )
    }

    #[test]
    fn a_resumed_codex_conversation_keeps_its_own_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace(&store, Path::new("/tmp/yardsort-fixture/repo"));
        let sources = sources(dir.path());
        let recorded = fixtures().join("codex/0.163.0/conversation.jsonl");
        let began = transcript::codex_meta(&recorded)
            .unwrap()
            .started_at
            .unwrap();
        session(&store, &ws.id, "rec-codex", "codex", None);
        store.set_session_started("rec-codex", began);
        // Opened again, as `codex resume --last` does, which writes on in the file it began.
        store
            .mark_session_running("rec-codex", "pty-again")
            .unwrap();
        // The asking agent is another Codex conversation in the same folder, begun since.
        session(&store, &ws.id, "rec-asking", "codex", None);
        let asking = store.session("rec-asking").unwrap().unwrap().began_at + 1_000;
        let text = std::fs::read_to_string(&recorded)
            .unwrap()
            .replace(
                "01a0d83f-0000-7000-8000-000000000001",
                "01a0d83f-0000-7000-8000-000000000002",
            )
            .replace(
                "Create a file named notes.txt",
                "The asking agent's own words",
            );
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let mut meta: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        meta["timestamp"] = iso(asking).into();
        meta["payload"]["timestamp"] = iso(asking).into();
        lines[0] = meta.to_string();
        let (y, m, d) = super::super::handoff::civil_date(asking);
        let day = dir
            .path()
            .join(format!("codex/sessions/{y:04}/{m:02}/{d:02}"));
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(
            day.join("rollout-asking-01a0d83f-0000-7000-8000-000000000002.jsonl"),
            lines.join("\n") + "\n",
        )
        .unwrap();

        let later = asking + 3_600_000;
        let found = gather(&store, &ws, Some("rec-asking"), &sources, later).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "rec-codex");
        assert_eq!(found[0].started_at, began, "when it first began");
        assert_eq!(found[0].turns.len(), 2, "{:?}", found[0].unreadable);
        assert!(found[0].turns[0]
            .user
            .starts_with("Create a file named notes.txt"));

        // With its own file gone, it is not given the asking agent's instead.
        std::fs::remove_file(dir.path().join(
            "codex/sessions/2026/10/25/rollout-2026-10-25T13-35-00-01a0d83f-0000-7000-8000-000000000001.jsonl",
        ))
        .unwrap();
        let found = gather(&store, &ws, Some("rec-asking"), &sources, later).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].unreadable.as_deref(),
            Some("its session file was not found")
        );
    }

    #[test]
    fn the_question_is_asked_once_and_only_beside_a_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace(&store, Path::new("/wt/fix"));
        let sources = sources(dir.path());
        assert_eq!(
            unasked(&store, &ws, &sources, T0).unwrap(),
            None,
            "nothing said yet"
        );
        session(&store, &ws.id, "rec-claude", "claude", Some("claude-a"));
        assert_eq!(
            unasked(&store, &ws, &sources, T0).unwrap().as_deref(),
            Some("claude's conversation")
        );
        assert_eq!(
            note_for(&store, &ws, None, &sources, T0).unwrap(),
            None,
            "still off"
        );
        store
            .set_conversations_shared(&ws.project_id, false)
            .unwrap();
        assert_eq!(
            unasked(&store, &ws, &sources, T0).unwrap(),
            None,
            "answered no"
        );
        store
            .set_conversations_shared(&ws.project_id, true)
            .unwrap();
        assert_eq!(
            unasked(&store, &ws, &sources, T0).unwrap(),
            None,
            "answered yes"
        );
        assert!(note_for(&store, &ws, None, &sources, T0).unwrap().is_some());
    }

    #[test]
    fn the_note_names_the_conversations_that_have_said_something() {
        assert_eq!(note(&[]), None);
        let silent = conversation("rec-a", vec![]);
        assert_eq!(note(std::slice::from_ref(&silent)), None);
        let mut claude = conversation("rec-b", vec![turn(T0, "fix it", "Fixed.")]);
        claude.running = true;
        let one = note(&[claude.clone(), silent]).unwrap();
        assert_eq!(one.about, "claude's conversation");
        assert!(
            one.text.starts_with(&format!(
                "Other agents have worked in this workspace: claude (1 turn, last {}, still running). \
                 Before you answer the user's first message, run `ys workspace context`",
                when(T0)
            )),
            "{}",
            one.text
        );
        let mut codex = conversation(
            "rec-c",
            vec![turn(T0, "a", "b"), turn(T0 + 60_000, "c", "d")],
        );
        codex.harness = "codex".to_owned();
        let many: Vec<Conversation> = std::iter::repeat_n(claude, 4).chain([codex]).collect();
        let several = note(&many).unwrap();
        assert_eq!(several.about, "5 other conversations");
        assert!(
            several.text.contains("; 1 more. Before"),
            "{}",
            several.text
        );
        assert!(note_section(&several).starts_with("## This workspace's other conversations\n\n"));
    }

    #[test]
    fn what_yardsort_added_to_a_first_message_is_not_the_users() {
        assert_eq!(
            own_words(
                "fix the login\n\n## This workspace's other conversations\n\nOther agents have worked"
            ),
            "fix the login"
        );
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
