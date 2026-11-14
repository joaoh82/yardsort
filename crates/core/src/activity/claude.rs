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
//! the assistant said — with one exception: a tool call that created a pull request keeps that
//! pull request's link, so the workspace can follow it (`crate::tracked`). The shapes are those of the payloads recorded under
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
/// The `PermissionRequest` hook waits for a device to answer ([`super::asks`]): Claude Code's own
/// default for a command hook, a little past how long the question stays open.
pub const ASK_TIMEOUT_SECS: u32 = 600;

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
    let mut asking = command.clone();
    asking["timeout"] = json!(ASK_TIMEOUT_SECS);
    let mut hooks = serde_json::Map::new();
    for event in EVENTS {
        let hook = if *event == "PermissionRequest" {
            &asking
        } else {
            &command
        };
        hooks.insert((*event).to_owned(), json!([{ "hooks": [hook] }]));
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
    // A temporary name of this writer's own: two launches at once — two tabs, or the app and
    // `ys` — each write their file and each rename it into place; the last one wins, and both
    // are the same content.
    let temporary = path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4()));
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
            if let Some(url) = created_pull_request(hook) {
                payload["pullRequestUrl"] = Value::String(url);
            }
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

/// Add what the conversation's transcript says about its context to a finished turn: the tokens
/// the last request carried (`contextTokens`), the model that answered, and the model Claude
/// Code's settings name (`configuredModel`), which is how a `[1m]` window is told apart. A turn
/// whose transcript cannot be read is left as it is. Claude Code writes the turn's answer just
/// after running the hook, so this waits for it — briefly; see [`super::context::SETTLE_WAIT`].
pub fn add_context(
    reported: &mut Reported,
    hook: &Value,
    env: &dyn Fn(&str) -> Option<String>,
    wait: std::time::Duration,
) {
    if reported.kind != "turn.completed" {
        return;
    }
    let Some(transcript) = hook.get("transcript_path").and_then(Value::as_str) else {
        return;
    };
    let prompt_id = hook.get("prompt_id").and_then(Value::as_str);
    let Some((tokens, model)) =
        super::context::claude_turn_waiting(Path::new(transcript), prompt_id, wait)
    else {
        return;
    };
    let cwd = hook.get("cwd").and_then(Value::as_str).map(Path::new);
    reported.payload["contextTokens"] = json!(tokens);
    reported.payload["model"] = json!(model);
    reported.payload["configuredModel"] = json!(super::context::claude_configured_model(cwd, env));
}

/// The link of the pull request a finished tool call created, if it created one. Claude Code
/// says so itself for a shell command (`tool_response.gitOperation.pr`, with `action` created);
/// a version that does not is judged by the command — `gh pr create`, with the link it prints —
/// and an MCP call by its tool, a GitHub server's `create_pull_request`, and what it answered.
/// Nothing else of the call is looked at, and only the link is kept.
fn created_pull_request(hook: &Value) -> Option<String> {
    let response = hook.get("tool_response")?;
    let pr = response.get("gitOperation").and_then(|git| git.get("pr"));
    if let Some(pr) = pr {
        if pr.get("action").and_then(Value::as_str) == Some("created") {
            if let Some(url) = pr.get("url").and_then(Value::as_str) {
                return crate::tracked::link_in(url);
            }
        }
        // Claude Code looked and found it was not a creation: a `gh pr view`, say.
        return None;
    }
    let tool = hook.get("tool_name").and_then(Value::as_str)?;
    if tool == "Bash" {
        let command = hook
            .get("tool_input")
            .and_then(|input| input.get("command"))
            .and_then(Value::as_str)?;
        return crate::tracked::created_by_command(
            command,
            response.get("stdout").and_then(Value::as_str)?,
        );
    }
    if crate::tracked::creates_pull_request_tool(tool) {
        return crate::tracked::link_in_tool_result(response);
    }
    None
}

/// What a `PermissionRequest` asks, for a device to answer: the tool, the command it would run,
/// the file it would touch, and which call it is. `None` for any other event.
pub fn ask_of(hook: &Value) -> Option<(String, Option<String>, Option<String>, String)> {
    if hook.get("hook_event_name").and_then(Value::as_str) != Some("PermissionRequest") {
        return None;
    }
    let tool = hook.get("tool_name").and_then(Value::as_str)?.to_owned();
    let input = hook.get("tool_input");
    let command = input
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)
        .map(|command| remote_clip(command, super::asks::MAX_COMMAND));
    let (path, _) = tool_path(input, hook.get("cwd").and_then(Value::as_str));
    Some((
        tool.clone(),
        command,
        path,
        super::asks::call_of(&tool, input),
    ))
}

/// Which tool call a hook that follows a permission is about: `PostToolUse`,
/// `PostToolUseFailure` or `PermissionDenied`. `Some(None)` for a hook that ends the turn, after
/// which no question of it is open. `None` for any other.
pub fn moved_past(hook: &Value) -> Option<Option<String>> {
    match hook.get("hook_event_name").and_then(Value::as_str)? {
        "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => {
            let tool = hook.get("tool_name").and_then(Value::as_str)?;
            Some(Some(super::asks::call_of(tool, hook.get("tool_input"))))
        }
        "Stop" | "StopFailure" | "UserPromptSubmit" | "SessionEnd" => Some(None),
        _ => None,
    }
}

/// How much of a transcript's end is read to find the tool call a permission is about: it was
/// written just before Claude Code asked.
const CALL_TAIL_BYTES: u64 = 4 * 1024 * 1024;

/// Whether a permission was answered at the terminal, from Claude Code's own transcript: a "No"
/// or Esc there runs no hook at all, but the transcript gets the call's result. The call is the
/// newest `tool_use` with the question's tool and input; once a `tool_result` names it, the
/// question has its answer. Reads what was appended since it last looked, never the whole file.
pub struct CallWatch {
    path: std::path::PathBuf,
    offset: u64,
    partial: Vec<u8>,
    tool: String,
    input: Value,
    call: Option<String>,
}

impl CallWatch {
    /// For a `PermissionRequest` payload that names its transcript.
    pub fn new(hook: &Value) -> Option<Self> {
        let path = std::path::PathBuf::from(hook.get("transcript_path").and_then(Value::as_str)?);
        let length = std::fs::metadata(&path).ok()?.len();
        Some(Self {
            path,
            offset: length.saturating_sub(CALL_TAIL_BYTES),
            partial: Vec::new(),
            tool: hook.get("tool_name").and_then(Value::as_str)?.to_owned(),
            input: hook.get("tool_input").cloned().unwrap_or(Value::Null),
            call: None,
        })
    }

    /// Whether the call has had its result since this last looked.
    pub fn answered(&mut self) -> bool {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return false;
        };
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return false;
        }
        let mut more = Vec::new();
        let Ok(read) = file.read_to_end(&mut more) else {
            return false;
        };
        self.offset += read as u64;
        self.partial.extend_from_slice(&more);
        // Whole lines only: the last one may still be being written.
        let Some(end) = self.partial.iter().rposition(|&b| b == b'\n') else {
            return false;
        };
        let lines: Vec<u8> = self.partial.drain(..=end).collect();
        // A later call with the same input is the one asking: only a result after it counts.
        let mut answered = false;
        for line in lines.split(|&b| b == b'\n') {
            let Ok(line) = std::str::from_utf8(line) else {
                continue;
            };
            match self.seen(line) {
                Seen::Call => answered = false,
                Seen::Result => answered = true,
                Seen::Nothing => {}
            }
        }
        answered
    }

    fn seen(&mut self, line: &str) -> Seen {
        let use_line = line.contains("\"tool_use\"");
        let result_line = self
            .call
            .as_ref()
            .is_some_and(|id| line.contains("\"tool_result\"") && line.contains(id.as_str()));
        if !use_line && !result_line {
            return Seen::Nothing;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            return Seen::Nothing;
        };
        let blocks = entry
            .pointer("/message/content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut seen = Seen::Nothing;
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use")
                    if block.get("name").and_then(Value::as_str) == Some(self.tool.as_str())
                        && block.get("input") == Some(&self.input) =>
                {
                    self.call = block.get("id").and_then(Value::as_str).map(str::to_owned);
                    seen = Seen::Call;
                }
                Some("tool_result")
                    if self.call.is_some()
                        && block.get("tool_use_id").and_then(Value::as_str)
                            == self.call.as_deref() =>
                {
                    seen = Seen::Result;
                }
                _ => {}
            }
        }
        seen
    }
}

enum Seen {
    Nothing,
    /// The call the question is about, or a newer one with the same input.
    Call,
    /// That call's result.
    Result,
}

/// What the hook prints to answer a permission.
pub fn decision_output(decision: super::asks::Decision) -> Value {
    let decision = match decision {
        super::asks::Decision::Allow => json!({ "behavior": "allow" }),
        super::asks::Decision::Deny => {
            json!({ "behavior": "deny", "message": "Denied from Yardsort on a phone." })
        }
    };
    json!({
        "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision }
    })
}

fn remote_clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_owned(),
        Some((at, _)) => format!("{}…", &text[..at]),
    }
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
    super::workspace_relative(path, cwd)
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

    /// Recorded with `scripts/record-claude-pr-hooks.sh`: `gh pr create` in a nested clone and
    /// a GitHub MCP server's `create_pull_request`, both stand-ins.
    fn pull_request_fixtures() -> Vec<(String, Value)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("claude-hooks-pull-requests")
            .join("2.1.319");
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
                (
                    name,
                    serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap(),
                )
            })
            .collect()
    }

    #[test]
    fn a_pull_request_a_tool_call_created_keeps_its_link_and_nothing_else() {
        let seen: Vec<(String, &'static str, Option<String>)> = pull_request_fixtures()
            .iter()
            .map(|(name, hook)| {
                let reported = normalize(hook).unwrap_or_else(|e| panic!("{name}: {e}"));
                let url = reported.payload["pullRequestUrl"]
                    .as_str()
                    .map(str::to_owned);
                (name.clone(), reported.kind, url)
            })
            .collect();
        let link = |n: u32| Some(format!("https://github.com/example/lib/pull/{n}"));
        assert_eq!(
            seen,
            [
                ("01-PreToolUse".into(), "tool.started", None),
                // `cd lib && gh pr create …`: Claude Code's own `gitOperation` says so.
                ("02-PostToolUse".into(), "tool.completed", link(41)),
                ("03-PreToolUse".into(), "tool.started", None),
                ("04-PostToolUse".into(), "tool.completed", None),
                ("05-PreToolUse".into(), "tool.started", None),
                // `mcp__github__create_pull_request`: its answer's `url`.
                ("06-PostToolUse".into(), "tool.completed", link(42)),
            ]
        );
        for (name, hook) in pull_request_fixtures() {
            let text = normalize(&hook).unwrap().payload.to_string();
            for content in [
                "Bump the client",
                "Moves to the new client",
                "cd lib",
                "--base",
                "Creating pull request",
                "2989102733",
                "\"head\"",
                "/tmp/yardsort-fixture",
            ] {
                assert!(!text.contains(content), "{name} leaked {content:?}: {text}");
            }
        }
    }

    #[test]
    fn without_claude_code_saying_so_the_command_or_the_tool_decides() {
        let bash = |command: &str, stdout: &str| {
            normalize(&json!({
                "hook_event_name": "PostToolUse", "session_id": "s", "tool_name": "Bash",
                "tool_input": { "command": command },
                "tool_response": { "stdout": stdout, "stderr": "" }
            }))
            .unwrap()
            .payload["pullRequestUrl"]
                .as_str()
                .map(str::to_owned)
        };
        let printed = "https://github.com/o/lib/pull/7\n";
        assert_eq!(
            bash("cd lib && gh pr create --fill", printed).as_deref(),
            Some("https://github.com/o/lib/pull/7")
        );
        assert_eq!(bash("gh pr view 7", printed), None, "only a creation");
        assert_eq!(bash("echo https://github.com/o/lib/pull/7", printed), None);
        assert_eq!(bash("gh pr create --fill", "no link here"), None);

        // Claude Code saw the command and says it created nothing: that is the answer.
        let viewed = normalize(&json!({
            "hook_event_name": "PostToolUse", "session_id": "s", "tool_name": "Bash",
            "tool_input": { "command": "gh pr create --fill" },
            "tool_response": { "stdout": printed,
                "gitOperation": { "pr": { "number": 7, "url": "https://github.com/o/lib/pull/7", "action": "viewed" } } }
        }))
        .unwrap();
        assert!(viewed.payload.get("pullRequestUrl").is_none());

        // A full answer from GitHub names other pull requests in its description first.
        let full = normalize(&json!({
            "hook_event_name": "PostToolUse", "session_id": "s",
            "tool_name": "mcp__claude_ai_GitHub__create_pull_request",
            "tool_input": { "owner": "o", "repo": "lib" },
            "tool_response": [{ "type": "text", "text": json!({
                "url": "https://api.github.com/repos/o/lib/pulls/9",
                "body": "Follows https://github.com/o/lib/pull/3.",
                "html_url": "https://github.com/o/lib/pull/9"
            }).to_string() }]
        }))
        .unwrap();
        assert_eq!(
            full.payload["pullRequestUrl"],
            "https://github.com/o/lib/pull/9"
        );
        assert!(!full.payload.to_string().contains("Follows"));

        // Only a finished call: one that failed or is about to run has no pull request yet.
        let failed = normalize(&json!({
            "hook_event_name": "PostToolUseFailure", "session_id": "s", "tool_name": "Bash",
            "tool_input": { "command": "gh pr create --fill" },
            "tool_response": { "stdout": printed }
        }))
        .unwrap();
        assert!(failed.payload.get("pullRequestUrl").is_none());
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
        assert_eq!(at("/w/s", "./rel/../b.rs"), (Some("b.rs".into()), false));
        // Climbing out is climbing out, however it is spelled — seen in review.
        assert_eq!(
            at("/work/repo", "../../home/alice/private.txt"),
            (None, true)
        );
        assert_eq!(
            at("/work/repo", "/work/repo/../../home/alice/private.txt"),
            (None, true)
        );
        assert_eq!(
            at("/work/repo", "/work/repo/../repo/src/a.rs"),
            (Some("src/a.rs".into()), false)
        );
        assert_eq!(at("C:\\w\\s", "..\\..\\..\\Users\\x\\secret"), (None, true));
        assert_eq!(
            at("c:\\w\\s", "C:/w/s/a.rs"),
            (Some("a.rs".into()), false),
            "drive case"
        );
        // A sibling that differs only in case is a different directory on Linux — seen in review.
        assert_eq!(at("/work/repo", "/work/REPO/private.txt"), (None, true));
        assert_eq!(at("/work/repo", "/Work/repo/a.rs"), (None, true));
        assert_eq!(
            tool_path(Some(&json!({ "file_path": "a.rs" })), None),
            (None, true)
        );
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
            // The one that waits for a device's answer has as long as Claude Code gives any.
            let timeout = if *event == "PermissionRequest" {
                ASK_TIMEOUT_SECS
            } else {
                HOOK_TIMEOUT_SECS
            };
            assert_eq!(hook["timeout"], timeout, "{event}");
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

        // Sixteen launches at once, as the app and `ys` and many tabs can be: every one arms,
        // and the file each named is whole. Seen failing in review with one shared temporary.
        let shared = std::sync::Arc::new(dir.path().to_path_buf());
        let writers: Vec<_> = (0..16)
            .map(|_| {
                let data_dir = std::sync::Arc::clone(&shared);
                std::thread::spawn(move || (0..20).all(|_| arm(&mut vec![], &data_dir).is_ok()))
            })
            .collect();
        assert!(writers.into_iter().all(|w| w.join().unwrap()));
        let written: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written["hooks"]["Stop"][0]["hooks"][0]["command"], expected);
        assert!(
            std::fs::read_dir(path.parent().unwrap())
                .unwrap()
                .all(|e| e.unwrap().path() == path),
            "no temporary files left behind"
        );

        let mut theirs = vec!["--settings".to_owned(), "mine.json".to_owned()];
        let why = arm(&mut theirs, dir.path()).unwrap_err();
        assert!(why.contains("--settings"), "{why}");
        assert_eq!(theirs.len(), 2, "untouched");
        let mut bare = vec!["--bare".to_owned()];
        assert!(arm(&mut bare, dir.path()).is_err());
    }

    #[test]
    fn a_permission_answered_at_the_terminal_is_seen_in_the_transcript_by_its_call() {
        let dir = tempfile::tempdir().unwrap();
        let transcript = dir.path().join("t.jsonl");
        let input = json!({"command": "curl -sI https://example.com", "description": "Fetch"});
        let tool_use = |id: &str, command: &str| {
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": id, "name": "Bash", "input": {"command": command, "description": "Fetch"}}]}}).to_string()
        };
        let result = |id: &str| {
            json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": id, "content": "The user doesn't want to proceed with this tool use.", "is_error": true}]}}).to_string()
        };
        std::fs::write(
            &transcript,
            format!(
                "{}\n{}\n{}\n",
                tool_use("toolu_old", "curl -sI https://example.com"),
                result("toolu_old"),
                tool_use("toolu_new", "curl -sI https://example.com")
            ),
        )
        .unwrap();
        let hook = json!({
            "hook_event_name": "PermissionRequest",
            "transcript_path": transcript.to_string_lossy(),
            "tool_name": "Bash",
            "tool_input": input,
        });
        let mut watch = CallWatch::new(&hook).unwrap();
        // The older call's result is not this one's.
        assert!(!watch.answered());
        let append = |text: String| {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&transcript)
                .unwrap();
            file.write_all(text.as_bytes()).unwrap();
        };
        // Another call's result, and half a line still being written, are not it either.
        append(format!("{}\n", result("toolu_other")));
        let whole = result("toolu_new");
        let (first, rest) = whole.split_at(20);
        append(first.to_owned());
        assert!(!watch.answered());
        append(format!("{rest}\n"));
        assert!(watch.answered());
    }
}
