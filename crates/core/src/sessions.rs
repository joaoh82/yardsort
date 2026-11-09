//! Session records: what lets a harness conversation outlive its process.
//!
//! A PTY session ends with its process. The conversation does not: the harness keeps it on disk,
//! filed under the workspace's folder. A record remembers which harness it was and how to name the
//! conversation to it, so it can be **resumed** (same conversation, new process) or **forked** (a
//! copy that goes its own way) at any later time, by the app, by `ys session resume` and `ys
//! session fork`, and so on a computer with no screen, after a restart ended its agents.

use std::collections::{BTreeMap, HashSet};

use pty_host::{PendingPrompt, SessionInfo, SessionState, TermSize};
use serde::Serialize;
use specta::Type;

use crate::activity::{self, digest, Continuation, RunDraft, RunKind};
use crate::error::{IpcError, IpcResult};
use crate::harness::{self, HarnessDef, HarnessOverride, LaunchValues, SessionIdMode};
use crate::launch::{
    Launcher, ResolvedLaunch, CONTEXT_LABEL, HARNESS_LABEL, HARNESS_SESSION_LABEL, RECORD_LABEL,
    WORKSPACE_LABEL,
};
use crate::store::{NewSession, SessionRow, Store};
use crate::usage::logs::Sources;

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

/// A workspace's records, newest first, with what each can do.
pub fn list(
    store: &Store,
    harnesses: &[HarnessOverride],
    workspace_id: &str,
) -> IpcResult<Vec<SessionRecord>> {
    let defs: Vec<HarnessDef> = harness::resolve_all(harnesses)
        .into_iter()
        .map(|resolved| resolved.def)
        .collect();
    Ok(describe(store.sessions(workspace_id)?, &defs))
}

/// What to do with a conversation whose record this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Continue {
    /// The same conversation, in a new process, under the same record.
    Resume,
    /// A copy that goes its own way, under a record of its own.
    Fork,
}

impl Launcher<'_> {
    /// Start a harness on an existing conversation, typing `prompt` into it once it is ready.
    ///
    /// A record that still says it is running, though its terminal has gone, is settled first:
    /// after the daemon or the whole computer stopped, nothing was there to hear the exit.
    pub fn continue_session(
        &self,
        record_id: &str,
        how: Continue,
        size: TermSize,
        prompt: Option<String>,
    ) -> IpcResult<SessionInfo> {
        let mut row = self.store.session(record_id)?.ok_or_else(|| {
            IpcError::new("unknown_session", "That session is no longer on record.")
        })?;
        if row.running && !self.still_there(row.pty_session_id.as_deref()) {
            let alive: Vec<String> = self
                .host
                .list()
                .into_iter()
                .filter(|s| s.state == SessionState::Running)
                .map(|s| s.id.0)
                .collect();
            self.store.end_interrupted_sessions(&alive)?;
            activity::end_interrupted(self.store, &alive);
            row = self.store.session(record_id)?.ok_or_else(|| {
                IpcError::new("unknown_session", "That session is no longer on record.")
            })?;
        }
        let record = list(self.store, self.harnesses, &row.workspace_id)?
            .into_iter()
            .find(|record| record.id == row.id)
            .ok_or_else(|| IpcError::internal("session vanished while listing"))?;
        let allowed = match how {
            Continue::Resume => record.resumable,
            Continue::Fork => record.forkable,
        };
        if !allowed {
            let why = match (how, row.running) {
                (Continue::Resume, true) => "This session is already running.".to_owned(),
                _ => record.unavailable_reason.unwrap_or_else(|| {
                    "This harness has no arguments configured for that.".to_owned()
                }),
            };
            return Err(IpcError::new("cannot_continue", why));
        }

        let workspace = self.store.workspace(&row.workspace_id)?.ok_or_else(|| {
            IpcError::new("unknown_workspace", "That workspace no longer exists.")
        })?;
        if !std::path::Path::new(&workspace.path).is_dir() {
            return Err(IpcError::new(
                "workspace_missing",
                format!("{} does not exist any more.", workspace.path),
            ));
        }
        let def = harness::find(&row.harness_id, self.harnesses)
            .ok_or_else(|| IpcError::new("unknown_harness", "That harness is not configured."))?;

        let fork = how == Continue::Fork;
        let new_session_id =
            (fork && def.fork_assigns_id()).then(|| uuid::Uuid::new_v4().to_string());
        let env = self.env;
        let sources = Sources::from_env(&|name| env.vars.get(name).cloned());
        let note =
            digest::note_on_resume(self.store, &workspace, &def, &row.id, &sources, now_ms())?;
        let args = def.continue_args(
            &LaunchValues {
                prompt: None,
                model: row.model.clone(),
                effort: row.effort.clone(),
                session_id: row.harness_session_id.clone(),
                new_session_id: new_session_id.clone(),
                context: note.as_ref().map(|note| note.text.clone()),
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
        if let Some(note) = &note {
            labels.insert(CONTEXT_LABEL.to_owned(), note.about.clone());
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
        let (session, run_id) = self.start_recorded(
            ResolvedLaunch {
                program: Some(def.command.clone()),
                args,
                labels,
                // No harness takes a message on its resume line, so it is typed in instead.
                paste_when_ready: prompt.map(|text| PendingPrompt {
                    text,
                    quiet_ms: def.stdin_ready_ms,
                }),
                record: None,
                env: vec![],
            },
            Some(workspace.path),
            size,
            &run,
        )?;
        if fork {
            self.store.add_session(&NewSession {
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
                self.recorder().link_session(run_id, &target_id);
            }
        } else {
            self.store.mark_session_running(&row.id, &session.id.0)?;
        }
        self.settle_record(&session);
        Ok(session)
    }

    /// Whether the terminal a record names still has its process in it.
    fn still_there(&self, pty_session_id: Option<&str>) -> bool {
        pty_session_id.is_some_and(|id| {
            self.host
                .info(&pty_host::SessionId(id.to_owned()))
                .is_ok_and(|info| info.state == SessionState::Running)
        })
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
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
            began_at: 1_790_000_000_000,
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

    /// A record that says it is running in a terminal the daemon no longer has, as after the daemon
    /// or the computer stopped with nobody to hear it: resuming settles it, then continues the
    /// conversation under the same record, in a real terminal.
    #[test]
    fn a_conversation_whose_terminal_went_with_the_daemon_is_resumed_under_its_record() {
        use std::sync::Arc;

        use crate::env::{EnvSource, ShellEnv};
        use crate::settings::ActivitySettings;

        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let path = dir.path().to_string_lossy().into_owned();
        store
            .add_project(&dir.path().file_name().unwrap().to_string_lossy(), &path)
            .unwrap();
        let ws = store.workspaces().unwrap()[0].id.clone();
        store
            .add_session(&NewSession {
                id: "conversation",
                workspace_id: &ws,
                harness_id: "stand-in",
                model: None,
                effort: None,
                harness_session_id: Some("its-own-id"),
                title: "Fix the flaky test",
                forked_from: None,
                pty_session_id: "a-terminal-the-daemon-took-with-it",
                prompt: Some("Fix the flaky test"),
            })
            .unwrap();

        let (events_tx, events) = std::sync::mpsc::channel();
        let events_tx = std::sync::Mutex::new(events_tx);
        let host = pty_host::PtyHost::new(Arc::new(move |event| {
            let _ = events_tx.lock().unwrap().send(event);
        }));
        let (command, flag) = if cfg!(windows) {
            ("cmd.exe", "/C")
        } else {
            ("sh", "-c")
        };
        let harnesses = [HarnessOverride {
            id: "stand-in".into(),
            builtin: Some(false),
            command: Some(command.into()),
            prompt_args: Some(vec![flag.into(), "{prompt}".into()]),
            // What only a resume runs, so its exit says it was a resume.
            resume_args: Some(vec![flag.into(), "exit 7".into()]),
            session_id_mode: Some(SessionIdMode::Assigned),
            ..Default::default()
        }];
        let env = Arc::new(ShellEnv {
            vars: std::env::vars().collect(),
            source: EnvSource::Process,
            warning: None,
        });
        let activity = ActivitySettings::default();
        let launcher = Launcher {
            store: &store,
            host: &host,
            env: &env,
            harnesses: &harnesses,
            activity: &activity,
            launched_by: activity::LaunchedBy::Cli,
            data_dir: dir.path(),
        };

        // Before: it claims to run, so it may not be resumed by the record alone.
        let before = &list(&store, &harnesses, &ws).unwrap()[0];
        assert!(before.running && !before.resumable);

        let terminal = launcher
            .continue_session(
                "conversation",
                Continue::Resume,
                TermSize { cols: 80, rows: 24 },
                None,
            )
            .unwrap();
        assert_eq!(terminal.labels[RECORD_LABEL], "conversation");
        assert_eq!(terminal.labels[HARNESS_SESSION_LABEL], "its-own-id");
        let row = store.session("conversation").unwrap().unwrap();
        assert_eq!(row.pty_session_id.as_deref(), Some(terminal.id.0.as_str()));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let exit = loop {
            match events.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            {
                Ok(pty_host::HostEvent::Exited { id, exit }) if id == terminal.id => break exit,
                Ok(_) => {}
                Err(_) => panic!("the resumed conversation never exited"),
            }
        };
        assert_eq!(exit.code, 7, "the harness's resume arguments ran");
        let runs = store.runs(&ws).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].session_id.as_deref(), Some("conversation"));
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
