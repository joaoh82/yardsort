//! Session records: what lets a harness conversation outlive its process.
//!
//! A PTY session dies with the app. The conversation does not — the harness keeps it on disk,
//! filed under the workspace's folder. A record remembers which harness it was and how to name
//! the conversation to it, so it can be **resumed** (same conversation, new process) or
//! **forked** (a copy that goes its own way) at any later time.

use std::collections::{BTreeMap, HashSet};

use pty_host::{SessionInfo, TermSize};
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use crate::error::{IpcError, IpcResult};
use crate::harness::{self, HarnessDef, LaunchValues, SessionIdMode};
use crate::state::{blocking, AppState};
use crate::store::{NewSession, SessionRow};
use crate::terminal::{
    settle_record, with_launcher, ResolvedLaunch, HARNESS_LABEL, HARNESS_SESSION_LABEL,
    RECORD_LABEL, WORKSPACE_LABEL,
};
use yardsort_core::activity::{self, Continuation, RunDraft, RunKind};

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub id: String,
    pub workspace_id: String,
    pub harness_id: String,
    /// The harness's display name, or its id if it is no longer configured.
    pub harness_label: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub title: String,
    pub forked_from: Option<String>,
    pub running: bool,
    /// The live PTY session, while running.
    pub pty_session_id: Option<String>,
    pub exit_code: Option<i32>,
    /// Ended without an exit status: the app quit or crashed underneath it.
    pub interrupted: bool,
    /// Milliseconds since the Unix epoch.
    pub started_at: f64,
    pub ended_at: Option<f64>,
    /// Whether Resume / Fork can work, and why not when they cannot.
    pub resumable: bool,
    pub forkable: bool,
    pub unavailable_reason: Option<String>,
}

/// Turn rows (newest first) into records, deciding what can be continued.
pub fn describe(rows: Vec<SessionRow>, harnesses: &[HarnessDef]) -> Vec<SessionRecord> {
    // Without an id of our own we can only ask for "the latest conversation in this folder" —
    // which is the right one only for the newest record of that harness.
    let mut latest_seen = HashSet::new();
    rows.into_iter()
        .map(|row| {
            let def = harnesses.iter().find(|def| def.id == row.harness_id);
            let is_latest = latest_seen.insert(row.harness_id.clone());
            let reason = match def {
                None => Some("This harness is no longer configured.".to_owned()),
                Some(def) if !def.enabled => {
                    Some(format!("{} is disabled in settings.", def.label))
                }
                Some(def) if row.harness_session_id.is_none() && !is_latest => Some(format!(
                    "{} can only continue its most recent conversation in a folder.",
                    def.label
                )),
                Some(def)
                    if row.harness_session_id.is_none()
                        && def.session_id_mode == SessionIdMode::Assigned =>
                {
                    Some("This conversation's id was not recorded.".to_owned())
                }
                Some(_) => None,
            };
            let can = |args: &[String]| reason.is_none() && !args.is_empty();
            SessionRecord {
                harness_label: def.map_or_else(|| row.harness_id.clone(), |d| d.label.clone()),
                resumable: !row.running && def.is_some_and(|d| can(&d.resume_args)),
                forkable: def.is_some_and(|d| can(&d.fork_args)),
                unavailable_reason: reason,
                interrupted: !row.running && row.exit_code.is_none(),
                exit_code: row
                    .exit_code
                    .map(|code| i32::try_from(code).unwrap_or(i32::MAX)),
                // Epoch milliseconds fit a double exactly for the next 285 000 years.
                started_at: row.started_at as f64,
                ended_at: row.ended_at.map(|ms| ms as f64),
                id: row.id,
                workspace_id: row.workspace_id,
                harness_id: row.harness_id,
                model: row.model,
                effort: row.effort,
                title: row.title,
                forked_from: row.forked_from,
                running: row.running,
                pty_session_id: row.pty_session_id,
            }
        })
        .collect()
}

fn enabled_harnesses(state: &AppState) -> Vec<HarnessDef> {
    harness::resolve_all(&state.settings.get().harnesses)
        .into_iter()
        .map(|resolved| resolved.def)
        .collect()
}

fn list(state: &AppState, workspace_id: &str) -> IpcResult<Vec<SessionRecord>> {
    Ok(describe(
        state.store.sessions(workspace_id)?,
        &enabled_harnesses(state),
    ))
}

enum Continue {
    Resume,
    Fork,
}

/// Start a harness on an existing conversation.
fn continue_session(
    state: &AppState,
    record_id: &str,
    how: Continue,
    size: TermSize,
) -> IpcResult<SessionInfo> {
    let row = state
        .store
        .session(record_id)?
        .ok_or_else(|| IpcError::new("unknown_session", "That session is no longer on record."))?;
    let record = list(state, &row.workspace_id)?
        .into_iter()
        .find(|record| record.id == row.id)
        .ok_or_else(|| IpcError::internal("session vanished while listing"))?;
    let allowed = match how {
        Continue::Resume => record.resumable,
        Continue::Fork => record.forkable,
    };
    if !allowed {
        let why = match (&how, row.running) {
            (Continue::Resume, true) => "This session is already running.".to_owned(),
            _ => record
                .unavailable_reason
                .unwrap_or_else(|| "This harness has no arguments configured for that.".to_owned()),
        };
        return Err(IpcError::new("cannot_continue", why));
    }

    let workspace = state
        .store
        .workspace(&row.workspace_id)?
        .ok_or_else(|| IpcError::new("unknown_workspace", "That workspace no longer exists."))?;
    if !std::path::Path::new(&workspace.path).is_dir() {
        return Err(IpcError::new(
            "workspace_missing",
            format!("{} does not exist any more.", workspace.path),
        ));
    }
    let def = harness::find(&row.harness_id, &state.settings.get().harnesses)
        .ok_or_else(|| IpcError::new("unknown_harness", "That harness is not configured."))?;

    let fork = matches!(how, Continue::Fork);
    let new_session_id = (fork && def.fork_assigns_id()).then(|| uuid::Uuid::new_v4().to_string());
    let args = def.continue_args(
        &LaunchValues {
            prompt: None,
            model: row.model.clone(),
            effort: row.effort.clone(),
            session_id: row.harness_session_id.clone(),
            new_session_id: new_session_id.clone(),
        },
        fork,
    );

    // A resume keeps its record; a fork is a conversation of its own.
    let target_id = if fork {
        uuid::Uuid::new_v4().to_string()
    } else {
        row.id.clone()
    };
    let harness_session_id = if fork {
        new_session_id
    } else {
        row.harness_session_id.clone()
    };
    let mut labels = BTreeMap::from([
        (WORKSPACE_LABEL.to_owned(), row.workspace_id.clone()),
        (HARNESS_LABEL.to_owned(), def.id.clone()),
        (RECORD_LABEL.to_owned(), target_id.clone()),
    ]);
    if let Some(id) = &harness_session_id {
        labels.insert(HARNESS_SESSION_LABEL.to_owned(), id.clone());
    }

    let run = RunDraft {
        workspace_id: row.workspace_id.clone(),
        // A resume's record exists; a fork's is written after the spawn and linked then.
        session_id: (!fork).then(|| row.id.clone()),
        kind: RunKind::Harness,
        harness_id: Some(def.id.clone()),
        harness_session_id: harness_session_id.clone(),
        model: row.model.clone(),
        effort: row.effort.clone(),
        program: activity::program_name(Some(&def.command)),
        continuation: if fork {
            Continuation::Forked {
                from: row.id.clone(),
            }
        } else {
            Continuation::Resumed
        },
    };
    let (session, run_id) = with_launcher(state, |launcher| {
        launcher.start_recorded(
            ResolvedLaunch {
                program: Some(def.command.clone()),
                args,
                labels,
                paste_when_ready: None,
                record: None,
            },
            Some(workspace.path),
            size,
            &run,
        )
    })?;
    if fork {
        state.store.add_session(&NewSession {
            id: &target_id,
            workspace_id: &row.workspace_id,
            harness_id: &row.harness_id,
            model: row.model.as_deref(),
            effort: row.effort.as_deref(),
            harness_session_id: harness_session_id.as_deref(),
            title: &row.title,
            forked_from: Some(&row.id),
            pty_session_id: &session.id.0,
            // A fork continues a conversation; nothing new was asked.
            prompt: None,
        })?;
        if let Some(run_id) = &run_id {
            with_launcher(state, |launcher| {
                launcher.recorder().link_session(run_id, &target_id);
            });
        }
    } else {
        state.store.mark_session_running(&row.id, &session.id.0)?;
    }
    settle_record(state, &session);
    Ok(session)
}

#[tauri::command]
#[specta::specta]
pub async fn sessions_list(app: AppHandle, workspace_id: String) -> IpcResult<Vec<SessionRecord>> {
    blocking(app, move |state| list(state, &workspace_id)).await
}

/// Continue a conversation whose process has ended, in a new terminal.
#[tauri::command]
#[specta::specta]
pub async fn session_resume(app: AppHandle, id: String, size: TermSize) -> IpcResult<SessionInfo> {
    blocking(app, move |state| {
        continue_session(state, &id, Continue::Resume, size)
    })
    .await
}

/// Start a copy of a conversation — running or not — that goes its own way from here.
#[tauri::command]
#[specta::specta]
pub async fn session_fork(app: AppHandle, id: String, size: TermSize) -> IpcResult<SessionInfo> {
    blocking(app, move |state| {
        continue_session(state, &id, Continue::Fork, size)
    })
    .await
}

/// Drop a record from the history. The harness's own copy of the conversation is not touched.
#[tauri::command]
#[specta::specta]
pub async fn session_forget(app: AppHandle, id: String) -> IpcResult<()> {
    blocking(app, move |state| {
        if state.store.session(&id)?.is_some_and(|row| row.running) {
            return Err(IpcError::new(
                "session_running",
                "Close the session's terminal before removing it from the history.",
            ));
        }
        state.store.remove_session(&id)?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, harness: &str, harness_session: Option<&str>, running: bool) -> SessionRow {
        SessionRow {
            id: id.into(),
            workspace_id: "ws".into(),
            harness_id: harness.into(),
            model: None,
            effort: None,
            harness_session_id: harness_session.map(str::to_owned),
            title: String::new(),
            forked_from: None,
            running,
            exit_code: (!running).then_some(0),
            pty_session_id: running.then(|| "pty".to_owned()),
            started_at: 1_790_000_000_000,
            ended_at: None,
        }
    }

    fn by_id(records: &[SessionRecord], id: &str) -> SessionRecord {
        records.iter().find(|r| r.id == id).unwrap().clone()
    }

    #[test]
    fn sessions_with_an_id_of_ours_can_always_be_continued() {
        let records = describe(
            vec![
                row("new", "claude", Some("h2"), false),
                row("old", "claude", Some("h1"), false),
            ],
            &harness::builtin(),
        );
        for id in ["new", "old"] {
            let record = by_id(&records, id);
            assert!(record.resumable && record.forkable, "{id}");
            assert_eq!(record.harness_label, "Claude Code");
        }
    }

    #[test]
    fn a_running_session_can_be_forked_but_not_resumed() {
        let record = &describe(
            vec![row("live", "claude", Some("h"), true)],
            &harness::builtin(),
        )[0];
        assert!(!record.resumable && record.forkable);
        assert!(!record.interrupted);
    }

    #[test]
    fn latest_in_folder_harnesses_continue_only_their_newest_conversation() {
        let records = describe(
            vec![
                row("codex-new", "codex", None, false),
                row("claude", "claude", Some("h"), false),
                row("codex-old", "codex", None, false),
            ],
            &harness::builtin(),
        );
        assert!(by_id(&records, "codex-new").resumable);
        let old = by_id(&records, "codex-old");
        assert!(!old.resumable && !old.forkable);
        assert!(old.unavailable_reason.unwrap().contains("most recent"));
    }

    #[test]
    fn missing_or_disabled_harnesses_explain_themselves() {
        let mut harnesses = harness::builtin();
        harnesses
            .iter_mut()
            .find(|h| h.id == "grok")
            .unwrap()
            .enabled = false;
        let records = describe(
            vec![
                row("gone", "retired-agent", Some("h"), false),
                row("off", "grok", Some("h"), false),
            ],
            &harnesses,
        );
        let gone = by_id(&records, "gone");
        assert_eq!(gone.harness_label, "retired-agent");
        assert!(
            !gone.resumable
                && gone
                    .unavailable_reason
                    .unwrap()
                    .contains("no longer configured")
        );
        assert!(by_id(&records, "off")
            .unavailable_reason
            .unwrap()
            .contains("disabled"));
    }

    #[test]
    fn an_end_without_an_exit_code_reads_as_interrupted() {
        let mut interrupted = row("cut", "claude", Some("h"), false);
        interrupted.exit_code = None;
        let record = &describe(vec![interrupted], &harness::builtin())[0];
        assert!(record.interrupted && record.resumable);
    }

    #[test]
    fn claude_forks_get_an_id_of_their_own_and_codex_forks_do_not() {
        let harnesses = harness::builtin();
        let find = |id: &str| harnesses.iter().find(|h| h.id == id).unwrap();
        assert!(find("claude").fork_assigns_id());
        assert!(!find("codex").fork_assigns_id());

        let args = find("claude").continue_args(
            &LaunchValues {
                session_id: Some("OLD".into()),
                new_session_id: Some("NEW".into()),
                ..Default::default()
            },
            true,
        );
        assert_eq!(
            args,
            ["--resume", "OLD", "--fork-session", "--session-id", "NEW"]
        );
    }
}
