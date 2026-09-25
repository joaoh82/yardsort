//! Claude Code's hooks, as a source of activity.
//!
//! Claude Code runs a command at points in its own life — a prompt submitted, a tool about to
//! run, a tool done, a turn over, the session ending — and hands it a JSON description on
//! stdin. Yardsort listens by giving *its own launches* of Claude Code one more settings file
//! (`--settings <file>`) whose hooks run this executable with `--yardsort-hook`. Claude Code
//! merges that with the user's settings, so their hooks still run and nothing of theirs is
//! edited; a launch without the switch has none of ours. Nothing is installed anywhere, and the
//! hook is spawned as an argument list, never through a shell.
//!
//! What is kept from a payload is metadata: which event, a tool's name, a path relative to the
//! workspace, a duration, ids. Never the prompt, a command, a tool's input or output, or what
//! the assistant said. The shapes are those of the payloads recorded under
//! `crates/core/fixtures/claude-hooks/` — see `docs/design/11-agent-events-stage-2-claude.md`
//! for which events were seen and which are only documented.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// The built-in harness this adapter belongs to.
pub const HARNESS_ID: &str = "claude";
pub const PRODUCER: &str = "claude";
pub const METHOD: &str = "hook";
/// The agent said so; Yardsort did not see it.
pub const FIDELITY: &str = "reported";
/// The Claude Code version whose payloads the fixtures — and so the mapping — come from.
pub const FIXTURE_VERSION: &str = "2.1.280";
/// A hook that has not written its file in this long is not going to; Claude Code moves on.
pub const HOOK_TIMEOUT_SECS: u32 = 5;

/// The hook events subscribed to, in the settings file.
pub const EVENTS: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "PermissionDenied",
    "Notification",
    "Stop",
    "StopFailure",
    "SubagentStart",
    "SubagentStop",
    "PostCompact",
    "PostModelSwitch",
];

/// Arguments to Claude Code that mean "leave the launch alone": it already names a settings
/// source of its own, or asked for no hooks at all.
const CONFLICTING_ARGS: &[&str] = &["--settings", "--bare"];

/// Where the settings file for Claude Code launches lives: written before each launch, so an
/// updated executable is always the one named in it.
pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("activity").join("hooks").join("claude.json")
}

/// The executable the hook should run: this one — or, for a Linux AppImage, the image itself,
/// because the mounted path this process runs from is gone once the app quits, and hooks keep
/// firing after that.
pub fn hook_executable() -> io::Result<PathBuf> {
    if let Some(image) = std::env::var_os("APPIMAGE").map(PathBuf::from) {
        if image.is_file() {
            return Ok(image);
        }
    }
    std::env::current_exe()
}

/// The settings Claude Code is given: one command hook per event, as an argument list.
pub fn settings_json(hook: &Path, inbox_dir: &Path) -> Value {
    let command = json!({
        "type": "command",
        "command": hook.to_string_lossy(),
        "args": [super::hook::HOOK_FLAG, HARNESS_ID, inbox_dir.to_string_lossy()],
        "timeout": HOOK_TIMEOUT_SECS,
    });
    let mut hooks = serde_json::Map::new();
    for event in EVENTS {
        hooks.insert((*event).to_owned(), json!([{ "hooks": [command] }]));
    }
    json!({ "hooks": hooks })
}

/// Give a Claude Code launch its hooks: write the settings file and add `--settings <file>` to
/// `args`. Refuses, saying why, when the arguments already carry a `--settings` or `--bare` —
/// the user's own configuration wins — or when nothing can be written.
pub fn arm(args: &mut Vec<String>, data_dir: &Path) -> Result<PathBuf, String> {
    if let Some(conflict) = args
        .iter()
        .find(|arg| CONFLICTING_ARGS.contains(&arg.as_str()))
    {
        return Err(format!(
            "the harness's arguments already carry `{conflict}`"
        ));
    }
    let hook = hook_executable().map_err(|error| format!("no executable to run: {error}"))?;
    let path = settings_path(data_dir);
    let dir = path
        .parent()
        .ok_or_else(|| "settings path has no directory".to_owned())?;
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let settings = settings_json(&hook, &super::inbox_dir(data_dir));
    let temporary = path.with_extension("json.tmp");
    let write = || -> io::Result<()> {
        std::fs::write(&temporary, serde_json::to_vec_pretty(&settings)?)?;
        std::fs::rename(&temporary, &path)
    };
    write().map_err(|error| format!("{}: {error}", path.display()))?;
    // Before any `--`: what follows one is the prompt, and an option there is prompt text.
    let at = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());
    args.insert(at, path.to_string_lossy().into_owned());
    args.insert(at, "--settings".to_owned());
    Ok(path)
}

/// One hook payload, reduced to what Yardsort records.
#[derive(Debug, Clone, PartialEq)]
pub struct Reported {
    pub kind: &'static str,
    pub native_session_id: Option<String>,
    pub payload: Value,
}

/// Turn a hook's stdin into an event, keeping metadata only. `Err` names a payload that is not
/// one of ours: not an object, or an event this build has no mapping for.
pub fn normalize(hook: &Value) -> Result<Reported, String> {
    let event = hook
        .get("hook_event_name")
        .and_then(Value::as_str)
        .ok_or_else(|| "no hook_event_name".to_owned())?;
    let text = |key: &str| hook.get(key).and_then(Value::as_str).map(str::to_owned);
    let number = |key: &str| hook.get(key).and_then(Value::as_i64);
    let native_session_id = text("session_id");
    let prompt_id = text("prompt_id");
    let permission_mode = text("permission_mode");

    let tool = || {
        let name = text("tool_name");
        let input = hook.get("tool_input");
        let (path, outside) = tool_path(input, text("cwd").as_deref());
        let mut payload = json!({
            "tool": name,
            "toolUseId": text("tool_use_id"),
            "promptId": prompt_id,
            "permissionMode": permission_mode,
        });
        if let Some(path) = path {
            payload["path"] = Value::String(path);
        }
        if outside {
            payload["pathOutsideWorkspace"] = Value::Bool(true);
        }
        if let Some(kind) = input
            .and_then(|input| input.get("subagent_type"))
            .and_then(Value::as_str)
        {
            payload["subagentType"] = Value::String(kind.to_owned());
        }
        payload
    };

    let (kind, payload) = match event {
        "SessionStart" => (
            "session.started",
            json!({
                "source": text("source"),
                "contextTokens": number("context_tokens"),
            }),
        ),
        "SessionEnd" => ("session.ended", json!({ "reason": text("reason") })),
        "UserPromptSubmit" => (
            "prompt.submitted",
            json!({
                "promptId": prompt_id,
                "chars": text("prompt").map(|prompt| prompt.chars().count()),
                "permissionMode": permission_mode,
            }),
        ),
        "PreToolUse" => ("tool.started", tool()),
        "PostToolUse" => {
            let mut payload = tool();
            payload["durationMs"] = json!(number("duration_ms"));
            ("tool.completed", payload)
        }
        "PostToolUseFailure" => {
            let mut payload = tool();
            payload["durationMs"] = json!(number("duration_ms"));
            payload["interrupted"] = json!(hook.get("is_interrupt").and_then(Value::as_bool));
            ("tool.failed", payload)
        }
        "PermissionRequest" => ("approval.requested", tool()),
        "PermissionDenied" => {
            let mut payload = tool();
            payload["decision"] = json!("denied");
            ("approval.resolved", payload)
        }
        "Notification" => (
            "agent.notified",
            json!({ "type": text("notification_type").or_else(|| text("type")) }),
        ),
        "Stop" => (
            "turn.completed",
            json!({
                "promptId": prompt_id,
                "stopHookActive": hook.get("stop_hook_active").and_then(Value::as_bool),
                "backgroundTasks": hook.get("background_tasks").and_then(Value::as_array).map(Vec::len),
            }),
        ),
        "StopFailure" => (
            "turn.failed",
            json!({
                "promptId": prompt_id,
                "errorType": text("error_type").or_else(|| text("type")),
            }),
        ),
        "SubagentStart" => (
            "agent.subagent_started",
            json!({ "agentId": text("agent_id"), "agentType": text("agent_type") }),
        ),
        "SubagentStop" => (
            "agent.subagent_stopped",
            json!({ "agentId": text("agent_id"), "agentType": text("agent_type") }),
        ),
        "PostCompact" => ("session.compacted", json!({ "trigger": text("trigger") })),
        "PostModelSwitch" => (
            "agent.model_switched",
            json!({
                "model": text("new_model").or_else(|| text("model")),
                "previousModel": text("previous_model").or_else(|| text("old_model")),
            }),
        ),
        other => return Err(format!("no mapping for hook event {other}")),
    };
    Ok(Reported {
        kind,
        native_session_id,
        payload,
    })
}

/// The file a tool is about, relative to the workspace — or nothing, and a flag, when it is
/// somewhere else. Absolute paths outside the worktree are the user's business.
fn tool_path(input: Option<&Value>, cwd: Option<&str>) -> (Option<String>, bool) {
    let Some(path) = input
        .and_then(|input| {
            input
                .get("file_path")
                .or_else(|| input.get("notebook_path"))
        })
        .and_then(Value::as_str)
    else {
        return (None, false);
    };
    let is_absolute = path.starts_with('/')
        || path.starts_with('\\')
        || path.get(1..3).is_some_and(|s| s == ":\\" || s == ":/");
    if !is_absolute {
        return (Some(path.replace('\\', "/")), false);
    }
    if let Some(cwd) = cwd {
        let cwd = cwd.trim_end_matches(['/', '\\']);
        if let Some(rest) = path.strip_prefix(cwd) {
            if let Some(relative) = rest.strip_prefix(['/', '\\']) {
                return (Some(relative.replace('\\', "/")), false);
            }
            if rest.is_empty() {
                return (Some(".".to_owned()), false);
            }
        }
    }
    (None, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> Vec<(String, Value)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("claude-hooks")
            .join(FIXTURE_VERSION);
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
            .collect();
        files.sort();
        files
            .into_iter()
            .map(|path| {
                let name = path.file_stem().unwrap().to_string_lossy().into_owned();
                let value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
                (name, value)
            })
            .collect()
    }

    #[test]
    fn every_recorded_payload_maps_to_an_event_in_the_order_claude_sent_them() {
        let kinds: Vec<&str> = fixtures()
            .iter()
            .map(|(name, hook)| {
                normalize(hook)
                    .unwrap_or_else(|e| panic!("{name}: {e}"))
                    .kind
            })
            .collect();
        assert_eq!(
            kinds,
            [
                // Fresh: a write, a read, a read that fails, a shell command, one turn.
                "session.started",
                "prompt.submitted",
                "tool.started",
                "tool.completed",
                "tool.started",
                "tool.completed",
                "tool.started",
                "tool.failed",
                "tool.started",
                "tool.completed",
                "turn.completed",
                "session.ended",
                // Resumed.
                "session.started",
                "prompt.submitted",
                "turn.completed",
                "session.ended",
                // Forked, under the id we chose.
                "session.started",
                "prompt.submitted",
                "turn.completed",
                "session.ended",
            ]
        );
    }

    #[test]
    fn what_is_kept_is_metadata_and_what_is_dropped_is_content() {
        let all = fixtures();
        let by_name = |name: &str| all.iter().find(|(n, _)| n == name).map(|(_, v)| v).unwrap();

        let write = normalize(by_name("03-PreToolUse")).unwrap();
        assert_eq!(
            write.payload,
            json!({
                "tool": "Write",
                "toolUseId": "toolu_01AQAx1rsDGUM6NAeePkNtJT",
                "promptId": "53ecea32-a99c-483f-9a76-d859619e4265",
                "permissionMode": "default",
                "path": "hello.txt",
            })
        );
        assert_eq!(
            write.native_session_id.as_deref(),
            Some("11111111-1111-4111-8111-111111111111")
        );

        let read_done = normalize(by_name("06-PostToolUse")).unwrap();
        assert_eq!(read_done.payload["durationMs"], 2);
        assert_eq!(read_done.payload["path"], "hello.txt");

        let failed = normalize(by_name("08-PostToolUseFailure")).unwrap();
        assert_eq!(failed.kind, "tool.failed");
        assert_eq!(failed.payload["tool"], "Read");
        assert_eq!(failed.payload["path"], "missing.txt");
        assert_eq!(failed.payload["durationMs"], 2);
        assert_eq!(failed.payload["interrupted"], false);
        assert!(failed.payload.get("error").is_none(), "no error text");

        let bash = normalize(by_name("09-PreToolUse")).unwrap();
        assert_eq!(bash.payload["tool"], "Bash");
        assert!(
            bash.payload.get("path").is_none(),
            "a command is not a file"
        );

        let prompt = normalize(by_name("02-UserPromptSubmit")).unwrap();
        assert_eq!(prompt.payload["chars"], 205);

        let resumed = normalize(by_name("13-SessionStart")).unwrap();
        assert_eq!(resumed.payload["source"], "resume");
        assert_eq!(resumed.payload["contextTokens"], 29691);
        let forked = normalize(by_name("17-SessionStart")).unwrap();
        assert_eq!(forked.payload["source"], "fork");
        assert_eq!(
            forked.native_session_id.as_deref(),
            Some("22222222-2222-4222-8222-222222222222")
        );
        let ended = normalize(by_name("12-SessionEnd")).unwrap();
        assert_eq!(ended.payload["reason"], "other");
        let stop = normalize(by_name("11-Stop")).unwrap();
        assert_eq!(stop.payload["stopHookActive"], false);
        assert_eq!(stop.payload["backgroundTasks"], 0);

        // Nothing that was typed, run, written, read or answered survives.
        for (name, hook) in &all {
            let text = normalize(hook).unwrap().payload.to_string();
            for content in [
                "Create a file",
                "ls /tmp",
                "\"hello\"",
                "Done.",
                "File does not exist",
                "transcript",
                "/tmp/yardsort-fixture",
                "/home/user",
            ] {
                assert!(!text.contains(content), "{name} leaked {content:?}: {text}");
            }
        }
    }

    #[test]
    fn events_documented_but_not_recorded_map_by_the_documented_fields() {
        let denied = normalize(&json!({
            "hook_event_name": "PermissionDenied", "session_id": "s",
            "tool_name": "Bash", "tool_input": { "command": "rm -rf /" }
        }))
        .unwrap();
        assert_eq!(denied.kind, "approval.resolved");
        assert_eq!(denied.payload["decision"], "denied");
        assert!(!denied.payload.to_string().contains("rm -rf"));

        let notified = normalize(&json!({
            "hook_event_name": "Notification", "session_id": "s",
            "notification_type": "permission_prompt", "message": "Claude needs your permission to use Bash"
        }))
        .unwrap();
        assert_eq!(notified.payload, json!({ "type": "permission_prompt" }));

        let sub = normalize(&json!({
            "hook_event_name": "SubagentStart", "session_id": "s",
            "agent_id": "a1", "agent_type": "Explore"
        }))
        .unwrap();
        assert_eq!(sub.kind, "agent.subagent_started");
        assert_eq!(sub.payload["agentType"], "Explore");

        let agent_tool = normalize(&json!({
            "hook_event_name": "PreToolUse", "session_id": "s", "tool_name": "Agent",
            "tool_input": { "subagent_type": "Plan", "prompt": "think hard about everything" }
        }))
        .unwrap();
        assert_eq!(agent_tool.payload["subagentType"], "Plan");
        assert!(!agent_tool.payload.to_string().contains("think hard"));

        assert!(normalize(&json!({ "hook_event_name": "MessageDisplay" })).is_err());
        assert!(normalize(&json!({ "nope": 1 })).is_err());
        assert!(normalize(&json!("a string")).is_err());
    }

    #[test]
    fn paths_are_relative_to_the_workspace_or_absent() {
        let at = |cwd: &str, file: &str| tool_path(Some(&json!({ "file_path": file })), Some(cwd));
        assert_eq!(
            at("/w/s", "/w/s/src/a.rs"),
            (Some("src/a.rs".into()), false)
        );
        assert_eq!(at("/w/s/", "/w/s/a.rs"), (Some("a.rs".into()), false));
        assert_eq!(at("/w/s", "/w/other/a.rs"), (None, true));
        assert_eq!(
            at("/w/s", "/w/s2/a.rs"),
            (None, true),
            "a prefix is not a parent"
        );
        assert_eq!(at("/w/s", "/w/s"), (Some(".".into()), false));
        assert_eq!(at("/w/s", "rel/b.rs"), (Some("rel/b.rs".into()), false));
        assert_eq!(
            at("C:\\w\\s", "C:\\w\\s\\src\\a.rs"),
            (Some("src/a.rs".into()), false)
        );
        assert_eq!(at("C:\\w\\s", "D:\\x\\a.rs"), (None, true));
        assert_eq!(
            tool_path(
                Some(&json!({ "notebook_path": "/w/s/n.ipynb" })),
                Some("/w/s")
            ),
            (Some("n.ipynb".into()), false)
        );
        assert_eq!(
            tool_path(Some(&json!({ "command": "ls" })), Some("/w/s")),
            (None, false)
        );
        assert_eq!(tool_path(None, None), (None, false));
    }

    #[test]
    fn the_settings_file_runs_this_executable_as_an_argument_list_for_every_event() {
        let settings = settings_json(Path::new("/opt/y a r d/yardsort"), Path::new("/d/inbox"));
        let hooks = settings["hooks"].as_object().unwrap();
        assert_eq!(hooks.len(), EVENTS.len());
        for event in EVENTS {
            let hook = &hooks[*event][0]["hooks"][0];
            assert_eq!(hook["type"], "command");
            assert_eq!(hook["command"], "/opt/y a r d/yardsort");
            assert_eq!(
                hook["args"],
                json!(["--yardsort-hook", "claude", "/d/inbox"]),
                "an argument list: spaces in a path are nobody's problem"
            );
            assert_eq!(hook["timeout"], HOOK_TIMEOUT_SECS);
            assert!(hook.get("matcher").is_none(), "everything of that event");
        }
    }

    #[test]
    fn arming_writes_the_file_and_names_it_unless_the_user_already_chose() {
        let dir = tempfile::tempdir().unwrap();
        let mut args = vec!["--model".to_owned(), "opus".to_owned()];
        let path = arm(&mut args, dir.path()).unwrap();
        assert_eq!(path, settings_path(dir.path()));
        assert_eq!(
            args[2..],
            ["--settings".to_owned(), path.to_string_lossy().into_owned()]
        );
        let written: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let expected = hook_executable().unwrap().to_string_lossy().into_owned();
        assert_eq!(written["hooks"]["Stop"][0]["hooks"][0]["command"], expected);
        assert_eq!(
            written["hooks"]["Stop"][0]["hooks"][0]["args"][2],
            json!(super::super::inbox_dir(dir.path()).to_string_lossy())
        );

        // The prompt follows a `--`, as Claude Code's own `prompt_args` have it; an option
        // after that would be read as part of the message. Seen live before this test existed.
        let mut with_prompt = vec![
            "--model".to_owned(),
            "haiku".to_owned(),
            "--".to_owned(),
            "Reply with ok.".to_owned(),
        ];
        arm(&mut with_prompt, dir.path()).unwrap();
        assert_eq!(
            with_prompt,
            [
                "--model",
                "haiku",
                "--settings",
                &path.to_string_lossy(),
                "--",
                "Reply with ok."
            ]
        );

        let mut theirs = vec!["--settings".to_owned(), "mine.json".to_owned()];
        let why = arm(&mut theirs, dir.path()).unwrap_err();
        assert!(why.contains("--settings"), "{why}");
        assert_eq!(theirs.len(), 2, "untouched");
        let mut bare = vec!["--bare".to_owned()];
        assert!(arm(&mut bare, dir.path()).is_err());
    }
}
