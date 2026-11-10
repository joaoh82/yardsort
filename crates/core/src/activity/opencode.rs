//! OpenCode, as a source of activity: a plugin given for one launch, through the environment.
//!
//! OpenCode loads plugins from its configuration, and `OPENCODE_CONFIG_CONTENT` is
//! configuration given in the environment for one process — merged with the user's own, so
//! their plugins keep running and ours is added for that launch alone. The plugin
//! ([`PLUGIN_TEMPLATE`]) runs inside OpenCode's process, listens to a few of its hooks and bus
//! events, keeps their metadata and hands each to this executable in hook mode on stdin. No
//! file of the user's is edited, nothing is installed, and `--pure` — OpenCode's own "no
//! external plugins" — is respected by not arming.
//!
//! The shapes are those of the fixtures under `crates/core/fixtures/opencode/`, recorded from
//! the raw hooks; the plugin sends a subset of the same shape, so both read alike. See
//! `docs/design/13-agent-events-stage-2-opencode.md`.
//!
//! OpenCode 2 changed the plugin API and where sessions run. The one module serves both
//! (OpenCode 1 starts it through `server`, 2 through `setup`), given as a file to 1's `plugin`
//! list and as its folder to 2's `plugins`; 2's deliveries carry `api: 2` and are read by
//! [`normalize_v2`]. OpenCode 2 runs every launch's sessions in one background service unless
//! told `--standalone`, and that service keeps the environment of whichever launch started it,
//! so an OpenCode 2 launch is given `--standalone` too: its sessions run in a server of its own,
//! with this launch's ids. Which OpenCode it is, [`major_version`] asks the program itself.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// The built-in harness this adapter belongs to.
pub const HARNESS_ID: &str = "opencode";
pub const PRODUCER: &str = "opencode";
pub const METHOD: &str = "plugin";
pub const FIDELITY: &str = "reported";
/// The OpenCode version the fixtures — and so the mapping — come from.
pub const FIXTURE_VERSION: &str = "1.18.31";
/// The OpenCode 2 version [`normalize_v2`]'s fixtures come from.
pub const FIXTURE_VERSION_2: &str = "2.2.6";
/// What gives an OpenCode 2 launch a server of its own, instead of the shared background one.
pub const STANDALONE: &str = "--standalone";
/// The environment variable OpenCode reads inline configuration from.
pub const CONFIG_ENV: &str = "OPENCODE_CONFIG_CONTENT";
/// The plugin, with the executable and inbox to substitute.
pub const PLUGIN_TEMPLATE: &str = include_str!("opencode-plugin.js");

/// Where the plugin file lives: written before each launch, so it names the current executable.
/// In a folder of its own, because OpenCode 2 takes a plugin as a folder and loads its `index.js`.
pub fn plugin_path(data_dir: &Path) -> PathBuf {
    plugin_dir(data_dir).join("index.js")
}

/// The folder OpenCode 2 is given.
pub fn plugin_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("activity").join("hooks").join("opencode")
}

/// Where the plugin was written before it served OpenCode 2, removed when the new one is.
fn old_plugin_path(data_dir: &Path) -> PathBuf {
    data_dir.join("activity").join("hooks").join("opencode.js")
}

/// The plugin's source for this profile.
pub fn plugin_source(hook: &Path, inbox_dir: &Path) -> String {
    let quoted = |path: &Path| {
        // A JSON string literal is a JavaScript one, backslashes and all.
        let text = serde_json::to_string(&path.to_string_lossy()).unwrap_or_default();
        text.trim_matches('"').to_owned()
    };
    PLUGIN_TEMPLATE
        .replace("__YARDSORT_EXE__", &quoted(hook))
        .replace("__YARDSORT_INBOX__", &quoted(inbox_dir))
}

/// A `file://` URL for a local path, as OpenCode's `plugin` list takes them. Every byte a URL
/// would read as something else — `#`, `%`, `?`, a space — is percent-encoded, so a profile
/// under `profile#one` loads the same as any other; `/` and a drive's `:` are kept.
pub fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                encoded.push(byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    if encoded.starts_with('/') {
        format!("file://{encoded}")
    } else {
        format!("file:///{encoded}")
    }
}

/// The value `OPENCODE_CONFIG_CONTENT` should have: the launch environment's own, if any,
/// with our plugin added to its `plugin` list (OpenCode 1, which takes the file) or its
/// `plugins` list (OpenCode 2, which takes the folder) — both, when which one it is is not
/// known, as each ignores the other's key. Refuses
/// configuration it cannot read: the user's settings are not to be replaced by ours.
pub fn config_content(
    existing: Option<&str>,
    plugin_url: &str,
    plugin_dir_url: &str,
    major: Option<u32>,
) -> Result<String, String> {
    let mut config = match existing.map(str::trim).filter(|s| !s.is_empty()) {
        Some(text) => serde_json::from_str::<Value>(text)
            .map_err(|error| format!("{CONFIG_ENV} is not JSON: {error}"))?,
        None => json!({}),
    };
    let object = config
        .as_object_mut()
        .ok_or_else(|| format!("{CONFIG_ENV} is not a JSON object"))?;
    // Known, only the key that version reads: a list the other one reads is not ours to check.
    let keys: &[(&str, &str)] = match major {
        Some(1) => &[("plugin", plugin_url)],
        Some(_) => &[("plugins", plugin_dir_url)],
        None => &[("plugin", plugin_url), ("plugins", plugin_dir_url)],
    };
    for &(key, url) in keys {
        let list = object
            .entry(key)
            .or_insert_with(|| Value::Array(vec![]))
            .as_array_mut()
            .ok_or_else(|| format!("{CONFIG_ENV} has a `{key}` that is not a list"))?;
        if !list.iter().any(|p| p.as_str() == Some(url)) {
            list.push(Value::String(url.to_owned()));
        }
    }
    Ok(config.to_string())
}

/// The major version an `opencode --version` printed: `opencode v2.2.6` for OpenCode 2,
/// `1.18.34` for 1.
/// Anything else — a program that is not OpenCode, printing a banner — is no answer.
pub fn parse_major(printed: &str) -> Option<u32> {
    let line = printed
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let words: Vec<&str> = line.split_whitespace().collect();
    let version = match words.as_slice() {
        [version] | ["opencode", version] => *version,
        _ => return None,
    };
    let version = version.strip_prefix('v').unwrap_or(version);
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    parts
        .next()
        .is_some_and(|minor| !minor.is_empty() && minor.bytes().all(|b| b.is_ascii_digit()))
        .then_some(major)
}

/// Which OpenCode `program` is, by asking it: `--version`, in the launch's environment and its
/// folder (a version manager's shim answers for the version that folder pins). Kept for a
/// minute per executable, folder and modification time, as a workflow can start several at
/// once; an answer that does not come within a few seconds is no answer, and is not kept. Only
/// a program named `opencode…` is asked: a wrapper (`npx`, `bunx`) would answer for itself.
/// `None` when it cannot be told; the launch is then armed for both versions without
/// `--standalone`, and an OpenCode 2 one reports nothing (the plugin, finding itself in the
/// shared service, stays quiet).
pub fn major_version(env: &crate::env::ShellEnv, program: &str, cwd: &Path) -> Option<u32> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};
    type Key = (PathBuf, PathBuf, Option<std::time::SystemTime>);
    type Asked = HashMap<Key, (Instant, u32)>;
    static ASKED: OnceLock<Mutex<Asked>> = OnceLock::new();
    const FRESH: Duration = Duration::from_secs(60);

    let path = env.find_program(program, cwd)?;
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("opencode"));
    if !named {
        return None;
    }
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let key = (path.clone(), cwd.to_path_buf(), modified);
    let asked = ASKED.get_or_init(Default::default);
    if let Some((when, major)) = asked.lock().ok()?.get(&key) {
        if when.elapsed() < FRESH {
            return Some(*major);
        }
    }
    let mut command = crate::program::Program::at(path, env).command(cwd);
    command
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let major = output_within(&mut command, Duration::from_secs(5))
        .as_deref()
        .and_then(parse_major)?;
    if let Ok(mut asked) = asked.lock() {
        asked.insert(key, (Instant::now(), major));
    }
    Some(major)
}

/// What a short-lived program printed, or `None` if it failed or was still running at
/// `limit`. It runs in a process group of its own, ended when it is still running then, or when
/// something it started (`cmd.exe` running a `.cmd` shim starts node) still holds the pipe after
/// it exited. Only then: a group that has ended has nothing left to signal.
fn output_within(
    command: &mut std::process::Command,
    limit: std::time::Duration,
) -> Option<String> {
    use std::io::Read;
    let mut child = crate::process::Group::spawn(command).ok()?;
    let mut stdout = child.stdout()?;
    let (sender, read) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = sender.send(stdout.read_to_string(&mut text).map(|_| text));
    });
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let left = limit.saturating_sub(started.elapsed());
                return match read.recv_timeout(left) {
                    Ok(text) => text.ok().filter(|_| status.success()),
                    Err(_) => {
                        child.kill_tree();
                        None
                    }
                };
            }
            Ok(None) if started.elapsed() < limit => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => {
                child.kill_tree();
                return None;
            }
        }
    }
}

/// Give an OpenCode launch its plugin: write the file and return the variable to set. Refuses
/// when the arguments carry `--pure` (no external plugins, the user said), or when the
/// environment's own configuration cannot be added to. For OpenCode 2 (`major` of 2 or more),
/// `--standalone` is added to the arguments, first, unless they have it: the shared background
/// service would run the sessions with another launch's ids, or with none. Refused when they
/// name a `--server` instead, whose sessions run where Yardsort's plugin is not.
pub fn arm(
    args: &mut Vec<String>,
    data_dir: &Path,
    existing_config: Option<&str>,
    major: Option<u32>,
) -> Result<(String, String), String> {
    if args.iter().any(|arg| arg == "--pure") {
        return Err("the harness's arguments carry `--pure`".to_owned());
    }
    let two = major.is_some_and(|major| major >= 2);
    if two
        && args
            .iter()
            .any(|arg| arg == "--server" || arg.starts_with("--server="))
    {
        return Err("the harness's arguments carry `--server`".to_owned());
    }
    let hook = super::claude::hook_executable()
        .map_err(|error| format!("no executable to run: {error}"))?;
    let path = plugin_path(data_dir);
    let dir = plugin_dir(data_dir);
    std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let source = plugin_source(&hook, &super::inbox_dir(data_dir));
    let temporary = dir.join(format!("index.js.{}.tmp", uuid::Uuid::new_v4()));
    let write = || -> io::Result<()> {
        std::fs::write(&temporary, source.as_bytes())?;
        std::fs::rename(&temporary, &path)
    };
    write().map_err(|error| format!("{}: {error}", path.display()))?;
    let _ = std::fs::remove_file(old_plugin_path(data_dir));
    let content = config_content(existing_config, &file_url(&path), &file_url(&dir), major)?;
    if two && !args.iter().any(|arg| arg == STANDALONE) {
        args.insert(0, STANDALONE.to_owned());
    }
    Ok((CONFIG_ENV.to_owned(), content))
}

/// One plugin delivery, reduced to what Yardsort records.
#[derive(Debug, Clone, PartialEq)]
pub struct Reported {
    pub kind: &'static str,
    pub native_session_id: Option<String>,
    pub payload: Value,
}

/// The pull request a finished call opened: `bash` running `gh pr create`, by what it printed;
/// a GitHub server's create-pull-request tool (`<server>_<tool>`), by its answer.
fn created_pull_request(input: &Value, output: &Value) -> Option<String> {
    let tool = input.get("tool").and_then(Value::as_str)?;
    if tool == "bash" {
        // `gh pr create` where a pull request exists already fails, printing that one's link.
        if output["metadata"]
            .get("exit")
            .and_then(Value::as_i64)
            .is_some_and(|code| code != 0)
        {
            return None;
        }
        let command = input["args"].get("command").and_then(Value::as_str)?;
        let printed = output
            .get("output")
            .or_else(|| output["metadata"].get("output"))
            .and_then(Value::as_str)?;
        return crate::tracked::created_by_command(command, printed);
    }
    if crate::tracked::is_create_pull_request("", tool) {
        return crate::tracked::link_in_tool_result(output.get("content")?);
    }
    None
}

/// Turn what the plugin sent — `{hook, payload}` — into an event. Reads the raw hook shapes,
/// of which the plugin's are a subset. `Err` names a call this build has no mapping for.
pub fn normalize(delivery: &Value) -> Result<Reported, String> {
    if delivery.get("api").and_then(Value::as_u64) == Some(2) {
        return normalize_v2(delivery);
    }
    let hook = delivery
        .get("hook")
        .and_then(Value::as_str)
        .ok_or_else(|| "no hook name".to_owned())?;
    let payload = delivery.get("payload").cloned().unwrap_or(Value::Null);
    let input = &payload["input"];
    let output = &payload["output"];
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
    // The directory the launch was given, which the plugin sends with every delivery: what a
    // file's path is measured against, unless the tool names its own `workdir`.
    let directory = text(delivery, "directory");
    let relative = |args: &Value, cwd: Option<&str>| -> (Option<String>, bool) {
        match text(args, "filePath") {
            Some(path) => super::workspace_relative(&path, cwd.or(directory.as_deref())),
            None => (None, false),
        }
    };
    let with_path = |mut payload: Value, (path, outside): (Option<String>, bool)| {
        if let Some(path) = path {
            payload["path"] = Value::String(path);
        }
        if outside {
            payload["pathOutsideWorkspace"] = Value::Bool(true);
        }
        payload
    };

    match hook {
        "event" => {
            let props = &payload["properties"];
            let session = text(props, "sessionID");
            let (kind, event) = match text(&payload, "type").as_deref() {
                Some("session.created") => (
                    "session.started",
                    json!({
                        "sessionId": session,
                        "version": text(&props["info"], "version"),
                        "parentId": text(&props["info"], "parentID"),
                    }),
                ),
                Some("session.idle") => ("turn.completed", json!({ "sessionId": session })),
                Some("session.error") => (
                    "turn.failed",
                    json!({ "sessionId": session, "errorType": text(&props["error"], "name") }),
                ),
                Some("file.edited") => {
                    let path = text(props, "file").unwrap_or_default();
                    (
                        "file.reported_write",
                        with_path(
                            json!({ "sessionId": session, "kind": "changed" }),
                            super::workspace_relative(&path, directory.as_deref()),
                        ),
                    )
                }
                // OpenCode's permission is the tool it is asked for — `bash`, `edit` — and
                // the timeline names an approval by its `tool`.
                Some("permission.asked") => {
                    let permission = text(props, "permission").or_else(|| text(props, "type"));
                    (
                        "approval.requested",
                        json!({
                            "sessionId": session,
                            "tool": permission,
                            "permission": permission,
                            "permissionId": text(props, "id"),
                            "toolUseId": text(props, "callID").or_else(|| text(&props["tool"], "callID")),
                        }),
                    )
                }
                Some("permission.replied") => (
                    "approval.resolved",
                    json!({
                        "sessionId": session,
                        "decision": text(props, "response"),
                        "permissionId": text(props, "permissionID"),
                    }),
                ),
                Some("message.updated") => {
                    let info = &props["info"];
                    if text(info, "role").as_deref() != Some("assistant")
                        || info["time"].get("completed").is_none()
                    {
                        return Err(
                            "message.updated is only recorded once an assistant message completes"
                                .to_owned(),
                        );
                    }
                    let tokens = &info["tokens"];
                    let n = |v: &Value, key: &str| v.get(key).and_then(Value::as_i64);
                    (
                        "usage.reported",
                        json!({
                            "sessionId": session,
                            "messageId": text(info, "id"),
                            "model": match (text(info, "providerID"), text(info, "modelID")) {
                                (Some(p), Some(m)) => Some(format!("{p}/{m}")),
                                (_, m) => m,
                            },
                            "agent": text(info, "agent"),
                            "finish": text(info, "finish"),
                            "inputTokens": n(tokens, "input"),
                            "outputTokens": n(tokens, "output"),
                            "reasoningOutputTokens": n(tokens, "reasoning"),
                            "cachedInputTokens": n(&tokens["cache"], "read"),
                            "cacheWriteTokens": n(&tokens["cache"], "write"),
                            "totalTokens": n(tokens, "total"),
                            "cost": info.get("cost").and_then(Value::as_f64),
                        }),
                    )
                }
                Some("message.part.updated") => {
                    let part = &props["part"];
                    let state = &part["state"];
                    if text(part, "type").as_deref() != Some("tool")
                        || text(state, "status").as_deref() != Some("error")
                    {
                        return Err(
                            "message.part.updated is only recorded for a tool that failed"
                                .to_owned(),
                        );
                    }
                    let duration = match (n64(&state["time"], "start"), n64(&state["time"], "end"))
                    {
                        (Some(start), Some(end)) => Some(end - start),
                        _ => None,
                    };
                    let cwd = text(&state["input"], "workdir");
                    (
                        "tool.failed",
                        with_path(
                            json!({
                                "tool": text(part, "tool"),
                                "toolUseId": text(part, "callID"),
                                "sessionId": session,
                                "messageId": text(part, "messageID"),
                                "durationMs": duration,
                            }),
                            relative(&state["input"], cwd.as_deref()),
                        ),
                    )
                }
                Some(other) => return Err(format!("no mapping for event {other}")),
                None => return Err("event without a type".to_owned()),
            };
            Ok(Reported {
                kind,
                native_session_id: session,
                payload: event,
            })
        }
        "chat.message" => {
            let message = &output["message"];
            let chars = output.get("chars").and_then(Value::as_u64).or_else(|| {
                output["parts"].as_array().map(|parts| {
                    parts
                        .iter()
                        .filter(|p| text(p, "type").as_deref() == Some("text"))
                        .filter_map(|p| text(p, "text"))
                        .map(|t| t.chars().count() as u64)
                        .sum()
                })
            });
            let session = text(input, "sessionID").or_else(|| text(message, "sessionID"));
            Ok(Reported {
                kind: "prompt.submitted",
                native_session_id: session.clone(),
                payload: json!({
                    "sessionId": session,
                    "messageId": text(message, "id"),
                    "chars": chars,
                    "agent": text(message, "agent"),
                    "model": match (text(&message["model"], "providerID"), text(&message["model"], "modelID")) {
                        (Some(p), Some(m)) => Some(format!("{p}/{m}")),
                        (_, m) => m,
                    },
                }),
            })
        }
        "tool.execute.before" | "tool.execute.after" => {
            let after = hook == "tool.execute.after";
            let args = if after {
                &input["args"]
            } else {
                &output["args"]
            };
            let cwd = text(args, "workdir");
            let session = text(input, "sessionID");
            let mut event = json!({
                "tool": text(input, "tool"),
                "toolUseId": text(input, "callID"),
                "sessionId": session,
            });
            if after {
                let metadata = &output["metadata"];
                if let Some(exit) = n64(metadata, "exit") {
                    event["exitCode"] = json!(exit);
                }
                if metadata.get("truncated").and_then(Value::as_bool) == Some(true) {
                    event["truncated"] = json!(true);
                }
                if metadata.get("exists").and_then(Value::as_bool) == Some(false) {
                    event["created"] = json!(true);
                }
                // A pull request the call opened keeps its link and nothing else: the one
                // exception to metadata only (docs/design/38-pull-requests-elsewhere.md).
                if let Some(url) = created_pull_request(input, output) {
                    event["pullRequestUrl"] = Value::String(url);
                }
            }
            Ok(Reported {
                kind: if after {
                    "tool.completed"
                } else {
                    "tool.started"
                },
                native_session_id: session,
                payload: with_path(event, relative(args, cwd.as_deref())),
            })
        }
        "permission.ask" => {
            let session = text(input, "sessionID");
            let permission = text(input, "permission").or_else(|| text(input, "type"));
            Ok(Reported {
                kind: "approval.requested",
                native_session_id: session.clone(),
                payload: json!({
                    "sessionId": session,
                    "tool": permission,
                    "permission": permission,
                    "permissionId": text(input, "id"),
                    "toolUseId": text(input, "callID"),
                }),
            })
        }
        other => Err(format!("no mapping for plugin hook {other}")),
    }
}

/// The pull request an OpenCode 2 call opened: its shell tool (`shell`) running `gh pr create`,
/// by what it printed; a GitHub server's create-pull-request tool, by its answer.
fn created_pull_request_v2(event: &Value) -> Option<String> {
    let tool = event.get("tool").and_then(Value::as_str)?;
    let result = &event["result"];
    if tool == "shell" || tool == "bash" {
        let exit = result["output"]
            .get("exit")
            .or_else(|| result["metadata"].get("exit"))
            .and_then(Value::as_i64);
        if exit.is_some_and(|code| code != 0) {
            return None;
        }
        let command = event["input"].get("command").and_then(Value::as_str)?;
        let printed = result["output"].get("output").and_then(Value::as_str)?;
        return crate::tracked::created_by_command(command, printed);
    }
    if crate::tracked::is_create_pull_request("", tool) {
        return crate::tracked::link_in_tool_result(result.get("content")?);
    }
    None
}

/// Turn what OpenCode 2's API reported — `{api: 2, hook, payload}` — into an event. Reads the raw
/// shapes of `crates/core/fixtures/opencode/2.2.6/`, of which the plugin's are a subset, plus the
/// two fields the plugin adds from its own memory: a failed call's `durationMs` and a step's
/// `model`.
pub fn normalize_v2(delivery: &Value) -> Result<Reported, String> {
    let hook = delivery
        .get("hook")
        .and_then(Value::as_str)
        .ok_or_else(|| "no hook name".to_owned())?;
    let payload = delivery.get("payload").cloned().unwrap_or(Value::Null);
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
    let n = |v: &Value, key: &str| v.get(key).and_then(Value::as_i64);
    let directory = text(delivery, "directory");
    let model_name = |model: &Value| match (text(model, "providerID"), text(model, "id")) {
        (Some(p), Some(m)) => Some(format!("{p}/{m}")),
        (_, m) => m,
    };
    let reported = |kind: &'static str, session: Option<String>, payload: Value| {
        Ok(Reported {
            kind,
            native_session_id: session,
            payload,
        })
    };

    match hook {
        "event" => {
            let data = &payload["data"];
            let session = text(data, "sessionID");
            match text(&payload, "type").as_deref() {
                Some("session.created") => reported(
                    "session.started",
                    session.clone(),
                    json!({
                        "sessionId": session,
                        "version": text(data, "version"),
                        "parentId": text(data, "parentID"),
                    }),
                ),
                Some("session.execution.succeeded") => reported(
                    "turn.completed",
                    session.clone(),
                    json!({ "sessionId": session }),
                ),
                // Stopped by the user: the turn ends without its answer, and is no failure.
                Some("session.execution.interrupted") => reported(
                    "turn.completed",
                    session.clone(),
                    json!({ "sessionId": session, "interrupted": true }),
                ),
                Some("session.execution.failed") => reported(
                    "turn.failed",
                    session.clone(),
                    json!({
                        "sessionId": session,
                        "errorType": text(&data["error"], "type")
                            .or_else(|| text(&data["error"], "_tag"))
                            .or_else(|| text(&data["error"], "name")),
                    }),
                ),
                Some("session.step.ended") => {
                    let tokens = &data["tokens"];
                    let input = n(tokens, "input");
                    let output = n(tokens, "output");
                    let reasoning = n(tokens, "reasoning");
                    let read = n(&tokens["cache"], "read");
                    let write = n(&tokens["cache"], "write");
                    // OpenCode 2 reports no total; it is the sum of what it does report.
                    let total = [input, output, reasoning, read, write]
                        .iter()
                        .flatten()
                        .sum::<i64>();
                    reported(
                        "usage.reported",
                        session.clone(),
                        json!({
                            "sessionId": session,
                            "messageId": text(data, "assistantMessageID"),
                            "model": model_name(&data["model"]),
                            "finish": text(data, "finish"),
                            "inputTokens": input,
                            "outputTokens": output,
                            "reasoningOutputTokens": reasoning,
                            "cachedInputTokens": read,
                            "cacheWriteTokens": write,
                            "totalTokens": (tokens.is_object()).then_some(total),
                            "cost": data.get("cost").and_then(Value::as_f64),
                        }),
                    )
                }
                // The action is the tool it is asked for — `shell`, `edit` — and the timeline
                // names an approval by its `tool`.
                Some("permission.asked") => {
                    let action = text(data, "action");
                    reported(
                        "approval.requested",
                        session.clone(),
                        json!({
                            "sessionId": session,
                            "tool": action,
                            "permission": action,
                            "permissionId": text(data, "id"),
                            "toolUseId": text(&data["source"], "id"),
                        }),
                    )
                }
                Some("permission.replied") => reported(
                    "approval.resolved",
                    session.clone(),
                    json!({
                        "sessionId": session,
                        "decision": text(data, "reply"),
                        "permissionId": text(data, "requestID"),
                    }),
                ),
                Some(other) => Err(format!("no mapping for OpenCode 2 event {other}")),
                None => Err("event without a type".to_owned()),
            }
        }
        "session.prompt" => {
            let session = text(&payload, "sessionID");
            let chars = payload.get("chars").and_then(Value::as_u64).or_else(|| {
                payload["prompt"]
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|t| t.chars().count() as u64)
            });
            reported(
                "prompt.submitted",
                session.clone(),
                json!({
                    "sessionId": session,
                    "messageId": text(&payload, "messageID"),
                    "chars": chars,
                }),
            )
        }
        "tool.execute.before" | "tool.execute.after" => {
            let input = &payload["input"];
            let session = text(&payload, "sessionID");
            let cwd = text(input, "workdir");
            let mut event = json!({
                "tool": text(&payload, "tool"),
                "toolUseId": text(&payload, "id"),
                "sessionId": session,
                "messageId": text(&payload, "messageID"),
            });
            let path = text(input, "path").or_else(|| text(input, "filePath"));
            let (path, outside) = match path {
                Some(path) => {
                    super::workspace_relative(&path, cwd.as_deref().or(directory.as_deref()))
                }
                None => (None, false),
            };
            if let Some(path) = path {
                event["path"] = Value::String(path);
            }
            if outside {
                event["pathOutsideWorkspace"] = Value::Bool(true);
            }
            if hook == "tool.execute.before" {
                return reported("tool.started", session, event);
            }
            match text(&payload, "status").as_deref() {
                Some("completed") => {
                    let result = &payload["result"];
                    let exit =
                        n(&result["output"], "exit").or_else(|| n(&result["metadata"], "exit"));
                    if let Some(exit) = exit {
                        event["exitCode"] = json!(exit);
                    }
                    let truncated = |v: &Value| v.get("truncated").and_then(Value::as_bool);
                    if truncated(&result["output"]).or_else(|| truncated(&result["metadata"]))
                        == Some(true)
                    {
                        event["truncated"] = json!(true);
                    }
                    if result["output"].get("existed").and_then(Value::as_bool) == Some(false) {
                        event["created"] = json!(true);
                    }
                    // OpenCode 2 has no event of a file edited, as 1 had: its writing tools'
                    // ends are the writes, and say so for provenance to count.
                    // `patch` writes too, but names no path a write could be put on.
                    if matches!(text(&payload, "tool").as_deref(), Some("write" | "edit")) {
                        event["wrote"] = json!(true);
                    }
                    // The one exception to metadata only (docs/design/38-pull-requests-elsewhere.md).
                    if let Some(url) = created_pull_request_v2(&payload) {
                        event["pullRequestUrl"] = Value::String(url);
                    }
                    reported("tool.completed", session, event)
                }
                Some("error") => {
                    if let Some(ms) = n(&payload, "durationMs") {
                        event["durationMs"] = json!(ms);
                    }
                    reported("tool.failed", session, event)
                }
                other => Err(format!("a tool's end with no known status: {other:?}")),
            }
        }
        other => Err(format!("no mapping for OpenCode 2 hook {other}")),
    }
}

fn n64(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> Vec<(String, Value)> {
        fixtures_in(&Path::new("opencode").join(FIXTURE_VERSION))
    }

    fn fixtures_in(under: &Path) -> Vec<(String, Value)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(under);
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

    /// Recorded with `scripts/record-pr-creation.sh opencode` (OpenCode 1.18.34): `gh pr create`
    /// and a GitHub MCP server's `create_pull_request`, both stand-ins. Read raw, as the plugin's
    /// deliveries are a subset of these shapes.
    #[test]
    fn a_pull_request_a_call_opened_keeps_its_link_and_nothing_else() {
        let mut links = Vec::new();
        for (name, delivery) in fixtures_in(Path::new("opencode-pull-requests/1.18.34")) {
            let Ok(reported) = normalize(&delivery) else {
                continue;
            };
            let payload = reported.payload.to_string();
            for content in [
                "gh pr create",
                "Creating pull request",
                "Bump",
                "2989102733",
            ] {
                assert!(!payload.contains(content), "{name}: {content} in {payload}");
            }
            if let Some(url) = reported.payload["pullRequestUrl"].as_str() {
                assert_eq!(reported.kind, "tool.completed", "{name}");
                links.push((name, url.to_owned()));
            }
        }
        assert_eq!(
            links,
            [
                (
                    "22-tool.execute.after-bash".to_owned(),
                    "https://github.com/example/lib/pull/41".to_owned()
                ),
                (
                    "26-tool.execute.after-github_create_pull_request".to_owned(),
                    "https://github.com/example/lib/pull/42".to_owned()
                ),
            ]
        );
    }

    fn fixtures_2() -> Vec<(String, Value)> {
        fixtures_in(&Path::new("opencode").join(FIXTURE_VERSION_2))
    }

    /// OpenCode 2.2.6, recorded with `scripts/record-opencode.sh`: a write, two reads (one of a
    /// file that is not there), a shell command asked for and allowed.
    #[test]
    fn the_recorded_opencode_2_run_maps_to_the_events_a_reader_would_expect_and_nothing_else() {
        let reported: Vec<(String, Reported)> = fixtures_2()
            .into_iter()
            .filter_map(|(name, delivery)| normalize(&delivery).ok().map(|r| (name, r)))
            .collect();
        let kinds: Vec<(&str, &str)> = reported
            .iter()
            .map(|(name, r)| (name.as_str(), r.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("02-session.prompt", "prompt.submitted"),
                ("10-tool.execute.before-write", "tool.started"),
                ("12-tool.execute.after-write-completed", "tool.completed"),
                ("14-event-session.step.ended", "usage.reported"),
                ("18-tool.execute.before-read", "tool.started"),
                ("19-tool.execute.before-read", "tool.started"),
                ("24-tool.execute.before-shell", "tool.started"),
                ("26-event-permission.asked", "approval.requested"),
                ("27-tool.execute.after-read-error", "tool.failed"),
                ("29-event-permission.replied", "approval.resolved"),
                ("30-tool.execute.after-read-completed", "tool.completed"),
                ("35-tool.execute.after-shell-completed", "tool.completed"),
                ("37-event-session.step.ended", "usage.reported"),
                ("40-event-session.step.ended", "usage.reported"),
                ("41-event-session.execution.succeeded", "turn.completed"),
            ]
        );
        let by = |prefix: &str| {
            &reported
                .iter()
                .find(|(n, _)| n.starts_with(prefix))
                .unwrap()
                .1
        };
        let session = Some("ses_00000000000fixture01".to_owned());
        assert!(reported.iter().all(|(_, r)| r.native_session_id == session));

        let write = by("12-");
        assert_eq!(write.payload["tool"], "write");
        assert_eq!(write.payload["path"], "hello.txt");
        assert_eq!(write.payload["created"], true);
        assert_eq!(write.payload["wrote"], true, "what provenance counts");
        assert_eq!(by("10-").payload["toolUseId"], write.payload["toolUseId"]);
        let read = by("30-");
        assert_eq!(read.payload["path"], "hello.txt");
        assert!(read.payload.get("wrote").is_none());
        let missing = by("27-");
        assert_eq!(missing.payload["tool"], "read");
        assert_eq!(missing.payload["path"], "missing.txt");
        let shell = by("35-");
        assert_eq!(shell.payload["tool"], "shell");
        assert_eq!(shell.payload["exitCode"], 0);
        assert!(shell.payload.get("path").is_none());
        let asked = by("26-");
        assert_eq!(asked.payload["tool"], "shell");
        assert_eq!(asked.payload["toolUseId"], by("24-").payload["toolUseId"]);
        let replied = by("29-");
        assert_eq!(replied.payload["decision"], "once");
        assert_eq!(
            replied.payload["permissionId"],
            asked.payload["permissionId"]
        );
        let usage = by("14-");
        assert_eq!(usage.payload["finish"], "tool-calls");
        let tokens = &usage.payload;
        let sum: i64 = [
            "inputTokens",
            "outputTokens",
            "reasoningOutputTokens",
            "cachedInputTokens",
            "cacheWriteTokens",
        ]
        .iter()
        .map(|k| tokens[k].as_i64().unwrap())
        .sum();
        assert_eq!(tokens["totalTokens"], sum);
        assert!(by("02-").payload["chars"].as_u64().unwrap() > 200);

        // Nothing of what was asked, run, read or answered.
        for (name, r) in &reported {
            let text = r.payload.to_string();
            for content in [
                "Create a file",
                "Created file",
                "File not found",
                "\"ls\"",
                "resources",
                "hello\"",
            ] {
                assert!(!text.contains(content), "{name}: {content} in {text}");
            }
        }
        // What the recording left in that is not ours, refused by name.
        let catalogue = fixtures_2()
            .into_iter()
            .find(|(n, _)| n.ends_with("event-session.inbox.enqueued"))
            .unwrap()
            .1;
        assert!(normalize(&catalogue)
            .unwrap_err()
            .contains("session.inbox.enqueued"));
    }

    /// Recorded with `scripts/record-pr-creation.sh opencode` against 2.2.6. OpenCode 2 runs an
    /// MCP tool from inside its `execute` tool, and the inner call has hooks of its own; the
    /// link is on that one, and on the shell's `gh pr create`.
    #[test]
    fn a_pull_request_an_opencode_2_call_opened_keeps_its_link_and_nothing_else() {
        let mut links = Vec::new();
        let under = Path::new("opencode-pull-requests").join(FIXTURE_VERSION_2);
        for (name, delivery) in fixtures_in(&under) {
            let Ok(reported) = normalize(&delivery) else {
                continue;
            };
            let payload = reported.payload.to_string();
            for content in [
                "gh pr create",
                "Creating pull request",
                "Bump",
                "2989102733",
            ] {
                assert!(!payload.contains(content), "{name}: {content} in {payload}");
            }
            if let Some(url) = reported.payload["pullRequestUrl"].as_str() {
                assert_eq!(reported.kind, "tool.completed", "{name}");
                links.push((name, url.to_owned()));
            }
        }
        assert_eq!(
            links,
            [
                (
                    "19-tool.execute.after-shell-completed".to_owned(),
                    "https://github.com/example/lib/pull/41".to_owned()
                ),
                (
                    "29-tool.execute.after-github_create_pull_request-completed".to_owned(),
                    "https://github.com/example/lib/pull/42".to_owned()
                ),
            ]
        );
        // A `gh pr create` that failed keeps no link, though it printed one.
        let (_, mut failed) = fixtures_in(&under)
            .into_iter()
            .find(|(n, _)| n.starts_with("19-"))
            .unwrap();
        failed["payload"]["result"]["output"]["exit"] = json!(1);
        failed["payload"]["result"]["metadata"]["exit"] = json!(1);
        let reported = normalize(&failed).unwrap();
        assert_eq!(reported.payload["exitCode"], 1);
        assert!(reported.payload.get("pullRequestUrl").is_none());
    }

    /// Shapes the recording does not have: a session the TUI started (`run` starts its own
    /// before the plugin listens), a turn stopped or failed, a failed call's duration, which the
    /// plugin measures.
    #[test]
    fn opencode_2_sessions_turns_and_failures_the_recording_lacks() {
        let event = |kind: &str, data: Value| json!({ "api": 2, "hook": "event", "payload": { "type": kind, "data": data } });
        // As the 2.2.6 TUI sent it, on 2026-11-10.
        let created = normalize(&event(
            "session.created",
            json!({ "sessionID": "ses_1", "projectID": "p", "slug": "kind-comet", "agent": "build",
                    "model": { "id": "big-pickle", "providerID": "opencode" }, "version": "2.2.6" }),
        ))
        .unwrap();
        assert_eq!(created.kind, "session.started");
        assert_eq!(created.native_session_id.as_deref(), Some("ses_1"));
        assert_eq!(created.payload["version"], "2.2.6");
        assert!(!created.payload.to_string().contains("kind-comet"));

        let stopped = normalize(&event(
            "session.execution.interrupted",
            json!({ "sessionID": "ses_1" }),
        ))
        .unwrap();
        assert_eq!(
            (stopped.kind, &stopped.payload["interrupted"]),
            ("turn.completed", &json!(true))
        );
        let failed = normalize(&event(
            "session.execution.failed",
            json!({ "sessionID": "ses_1", "error": { "type": "provider.auth", "message": "secret" } }),
        ))
        .unwrap();
        assert_eq!(failed.kind, "turn.failed");
        assert_eq!(failed.payload["errorType"], "provider.auth");
        assert!(!failed.payload.to_string().contains("secret"));

        let tool_failed = normalize(
            &json!({ "api": 2, "hook": "tool.execute.after", "directory": "/w",
            "payload": { "tool": "edit", "sessionID": "ses_1", "id": "c1", "status": "error",
                         "input": { "path": "/w/src/a.rs" }, "durationMs": 42 } }),
        )
        .unwrap();
        assert_eq!(tool_failed.kind, "tool.failed");
        assert_eq!(tool_failed.payload["durationMs"], 42);
        assert_eq!(tool_failed.payload["path"], "src/a.rs");
        assert!(
            tool_failed.payload.get("wrote").is_none(),
            "a failed edit wrote nothing"
        );
        let outside = normalize(&json!({ "api": 2, "hook": "tool.execute.before", "directory": "/w",
            "payload": { "tool": "read", "sessionID": "ses_1", "id": "c2", "input": { "path": "/etc/passwd" } } }))
        .unwrap();
        assert!(outside.payload.get("path").is_none());
        assert_eq!(outside.payload["pathOutsideWorkspace"], true);

        // An OpenCode 1 delivery is not read as 2's, nor 2's as 1's.
        assert!(normalize(&json!({ "api": 2, "hook": "chat.message", "payload": {} })).is_err());
        assert!(normalize(&json!({ "hook": "session.prompt", "payload": {} })).is_err());
    }

    /// `gh pr create` on a branch that has a pull request already fails, and prints that one's
    /// link: not one the call opened.
    #[test]
    fn a_failed_create_keeps_no_link() {
        let mut delivery = fixtures_in(Path::new("opencode-pull-requests/1.18.34"))
            .into_iter()
            .find(|(name, _)| name == "22-tool.execute.after-bash")
            .unwrap()
            .1;
        delivery["payload"]["output"]["metadata"]["exit"] = json!(1);
        let reported = normalize(&delivery).unwrap();
        assert!(
            reported.payload.get("pullRequestUrl").is_none(),
            "{}",
            reported.payload
        );
    }

    #[test]
    fn the_recorded_run_maps_to_the_events_a_reader_would_expect_and_nothing_else() {
        let mapped: Vec<(String, &'static str)> = fixtures()
            .iter()
            .filter_map(|(name, delivery)| normalize(delivery).ok().map(|r| (name.clone(), r.kind)))
            .collect();
        let kinds: Vec<&str> = mapped.iter().map(|(_, k)| *k).collect();
        assert_eq!(
            kinds,
            [
                "session.started",
                "prompt.submitted",
                "tool.started", // write
                "file.reported_write",
                "tool.completed", // write
                "usage.reported",
                "tool.started",   // read
                "tool.completed", // read
                "usage.reported",
                "tool.started", // read, of a file that is not there
                "tool.failed",
                "usage.reported",
                "tool.started",   // bash
                "tool.completed", // bash
                "usage.reported",
                "usage.reported",
                "turn.completed",
            ],
            "{mapped:?}"
        );
    }

    #[test]
    fn what_is_kept_is_metadata_and_what_is_dropped_is_content() {
        let all = fixtures();
        let by_name = |prefix: &str| {
            all.iter()
                .find(|(n, _)| n.starts_with(prefix))
                .map(|(_, v)| v)
                .unwrap_or_else(|| panic!("fixture {prefix}"))
        };
        let write_done = normalize(by_name("18-tool.execute.after-write")).unwrap();
        assert_eq!(write_done.kind, "tool.completed");
        assert_eq!(write_done.payload["tool"], "write");
        assert_eq!(write_done.payload["created"], true);
        assert_eq!(
            write_done.payload["path"], "hello.txt",
            "measured against the launch directory the plugin sends"
        );
        let edited = normalize(by_name("17-event-file.edited")).unwrap();
        assert_eq!(edited.kind, "file.reported_write");
        assert_eq!(edited.payload["path"], "hello.txt");

        let failed = normalize(by_name("47-event-message.part.updated-tool-read-error")).unwrap();
        assert_eq!(failed.kind, "tool.failed");
        assert_eq!(failed.payload["tool"], "read");
        assert_eq!(failed.payload["durationMs"], 5);
        assert!(!failed.payload.to_string().contains("File not found"));

        let bash = normalize(by_name("63-tool.execute.after-bash")).unwrap();
        assert_eq!(bash.payload["exitCode"], 0);
        assert!(!bash.payload.to_string().contains("ls"), "{}", bash.payload);

        let prompt = normalize(by_name("04-chat.message")).unwrap();
        assert_eq!(prompt.payload["chars"], 247);
        assert_eq!(prompt.payload["model"], "opencode/big-pickle");

        let usage = normalize(by_name("76-event-message.updated-assistant-completed")).unwrap();
        assert_eq!(usage.payload["totalTokens"], 10814);
        assert_eq!(usage.payload["cachedInputTokens"], 10772);
        assert_eq!(usage.payload["model"], "opencode/big-pickle");

        let idle = normalize(by_name("79-event-session.idle")).unwrap();
        assert_eq!(idle.kind, "turn.completed");
        assert_eq!(
            idle.native_session_id.as_deref(),
            Some("ses_00000000000fixture01")
        );

        for (name, delivery) in &all {
            let Ok(reported) = normalize(delivery) else {
                continue;
            };
            let text = reported.payload.to_string();
            for content in [
                "Create a file",
                "hello\n",
                "Wrote file",
                "File not found",
                "/tmp/yardsort-fixture",
                "\"ls\"",
                "big-pickle\"}",
            ] {
                assert!(!text.contains(content), "{name} leaked {content:?}: {text}");
            }
        }
    }

    #[test]
    fn paths_are_relative_to_the_tools_working_directory() {
        let after = json!({
            "hook": "tool.execute.after",
            "payload": {
                "input": { "tool": "edit", "sessionID": "s", "callID": "c",
                           "args": { "filePath": "/w/s/src/a.rs", "workdir": "/w/s" } },
                "output": { "metadata": {} }
            }
        });
        let reported = normalize(&after).unwrap();
        assert_eq!(reported.payload["path"], "src/a.rs");
        let edited = json!({
            "hook": "event", "directory": "/w/s",
            "payload": { "type": "file.edited", "properties": { "file": "/w/s/README.md" } }
        });
        assert_eq!(normalize(&edited).unwrap().payload["path"], "README.md");
        let elsewhere = json!({
            "hook": "event", "directory": "/w/s",
            "payload": { "type": "file.edited", "properties": { "file": "/etc/hosts" } }
        });
        let reported = normalize(&elsewhere).unwrap();
        assert!(reported.payload.get("path").is_none());
        assert_eq!(reported.payload["pathOutsideWorkspace"], true);
    }

    #[test]
    fn calls_this_build_does_not_record_are_refused() {
        assert!(normalize(&json!({ "hook": "shell.env", "payload": {} })).is_err());
        assert!(
            normalize(&json!({ "hook": "event", "payload": { "type": "session.status" } }))
                .is_err()
        );
        assert!(normalize(&json!({ "hook": "event", "payload": {
            "type": "message.updated", "properties": { "info": { "role": "user" } } } }))
        .is_err());
        assert!(normalize(&json!({ "nope": 1 })).is_err());
        let asked = normalize(&json!({ "hook": "permission.ask", "payload": {
            "input": { "sessionID": "s", "permission": "bash", "callID": "c" }, "output": {} } }))
        .unwrap();
        assert_eq!(asked.kind, "approval.requested");
        assert_eq!(asked.payload["permission"], "bash");
        assert_eq!(asked.payload["tool"], "bash", "what the timeline names");
        // The bus event, which is the one this version delivers: the call sits under `tool`.
        let asked = normalize(&json!({ "hook": "event", "payload": { "type": "permission.asked",
            "properties": { "id": "per_1", "sessionID": "s", "permission": "bash",
                            "patterns": ["rm -rf *"], "tool": { "messageID": "m", "callID": "c" } } } }))
        .unwrap();
        assert_eq!(asked.payload["toolUseId"], "c");
        assert_eq!(asked.payload["tool"], "bash");
        assert!(!asked.payload.to_string().contains("rm -rf"));
        let replied = normalize(
            &json!({ "hook": "event", "payload": { "type": "permission.replied",
            "properties": { "sessionID": "s", "permissionID": "per_1", "response": "once" } } }),
        )
        .unwrap();
        assert_eq!(replied.kind, "approval.resolved");
        assert_eq!(replied.payload["decision"], "once");
    }

    #[test]
    fn the_plugin_names_this_executable_and_is_added_to_the_launch_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let mut args = vec![];
        let (name, content) = arm(&mut args, dir.path(), None, Some(1)).unwrap();
        assert_eq!(name, CONFIG_ENV);
        assert!(
            args.is_empty(),
            "OpenCode 1 is given nothing on its command line"
        );
        // OpenCode 1 takes the file, 2 the folder: the key of the one it is, and both when
        // that is not known, as each ignores the other's.
        let config: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(config["plugin"][0], file_url(&plugin_path(dir.path())));
        assert!(config.get("plugins").is_none(), "{config}");
        let (_, content) = arm(&mut vec![], dir.path(), None, Some(2)).unwrap();
        let config: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(config["plugins"][0], file_url(&plugin_dir(dir.path())));
        assert!(config.get("plugin").is_none(), "{config}");
        // A list only the other version reads is not looked at.
        assert!(arm(
            &mut vec![],
            dir.path(),
            Some(r#"{"plugins": "x"}"#),
            Some(1)
        )
        .is_ok());
        assert!(arm(&mut vec![], dir.path(), Some(r#"{"plugin": "x"}"#), Some(2)).is_ok());
        let (_, content) = arm(&mut args, dir.path(), None, None).unwrap();
        let config: Value = serde_json::from_str(&content).unwrap();
        let url = config["plugin"][0].as_str().unwrap();
        assert_eq!(url, file_url(&plugin_path(dir.path())));
        assert!(url.starts_with("file:///"));
        assert_eq!(config["plugins"][0], file_url(&plugin_dir(dir.path())));
        assert_eq!(
            plugin_path(dir.path()).parent(),
            Some(plugin_dir(dir.path()).as_path())
        );
        let source = std::fs::read_to_string(plugin_path(dir.path())).unwrap();
        let exe = super::super::claude::hook_executable().unwrap();
        assert!(source.contains(&format!(
            "const EXE = \"{}\";",
            serde_json::to_string(&exe.to_string_lossy())
                .unwrap()
                .trim_matches('"')
        )));
        assert!(source.contains("--yardsort-hook"));
        assert!(!source.contains("__YARDSORT_"), "every placeholder filled");
        // OpenCode 1 calls every export as a plugin, OpenCode 2 reads the default: one export,
        // the default, with an id and both entry points.
        assert_eq!(source.matches("\nexport ").count(), 1, "{source}");
        assert!(source
            .contains("\nexport default {\n  id: \"yardsort.activity\",\n  server,\n  setup,"));

        // The user's own inline configuration is kept, and their plugins with it.
        let theirs = r#"{"model":"anthropic/claude","plugin":["file:///home/u/mine.js"],"plugins":[{"package":"acme"}]}"#;
        let (_, merged) = arm(&mut args, dir.path(), Some(theirs), None).unwrap();
        let config: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(config["model"], "anthropic/claude");
        assert_eq!(config["plugin"][0], "file:///home/u/mine.js");
        assert_eq!(config["plugin"].as_array().unwrap().len(), 2);
        assert_eq!(config["plugins"][0]["package"], "acme");
        assert_eq!(config["plugins"].as_array().unwrap().len(), 2);

        // Twice is once.
        let (_, again) = arm(&mut args, dir.path(), Some(&merged), None).unwrap();
        let again: Value = serde_json::from_str(&again).unwrap();
        assert_eq!(again["plugin"].as_array().unwrap().len(), 2);
        assert_eq!(again["plugins"].as_array().unwrap().len(), 2);

        // `--pure` means no external plugins; theirs that is not JSON is not ours to replace.
        assert!(arm(&mut vec!["--pure".to_owned()], dir.path(), None, None).is_err());
        assert!(arm(&mut vec![], dir.path(), Some("not json"), None).is_err());
        assert!(arm(&mut vec![], dir.path(), Some(r#"{"plugin": "one"}"#), None).is_err());
        assert!(arm(&mut vec![], dir.path(), Some(r#"{"plugins": "one"}"#), None).is_err());
    }

    /// OpenCode 2's sessions run in a background service every launch shares, which keeps the
    /// environment of the launch that started it: found on 2026-11-10 against 2.2.6, a second
    /// launch was served by the first's plugin, with the first's run id. So an OpenCode 2 launch
    /// gets a server of its own.
    #[test]
    fn an_opencode_2_launch_is_given_a_server_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let mut args = vec!["run".to_owned(), "hello".to_owned()];
        arm(&mut args, dir.path(), None, Some(2)).unwrap();
        assert_eq!(
            args,
            ["--standalone", "run", "hello"],
            "first, before a subcommand"
        );
        arm(&mut args, dir.path(), None, Some(2)).unwrap();
        assert_eq!(args.iter().filter(|a| *a == STANDALONE).count(), 1);

        // Their own server is where their sessions run, and where Yardsort's plugin is not.
        for server in [
            &["--server", "http://127.0.0.1:4096"][..],
            &["--server=http://x"],
        ] {
            let mut args: Vec<String> = server.iter().map(|a| a.to_string()).collect();
            assert!(
                arm(&mut args, dir.path(), None, Some(2)).is_err(),
                "{args:?}"
            );
        }
        // Not OpenCode 2, or not known to be: nothing added.
        for major in [Some(1), None] {
            let mut args = vec!["--continue".to_owned()];
            arm(&mut args, dir.path(), None, major).unwrap();
            assert_eq!(args, ["--continue"], "{major:?}");
        }
    }

    #[test]
    fn the_version_is_read_from_what_opencode_prints_and_nothing_else() {
        assert_eq!(parse_major("opencode v2.2.6\n"), Some(2));
        assert_eq!(parse_major("1.18.34\n"), Some(1));
        assert_eq!(parse_major("\n  2.0.32  \n"), Some(2));
        for other in [
            "",
            "GNU bash, version 5.2.37(1)-release (x86_64-pc-linux-gnu)",
            "Microsoft Windows [Version 10.0.26100.4061]",
            "opencode",
            "v2",
            "2.x",
            "something 2.2.6",
        ] {
            assert_eq!(parse_major(other), None, "{other}");
        }
    }

    /// The real thing, on a stand-in: the program is asked, and what it prints decides.
    #[cfg(unix)]
    #[test]
    fn the_program_is_asked_which_opencode_it_is() {
        use crate::env::{EnvSource, ShellEnv};
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let program = |name: &str, printed: &str| {
            crate::program::testing::executable(
                bin.join(name),
                format!("#!/bin/sh\n[ \"$1\" = --version ] && printf '{printed}\\n'\n"),
            )
        };
        program("opencode-two", "opencode v2.2.6");
        program("opencode-one", "1.18.34");
        program("opencode-hello", "hello");
        // A wrapper answers for itself, and is not asked.
        program("npx", "10.9.2");
        crate::program::testing::executable(bin.join("opencode-failing"), "#!/bin/sh\nexit 3\n");
        let env = ShellEnv {
            vars: [("PATH".to_owned(), bin.to_string_lossy().into_owned())].into(),
            source: EnvSource::Process,
            warning: None,
        };
        assert_eq!(major_version(&env, "opencode-two", dir.path()), Some(2));
        assert_eq!(major_version(&env, "opencode-one", dir.path()), Some(1));
        assert_eq!(major_version(&env, "opencode-hello", dir.path()), None);
        assert_eq!(major_version(&env, "npx", dir.path()), None);
        assert_eq!(major_version(&env, "opencode-failing", dir.path()), None);
        assert_eq!(major_version(&env, "missing", dir.path()), None);
    }

    #[test]
    fn file_urls_are_made_for_both_kinds_of_path_and_encode_what_a_url_would_misread() {
        assert_eq!(file_url(Path::new("/a/b c.js")), "file:///a/b%20c.js");
        assert_eq!(
            file_url(Path::new(r"C:\Users\x\p.js")),
            "file:///C:/Users/x/p.js"
        );
        // Seen in review: `#` starts a fragment and `%` an escape, and a module under either
        // was not found.
        assert_eq!(
            file_url(Path::new("/home/u/profile#one/hooks/opencode.js")),
            "file:///home/u/profile%23one/hooks/opencode.js"
        );
        assert_eq!(
            file_url(Path::new("/home/u/profile%20one/x.js")),
            "file:///home/u/profile%2520one/x.js"
        );
        assert_eq!(
            file_url(Path::new("/q?a=1/é.js")),
            "file:///q%3Fa%3D1/%C3%A9.js"
        );
    }
}
