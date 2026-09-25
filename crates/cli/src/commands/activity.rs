//! `ys activity …` — what Yardsort recorded about the processes it started.
//!
//! `list` is the timeline as a table (or JSON); `export` is the same rows as NDJSON, one event
//! per line, which is the one export format the design commits to. Both read the database
//! only; nothing here asks the daemon anything, though the exit spool is drained first so a
//! process that ended while no window was open shows its real exit.

use serde::Serialize;
use yardsort_core::store::EventRow;

use crate::commands::workspace::short;
use crate::{table, Failure, Output, Yardsort};

#[derive(clap::Subcommand)]
pub enum Command {
    /// The newest events, one workspace's or every workspace's.
    List {
        /// Only this workspace's, by name or id.
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
        /// How many to show, newest first.
        #[arg(long, value_name = "N", default_value_t = 50)]
        limit: usize,
    },
    /// Every event as NDJSON on stdout, oldest first — the public schema, one object per line.
    Export {
        /// Only this workspace's, by name or id.
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
    },
}

/// An event as it is printed: the row, plus the workspace's name for a person.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    seq: i64,
    id: String,
    schema_version: i64,
    workspace_id: String,
    workspace: String,
    session_id: Option<String>,
    run_id: Option<String>,
    occurred_at: i64,
    received_at: i64,
    kind: String,
    producer: String,
    method: String,
    fidelity: String,
    privacy_class: String,
    payload: serde_json::Value,
}

impl Event {
    fn from_row(row: EventRow, workspace: &str) -> Self {
        Self {
            payload: serde_json::from_str(&row.payload)
                .unwrap_or(serde_json::Value::String(row.payload)),
            seq: row.seq,
            id: row.id,
            schema_version: row.schema_version,
            workspace_id: row.workspace_id,
            workspace: workspace.to_owned(),
            session_id: row.session_id,
            run_id: row.run_id,
            occurred_at: row.occurred_at,
            received_at: row.received_at,
            kind: row.kind,
            producer: row.producer,
            method: row.method,
            fidelity: row.fidelity,
            privacy_class: row.privacy_class,
        }
    }
}

pub fn run(ys: &Yardsort, command: Command, out: &Output) -> Result<(), Failure> {
    ys.drain_spool();
    let workspaces = ys.store.workspaces()?;
    let name_of = |id: &str| {
        workspaces
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.name.clone())
            .unwrap_or_else(|| short(id))
    };
    let wanted = |name: &Option<String>| -> Result<Option<String>, Failure> {
        match name {
            None => Ok(None),
            Some(name) => workspaces
                .iter()
                .find(|w| w.id == *name || w.name.eq_ignore_ascii_case(name))
                .map(|w| Some(w.id.clone()))
                .ok_or_else(|| Failure::new(format!("No workspace called {name:?}."))),
        }
    };

    match command {
        Command::List { workspace, limit } => {
            let wanted = wanted(&workspace)?;
            let mut rows = match &wanted {
                Some(id) => ys.store.events(id, None, limit)?,
                None => {
                    let mut all = ys.store.all_events(None)?;
                    all.reverse();
                    all.truncate(limit);
                    all
                }
            };
            rows.retain(|row| wanted.as_ref().is_none_or(|id| row.workspace_id == *id));
            let events: Vec<Event> = rows
                .into_iter()
                .map(|row| {
                    let workspace = name_of(&row.workspace_id);
                    Event::from_row(row, &workspace)
                })
                .collect();
            out.emit(&events, || {
                if events.is_empty() {
                    println!("No activity recorded.");
                    return;
                }
                let mut lines = vec![vec![
                    "WHEN".to_owned(),
                    "WORKSPACE".to_owned(),
                    "EVENT".to_owned(),
                    "SOURCE".to_owned(),
                    "DETAILS".to_owned(),
                ]];
                lines.extend(events.iter().map(|event| {
                    vec![
                        when(event.occurred_at),
                        event.workspace.clone(),
                        event.kind.clone(),
                        format!("{}/{}", event.producer, event.method),
                        details(&event.payload),
                    ]
                }));
                table(&lines);
            })
        }
        Command::Export { workspace } => {
            let wanted = wanted(&workspace)?;
            for row in ys.store.all_events(wanted.as_deref())? {
                let workspace = name_of(&row.workspace_id);
                let line = serde_json::to_string(&Event::from_row(row, &workspace))
                    .map_err(|e| Failure::new(format!("cannot render JSON: {e}")))?;
                println!("{line}");
            }
            Ok(())
        }
    }
}

/// Epoch milliseconds as a local-looking clock time, without a timezone dependency: the date
/// and time in UTC, which is enough to read a table.
fn when(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Civil date from days since the epoch (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}Z")
}

/// The handful of payload fields worth a column.
fn details(payload: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    let text = |key: &str| payload.get(key).and_then(|v| v.as_str()).map(str::to_owned);
    if let Some(program) = text("harnessId").or_else(|| text("program")) {
        parts.push(program);
    }
    // What an agent reported: the tool and the file, a duration, the source of a session.
    if let Some(tool) = text("tool") {
        parts.push(match text("path") {
            Some(path) => format!("{tool} {path}"),
            None => tool,
        });
    }
    if let Some(ms) = payload.get("durationMs").and_then(|v| v.as_i64()) {
        parts.push(format!("{ms} ms"));
    }
    for key in [
        "source",
        "decision",
        "type",
        "errorType",
        "agentType",
        "model",
    ] {
        if let Some(value) = text(key) {
            parts.push(value);
        }
    }
    if let Some(chars) = payload.get("chars").and_then(|v| v.as_i64()) {
        parts.push(format!("{chars} chars"));
    }
    if let Some(capture) = text("capture") {
        parts.push(format!("reporting via {capture}"));
    }
    if let Some(continuation) = text("continuation").filter(|c| c != "fresh") {
        parts.push(continuation);
    }
    if let Some(code) = payload.get("exitCode") {
        parts.push(match code.as_i64() {
            Some(code) => format!("exit {code}"),
            None => text("reason").unwrap_or_else(|| "no exit status".to_owned()),
        });
    }
    if let Some(via) = text("via") {
        parts.push(format!("via {via}"));
    }
    if let Some(reason) = text("reason").filter(|_| payload.get("exitCode").is_none()) {
        parts.push(reason);
    }
    if let Some(from) = text("fromSessionId") {
        parts.push(format!("from {}", short(&from)));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_read_as_utc_clock_times() {
        assert_eq!(when(0), "1970-01-01 00:00:00Z");
        assert_eq!(when(1_790_000_000_000), "2026-09-21 14:13:20Z");
        assert_eq!(when(951_782_400_000), "2000-02-29 00:00:00Z", "leap day");
    }

    #[test]
    fn details_pick_out_what_a_person_wants_to_see() {
        let started = serde_json::json!({"harnessId": "claude", "continuation": "resumed"});
        assert_eq!(details(&started), "claude, resumed");
        let exited = serde_json::json!({"exitCode": 3, "via": "spool"});
        assert_eq!(details(&exited), "exit 3, via spool");
        let interrupted =
            serde_json::json!({"exitCode": null, "reason": "interrupted", "via": "reconcile"});
        assert_eq!(details(&interrupted), "interrupted, via reconcile");
        let failed = serde_json::json!({"program": "claude", "reason": "not found"});
        assert_eq!(details(&failed), "claude, not found");
        let tool = serde_json::json!({"tool": "Edit", "path": "src/a.rs", "durationMs": 12});
        assert_eq!(details(&tool), "Edit src/a.rs, 12 ms");
        let started = serde_json::json!({"source": "resume", "contextTokens": 29241});
        assert_eq!(details(&started), "resume");
        let prompt = serde_json::json!({"promptId": "p", "chars": 40});
        assert_eq!(details(&prompt), "40 chars");
        let hooked = serde_json::json!({"harnessId": "claude", "capture": "hook"});
        assert_eq!(details(&hooked), "claude, reporting via hook");
    }
}
