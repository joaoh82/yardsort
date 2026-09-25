//! Codex CLI, as a source of activity: its `notify` program, and the session file it names.
//!
//! Codex has hooks shaped like Claude Code's, but a hook it is given for one launch is
//! *untrusted* until the user reviews it in Codex's own UI, and the only way round that is a
//! flag that waives trust for every other hook too — not something to pass on a user's behalf.
//! So Yardsort uses the other channel Codex offers: `notify`, a program Codex runs, as an
//! argument list with no shell, at the end of every turn, with a JSON description as its last
//! argument. Given per launch (`-c notify=[…]`), it needs no trust, touches no file of the
//! user's, and inherits the launch environment. A `notify` of the user's own, from their
//! `config.toml`, is chained after ours so it still runs.
//!
//! A turn's end is thin on its own — a thread id, a turn id — so the hook leaves a *trigger* in
//! the inbox, and the drain reads the rest from the session file Codex writes for that thread:
//! each command it ran and how it exited, each file it changed, the tokens the turn cost, and
//! how long it took. Metadata only, as everywhere here: no command line, no file content, no
//! message. The shapes are those of the fixtures under `crates/core/fixtures/codex/`; see
//! `docs/design/12-agent-events-stage-2-codex.md` for what was seen.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::inbox::InboxEntry;

/// The built-in harness this adapter belongs to.
pub const HARNESS_ID: &str = "codex";
pub const PRODUCER: &str = "codex";
/// The trigger: Codex's `notify` program, run at the end of a turn.
pub const METHOD_NOTIFY: &str = "notify";
/// Everything read from the session file that trigger names.
pub const METHOD_SESSION_FILE: &str = "session_file";
pub const FIDELITY: &str = "reported";
/// The Codex version whose payloads and session file the fixtures come from.
pub const FIXTURE_VERSION: &str = "0.156.1";
/// The inbox-only kind of a trigger entry. Never an event: the drain expands it.
pub const TRIGGER_KIND: &str = "codex.turn";
/// Read no session file larger than this: a long conversation is a few megabytes.
pub const MAX_ROLLOUT_BYTES: u64 = 64 * 1024 * 1024;
/// A trigger whose turn the session file does not yet show complete waits this long for the
/// file to catch up; after that, the turn is recorded from the trigger alone.
pub const NOT_READY_GRACE_MS: i64 = 5 * 60 * 1000;

/// Marks the chained program in the hook's arguments: what follows, up to the payload Codex
/// appends last, is the user's own `notify` and its arguments.
pub const THEN_FLAG: &str = "--then";

/// Where Codex keeps its state, from the environment a launch is given: `CODEX_HOME`, else
/// `.codex` under the home directory.
pub fn codex_home(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(home) = var("CODEX_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(home));
    }
    var("HOME")
        .or_else(|| var("USERPROFILE"))
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join(".codex"))
}

/// The user's own `notify`, if their `config.toml` has one, so it can be chained after ours.
pub fn users_notify(codex_home: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(codex_home.join("config.toml")).ok()?;
    let config: toml::Value = toml::from_str(&text).ok()?;
    let argv: Vec<String> = config
        .get("notify")?
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_owned))
        .collect::<Option<_>>()?;
    (!argv.is_empty()).then_some(argv)
}

/// The `notify` value Codex is given: this executable in hook mode, then the user's program.
pub fn notify_value(hook: &Path, inbox_dir: &Path, then: &[String]) -> toml::Value {
    let mut argv = vec![
        hook.to_string_lossy().into_owned(),
        super::hook::HOOK_FLAG.to_owned(),
        HARNESS_ID.to_owned(),
        inbox_dir.to_string_lossy().into_owned(),
    ];
    if !then.is_empty() {
        argv.push(THEN_FLAG.to_owned());
        argv.extend(then.iter().cloned());
    }
    toml::Value::Array(argv.into_iter().map(toml::Value::String).collect())
}

/// Give a Codex launch its `notify`: add `-c notify=[…]` to `args`. Refuses, saying why, when
/// the arguments already set `notify` — the user's own configuration wins.
pub fn arm(
    args: &mut Vec<String>,
    data_dir: &Path,
    codex_home: Option<&Path>,
) -> Result<(), String> {
    if args.iter().any(|arg| {
        arg.trim_start().starts_with("notify=") || arg.trim_start().starts_with("notify =")
    }) {
        return Err("the harness's arguments already set `notify`".to_owned());
    }
    let hook = super::claude::hook_executable()
        .map_err(|error| format!("no executable to run: {error}"))?;
    let then = codex_home.and_then(users_notify).unwrap_or_default();
    let value = notify_value(&hook, &super::inbox_dir(data_dir), &then);
    // Before any `--`: what follows one is the prompt, and an option there is a usage error.
    let at = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());
    args.insert(at, format!("notify={value}"));
    args.insert(at, "-c".to_owned());
    Ok(())
}

/// One `notify` payload, reduced to the trigger the drain expands.
#[derive(Debug, Clone, PartialEq)]
pub struct Reported {
    pub kind: &'static str,
    pub native_session_id: Option<String>,
    pub payload: Value,
}

/// Turn the argument Codex gave the hook into a trigger. Only the turn's ids are kept: the
/// payload also carries the messages of the turn, which are not ours to record.
pub fn normalize_notify(payload: &Value, codex_home: Option<&Path>) -> Result<Reported, String> {
    let text = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_owned);
    match text("type").as_deref() {
        Some("agent-turn-complete") => {}
        Some(other) => return Err(format!("no mapping for notify event {other}")),
        None => return Err("no notify type".to_owned()),
    }
    let thread = text("thread-id").ok_or_else(|| "no thread-id".to_owned())?;
    Ok(Reported {
        kind: TRIGGER_KIND,
        native_session_id: Some(thread.clone()),
        payload: json!({
            "threadId": thread,
            "turnId": text("turn-id"),
            "client": text("client"),
            "codexHome": codex_home.map(|p| p.to_string_lossy().into_owned()),
        }),
    })
}

/// One event read from the session file.
#[derive(Debug, Clone, PartialEq)]
pub struct Derived {
    pub kind: &'static str,
    /// Its own key, so a turn read twice — two drains, a retry — is one row per fact.
    pub source_key: String,
    /// The session file's own timestamp for the line: the source's clock.
    pub occurred_at: i64,
    pub payload: Value,
}

/// What expanding a trigger found.
#[derive(Debug, Clone, PartialEq)]
pub enum Expanded {
    Events(Vec<Derived>),
    /// The session file has not yet recorded the turn's end; ask again later.
    NotYet(String),
}

/// The event a trigger stands for on its own, when the session file cannot be read or has not
/// caught up: the turn ended, and nothing more.
pub fn fallback(trigger: &InboxEntry) -> Derived {
    let thread = trigger.payload["threadId"].as_str().unwrap_or("?");
    let turn = trigger.payload["turnId"].as_str().unwrap_or("?");
    Derived {
        kind: "turn.completed",
        source_key: format!("codex:{thread}:{turn}:turn"),
        occurred_at: trigger.at_ms,
        payload: json!({ "threadId": thread, "turnId": turn, "detail": "notify" }),
    }
}

/// Read the turn a trigger names out of its thread's session file. `run_id` keys the
/// `session.started` event, so a resumed thread says so once per run.
pub fn expand(trigger: &InboxEntry, run_id: Option<&str>) -> Result<Expanded, String> {
    let thread = trigger.payload["threadId"]
        .as_str()
        .ok_or_else(|| "trigger names no thread".to_owned())?;
    let turn = trigger.payload["turnId"].as_str().unwrap_or("");
    let home = trigger.payload["codexHome"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| "trigger names no CODEX_HOME".to_owned())?;
    let path = find_rollout(&home, thread).ok_or_else(|| {
        format!(
            "no session file for thread {thread} under {}",
            home.display()
        )
    })?;
    let size = std::fs::metadata(&path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .len();
    if size > MAX_ROLLOUT_BYTES {
        return Err(format!(
            "session file {} is {size} bytes, more than this build reads",
            path.display()
        ));
    }
    let text =
        std::fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(expand_rollout(&text, thread, turn, run_id))
}

/// `<codex-home>/sessions/<y>/<m>/<d>/rollout-<time>-<thread>[_<rollout>].jsonl`.
pub fn find_rollout(codex_home: &Path, thread: &str) -> Option<PathBuf> {
    let sessions = codex_home.join("sessions");
    let mut found = Vec::new();
    for year in read_dirs(&sessions) {
        for month in read_dirs(&year) {
            for day in read_dirs(&month) {
                let Ok(entries) = std::fs::read_dir(&day) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().into_owned();
                    // A thread id has dashes of its own; a fork's file adds `_<rollout id>`.
                    let stem = name.strip_suffix(".jsonl").unwrap_or("");
                    if stem.ends_with(&format!("-{thread}"))
                        || stem.contains(&format!("-{thread}_"))
                    {
                        found.push(path);
                    }
                }
            }
        }
    }
    found.sort();
    found.pop()
}

fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}

/// The events of one turn, from the lines of a session file.
pub fn expand_rollout(text: &str, thread: &str, turn: &str, run_id: Option<&str>) -> Expanded {
    let mut events = Vec::new();
    let mut cwd: Option<String> = None;
    let mut model: Option<String> = None;
    let mut usage: Option<(i64, Value)> = None;
    let mut ended = false;
    let key = |what: &str| format!("codex:{thread}:{turn}:{what}");

    for line in text.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let at = record
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(iso_to_ms)
            .unwrap_or(0);
        let payload = &record["payload"];
        let text = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_owned);
        let in_turn = text("turn_id").as_deref() == Some(turn);
        match record.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                cwd = text("cwd");
                events.push(Derived {
                    kind: "session.started",
                    source_key: format!("codex:{thread}:{}:session", run_id.unwrap_or("-")),
                    occurred_at: at,
                    payload: json!({
                        "threadId": thread,
                        "cliVersion": text("cli_version"),
                        "source": text("source"),
                        "originator": text("originator"),
                    }),
                });
            }
            Some("turn_context") if in_turn => {
                model = text("model");
                if cwd.is_none() {
                    cwd = text("cwd");
                }
            }
            Some("token_usage_record") if in_turn => {
                if let Some(record) = payload.get("turn_token_usage") {
                    usage = Some((at, record.clone()));
                }
            }
            Some("event_msg") if in_turn => match text("type").as_deref() {
                Some("item_completed") => {
                    let item = &payload["item"];
                    events.extend(item_events(item, thread, turn, at, cwd.as_deref()));
                }
                Some("task_complete") => {
                    ended = true;
                    events.push(Derived {
                        kind: "turn.completed",
                        source_key: key("turn"),
                        occurred_at: at,
                        payload: json!({
                            "threadId": thread,
                            "turnId": turn,
                            "model": model,
                            "durationMs": payload.get("duration_ms"),
                            "timeToFirstTokenMs": payload.get("time_to_first_token_ms"),
                        }),
                    });
                }
                Some("turn_aborted") => {
                    ended = true;
                    events.push(Derived {
                        kind: "turn.failed",
                        source_key: key("turn"),
                        occurred_at: at,
                        payload: json!({
                            "threadId": thread,
                            "turnId": turn,
                            "reason": text("reason"),
                        }),
                    });
                }
                _ => {}
            },
            _ => {}
        }
    }
    if !ended {
        return Expanded::NotYet(format!("turn {turn} has no end in the session file yet"));
    }
    if let Some((at, usage)) = usage {
        let n = |k: &str| usage.get(k).and_then(Value::as_i64);
        events.push(Derived {
            kind: "usage.reported",
            source_key: key("usage"),
            occurred_at: at,
            payload: json!({
                "threadId": thread,
                "turnId": turn,
                "inputTokens": n("input_tokens"),
                "cachedInputTokens": n("cached_input_tokens"),
                "outputTokens": n("output_tokens"),
                "reasoningOutputTokens": n("reasoning_output_tokens"),
                "totalTokens": n("total_tokens"),
            }),
        });
    }
    Expanded::Events(events)
}

/// A completed item of the turn: the user's message, a command, a file change, a tool.
fn item_events(item: &Value, thread: &str, turn: &str, at: i64, cwd: Option<&str>) -> Vec<Derived> {
    let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_owned);
    let id = text("id").unwrap_or_default();
    let key = |what: &str| format!("codex:{thread}:{id}:{what}");
    match text("type").as_deref() {
        Some("UserMessage") => {
            let chars: usize = item["content"]
                .as_array()
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                        .map(|t| t.chars().count())
                        .sum()
                })
                .unwrap_or(0);
            vec![Derived {
                kind: "prompt.submitted",
                source_key: key("prompt"),
                occurred_at: at,
                payload: json!({ "threadId": thread, "turnId": turn, "chars": chars }),
            }]
        }
        Some("CommandExecution") => {
            let failed = text("status").as_deref() == Some("failed");
            let duration_ms = item.get("duration").map(|d| {
                d.get("secs").and_then(Value::as_i64).unwrap_or(0) * 1000
                    + d.get("nanos").and_then(Value::as_i64).unwrap_or(0) / 1_000_000
            });
            vec![Derived {
                kind: if failed {
                    "tool.failed"
                } else {
                    "tool.completed"
                },
                source_key: key("tool"),
                occurred_at: at,
                payload: json!({
                    "tool": "shell",
                    "toolUseId": id,
                    "threadId": thread,
                    "turnId": turn,
                    "exitCode": item.get("exit_code"),
                    "status": text("status"),
                    "durationMs": duration_ms,
                }),
            }]
        }
        Some("FileChange") => {
            let Some(changes) = item["changes"].as_object() else {
                return vec![];
            };
            changes
                .iter()
                .enumerate()
                .map(|(index, (path, change))| {
                    let (relative, outside) = super::workspace_relative(path, cwd);
                    let mut payload = json!({
                        "toolUseId": id,
                        "threadId": thread,
                        "turnId": turn,
                        "kind": change.get("type"),
                        "status": text("status"),
                    });
                    if let Some(relative) = relative {
                        payload["path"] = Value::String(relative);
                    }
                    if outside {
                        payload["pathOutsideWorkspace"] = Value::Bool(true);
                    }
                    Derived {
                        kind: "file.reported_write",
                        source_key: key(&format!("file:{index}")),
                        occurred_at: at,
                        payload,
                    }
                })
                .collect()
        }
        Some("McpToolCall") => vec![Derived {
            kind: if text("status").as_deref() == Some("failed") {
                "tool.failed"
            } else {
                "tool.completed"
            },
            source_key: key("tool"),
            occurred_at: at,
            payload: json!({
                "tool": match (text("server"), text("tool")) {
                    (Some(server), Some(tool)) => format!("mcp:{server}/{tool}"),
                    (_, Some(tool)) => format!("mcp:{tool}"),
                    _ => "mcp".to_owned(),
                },
                "toolUseId": id,
                "threadId": thread,
                "turnId": turn,
                "status": text("status"),
            }),
        }],
        // Agent messages and reasoning are content; the rest this build does not know.
        _ => vec![],
    }
}

pub use super::iso_to_ms;

#[cfg(test)]
mod tests {
    use super::super::inbox::INBOX_VERSION;
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/codex")
            .join(FIXTURE_VERSION)
            .join(name)
    }

    fn trigger(home: &Path) -> InboxEntry {
        let notify: Value =
            serde_json::from_slice(&std::fs::read(fixture("notify/01.json")).unwrap()).unwrap();
        let reported = normalize_notify(&notify, Some(home)).unwrap();
        InboxEntry {
            version: INBOX_VERSION,
            id: "t-1".into(),
            at_ms: 1_790_334_464_000,
            producer: PRODUCER.into(),
            method: METHOD_NOTIFY.into(),
            fidelity: FIDELITY.into(),
            run_id: Some("run-1".into()),
            workspace_id: None,
            session_record_id: None,
            native_session_id: reported.native_session_id,
            kind: reported.kind.into(),
            privacy_class: "metadata".into(),
            payload: reported.payload,
        }
    }

    /// A throwaway CODEX_HOME holding the fixture session file where Codex would put it.
    fn codex_home_with_fixture(dir: &Path) -> PathBuf {
        let home = dir.join("codex");
        let day = home.join("sessions/2026/09/25");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::copy(
            fixture("rollout.jsonl"),
            day.join("rollout-2026-09-25T13-07-32-01a0d83f-c3ea-7ae0-88df-82c9431d3f8b.jsonl"),
        )
        .unwrap();
        home
    }

    #[test]
    fn the_notify_payload_becomes_a_trigger_that_keeps_ids_and_drops_the_messages() {
        let notify: Value =
            serde_json::from_slice(&std::fs::read(fixture("notify/01.json")).unwrap()).unwrap();
        let reported = normalize_notify(&notify, Some(Path::new("/h/.codex"))).unwrap();
        assert_eq!(reported.kind, TRIGGER_KIND);
        assert_eq!(
            reported.payload,
            json!({
                "threadId": "01a0d83f-c3ea-7ae0-88df-82c9431d3f8b",
                "turnId": "01a0d83f-c42b-7fd3-b4af-c46bc45458cc",
                "client": "codex_exec",
                "codexHome": "/h/.codex",
            })
        );
        assert_eq!(
            reported.native_session_id.as_deref(),
            Some("01a0d83f-c3ea-7ae0-88df-82c9431d3f8b")
        );
        assert!(normalize_notify(&json!({ "type": "something-else" }), None).is_err());
        assert!(normalize_notify(&json!({ "thread-id": "x" }), None).is_err());
    }

    #[test]
    fn a_turn_is_read_from_the_session_file_as_metadata_with_the_files_own_times() {
        let dir = tempfile::tempdir().unwrap();
        let home = codex_home_with_fixture(dir.path());
        let Expanded::Events(events) = expand(&trigger(&home), Some("run-1")).unwrap() else {
            panic!("the fixture's turn is complete");
        };
        let kinds: Vec<&str> = events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                "session.started",
                "prompt.submitted",
                "file.reported_write",
                "tool.completed",
                "tool.failed",
                "tool.completed",
                "turn.completed",
                "usage.reported",
            ]
        );
        let by_kind = |kind: &str| events.iter().find(|e| e.kind == kind).unwrap();
        let session = by_kind("session.started");
        assert_eq!(
            session.source_key,
            "codex:01a0d83f-c3ea-7ae0-88df-82c9431d3f8b:run-1:session"
        );
        assert_eq!(session.payload["cliVersion"], "0.156.1");
        assert_eq!(session.payload["source"], "exec");
        assert_eq!(
            session.occurred_at,
            iso_to_ms("2026-09-25T11:07:32.796Z").unwrap()
        );
        assert_eq!(by_kind("prompt.submitted").payload["chars"], 214);
        let file = by_kind("file.reported_write");
        assert_eq!(file.payload["path"], "hello.txt");
        assert_eq!(file.payload["kind"], "add");
        let failed = by_kind("tool.failed");
        assert_eq!(failed.payload["tool"], "shell");
        assert_eq!(failed.payload["exitCode"], 1);
        assert_eq!(failed.payload["durationMs"], 0);
        let turn = by_kind("turn.completed");
        assert_eq!(turn.payload["durationMs"], 11033);
        assert_eq!(turn.payload["model"], "gpt-6-astra");
        assert_eq!(
            turn.source_key,
            "codex:01a0d83f-c3ea-7ae0-88df-82c9431d3f8b:01a0d83f-c42b-7fd3-b4af-c46bc45458cc:turn"
        );
        let usage = by_kind("usage.reported");
        assert_eq!(
            usage.payload["totalTokens"], 29842,
            "the turn's last record"
        );
        assert_eq!(usage.payload["cachedInputTokens"], 26752);

        for event in &events {
            let text = event.payload.to_string();
            for content in [
                "hello\n",
                "cat hello.txt",
                "Done",
                "apply_patch",
                "/tmp/yardsort",
                "bash",
            ] {
                assert!(
                    !text.contains(content),
                    "{} leaked {content:?}: {text}",
                    event.kind
                );
            }
        }
    }

    #[test]
    fn a_turn_the_file_has_not_finished_waits_and_an_unknown_thread_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let home = codex_home_with_fixture(dir.path());
        let mut t = trigger(&home);
        t.payload["turnId"] = json!("some-other-turn");
        assert!(matches!(expand(&t, None).unwrap(), Expanded::NotYet(_)));

        let mut unknown = trigger(&home);
        unknown.payload["threadId"] = json!("no-such-thread");
        assert!(expand(&unknown, None)
            .unwrap_err()
            .contains("no session file"));

        let fallback = fallback(&trigger(&home));
        assert_eq!(fallback.kind, "turn.completed");
        assert_eq!(fallback.payload["detail"], "notify");
    }

    #[test]
    fn the_session_file_is_found_by_thread_id_including_a_forks_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let day = home.join("sessions/2026/01/02");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-2026-01-02T00-00-00-aaaa.jsonl"), "").unwrap();
        std::fs::write(day.join("rollout-2026-01-02T00-00-01-aaaa_bbbb.jsonl"), "").unwrap();
        std::fs::write(day.join("rollout-2026-01-02T00-00-02-aaaab.jsonl"), "").unwrap();
        let found = find_rollout(home, "aaaa").unwrap();
        assert!(
            found.ends_with("rollout-2026-01-02T00-00-01-aaaa_bbbb.jsonl"),
            "{found:?}"
        );
        assert!(find_rollout(home, "cccc").is_none());
        assert!(find_rollout(Path::new("/nowhere"), "aaaa").is_none());
    }

    #[test]
    fn arming_adds_notify_as_an_argument_list_and_chains_the_users_own() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let home = dir.path().join("codex");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join("config.toml"),
            "model = \"gpt-6\"\nnotify = [\"/usr/bin/say\", \"done\"]\n",
        )
        .unwrap();
        let mut args = vec![
            "-c".to_owned(),
            "model_reasoning_effort=\"high\"".to_owned(),
        ];
        arm(&mut args, &data_dir, Some(&home)).unwrap();
        assert_eq!(args[2], "-c");
        let value: toml::Value = toml::from_str(&args[3]).unwrap();
        let argv: Vec<&str> = value["notify"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let exe = super::super::claude::hook_executable().unwrap();
        assert_eq!(
            argv,
            [
                exe.to_string_lossy().as_ref(),
                "--yardsort-hook",
                "codex",
                super::super::inbox_dir(&data_dir)
                    .to_string_lossy()
                    .as_ref(),
                "--then",
                "/usr/bin/say",
                "done",
            ]
        );

        // The prompt follows a `--`; the override goes before it. Seen live before this test.
        let mut with_prompt = vec!["--".to_owned(), "Reply with ok.".to_owned()];
        arm(&mut with_prompt, &data_dir, None).unwrap();
        assert_eq!(with_prompt[0], "-c");
        assert!(with_prompt[1].starts_with("notify=["));
        assert_eq!(with_prompt[2..], ["--", "Reply with ok."]);

        // No config, or none with a notify: nothing chained.
        let mut plain = vec![];
        arm(&mut plain, &data_dir, Some(&dir.path().join("empty"))).unwrap();
        assert!(!plain[1].contains("--then"));
        assert_eq!(users_notify(&dir.path().join("empty")), None);

        // The user's own notify on the command line wins.
        let mut theirs = vec!["-c".to_owned(), "notify=[\"x\"]".to_owned()];
        assert!(arm(&mut theirs, &data_dir, None).is_err());
        assert_eq!(theirs.len(), 2);
    }

    #[test]
    fn codex_home_follows_the_variable_then_the_home_directory() {
        let at = |vars: &[(&str, &str)]| {
            codex_home(&|k| {
                vars.iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).to_owned())
            })
        };
        assert_eq!(
            at(&[("CODEX_HOME", "/x"), ("HOME", "/h")]),
            Some(PathBuf::from("/x"))
        );
        assert_eq!(at(&[("HOME", "/h")]), Some(PathBuf::from("/h/.codex")));
        assert_eq!(
            at(&[("USERPROFILE", "C:\\u")]),
            Some(PathBuf::from("C:\\u").join(".codex"))
        );
        assert_eq!(at(&[("CODEX_HOME", "")]), None);
    }

    #[test]
    fn timestamps_parse_to_the_millisecond() {
        assert_eq!(iso_to_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_to_ms("2026-09-21T14:13:20Z"), Some(1_790_000_000_000));
        assert_eq!(
            iso_to_ms("2026-09-25T11:07:32.796Z"),
            Some(1_790_334_452_796)
        );
        assert_eq!(iso_to_ms("2000-02-29T00:00:00.5Z"), Some(951_782_400_500));
        assert_eq!(iso_to_ms("not a time"), None);
    }
}
