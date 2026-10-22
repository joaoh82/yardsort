//! A recorded event, as a remote client may see it.
//!
//! An event's payload is JSON whose shape depends on its kind and its producer, and some of what
//! producers keep is for this host alone: Codex's home folder, an agent's own session id, the
//! PTY id, the text of a launch error (which can name a path). So nothing is copied across
//! whole. Each field of [`ActivityDetail`] is read from a named payload field and kept only if
//! its value passes the check for its class — a short code, a name, a count, a path inside the
//! workspace — and everything else is left behind.
//!
//! Only events of the `metadata` privacy class are read at all (see
//! `migrations/0009_agent_runs_and_events.sql`): a later content class, when one exists, needs
//! its own decision about leaving the machine.

use remote_proto::{ActivityDetail, ActivityEvent, EventId, Provenance, RunId, SessionRecordId};
use serde_json::{Map, Value};

use crate::activity::PRIVACY_METADATA;
use crate::store::EventRow;

/// The privacy class a remote reader is shown.
pub const SHOWN_CLASS: &str = PRIVACY_METADATA;

/// Longest code, such as a reason or a decision.
const MAX_CODE: usize = 64;
/// Longest name, such as a tool's or a model's.
const MAX_NAME: usize = 128;
/// Longest path inside a workspace.
const MAX_PATH: usize = 1024;

/// The event as a client sees it, or `None` when its own ids cannot be expressed in the
/// protocol, which only a damaged database does.
pub fn event(row: &EventRow) -> Option<ActivityEvent> {
    if row.privacy_class != SHOWN_CLASS {
        return None;
    }
    let payload = serde_json::from_str::<Value>(&row.payload)
        .ok()
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();
    Some(ActivityEvent {
        id: EventId::new(row.id.as_str()).ok()?,
        workspace: row.workspace_id.clone().try_into().ok()?,
        session: row
            .session_id
            .as_deref()
            .and_then(|id| SessionRecordId::new(id).ok()),
        run: row.run_id.as_deref().and_then(|id| RunId::new(id).ok()),
        occurred_at: row.occurred_at,
        kind: code(&row.kind)?.to_owned(),
        source: Provenance {
            producer: code(&row.producer)?.to_owned(),
            method: code(&row.method)?.to_owned(),
            fidelity: code(&row.fidelity)?.to_owned(),
        },
        detail: detail(&row.kind, &payload),
    })
}

fn detail(kind: &str, p: &Map<String, Value>) -> ActivityDetail {
    let codes = |field: &str| p.get(field).and_then(Value::as_str).and_then(code);
    let names = |field: &str| p.get(field).and_then(Value::as_str).and_then(name);
    let count = |field: &str| p.get(field).and_then(Value::as_u64);
    let (run_kind, change) = match kind {
        k if k.starts_with("process.") => (codes("kind"), None),
        "file.reported_write" => (None, codes("kind")),
        _ => (None, None),
    };
    ActivityDetail {
        harness: names("harnessId"),
        run_kind: run_kind.map(str::to_owned),
        program: names("program").filter(|n| !n.contains(['/', '\\'])),
        model: names("model"),
        continuation: codes("continuation").map(str::to_owned),
        launched_by: codes("launchedBy").map(str::to_owned),
        capture: codes("capture").map(str::to_owned),
        forked_from: (kind == "session.forked")
            .then(|| p.get("fromSessionId").and_then(Value::as_str))
            .flatten()
            .and_then(|id| SessionRecordId::new(id).ok()),
        exit_code: p.get("exitCode").and_then(Value::as_i64),
        success: p.get("success").and_then(Value::as_bool),
        signal: codes("signal").map(str::to_owned),
        via: codes("via").map(str::to_owned),
        reason: codes("reason").map(str::to_owned),
        tool: names("tool").or_else(|| names("permission")),
        subagent_type: names("subagentType"),
        path: p
            .get("path")
            .and_then(Value::as_str)
            .and_then(inside_workspace)
            .map(str::to_owned),
        path_outside_workspace: p.get("pathOutsideWorkspace").and_then(Value::as_bool),
        change: change.map(str::to_owned),
        status: codes("status").map(str::to_owned),
        duration_ms: count("durationMs"),
        chars: count("chars"),
        decision: codes("decision").map(str::to_owned),
        wait_ms: count("waitMs"),
        notification: (kind == "agent.notified")
            .then(|| codes("type"))
            .flatten()
            .map(str::to_owned),
        error_type: codes("errorType").map(str::to_owned),
        agent_type: names("agentType"),
        trigger: codes("trigger").map(str::to_owned),
        outcome: codes("outcome").map(str::to_owned),
        turn_number: count("turnNumber"),
        context_tokens: count("contextTokens"),
        context_window: count("contextWindow"),
        total_tokens: count("totalTokens"),
        output_tokens: count("outputTokens"),
        note: codes("detail").map(str::to_owned),
    }
}

/// A short machine code: `prompt_input_exit`, `SIGKILL`, `tool.completed`. Anything with a
/// space, a slash or punctuation beyond `_.:-` is not one — a sentence, a path, a command —
/// and is left out.
fn code(text: &str) -> Option<&str> {
    (!text.is_empty()
        && text.len() <= MAX_CODE
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b)))
    .then_some(text)
}

/// A name a person reads: a tool's (`mcp:server/tool`), a model's, a subagent's type. One line,
/// bounded, and not a path.
fn name(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()
        && text.chars().count() <= MAX_NAME
        && !text.chars().any(char::is_control)
        && !looks_absolute(text))
    .then(|| text.to_owned())
}

/// A path relative to the workspace, as the adapters record it, checked again here: an
/// absolute path, a drive, or a step out through `..` is not inside the workspace.
fn inside_workspace(path: &str) -> Option<&str> {
    (!path.is_empty()
        && path.len() <= MAX_PATH
        && !path.chars().any(char::is_control)
        && !looks_absolute(path)
        && !path.split(['/', '\\']).any(|part| part == ".."))
    .then_some(path)
}

/// Absolute on any platform: `/x`, `\x`, `~/x`, `C:…`, `\\server\share`.
fn looks_absolute(text: &str) -> bool {
    let bytes = text.as_bytes();
    matches!(bytes.first(), Some(b'/' | b'\\' | b'~'))
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_short_and_plain() {
        assert_eq!(code("prompt_input_exit"), Some("prompt_input_exit"));
        assert_eq!(code("SIGKILL"), Some("SIGKILL"));
        assert_eq!(code("`claude` was not found on PATH"), None);
        assert_eq!(code("/usr/bin/claude"), None);
        assert_eq!(code(""), None);
        assert_eq!(code(&"a".repeat(MAX_CODE + 1)), None);
    }

    #[test]
    fn names_are_one_line_and_never_a_path() {
        assert_eq!(name("mcp:server/tool").as_deref(), Some("mcp:server/tool"));
        assert_eq!(name("Agent").as_deref(), Some("Agent"));
        assert_eq!(name("/home/someone/bin/tool"), None);
        assert_eq!(name("C:\\Users\\someone"), None);
        assert_eq!(name("two\nlines"), None);
    }

    #[test]
    fn only_paths_inside_the_workspace_pass() {
        assert_eq!(inside_workspace("src/app.rs"), Some("src/app.rs"));
        assert_eq!(inside_workspace("src\\app.rs"), Some("src\\app.rs"));
        for outside in [
            "/etc/passwd",
            "\\\\server\\share\\x",
            "C:/Users/someone/x",
            "~/.ssh/id_ed25519",
            "../other/secret",
            "src/../../secret",
            "",
        ] {
            assert_eq!(inside_workspace(outside), None, "{outside}");
        }
    }
}
