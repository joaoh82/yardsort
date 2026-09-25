//! The Cursor agent CLI (`cursor-agent`), as a source of activity: a plugin directory given for
//! one process.
//!
//! Cursor's hooks are Claude Code's idea in Cursor's shape: a `hooks.json` naming commands per
//! event, each given the event as JSON on stdin. Hooks load from the user's and the project's
//! files and from plugins — and `--plugin-dir <dir>` loads a plugin for one process, merged with
//! the user's hooks, with no trust prompt. So a recorded launch of `cursor-agent` is given a
//! directory under Yardsort's data directory holding a plugin manifest and a `hooks.json` whose
//! commands run this executable in hook mode. Nothing of the user's is edited.
//!
//! Only events that never decide anything are subscribed to: the ones whose output Cursor does
//! not read, or reads as optional. Yardsort's hook prints nothing, which for these is "no
//! opinion"; a deciding hook that printed nothing might be read as an invalid answer and block
//! the action, and Yardsort observes.
//!
//! Correlation is by the launch environment alone: the hook inherits the agent's, so the run id
//! in it names the run. The payload's `conversation_id` is kept as the native session id, but
//! Yardsort does not choose it at launch, so it links nothing on its own — and `workspace_roots`
//! is never used as a key: a path is not an identity.
//!
//! **Recorded against nothing yet.** The Cursor agent CLI was not logged in on the machine this
//! was built on, so the shapes here are Cursor's documented ones (`cursor.com/docs/hooks`), and
//! `scripts/record-cursor.sh` is ready for the day it is. See
//! `docs/design/16-agent-events-stage-2-cursor.md`.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// The built-in harness this adapter belongs to.
pub const HARNESS_ID: &str = "cursor";
pub const PRODUCER: &str = "cursor";
pub const METHOD: &str = "hook";
pub const FIDELITY: &str = "reported";
/// The version the documented shapes were read against.
pub const DOCUMENTED_VERSION: &str = "2026.09.23-86fc751";
/// Cursor's default hook timeout is a minute; a file write does not need it.
pub const HOOK_TIMEOUT_SECS: u32 = 10;

/// The hook events subscribed to: those Cursor treats as passive, or whose answer is optional.
/// `stop` and `afterAgentResponse` are among them although, in the build this was written
/// against, Cursor fires them only for hooks from the user's or the project's own file.
pub const EVENTS: &[&str] = &[
    "sessionStart",
    "sessionEnd",
    "beforeSubmitPrompt",
    "postToolUse",
    "postToolUseFailure",
    "afterShellExecution",
    "afterMCPExecution",
    "afterFileEdit",
    "subagentStart",
    "subagentStop",
    "stop",
    "afterAgentResponse",
];

/// Where the plugin directory lives: written before each launch.
pub fn plugin_dir(data_dir: &Path) -> PathBuf {
    data_dir
        .join("activity")
        .join("hooks")
        .join("cursor-plugin")
}

/// A shell word: single-quoted, which both `sh -c` and PowerShell read literally, and which
/// Cursor prefixes with `&` on Windows when a command starts with a quote.
pub fn shell_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The plugin's `hooks.json`.
pub fn hooks_json(hook: &Path, inbox_dir: &Path) -> Value {
    let command = format!(
        "{} --yardsort-hook {HARNESS_ID} {}",
        shell_word(&hook.to_string_lossy()),
        shell_word(&inbox_dir.to_string_lossy())
    );
    let mut hooks = serde_json::Map::new();
    for event in EVENTS {
        hooks.insert(
            (*event).to_owned(),
            json!([{ "command": command, "type": "command", "timeout": HOOK_TIMEOUT_SECS }]),
        );
    }
    json!({ "version": 1, "hooks": hooks })
}

/// Give a Cursor launch its plugin: write the directory and add `--plugin-dir <dir>` to `args`,
/// before any `--` (what follows one is the prompt).
pub fn arm(args: &mut Vec<String>, data_dir: &Path) -> Result<PathBuf, String> {
    let hook = super::claude::hook_executable()
        .map_err(|error| format!("no executable to run: {error}"))?;
    let dir = plugin_dir(data_dir);
    let manifest_dir = dir.join(".cursor-plugin");
    let hooks_dir = dir.join("hooks");
    for d in [&manifest_dir, &hooks_dir] {
        std::fs::create_dir_all(d).map_err(|error| format!("{}: {error}", d.display()))?;
    }
    let write = |path: &Path, value: &Value| -> io::Result<()> {
        let temporary = path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
        std::fs::rename(&temporary, path)
    };
    let manifest = json!({
        "name": "yardsort-activity",
        "description": "Reports this session's activity to Yardsort. Written by Yardsort before each launch.",
        "hooks": "hooks/hooks.json",
    });
    write(&manifest_dir.join("plugin.json"), &manifest)
        .map_err(|error| format!("{}: {error}", manifest_dir.display()))?;
    write(
        &hooks_dir.join("hooks.json"),
        &hooks_json(&hook, &super::inbox_dir(data_dir)),
    )
    .map_err(|error| format!("{}: {error}", hooks_dir.display()))?;
    let at = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());
    args.insert(at, dir.to_string_lossy().into_owned());
    args.insert(at, "--plugin-dir".to_owned());
    Ok(dir)
}

/// One hook payload, reduced to what Yardsort records.
#[derive(Debug, Clone, PartialEq)]
pub struct Reported {
    pub kind: &'static str,
    /// Cursor's `conversation_id`. Not one Yardsort chose, so it links nothing by itself.
    pub native_session_id: Option<String>,
    pub payload: Value,
}

/// Turn a hook's stdin into an event, keeping metadata only.
pub fn normalize(hook: &Value) -> Result<Reported, String> {
    let event = hook
        .get("hook_event_name")
        .and_then(Value::as_str)
        .ok_or_else(|| "no hook_event_name".to_owned())?;
    let text = |key: &str| hook.get(key).and_then(Value::as_str).map(str::to_owned);
    let number = |key: &str| hook.get(key).and_then(Value::as_i64);
    let conversation = text("conversation_id");
    let root = hook["workspace_roots"]
        .as_array()
        .and_then(|roots| roots.first())
        .and_then(Value::as_str)
        .map(str::to_owned);
    let cwd = text("cwd").or_else(|| root.clone());
    let with_path = |mut payload: Value, path: Option<&str>| {
        if let Some(path) = path {
            let (relative, outside) = super::workspace_relative(path, cwd.as_deref());
            if let Some(relative) = relative {
                payload["path"] = Value::String(relative);
            }
            if outside {
                payload["pathOutsideWorkspace"] = Value::Bool(true);
            }
        }
        payload
    };
    let tokens = || {
        json!({
            "inputTokens": number("input_tokens"),
            "outputTokens": number("output_tokens"),
            "cachedInputTokens": number("cache_read_tokens"),
            "cacheWriteTokens": number("cache_write_tokens"),
        })
    };
    let tool = || {
        with_path(
            json!({
                "tool": text("tool_name"),
                "toolUseId": text("tool_use_id"),
                "conversationId": conversation,
                "generationId": text("generation_id"),
                "durationMs": number("duration"),
            }),
            hook["tool_input"]
                .get("file_path")
                .or_else(|| hook["tool_input"].get("path"))
                .and_then(Value::as_str),
        )
    };

    let (kind, payload) = match event {
        "sessionStart" => (
            "session.started",
            json!({
                "sessionId": text("session_id").or_else(|| conversation.clone()),
                "model": text("model").or_else(|| text("model_id")),
                "composerMode": text("composer_mode"),
                "background": hook.get("is_background_agent").and_then(Value::as_bool),
            }),
        ),
        "sessionEnd" => (
            "session.ended",
            json!({
                "sessionId": text("session_id").or_else(|| conversation.clone()),
                "reason": text("reason"),
                "durationMs": number("duration_ms"),
                "status": text("final_status"),
            }),
        ),
        "beforeSubmitPrompt" => (
            "prompt.submitted",
            json!({
                "conversationId": conversation,
                "generationId": text("generation_id"),
                "chars": text("prompt").map(|p| p.chars().count()),
                "attachments": hook["attachments"].as_array().map(Vec::len),
            }),
        ),
        "postToolUse" => ("tool.completed", tool()),
        "postToolUseFailure" => ("tool.failed", tool()),
        "afterShellExecution" => (
            "tool.completed",
            json!({
                "tool": "shell",
                "conversationId": conversation,
                "generationId": text("generation_id"),
                "durationMs": number("duration"),
                "sandbox": text("sandbox"),
            }),
        ),
        "afterMCPExecution" => (
            "tool.completed",
            json!({
                "tool": text("tool_name").map(|t| format!("mcp:{t}")),
                "server": text("mcp_server_name"),
                "conversationId": conversation,
                "generationId": text("generation_id"),
                "durationMs": number("duration"),
            }),
        ),
        "afterFileEdit" => (
            "file.reported_write",
            with_path(
                json!({
                    "conversationId": conversation,
                    "generationId": text("generation_id"),
                    "kind": "changed",
                    "edits": hook["edits"].as_array().map(Vec::len),
                }),
                text("file_path").as_deref(),
            ),
        ),
        "subagentStart" => (
            "agent.subagent_started",
            json!({
                "agentId": text("subagent_id"),
                "agentType": text("subagent_type"),
                "conversationId": conversation,
                "model": text("subagent_model"),
            }),
        ),
        "subagentStop" => (
            "agent.subagent_stopped",
            json!({
                "agentType": text("subagent_type"),
                "conversationId": conversation,
                "status": text("status"),
                "durationMs": number("duration_ms"),
            }),
        ),
        "stop" => {
            let mut payload = json!({
                "conversationId": conversation,
                "generationId": text("generation_id"),
                "status": text("status"),
                "loopCount": number("loop_count"),
            });
            for (k, v) in tokens().as_object().unwrap() {
                payload[k] = v.clone();
            }
            (
                if text("status").as_deref() == Some("error") {
                    "turn.failed"
                } else {
                    "turn.completed"
                },
                payload,
            )
        }
        "afterAgentResponse" => {
            let mut payload = json!({
                "conversationId": conversation,
                "generationId": text("generation_id"),
                "model": text("model"),
                "chars": text("text").map(|t| t.chars().count()),
            });
            for (k, v) in tokens().as_object().unwrap() {
                payload[k] = v.clone();
            }
            ("usage.reported", payload)
        }
        other => return Err(format!("no mapping for hook event {other}")),
    };
    Ok(Reported {
        kind,
        native_session_id: conversation,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shapes as `cursor.com/docs/hooks` documents them: nothing here was recorded.
    fn documented(event: &str, extra: Value) -> Value {
        let mut base = json!({
            "conversation_id": "conv-1",
            "generation_id": "gen-1",
            "model": "composer-2",
            "hook_event_name": event,
            "cursor_version": DOCUMENTED_VERSION,
            "workspace_roots": ["/w/s"],
            "transcript_path": "/home/user/.cursor/projects/w-s/agent-transcripts/conv-1.jsonl",
        });
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        base
    }

    #[test]
    fn the_documented_shapes_map_to_metadata_and_nothing_more() {
        let cases = [
            (
                documented(
                    "sessionStart",
                    json!({ "session_id": "conv-1", "composer_mode": "agent", "is_background_agent": false }),
                ),
                "session.started",
            ),
            (
                documented(
                    "beforeSubmitPrompt",
                    json!({ "prompt": "fix the login", "attachments": [{ "type": "file", "file_path": "/w/s/a.rs" }] }),
                ),
                "prompt.submitted",
            ),
            (
                documented(
                    "postToolUse",
                    json!({ "tool_name": "Read", "tool_input": { "path": "/w/s/src/a.rs" }, "tool_output": "{\"contents\":\"secret\"}", "tool_use_id": "t-1", "cwd": "/w/s", "duration": 12 }),
                ),
                "tool.completed",
            ),
            (
                documented(
                    "postToolUseFailure",
                    json!({ "tool_name": "Read", "tool_input": { "path": "/etc/hosts" }, "tool_use_id": "t-2", "cwd": "/w/s" }),
                ),
                "tool.failed",
            ),
            (
                documented(
                    "afterShellExecution",
                    json!({ "command": "rm -rf build", "output": "gone", "duration": 30, "sandbox": "enabled" }),
                ),
                "tool.completed",
            ),
            (
                documented(
                    "afterFileEdit",
                    json!({ "file_path": "/w/s/src/b.rs", "edits": [{ "old_string": "a", "new_string": "b" }] }),
                ),
                "file.reported_write",
            ),
            (
                documented(
                    "stop",
                    json!({ "status": "completed", "loop_count": 1, "input_tokens": 1200, "output_tokens": 300 }),
                ),
                "turn.completed",
            ),
            (
                documented(
                    "afterAgentResponse",
                    json!({ "text": "Done, I fixed it.", "input_tokens": 1200, "output_tokens": 300, "cache_read_tokens": 900 }),
                ),
                "usage.reported",
            ),
            (
                documented(
                    "subagentStop",
                    json!({ "subagent_type": "explore", "status": "completed", "task": "find the bug", "summary": "found it", "duration_ms": 8000 }),
                ),
                "agent.subagent_stopped",
            ),
            (
                documented(
                    "sessionEnd",
                    json!({ "session_id": "conv-1", "reason": "completed", "duration_ms": 90000, "final_status": "completed" }),
                ),
                "session.ended",
            ),
        ];
        for (hook, expected) in &cases {
            let reported = normalize(hook).unwrap_or_else(|e| panic!("{expected}: {e}"));
            assert_eq!(reported.kind, *expected);
            assert_eq!(reported.native_session_id.as_deref(), Some("conv-1"));
            let text = reported.payload.to_string();
            for content in [
                "fix the login",
                "secret",
                "rm -rf",
                "gone",
                "old_string",
                "Done, I fixed",
                "find the bug",
                "found it",
                "transcript",
                "/w/s",
            ] {
                assert!(
                    !text.contains(content),
                    "{expected} leaked {content:?}: {text}"
                );
            }
        }
        let read = normalize(&cases[2].0).unwrap();
        assert_eq!(read.payload["path"], "src/a.rs");
        assert_eq!(read.payload["durationMs"], 12);
        let outside = normalize(&cases[3].0).unwrap();
        assert_eq!(outside.payload["pathOutsideWorkspace"], true);
        let prompt = normalize(&cases[1].0).unwrap();
        assert_eq!(prompt.payload["chars"], 13);
        assert_eq!(prompt.payload["attachments"], 1);
        let stop = normalize(&cases[6].0).unwrap();
        assert_eq!(stop.payload["inputTokens"], 1200);
        let usage = normalize(&cases[7].0).unwrap();
        assert_eq!(usage.payload["cachedInputTokens"], 900);
        assert_eq!(usage.payload["chars"], 17);
        let failed_turn = normalize(&documented("stop", json!({ "status": "error" }))).unwrap();
        assert_eq!(failed_turn.kind, "turn.failed");
        assert!(
            normalize(&documented("preToolUse", json!({}))).is_err(),
            "never subscribed to, never mapped"
        );
        assert!(normalize(&json!({ "nope": 1 })).is_err());
    }

    #[test]
    fn the_plugin_directory_carries_a_manifest_and_hooks_that_run_this_executable() {
        let dir = tempfile::tempdir().unwrap();
        let mut args = vec!["--".to_owned(), "fix it".to_owned()];
        let plugin = arm(&mut args, dir.path()).unwrap();
        assert_eq!(plugin, plugin_dir(dir.path()));
        assert_eq!(args[0], "--plugin-dir");
        assert_eq!(args[1], plugin.to_string_lossy());
        assert_eq!(args[2..], ["--", "fix it"]);
        let manifest: Value = serde_json::from_slice(
            &std::fs::read(plugin.join(".cursor-plugin/plugin.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["name"], "yardsort-activity");
        assert_eq!(manifest["hooks"], "hooks/hooks.json");
        let hooks: Value =
            serde_json::from_slice(&std::fs::read(plugin.join("hooks/hooks.json")).unwrap())
                .unwrap();
        assert_eq!(hooks["version"], 1);
        let exe = super::super::claude::hook_executable().unwrap();
        for event in EVENTS {
            let hook = &hooks["hooks"][*event][0];
            assert_eq!(hook["type"], "command");
            assert_eq!(hook["timeout"], HOOK_TIMEOUT_SECS);
            let command = hook["command"].as_str().unwrap();
            assert!(
                command.starts_with(&format!(
                    "'{}' --yardsort-hook cursor '",
                    exe.to_string_lossy()
                )),
                "{command}"
            );
        }
        assert!(
            hooks["hooks"].get("preToolUse").is_none(),
            "nothing that decides"
        );
        assert!(hooks["hooks"].get("beforeShellExecution").is_none());
    }

    #[test]
    fn shell_words_survive_spaces_and_quotes_in_both_shells() {
        assert_eq!(
            shell_word("/opt/y a r d/yardsort"),
            "'/opt/y a r d/yardsort'"
        );
        assert_eq!(shell_word("it's"), "'it'\\''s'");
    }
}
