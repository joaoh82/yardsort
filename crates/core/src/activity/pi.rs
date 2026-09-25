//! The pi family — pi, and OMP (Oh My Pi), its fork — as a source of activity: an extension
//! given for one launch.
//!
//! Both load extensions the same way: a module exporting a factory that subscribes with
//! `pi.on(...)`, given on the command line with `-e <file>`, loaded beside whatever the user has
//! configured, with no trust prompt, in the agent's own process, where the launch environment is
//! visible. The extension ([`EXTENSION_TEMPLATE`]) keeps a whitelist of fields from a few
//! notification events and hands each to this executable in hook mode on stdin. Nothing of the
//! user's is edited; a launch without the switch has no extension of ours.
//!
//! One adapter, two harness ids: the producer is whichever launched. The shapes are those of the
//! fixtures under `crates/core/fixtures/omp/` (a full turn) and `crates/core/fixtures/pi/` (what
//! pi delivered before its missing provider stopped it, which is the same channel and the same
//! session id Yardsort chose). See `docs/design/15-agent-events-stage-2-pi-omp.md`.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// The harnesses this adapter serves.
pub const PI: &str = "pi";
pub const OMP: &str = "omp";
pub const METHOD: &str = "extension";
pub const FIDELITY: &str = "reported";
/// The versions the fixtures come from.
pub const PI_FIXTURE_VERSION: &str = "0.87.1";
pub const OMP_FIXTURE_VERSION: &str = "18.2.11";
/// The extension, with the executable, inbox and harness to substitute.
pub const EXTENSION_TEMPLATE: &str = include_str!("pi-extension.ts");

/// Whether `harness` is one of ours.
pub fn serves(harness: &str) -> bool {
    harness == PI || harness == OMP
}

/// Where the extension file for a harness lives: written before each launch.
pub fn extension_path(data_dir: &Path, harness: &str) -> PathBuf {
    data_dir
        .join("activity")
        .join("hooks")
        .join(format!("{harness}.ts"))
}

/// The extension's source for this profile and harness.
pub fn extension_source(hook: &Path, inbox_dir: &Path, harness: &str) -> String {
    let quoted = |text: &str| {
        serde_json::to_string(text)
            .unwrap_or_default()
            .trim_matches('"')
            .to_owned()
    };
    EXTENSION_TEMPLATE
        .replace("__YARDSORT_EXE__", &quoted(&hook.to_string_lossy()))
        .replace("__YARDSORT_INBOX__", &quoted(&inbox_dir.to_string_lossy()))
        .replace("__YARDSORT_HARNESS__", &quoted(harness))
}

/// Give a pi-family launch its extension: write the file and add `-e <file>` to `args`, before
/// any `--` (what follows one is the prompt). Refuses when the arguments carry
/// `--trusted-extension`, which the agent will not combine with `-e`.
pub fn arm(args: &mut Vec<String>, data_dir: &Path, harness: &str) -> Result<PathBuf, String> {
    if !serves(harness) {
        return Err(format!("no pi-family adapter for `{harness}`"));
    }
    if args
        .iter()
        .any(|arg| arg.starts_with("--trusted-extension"))
    {
        return Err(
            "the harness's arguments carry `--trusted-extension`, which excludes `-e`".to_owned(),
        );
    }
    let hook = super::claude::hook_executable()
        .map_err(|error| format!("no executable to run: {error}"))?;
    let path = extension_path(data_dir, harness);
    let dir = path
        .parent()
        .ok_or_else(|| "extension path has no directory".to_owned())?;
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let source = extension_source(&hook, &super::inbox_dir(data_dir), harness);
    let temporary = path.with_extension(format!("ts.{}.tmp", uuid::Uuid::new_v4()));
    let write = || -> io::Result<()> {
        std::fs::write(&temporary, source.as_bytes())?;
        std::fs::rename(&temporary, &path)
    };
    write().map_err(|error| format!("{}: {error}", path.display()))?;
    let at = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());
    args.insert(at, path.to_string_lossy().into_owned());
    args.insert(at, "-e".to_owned());
    Ok(path)
}

/// One extension delivery, reduced to what Yardsort records.
#[derive(Debug, Clone, PartialEq)]
pub struct Reported {
    pub kind: &'static str,
    pub native_session_id: Option<String>,
    pub payload: Value,
}

/// Turn what the extension sent — `{event, payload, session}` — into an event. Reads the raw
/// event shapes, of which the extension's are a subset.
pub fn normalize(delivery: &Value) -> Result<Reported, String> {
    let event = delivery
        .get("event")
        .and_then(Value::as_str)
        .ok_or_else(|| "no event name".to_owned())?;
    let payload = &delivery["payload"];
    let session = &delivery["session"];
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
    let session_id = text(session, "id");
    let cwd = text(session, "cwd");
    let model = |m: &Value| match (text(m, "provider"), text(m, "id")) {
        (Some(p), Some(id)) => Some(format!("{p}/{id}")),
        (_, id) => id,
    };
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
    let tool = |extra: Value| {
        let mut base = json!({
            "tool": text(payload, "toolName"),
            "toolUseId": text(payload, "toolCallId"),
            "sessionId": session_id,
        });
        if let Some(extra) = extra.as_object() {
            for (k, v) in extra {
                base[k] = v.clone();
            }
        }
        base
    };

    let (kind, event_payload) = match event {
        "session_start" => (
            "session.started",
            json!({
                "sessionId": session_id,
                "reason": text(payload, "reason"),
                "model": model(&session["model"]),
            }),
        ),
        "session_shutdown" => (
            "session.ended",
            json!({ "sessionId": session_id, "reason": text(payload, "reason") }),
        ),
        "input" => (
            "prompt.submitted",
            json!({
                "sessionId": session_id,
                "chars": payload.get("chars").and_then(Value::as_u64)
                    .or_else(|| text(payload, "text").map(|t| t.chars().count() as u64)),
                "source": text(payload, "source"),
            }),
        ),
        "turn_start" => (
            "turn.started",
            json!({
                "sessionId": session_id,
                "turnNumber": payload.get("turnIndex"),
                "model": model(&session["model"]),
            }),
        ),
        "turn_end" => {
            let message = &payload["message"];
            let usage = &message["usage"];
            let n = |key: &str| usage.get(key).and_then(Value::as_i64);
            (
                "turn.completed",
                json!({
                    "sessionId": session_id,
                    "turnNumber": payload.get("turnIndex"),
                    "model": match (text(message, "provider"), text(message, "model")) {
                        (Some(p), Some(m)) => Some(format!("{p}/{m}")),
                        (_, m) => m,
                    },
                    "stopReason": text(message, "stopReason"),
                    "durationMs": message.get("duration").and_then(Value::as_f64).map(|d| d as i64),
                    "inputTokens": n("input"),
                    "outputTokens": n("output"),
                    "cachedInputTokens": n("cacheRead"),
                    "cacheWriteTokens": n("cacheWrite"),
                    "reasoningOutputTokens": n("reasoningTokens").or_else(|| n("reasoning")),
                    "totalTokens": n("totalTokens"),
                    "cost": message["cost"].get("total").and_then(Value::as_f64)
                        .or_else(|| usage["cost"].get("total").and_then(Value::as_f64)),
                }),
            )
        }
        "tool_execution_start" => (
            "tool.started",
            with_path(tool(json!({})), text(&payload["args"], "path").as_deref()),
        ),
        "tool_execution_end" => {
            let details = &payload["result"]["details"];
            let path =
                text(details, "resolvedPath").or_else(|| text(&details["meta"]["source"], "value"));
            let failed = payload.get("isError").and_then(Value::as_bool) == Some(true);
            (
                if failed {
                    "tool.failed"
                } else {
                    "tool.completed"
                },
                with_path(
                    tool(json!({
                        "durationMs": details.get("wallTimeMs").and_then(Value::as_f64).map(|ms| ms as i64),
                    })),
                    path.as_deref(),
                ),
            )
        }
        "tool_approval_requested" => (
            "approval.requested",
            tool(json!({ "approvalMode": text(payload, "approvalMode") })),
        ),
        "tool_approval_resolved" => (
            "approval.resolved",
            tool(json!({
                "decision": match payload.get("approved").and_then(Value::as_bool) {
                    Some(true) => "allow",
                    Some(false) => "deny",
                    None => "resolved",
                },
            })),
        ),
        "model_select" => (
            "agent.model_switched",
            json!({
                "sessionId": session_id,
                "model": model(&payload["model"]),
                "previousModel": model(&payload["previousModel"]),
                "source": text(payload, "source"),
            }),
        ),
        other => return Err(format!("no mapping for extension event {other}")),
    };
    Ok(Reported {
        kind,
        native_session_id: session_id,
        payload: event_payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures(harness: &str, version: &str) -> Vec<(String, Value)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(harness)
            .join(version);
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
    fn omps_recorded_turn_maps_to_the_events_a_reader_would_expect_and_nothing_else() {
        let all = fixtures(OMP, OMP_FIXTURE_VERSION);
        let mapped: Vec<&str> = all
            .iter()
            .filter_map(|(_, d)| normalize(d).ok().map(|r| r.kind))
            .collect();
        assert_eq!(
            mapped,
            [
                "session.started",
                "turn.started",
                "tool.started",   // write
                "tool.completed", // write
                "turn.completed",
                "turn.started",
                "tool.started",   // read
                "tool.started",   // read, missing
                "tool.started",   // bash
                "tool.completed", // read
                "tool.failed",    // read, missing
                "tool.completed", // bash
                "turn.completed",
                "turn.started",
                "turn.completed",
                "session.ended",
            ],
            "messages, tool_call/tool_result, agent and session_stop are content or duplicates"
        );
        let by = |prefix: &str| {
            all.iter()
                .find(|(n, _)| n.starts_with(prefix))
                .map(|(_, v)| v)
                .unwrap()
        };
        let started = normalize(by("01-session_start")).unwrap();
        assert_eq!(started.payload["model"], "openrouter/z-ai/glm-5.3-flash");
        assert_eq!(
            started.native_session_id.as_deref(),
            Some("22222222-2222-4222-8222-000000000001")
        );
        let write = normalize(by("10-tool_execution_start-write")).unwrap();
        assert_eq!(write.payload["path"], "hello.txt");
        assert!(!write.payload.to_string().contains("content"));
        let write_done = normalize(by("12-tool_execution_end-write")).unwrap();
        assert_eq!(write_done.payload["path"], "hello.txt", "from resolvedPath");
        let read_done = normalize(by("26-tool_execution_end-read")).unwrap();
        assert_eq!(read_done.payload["path"], "hello.txt", "from meta.source");
        let bash = normalize(by("34-tool_execution_end-bash")).unwrap();
        assert_eq!(bash.payload["durationMs"], 18);
        assert!(bash.payload.get("path").is_none());
        let turn = normalize(by("15-turn_end")).unwrap();
        assert_eq!(turn.payload["totalTokens"], 19963);
        assert_eq!(turn.payload["cachedInputTokens"], 13940);
        assert_eq!(turn.payload["model"], "openrouter/z-ai/glm-5.3-flash");
        assert_eq!(turn.payload["durationMs"], 11481);
        assert!(turn.payload["cost"].as_f64().unwrap() > 0.0);
        let ended = normalize(by("44-session_shutdown")).unwrap();
        assert_eq!(ended.kind, "session.ended");

        for (name, d) in &all {
            let Ok(reported) = normalize(d) else { continue };
            let text = reported.payload.to_string();
            for content in [
                "Create a file",
                "\"hello\"",
                "Successfully wrote",
                "Wall time",
                "/tmp/yardsort-fixture",
                "thinking",
                "\"ls\"",
            ] {
                assert!(!text.contains(content), "{name} leaked {content:?}: {text}");
            }
        }
    }

    #[test]
    fn pis_partial_recording_is_the_same_channel_under_the_id_yardsort_chose() {
        let all = fixtures(PI, PI_FIXTURE_VERSION);
        let mapped: Vec<(&str, Option<String>)> = all
            .iter()
            .filter_map(|(_, d)| normalize(d).ok().map(|r| (r.kind, r.native_session_id)))
            .collect();
        assert_eq!(mapped.len(), 3);
        assert_eq!(mapped[0].0, "session.started");
        assert_eq!(mapped[1].0, "prompt.submitted");
        assert_eq!(mapped[2].0, "session.ended");
        for (_, id) in &mapped {
            assert_eq!(id.as_deref(), Some("11111111-1111-4111-8111-111111111111"));
        }
        let (_, input) = all.iter().find(|(n, _)| n.starts_with("02-input")).unwrap();
        let prompt = normalize(input).unwrap();
        assert_eq!(prompt.payload["chars"], 184);
        assert!(!prompt.payload.to_string().contains("Create a file"));
    }

    #[test]
    fn approvals_and_model_switches_map_from_the_documented_fields() {
        let session = json!({ "id": "s", "cwd": "/w" });
        let asked = normalize(&json!({
            "event": "tool_approval_requested", "session": session,
            "payload": { "type": "tool_approval_requested", "toolCallId": "c", "toolName": "bash", "approvalMode": "write" }
        }))
        .unwrap();
        assert_eq!(asked.kind, "approval.requested");
        assert_eq!(asked.payload["tool"], "bash");
        let refused = normalize(&json!({
            "event": "tool_approval_resolved", "session": session,
            "payload": { "type": "tool_approval_resolved", "toolCallId": "c", "toolName": "bash", "approved": false }
        }))
        .unwrap();
        assert_eq!(refused.payload["decision"], "deny");
        let switched = normalize(&json!({
            "event": "model_select", "session": session,
            "payload": { "type": "model_select", "model": { "id": "opus", "provider": "anthropic" },
                         "previousModel": { "id": "sonnet", "provider": "anthropic" }, "source": "cycle" }
        }))
        .unwrap();
        assert_eq!(switched.kind, "agent.model_switched");
        assert_eq!(switched.payload["model"], "anthropic/opus");
        assert!(normalize(&json!({ "event": "message_end", "payload": {} })).is_err());
        assert!(normalize(&json!({ "payload": {} })).is_err());
    }

    #[test]
    fn arming_writes_the_extension_and_names_it_before_the_prompt_for_either_harness() {
        let dir = tempfile::tempdir().unwrap();
        for harness in [PI, OMP] {
            let mut args = vec!["--".to_owned(), " Reply with ok.".to_owned()];
            let path = arm(&mut args, dir.path(), harness).unwrap();
            assert_eq!(path, extension_path(dir.path(), harness));
            assert_eq!(args[0], "-e");
            assert_eq!(args[1], path.to_string_lossy());
            assert_eq!(args[2..], ["--", " Reply with ok."]);
            let source = std::fs::read_to_string(&path).unwrap();
            assert!(source.contains(&format!("const HARNESS = \"{harness}\";")));
            assert!(!source.contains("__YARDSORT_"));
            assert!(source.contains("--yardsort-hook"));
        }
        let mut trusted = vec!["--trusted-extension=/x.ts".to_owned()];
        assert!(arm(&mut trusted, dir.path(), OMP).is_err());
        assert!(arm(&mut vec![], dir.path(), "grok").is_err());
    }
}
