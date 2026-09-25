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
/// The environment variable OpenCode reads inline configuration from.
pub const CONFIG_ENV: &str = "OPENCODE_CONFIG_CONTENT";
/// The plugin, with the executable and inbox to substitute.
pub const PLUGIN_TEMPLATE: &str = include_str!("opencode-plugin.js");

/// Where the plugin file lives: written before each launch, so it names the current executable.
pub fn plugin_path(data_dir: &Path) -> PathBuf {
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
/// with our plugin added to its `plugin` list. Refuses configuration it cannot read: the
/// user's settings are not to be replaced by ours.
pub fn config_content(existing: Option<&str>, plugin_url: &str) -> Result<String, String> {
    let mut config = match existing.map(str::trim).filter(|s| !s.is_empty()) {
        Some(text) => serde_json::from_str::<Value>(text)
            .map_err(|error| format!("{CONFIG_ENV} is not JSON: {error}"))?,
        None => json!({}),
    };
    let object = config
        .as_object_mut()
        .ok_or_else(|| format!("{CONFIG_ENV} is not a JSON object"))?;
    let plugins = object
        .entry("plugin")
        .or_insert_with(|| Value::Array(vec![]));
    let list = plugins
        .as_array_mut()
        .ok_or_else(|| format!("{CONFIG_ENV} has a `plugin` that is not a list"))?;
    if !list.iter().any(|p| p.as_str() == Some(plugin_url)) {
        list.push(Value::String(plugin_url.to_owned()));
    }
    Ok(config.to_string())
}

/// Give an OpenCode launch its plugin: write the file and return the variable to set. Refuses
/// when the arguments carry `--pure` (no external plugins, the user said), or when the
/// environment's own configuration cannot be added to.
pub fn arm(
    args: &[String],
    data_dir: &Path,
    existing_config: Option<&str>,
) -> Result<(String, String), String> {
    if args.iter().any(|arg| arg == "--pure") {
        return Err("the harness's arguments carry `--pure`".to_owned());
    }
    let hook = super::claude::hook_executable()
        .map_err(|error| format!("no executable to run: {error}"))?;
    let path = plugin_path(data_dir);
    let dir = path
        .parent()
        .ok_or_else(|| "plugin path has no directory".to_owned())?;
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let source = plugin_source(&hook, &super::inbox_dir(data_dir));
    let temporary = path.with_extension(format!("js.{}.tmp", uuid::Uuid::new_v4()));
    let write = || -> io::Result<()> {
        std::fs::write(&temporary, source.as_bytes())?;
        std::fs::rename(&temporary, &path)
    };
    write().map_err(|error| format!("{}: {error}", path.display()))?;
    let content = config_content(existing_config, &file_url(&path))?;
    Ok((CONFIG_ENV.to_owned(), content))
}

/// One plugin delivery, reduced to what Yardsort records.
#[derive(Debug, Clone, PartialEq)]
pub struct Reported {
    pub kind: &'static str,
    pub native_session_id: Option<String>,
    pub payload: Value,
}

/// Turn what the plugin sent — `{hook, payload}` — into an event. Reads the raw hook shapes,
/// of which the plugin's are a subset. `Err` names a call this build has no mapping for.
pub fn normalize(delivery: &Value) -> Result<Reported, String> {
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

fn n64(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> Vec<(String, Value)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("opencode")
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
        let (name, content) = arm(&[], dir.path(), None).unwrap();
        assert_eq!(name, CONFIG_ENV);
        let config: Value = serde_json::from_str(&content).unwrap();
        let url = config["plugin"][0].as_str().unwrap();
        assert_eq!(url, file_url(&plugin_path(dir.path())));
        assert!(url.starts_with("file:///"));
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
        // OpenCode calls every export as a plugin: there must be exactly one.
        assert_eq!(source.matches("\nexport ").count(), 1, "{source}");

        // The user's own inline configuration is kept, and their plugins with it.
        let theirs = r#"{"model":"anthropic/claude","plugin":["file:///home/u/mine.js"]}"#;
        let (_, merged) = arm(&[], dir.path(), Some(theirs)).unwrap();
        let config: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(config["model"], "anthropic/claude");
        assert_eq!(config["plugin"][0], "file:///home/u/mine.js");
        assert_eq!(config["plugin"].as_array().unwrap().len(), 2);

        // Twice is once.
        let (_, again) = arm(&[], dir.path(), Some(&merged)).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&again).unwrap()["plugin"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        // `--pure` means no external plugins; theirs that is not JSON is not ours to replace.
        assert!(arm(&["--pure".to_owned()], dir.path(), None).is_err());
        assert!(arm(&[], dir.path(), Some("not json")).is_err());
        assert!(arm(&[], dir.path(), Some(r#"{"plugin": "one"}"#)).is_err());
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
