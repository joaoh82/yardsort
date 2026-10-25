//! What was said in a conversation, read from the agent's own transcript.
//!
//! The agents keep their conversations on disk, and these readers take from them only what a
//! person would call the conversation: each message the user typed, and the text the agent wrote
//! in reply. Tool calls, their output, the agent's reasoning and anything the harness injects on
//! its own are left where they are. Nothing here reads a terminal. See
//! `docs/design/29-conversation-context.md`.
//!
//! - **Claude Code**: `projects/<encoded cwd>/<session id>.jsonl` under its config directory, one
//!   JSON object per line. A typed message is a `user` line whose content is text; tool results
//!   are `user` lines too, with `tool_result` content. An assistant message is written once per
//!   content block, several lines sharing a `message.id`. A fork copies the conversation it came
//!   from, with the original times.
//! - **Codex**: `sessions/<y>/<m>/<d>/rollout-<time>-<thread>.jsonl` under `CODEX_HOME`. Its
//!   `event_msg` `item_completed` events carry the user's message (`UserMessage`) and the agent's
//!   (`AgentMessage`, whose `phase` is `final_answer` for the reply); `task_complete` repeats the
//!   last one. A resumed thread is appended to the same file.
//!
//! The files can be large — a long conversation is hundreds of megabytes, nearly all of it tool
//! output — so they are read line by line, and a line that cannot hold anything wanted is skipped
//! before it is parsed. A line without its newline is one still being written, and is not read.

use std::io::{self, BufRead};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::iso_to_ms;

/// One turn: what the user said, and what the agent said back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// When the user's message was written, in epoch milliseconds.
    pub at: i64,
    pub user: String,
    /// Every text the agent wrote in this turn, in order: commentary between tool calls, then
    /// the reply. Empty when it has not answered yet, or wrote only tool calls.
    pub said: Vec<String>,
}

impl Turn {
    /// The agent's last text in the turn: where it says what it did and what is left.
    pub fn reply(&self) -> Option<&str> {
        self.said.last().map(String::as_str)
    }
}

/// Call `each` with every whole line, in order, reading as it goes: a transcript can be larger
/// than is sensible to hold. A line that is not UTF-8 is skipped; a last line without its newline
/// is one the agent is still writing, and is left for the next read.
fn each_line(mut reader: impl BufRead, mut each: impl FnMut(&str)) -> io::Result<()> {
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        if reader.read_until(b'\n', &mut buffer)? == 0 || buffer.last() != Some(&b'\n') {
            return Ok(());
        }
        if let Ok(line) = std::str::from_utf8(&buffer) {
            each(line.trim_end_matches(['\n', '\r']));
        }
    }
}

/// Where Claude Code keeps a conversation, by the session id Yardsort gave it: the first
/// `projects/<any>/<id>.jsonl` under any of its config directories.
pub fn claude_file(config_dirs: &[PathBuf], session_id: &str) -> Option<PathBuf> {
    let name = format!("{session_id}.jsonl");
    config_dirs.iter().find_map(|dir| {
        let projects = std::fs::read_dir(dir.join("projects")).ok()?;
        let mut folders: Vec<PathBuf> = projects.flatten().map(|entry| entry.path()).collect();
        folders.sort();
        folders
            .into_iter()
            .map(|folder| folder.join(&name))
            .find(|file| file.is_file())
    })
}

/// Text that Claude Code puts in a `user` line itself: a slash command and its output, a
/// background task's report, an interruption.
const CLAUDE_INJECTED: &[&str] = &[
    "<command-name>",
    "<command-message>",
    "<command-args>",
    "<local-command-",
    "<task-notification>",
    "<bash-input>",
    "<bash-stdout>",
    "<bash-stderr>",
    "<system-reminder>",
    "[Request interrupted",
];

/// The turns of a Claude Code transcript.
pub fn read_claude(reader: impl BufRead) -> io::Result<Vec<Turn>> {
    let mut turns: Vec<Turn> = Vec::new();
    // The message the reply is being built from: Claude Code writes one line per content block.
    let mut message_id: Option<String> = None;
    each_line(reader, |line| {
        // A tool result is most of a transcript. `"type":"tool_result"` cannot appear in text a
        // person typed, whose quotes are escaped.
        if line.contains("\"type\":\"tool_result\"")
            || !(line.contains("\"user\"") || line.contains("\"assistant\""))
        {
            return;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if entry["isSidechain"].as_bool() == Some(true) {
            return;
        }
        match entry["type"].as_str() {
            Some("user") => {
                if let Some(text) = claude_typed(&entry) {
                    let at = entry["timestamp"].as_str().and_then(iso_to_ms).unwrap_or(0);
                    turns.push(Turn {
                        at,
                        user: text,
                        said: Vec::new(),
                    });
                    message_id = None;
                }
            }
            Some("assistant") => {
                let message = &entry["message"];
                if message["model"].as_str() == Some("<synthetic>") {
                    return;
                }
                let Some(turn) = turns.last_mut() else {
                    return;
                };
                let texts: Vec<&str> = message["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|block| block["type"].as_str() == Some("text"))
                    .filter_map(|block| block["text"].as_str())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .collect();
                if texts.is_empty() {
                    return;
                }
                let id = message["id"].as_str().map(str::to_owned);
                let same = id.is_some() && id == message_id;
                for text in texts {
                    match turn.said.last_mut() {
                        // A message written again with the same block is not said twice.
                        Some(last) if same && last.ends_with(text) => {}
                        Some(last) if same => {
                            last.push_str("\n\n");
                            last.push_str(text);
                        }
                        _ => turn.said.push(text.to_owned()),
                    }
                }
                message_id = id;
            }
            _ => {}
        }
    })?;
    Ok(turns)
}

/// What the user typed, when this `user` line is a message they typed.
fn claude_typed(entry: &Value) -> Option<String> {
    if entry["isMeta"].as_bool() == Some(true) || entry["isCompactSummary"].as_bool() == Some(true)
    {
        return None;
    }
    // Newer versions say who wrote it; a background task's report is not the user.
    if let Some(kind) = entry["origin"]["kind"].as_str() {
        if kind != "human" {
            return None;
        }
    }
    let content = &entry["message"]["content"];
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| match block["type"].as_str() {
                Some("text") => block["text"].as_str().map(str::to_owned),
                Some("image") => Some("[image]".to_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let text = text.trim();
    if text.is_empty() || CLAUDE_INJECTED.iter().any(|tag| text.starts_with(tag)) {
        return None;
    }
    Some(text.to_owned())
}

/// The first line of a Codex session file: its thread id, folder and start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexMeta {
    pub thread: String,
    pub cwd: Option<String>,
    pub started_at: Option<i64>,
}

/// Read only the `session_meta` line at the top of a Codex session file.
pub fn codex_meta(path: &Path) -> Option<CodexMeta> {
    let file = std::fs::File::open(path).ok()?;
    let mut first = String::new();
    // The line carries the model's base instructions, which can be long; nothing else is read.
    io::BufReader::new(file).read_line(&mut first).ok()?;
    let entry: Value = serde_json::from_str(first.trim_end()).ok()?;
    if entry["type"].as_str() != Some("session_meta") {
        return None;
    }
    let payload = &entry["payload"];
    Some(CodexMeta {
        thread: payload["id"].as_str()?.to_owned(),
        cwd: payload["cwd"].as_str().map(str::to_owned),
        started_at: payload["timestamp"]
            .as_str()
            .or_else(|| entry["timestamp"].as_str())
            .and_then(iso_to_ms),
    })
}

/// A Codex session file, by its first line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexFile {
    pub thread: String,
    /// Epoch milliseconds.
    pub started_at: i64,
    pub path: PathBuf,
}

/// The Codex session files that began in `cwd` between `from` and `to` (epoch milliseconds),
/// earliest first: for a conversation whose thread id was never reported. The day folders are
/// Codex's local dates, so the day either side is looked in too.
pub fn codex_files_in(codex_home: &Path, cwd: &Path, from: i64, to: i64) -> Vec<CodexFile> {
    let cwd = dunce::simplified(cwd);
    let sessions = codex_home.join("sessions");
    let day = 86_400_000;
    let mut found: Vec<CodexFile> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut at = from - day;
    while at <= to + day {
        let (y, m, d) = super::handoff::civil_date(at);
        let folder = sessions
            .join(format!("{y:04}"))
            .join(format!("{m:02}"))
            .join(format!("{d:02}"));
        at += day;
        if !seen.insert(folder.clone()) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            let named = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !(named.starts_with("rollout-") && named.ends_with(".jsonl")) {
                continue;
            }
            let Some(meta) = codex_meta(&path) else {
                continue;
            };
            let here = meta
                .cwd
                .as_deref()
                .is_some_and(|c| dunce::simplified(Path::new(c)) == cwd);
            match meta.started_at {
                Some(started) if here && (from..=to).contains(&started) => {
                    found.push(CodexFile {
                        thread: meta.thread,
                        started_at: started,
                        path,
                    });
                }
                _ => {}
            }
        }
    }
    found.sort_by(|a, b| (a.started_at, &a.path).cmp(&(b.started_at, &b.path)));
    found
}

/// The turns of a Codex session file. Only the thread the file is about: a subagent's
/// messages, when Codex writes them here, are its own.
pub fn read_codex(reader: impl BufRead) -> io::Result<Vec<Turn>> {
    let mut turns: Vec<Turn> = Vec::new();
    let mut thread: Option<String> = None;
    let mut turn_id: Option<String> = None;
    // Whether the current turn has its final answer, which `task_complete` would otherwise give.
    let mut answered = false;
    each_line(reader, |line| {
        let wanted = line.contains("\"UserMessage\"")
            || line.contains("\"AgentMessage\"")
            || line.contains("\"task_complete\"")
            || (thread.is_none() && line.contains("\"session_meta\""));
        if !wanted {
            return;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let payload = &entry["payload"];
        if entry["type"].as_str() == Some("session_meta") {
            thread = payload["id"].as_str().map(str::to_owned);
            return;
        }
        if entry["type"].as_str() != Some("event_msg") {
            return;
        }
        let of_thread = payload["thread_id"]
            .as_str()
            .is_none_or(|t| thread.as_deref().is_none_or(|ours| ours == t));
        if !of_thread {
            return;
        }
        match payload["type"].as_str() {
            Some("item_completed") => {
                let item = &payload["item"];
                let text = codex_text(&item["content"]);
                match item["type"].as_str() {
                    Some("UserMessage") => {
                        let Some(text) = text else { return };
                        let at = payload["started_at_ms"]
                            .as_i64()
                            .or_else(|| entry["timestamp"].as_str().and_then(iso_to_ms))
                            .unwrap_or(0);
                        turns.push(Turn {
                            at,
                            user: text,
                            said: Vec::new(),
                        });
                        turn_id = payload["turn_id"].as_str().map(str::to_owned);
                        answered = false;
                    }
                    Some("AgentMessage") => {
                        let (Some(turn), Some(text)) = (turns.last_mut(), text) else {
                            return;
                        };
                        answered |= item["phase"].as_str() == Some("final_answer");
                        turn.said.push(text);
                    }
                    _ => {}
                }
            }
            Some("task_complete") => {
                let same_turn = payload["turn_id"].as_str() == turn_id.as_deref();
                let last = payload["last_agent_message"].as_str().map(str::trim);
                if let (Some(turn), Some(last), true, false) =
                    (turns.last_mut(), last, same_turn, answered)
                {
                    if !last.is_empty() && turn.reply() != Some(last) {
                        turn.said.push(last.to_owned());
                    }
                }
                answered = true;
            }
            _ => {}
        }
    })?;
    Ok(turns)
}

/// The text of a Codex message's content: its text items, an image as `[image]`.
fn codex_text(content: &Value) -> Option<String> {
    let parts: Vec<String> = content
        .as_array()?
        .iter()
        .filter_map(|part| match part["type"].as_str() {
            Some("text" | "Text" | "input_text" | "output_text") => {
                part["text"].as_str().map(str::to_owned)
            }
            Some("local_image" | "image" | "input_image") => Some("[image]".to_owned()),
            _ => None,
        })
        .collect();
    let text = parts.join("\n").trim().to_owned();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(path: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(path)
    }

    fn open(path: &str) -> io::BufReader<std::fs::File> {
        io::BufReader::new(std::fs::File::open(fixture(path)).unwrap())
    }

    #[test]
    fn a_claude_transcript_reads_as_the_users_messages_and_what_the_agent_said() {
        let turns = read_claude(open("claude-transcript/2.1.302/conversation.jsonl")).unwrap();
        assert_eq!(turns.len(), 2, "{turns:#?}");
        assert!(turns[0].user.starts_with("Create a file named notes.txt"));
        assert_eq!(
            turns[1].user,
            "Thanks. In one sentence, what is left to do?"
        );
        // The commentary between tool calls is kept, the reply is the last of it.
        assert!(turns[0].said.len() >= 2, "{:#?}", turns[0].said);
        assert!(turns[0].reply().unwrap().contains("notes.txt"));
        assert!(turns[1].reply().unwrap().starts_with("Nothing is left"));
        // Tool calls, their results and the reasoning are not in it.
        let all: String = turns
            .iter()
            .flat_map(|t| std::iter::once(&t.user).chain(&t.said))
            .cloned()
            .collect();
        assert!(!all.contains("tool_use") && !all.contains("File created successfully"));
        assert!(!all.contains("1\tthe tests need TZ=UTC"), "{all}");
        assert!(turns[0].at > 1_790_000_000_000 && turns[1].at > turns[0].at);
    }

    #[test]
    fn a_claude_fork_carries_the_conversation_it_came_from_with_the_original_times() {
        let parent = read_claude(open("claude-transcript/2.1.302/conversation.jsonl")).unwrap();
        let fork = read_claude(open("claude-transcript/2.1.302/fork.jsonl")).unwrap();
        assert_eq!(fork.len(), 3);
        assert_eq!(fork[..2], parent[..]);
        assert_eq!(fork[2].user, "In one sentence, say which file you created.");
    }

    #[test]
    fn what_claude_code_injects_is_not_the_user() {
        let line = |fields: &str| format!("{{{fields}}}\n");
        let text = [
            line(r#""type":"user","timestamp":"2026-10-25T10:00:00.000Z","origin":{"kind":"human"},"promptSource":"typed","message":{"role":"user","content":"fix the redirect"}"#),
            line(r#""type":"assistant","message":{"id":"m1","model":"claude-x","content":[{"type":"thinking","thinking":"hm"}]}"#),
            line(r#""type":"assistant","message":{"id":"m1","model":"claude-x","content":[{"type":"text","text":"Looking."}]}"#),
            line(r#""type":"assistant","message":{"id":"m1","model":"claude-x","content":[{"type":"text","text":"Looking."}]}"#),
            line(r#""type":"user","isMeta":true,"message":{"role":"user","content":"[Image: original 100x100]"}"#),
            line(r#""type":"user","message":{"role":"user","content":"<command-name>/mcp</command-name>"}"#),
            line(r#""type":"user","message":{"role":"user","content":"<local-command-stdout>ok</local-command-stdout>"}"#),
            line(r#""type":"user","origin":{"kind":"task-notification"},"message":{"role":"user","content":"<task-notification> done"}"#),
            line(r#""type":"user","isCompactSummary":true,"message":{"role":"user","content":"This session is being continued"}"#),
            line(r#""type":"user","isSidechain":true,"message":{"role":"user","content":"a subagent's task"}"#),
            line(r#""type":"assistant","isSidechain":true,"message":{"id":"s1","model":"claude-x","content":[{"type":"text","text":"subagent says"}]}"#),
            line(r#""type":"user","message":{"role":"user","content":"[Request interrupted by user]"}"#),
            line(r#""type":"assistant","message":{"id":"m2","model":"<synthetic>","content":[{"type":"text","text":"No response requested."}]}"#),
            line(r#""type":"assistant","message":{"id":"m3","model":"claude-x","content":[{"type":"text","text":"Fixed it."}]}"#),
            line(r#""type":"user","timestamp":"2026-10-25T10:05:00.000Z","message":{"role":"user","content":[{"type":"text","text":"and this"},{"type":"image","source":{}}]}"#),
            // Being written: no newline yet.
            r#"{"type":"assistant","message":{"id":"m4","model":"claude-x","content":[{"type":"text","text":"half"#.to_owned(),
        ]
        .concat();
        let turns = read_claude(text.as_bytes()).unwrap();
        assert_eq!(
            turns,
            vec![
                Turn {
                    at: iso_to_ms("2026-10-25T10:00:00.000Z").unwrap(),
                    user: "fix the redirect".into(),
                    said: vec!["Looking.".into(), "Fixed it.".into()],
                },
                Turn {
                    at: iso_to_ms("2026-10-25T10:05:00.000Z").unwrap(),
                    user: "and this\n[image]".into(),
                    said: vec![],
                },
            ]
        );
    }

    #[test]
    fn a_codex_session_file_reads_as_its_turns_across_a_resume() {
        let turns = read_codex(open("codex/0.163.0/conversation.jsonl")).unwrap();
        assert_eq!(turns.len(), 2, "{turns:#?}");
        assert!(turns[0].user.starts_with("Create a file named notes.txt"));
        assert_eq!(
            turns[1].user,
            "Thanks. In one sentence, what is left to do?"
        );
        assert!(turns[0].said.len() >= 2, "{:#?}", turns[0].said);
        assert!(turns[0].reply().unwrap().contains("notes.txt"));
        assert!(turns[1].reply().unwrap().starts_with("Nothing is left"));
        // The context Codex injects as `user` messages is not the user.
        let users: String = turns.iter().map(|t| t.user.clone()).collect();
        assert!(!users.contains("<environment_context>"), "{users}");
        assert!(turns[1].at > turns[0].at);
    }

    #[test]
    fn an_older_codex_session_file_reads_too() {
        let turns = read_codex(open("codex/0.156.1/rollout.jsonl")).unwrap();
        assert_eq!(turns.len(), 1);
        assert!(turns[0].user.starts_with("Use apply_patch"));
        assert_eq!(turns[0].reply(), Some("Done"));
    }

    #[test]
    fn a_codex_turn_with_no_final_answer_takes_task_completes() {
        let text = concat!(
            r#"{"type":"session_meta","payload":{"id":"t1"}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"item_completed","thread_id":"t1","turn_id":"u1","started_at_ms":5,"item":{"type":"UserMessage","content":[{"type":"text","text":"go"},{"type":"local_image","path":"/x.png"}]}}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"item_completed","thread_id":"other","turn_id":"u9","item":{"type":"UserMessage","content":[{"type":"text","text":"a subagent"}]}}}"#,
            "\n",
            r#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"u1","last_agent_message":"All done."}}"#,
            "\n",
        );
        let turns = read_codex(text.as_bytes()).unwrap();
        assert_eq!(
            turns,
            vec![Turn {
                at: 5,
                user: "go\n[image]".into(),
                said: vec!["All done.".into()],
            }]
        );
    }

    #[test]
    fn a_claude_conversation_is_found_by_its_id_under_any_project_folder() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("b").join("projects").join("-tmp-repo");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("abc.jsonl"), "").unwrap();
        let dirs = [dir.path().join("a"), dir.path().join("b")];
        assert_eq!(claude_file(&dirs, "abc"), Some(folder.join("abc.jsonl")));
        assert_eq!(claude_file(&dirs, "nope"), None);
    }

    #[test]
    fn a_codex_conversation_is_found_by_where_and_when_it_began() {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("sessions/2026/10/25");
        std::fs::create_dir_all(&day).unwrap();
        let meta = |id: &str, cwd: &str, at: &str| {
            format!(
                r#"{{"type":"session_meta","payload":{{"id":"{id}","cwd":"{cwd}","timestamp":"{at}"}}}}"#
            ) + "\n"
        };
        std::fs::write(
            day.join("rollout-a.jsonl"),
            meta("a", "/work/one", "2026-10-25T10:00:00.000Z"),
        )
        .unwrap();
        std::fs::write(
            day.join("rollout-b.jsonl"),
            meta("b", "/work/two", "2026-10-25T10:00:01.000Z"),
        )
        .unwrap();
        std::fs::write(
            day.join("rollout-c.jsonl"),
            meta("c", "/work/one", "2026-10-25T12:00:00.000Z"),
        )
        .unwrap();
        let at = |s: &str| iso_to_ms(s).unwrap();
        let found = |cwd: &str, from: &str, to: &str| {
            codex_files_in(dir.path(), Path::new(cwd), at(from), at(to))
                .into_iter()
                .next()
                .map(|file| file.thread)
        };
        assert_eq!(
            found(
                "/work/one",
                "2026-10-25T09:59:00.000Z",
                "2026-10-25T13:00:00.000Z"
            )
            .as_deref(),
            Some("a")
        );
        assert_eq!(
            found(
                "/work/one",
                "2026-10-25T11:00:00.000Z",
                "2026-10-25T13:00:00.000Z"
            )
            .as_deref(),
            Some("c")
        );
        assert_eq!(
            found(
                "/work/two",
                "2026-10-25T09:59:00.000Z",
                "2026-10-25T13:00:00.000Z"
            )
            .as_deref(),
            Some("b")
        );
        assert_eq!(
            found(
                "/work/three",
                "2026-10-25T09:59:00.000Z",
                "2026-10-25T13:00:00.000Z"
            ),
            None
        );
    }
}
