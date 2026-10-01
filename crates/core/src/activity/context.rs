//! How full an agent's context is, and whether to suggest compacting it.
//!
//! Nothing here reads the terminal. The numbers come from the agents' own records, through the
//! adapters that already report their turns:
//!
//! - **Claude Code**: at the end of each turn (the `Stop` hook) the hook reads the turn's last
//!   main-thread assistant message from the conversation's transcript, whose `message.usage`
//!   says how many tokens the request carried: `input_tokens + cache_read_input_tokens +
//!   cache_creation_input_tokens`. The turn is what follows the user line carrying the hook's
//!   `prompt_id`. That count rides on the `turn.completed` event as `contextTokens`. Claude Code
//!   does not say how large its window is, so [`claude_window`] works it out from the model.
//! - **Codex**: the session file's `token_count` events carry `info.last_token_usage` and
//!   `info.model_context_window`; the last one of a turn rides on its `turn.completed` as
//!   `contextTokens` and `contextWindow`.
//!
//! A compaction (Claude Code's `PostCompact` hook, `session.compacted`) clears the reading until
//! the next turn reports a new one.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use specta::Type;

use crate::store::{SessionRow, Store, StoreResult};

/// Suggest compacting from this share of the window on.
pub const SUGGEST_PERCENT: u8 = 80;
/// Claude Code's usual window, and the one assumed when nothing says otherwise.
pub const CLAUDE_WINDOW: u64 = 200_000;
/// The window of a model chosen with the `[1m]` suffix (`opus[1m]`, `claude-opus-5-5[1m]`).
pub const CLAUDE_LARGE_WINDOW: u64 = 1_000_000;
/// How much of a transcript's end is read looking for the turn's last assistant message, which
/// is at the end, or about to be.
const TRANSCRIPT_TAIL_BYTES: u64 = 512 * 1024;

/// The event kinds that say something about the context, newest of which wins.
pub const KINDS: &[&str] = &["turn.completed", "session.compacted"];

/// A session's context, as last reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    /// Tokens the last request carried.
    pub used_tokens: u32,
    pub window_tokens: u32,
    /// `used / window`, rounded down, at most 100.
    pub percent: u8,
    /// At or past [`SUGGEST_PERCENT`], and the harness has a command to compact with.
    pub suggest: bool,
    /// What to type to compact, from the harness definition.
    pub compact_command: Option<String>,
}

/// The tokens a request carried and the model that answered.
pub type Reading = (u64, Option<String>);

/// What a Claude Code transcript says about a turn's last request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Turn {
    /// The turn's final answer is written: an assistant message after the prompt, with nothing
    /// from the user side (a tool's result) after it.
    Settled(Reading),
    /// Still being written; the newest reading there is so far, if any.
    Unsettled(Option<Reading>),
}

/// Claude Code writes a turn's last message to the transcript a moment *after* running the
/// `Stop` hook — some 70 ms on 2.1.284. The hook waits this long at most for it, and then takes
/// the newest reading there is: a turn's end should not be held up for a number.
pub const SETTLE_WAIT: Duration = Duration::from_millis(1000);
const SETTLE_POLL: Duration = Duration::from_millis(25);

/// The context of the turn begun by the prompt `prompt_id`, waiting up to `wait` for the
/// transcript to settle. `None` when the transcript cannot be read or holds no reading at all.
pub fn claude_turn_waiting(
    path: &Path,
    prompt_id: Option<&str>,
    wait: Duration,
) -> Option<Reading> {
    let deadline = Instant::now() + wait;
    loop {
        match claude_turn(path, prompt_id)? {
            Turn::Settled(reading) => return Some(reading),
            Turn::Unsettled(best) if Instant::now() >= deadline => return best,
            Turn::Unsettled(_) => std::thread::sleep(SETTLE_POLL),
        }
    }
}

/// Read the end of a transcript for the turn begun by `prompt_id` (any turn, without one).
pub fn claude_turn(path: &Path, prompt_id: Option<&str>) -> Option<Turn> {
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(TRANSCRIPT_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    // A prompt older than the tail began before everything in it.
    let prompt_in_tail = prompt_id.is_some_and(|id| text.contains(id));
    let mut in_turn = prompt_id.is_none() || (start > 0 && !prompt_in_tail);
    let mut newest: Option<Reading> = None;
    let mut turns: Option<Reading> = None;
    let mut waiting = in_turn;
    // The first line of a tail that does not start at the beginning is a fragment; it fails to
    // parse and is skipped like any other line that is not a record.
    for record in text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        if record.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        match record.get("type").and_then(Value::as_str) {
            Some("user") => {
                if prompt_id.is_some()
                    && record.get("promptId").and_then(Value::as_str) == prompt_id
                {
                    in_turn = true;
                }
                waiting = in_turn;
            }
            Some("assistant") => {
                if let Some(reading) = claude_usage(&record) {
                    newest = Some(reading.clone());
                    if in_turn {
                        turns = Some(reading);
                        waiting = false;
                    }
                }
            }
            _ => {}
        }
    }
    Some(match turns {
        Some(reading) if !waiting => Turn::Settled(reading),
        turns => Turn::Unsettled(turns.or(newest)),
    })
}

/// The reading of a main-thread assistant message, if it has real usage.
fn claude_usage(record: &Value) -> Option<Reading> {
    let message = record.get("message")?;
    let model = message.get("model").and_then(Value::as_str);
    // Messages Claude Code writes itself (an interruption, an error) carry no real usage.
    if model == Some("<synthetic>") {
        return None;
    }
    let usage = message.get("usage")?;
    let n = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    let used = n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
    (used > 0).then(|| (used, model.map(str::to_owned)))
}

/// The model a Claude Code launch is configured with when its command line names none: the
/// `ANTHROPIC_MODEL` variable, else the `model` of the project's local and shared settings,
/// else the user's. Only to tell a `[1m]` window from the usual one.
pub fn claude_configured_model(
    cwd: Option<&Path>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if let Some(model) = env("ANTHROPIC_MODEL").filter(|m| !m.is_empty()) {
        return Some(model);
    }
    let config_dir = env("CLAUDE_CONFIG_DIR")
        .filter(|dir| !dir.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            env("HOME")
                .or_else(|| env("USERPROFILE"))
                .map(|home| Path::new(&home).join(".claude"))
        });
    let mut files = Vec::new();
    if let Some(cwd) = cwd {
        files.push(cwd.join(".claude").join("settings.local.json"));
        files.push(cwd.join(".claude").join("settings.json"));
    }
    if let Some(dir) = config_dir {
        files.push(dir.join("settings.json"));
    }
    files.iter().find_map(|file| {
        let text = std::fs::read(file).ok()?;
        let settings: Value = serde_json::from_slice(&text).ok()?;
        settings
            .get("model")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .map(str::to_owned)
    })
}

/// Claude Code's window: a million tokens for a model picked with `[1m]`, or once a request
/// has carried more than the usual window could hold; otherwise the usual window. The model
/// Yardsort launched with wins over the one Claude Code's settings name.
pub fn claude_window(used: u64, launched: Option<&str>, configured: Option<&str>) -> u64 {
    let large = launched
        .or(configured)
        .is_some_and(|model| model.to_ascii_lowercase().contains("[1m]"));
    if large || used > CLAUDE_WINDOW {
        CLAUDE_LARGE_WINDOW
    } else {
        CLAUDE_WINDOW
    }
}

/// The context of `session` as its newest report has it, or `None` when nothing was reported
/// since it started or last compacted.
pub fn session_context(
    store: &Store,
    session: &SessionRow,
    compact_command: Option<String>,
) -> StoreResult<Option<ContextUsage>> {
    let Some(event) =
        store.latest_session_event_of_kinds(&session.workspace_id, &session.id, KINDS)?
    else {
        return Ok(None);
    };
    let payload: Value = serde_json::from_str(&event.payload).unwrap_or(Value::Null);
    let n = |key: &str| payload.get(key).and_then(Value::as_u64);
    let text = |key: &str| payload.get(key).and_then(Value::as_str);
    if event.kind != "turn.completed" {
        return Ok(None);
    }
    let Some(used) = n("contextTokens") else {
        return Ok(None);
    };
    // Codex says how large its window is; Claude Code does not.
    let window = n("contextWindow")
        .unwrap_or_else(|| claude_window(used, session.model.as_deref(), text("configuredModel")));
    Ok(Some(usage(used, window, compact_command)))
}

/// A reading, with its share of the window and whether to suggest compacting.
pub fn usage(used: u64, window: u64, compact_command: Option<String>) -> ContextUsage {
    let percent = used
        .saturating_mul(100)
        .checked_div(window)
        .map_or(0, |share| share.min(100) as u8);
    ContextUsage {
        used_tokens: u32::try_from(used).unwrap_or(u32::MAX),
        window_tokens: u32::try_from(window).unwrap_or(u32::MAX),
        percent,
        suggest: percent >= SUGGEST_PERCENT && compact_command.is_some(),
        compact_command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(value: Value) -> String {
        value.to_string()
    }

    fn assistant(input: u64, read: u64, created: u64, model: &str, sidechain: bool) -> String {
        line(serde_json::json!({
            "type": "assistant",
            "isSidechain": sidechain,
            "message": {
                "model": model,
                "usage": {
                    "input_tokens": input,
                    "cache_read_input_tokens": read,
                    "cache_creation_input_tokens": created,
                    "output_tokens": 900,
                },
            },
        }))
    }

    fn prompt(id: &str) -> String {
        line(serde_json::json!({"type": "user", "promptId": id, "message": {"content": "hi"}}))
    }

    fn tool_result() -> String {
        line(serde_json::json!({"type": "user", "message": {"content": [{"type": "tool_result"}]}}))
    }

    fn write(path: &Path, lines: &[String]) {
        std::fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn the_turns_last_main_thread_assistant_message_gives_the_context() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        write(
            &path,
            &[
                prompt("p-1"),
                assistant(2, 1000, 50, "claude-opus-5-5", false),
                prompt("p-2"),
                assistant(3, 140_000, 0, "claude-opus-5-5", false),
                tool_result(),
                assistant(3, 150_000, 4_000, "claude-opus-5-5", false),
                // A subagent's request is not the conversation's context.
                assistant(1, 9, 0, "claude-haiku-4-5", true),
                // Nor is a message Claude Code wrote itself.
                assistant(0, 0, 0, "<synthetic>", false),
                line(serde_json::json!({"type": "system", "subtype": "stop_hook_summary"})),
            ],
        );
        let settled = Turn::Settled((154_003, Some("claude-opus-5-5".into())));
        assert_eq!(claude_turn(&path, Some("p-2")), Some(settled.clone()));
        assert_eq!(claude_turn(&path, None), Some(settled));
        assert_eq!(claude_turn(&dir.path().join("missing.jsonl"), None), None);
    }

    #[test]
    fn a_turn_whose_answer_is_not_written_yet_is_unsettled_and_waited_for_briefly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let earlier = assistant(2, 1000, 0, "m", false);
        // Claude Code runs the hook before the turn's answer reaches the file.
        write(&path, &[prompt("p-1"), earlier.clone(), prompt("p-2")]);
        assert_eq!(
            claude_turn(&path, Some("p-2")),
            Some(Turn::Unsettled(Some((1002, Some("m".into())))))
        );
        // A tool's result after the turn's last answer: the next answer is still to come.
        let mid = assistant(3, 5000, 0, "m", false);
        write(&path, &[prompt("p-2"), mid.clone(), tool_result()]);
        assert_eq!(
            claude_turn(&path, Some("p-2")),
            Some(Turn::Unsettled(Some((5003, Some("m".into())))))
        );

        // The wait ends as soon as the answer lands; or, at the latest, with the best there is.
        write(&path, &[prompt("p-2"), mid, tool_result()]);
        let writer = {
            let path = path.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(60));
                let mut text = std::fs::read_to_string(&path).unwrap();
                text.push_str(&assistant(4, 6000, 0, "m", false));
                text.push('\n');
                std::fs::write(&path, text).unwrap();
            })
        };
        let begun = Instant::now();
        let reading = claude_turn_waiting(&path, Some("p-2"), Duration::from_secs(5));
        writer.join().unwrap();
        assert_eq!(reading, Some((6004, Some("m".into()))));
        assert!(begun.elapsed() < Duration::from_secs(2));

        write(&path, &[earlier, prompt("p-3")]);
        let reading = claude_turn_waiting(&path, Some("p-3"), Duration::from_millis(50));
        assert_eq!(reading, Some((1002, Some("m".into()))));
    }

    #[test]
    fn only_the_tail_of_a_long_transcript_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let filler = line(serde_json::json!({"type": "attachment", "x": "x".repeat(1000)}));
        let mut text = prompt("p-1") + "\n" + &assistant(1, 10, 0, "old", false) + "\n";
        for _ in 0..(TRANSCRIPT_TAIL_BYTES / 1000 + 10) {
            text.push_str(&filler);
            text.push('\n');
        }
        text.push_str(&assistant(5, 100, 0, "new", false));
        std::fs::write(&path, text).unwrap();
        // The prompt is older than the tail: everything read belongs to its turn.
        assert_eq!(
            claude_turn(&path, Some("p-1")),
            Some(Turn::Settled((105, Some("new".into()))))
        );
    }

    #[test]
    fn the_window_is_large_for_a_1m_model_or_a_context_past_the_usual_window() {
        assert_eq!(claude_window(10, None, None), CLAUDE_WINDOW);
        assert_eq!(
            claude_window(10, None, Some("opus[1m]")),
            CLAUDE_LARGE_WINDOW
        );
        assert_eq!(
            claude_window(10, Some("claude-opus-5-5[1M]"), None),
            CLAUDE_LARGE_WINDOW
        );
        // What Yardsort launched with wins over the settings file.
        assert_eq!(
            claude_window(10, Some("sonnet"), Some("opus[1m]")),
            CLAUDE_WINDOW
        );
        assert_eq!(
            claude_window(250_000, Some("sonnet"), None),
            CLAUDE_LARGE_WINDOW
        );
    }

    #[test]
    fn the_configured_model_follows_the_variable_then_project_then_user_settings() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = dir.path().join("project");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::create_dir_all(project.join(".claude")).unwrap();
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"model":"opus[1m]"}"#,
        )
        .unwrap();
        let home_str = home.to_string_lossy().into_owned();
        let env = |name: &str| (name == "HOME").then(|| home_str.clone());
        assert_eq!(
            claude_configured_model(Some(&project), &env).as_deref(),
            Some("opus[1m]")
        );
        std::fs::write(
            project.join(".claude/settings.json"),
            r#"{"model":"sonnet"}"#,
        )
        .unwrap();
        assert_eq!(
            claude_configured_model(Some(&project), &env).as_deref(),
            Some("sonnet")
        );
        let with_variable = |name: &str| match name {
            "ANTHROPIC_MODEL" => Some("haiku".to_owned()),
            other => env(other),
        };
        assert_eq!(
            claude_configured_model(Some(&project), &with_variable).as_deref(),
            Some("haiku")
        );
    }

    #[test]
    fn the_newest_report_of_a_session_decides_and_a_compaction_clears_it() {
        use crate::store::{NewEvent, NewSession};
        let store = Store::in_memory();
        let project = store.add_project("app", "/code/app").unwrap();
        let ws = store
            .add_worktree(&project.id, "fix", "/wt/fix", Some("ys/fix"), Some("main"))
            .unwrap()
            .id;
        for (id, model) in [("rec-1", None), ("rec-2", Some("sonnet[1m]"))] {
            store
                .add_session(&NewSession {
                    id,
                    workspace_id: &ws,
                    harness_id: "claude",
                    model,
                    title: "",
                    pty_session_id: id,
                    ..Default::default()
                })
                .unwrap();
        }
        let mut at = 0;
        let mut add = |session: &str, kind: &str, payload: Value| {
            at += 1;
            store
                .add_event(&NewEvent {
                    id: &uuid::Uuid::new_v4().to_string(),
                    schema_version: 1,
                    workspace_id: &ws,
                    session_id: Some(session),
                    occurred_at: at,
                    kind,
                    producer: "claude",
                    method: "hook",
                    fidelity: "reported",
                    privacy_class: "metadata",
                    payload: &payload.to_string(),
                    ..Default::default()
                })
                .unwrap();
        };
        let read = |id: &str| {
            let session = store.session(id).unwrap().unwrap();
            session_context(&store, &session, Some("/compact".into())).unwrap()
        };

        assert_eq!(read("rec-1"), None, "nothing reported yet");
        add(
            "rec-1",
            "turn.completed",
            serde_json::json!({ "contextTokens": 120_000 }),
        );
        let first = read("rec-1").unwrap();
        assert_eq!((first.percent, first.suggest), (60, false));
        // Other kinds and other sessions do not count.
        add("rec-1", "tool.completed", serde_json::json!({}));
        add(
            "rec-2",
            "turn.completed",
            serde_json::json!({ "contextTokens": 20 }),
        );
        add(
            "rec-1",
            "turn.completed",
            serde_json::json!({ "contextTokens": 170_000 }),
        );
        let full = read("rec-1").unwrap();
        assert_eq!((full.used_tokens, full.window_tokens), (170_000, 200_000));
        assert_eq!((full.percent, full.suggest), (85, true));
        // The settings' `[1m]` model makes the same count a small share.
        add(
            "rec-1",
            "turn.completed",
            serde_json::json!({ "contextTokens": 170_000, "configuredModel": "opus[1m]" }),
        );
        assert_eq!(read("rec-1").unwrap().percent, 17);
        // The model Yardsort launched with says so too.
        assert_eq!(read("rec-2").unwrap().window_tokens, 1_000_000);

        add(
            "rec-1",
            "session.compacted",
            serde_json::json!({ "trigger": "manual" }),
        );
        assert_eq!(read("rec-1"), None, "compacted: wait for the next turn");
        // A turn that carried no reading says nothing either.
        add("rec-1", "turn.completed", serde_json::json!({}));
        assert_eq!(read("rec-1"), None);
        add(
            "rec-1",
            "turn.completed",
            serde_json::json!({
                "threadId": "t", "turnId": "u", "model": "gpt-6",
                "contextTokens": 230_000, "contextWindow": 258_400,
            }),
        );
        let codex = read("rec-1").unwrap();
        assert_eq!((codex.percent, codex.suggest), (89, true));
    }

    #[test]
    fn a_suggestion_needs_the_threshold_and_a_command() {
        let compact = || Some("/compact".to_owned());
        assert!(!usage(159_999, 200_000, compact()).suggest);
        let full = usage(160_000, 200_000, compact());
        assert_eq!((full.percent, full.suggest), (80, true));
        assert!(!usage(190_000, 200_000, None).suggest);
        assert_eq!(usage(300_000, 200_000, compact()).percent, 100);
        assert_eq!(usage(5, 0, compact()).percent, 0);
    }
}
