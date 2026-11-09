//! Session records: what lets a harness conversation outlive its process.
//!
//! A PTY session ends with its process. The conversation does not: the harness keeps it on disk,
//! filed under the workspace's folder. A record remembers which harness it was and how to name the
//! conversation to it, so it can be **resumed** (same conversation, new process) or **forked** (a
//! copy that goes its own way) at any later time, by the app, by `ys session resume` and `ys
//! session fork`, and so on a computer with no screen, after a restart ended its agents.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, PoisonError};

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

/// The records being resumed in this process now. A resume claims its record before it checks
/// that the record has ended and holds it until the record says which terminal it runs in, so two
/// devices pressing **Resume** together start one process, not two that share a conversation.
/// Each is the store's address and the record's id: two profiles open in one process, as tests
/// have, keep their own.
static RESUMING: Mutex<Option<HashSet<(usize, String)>>> = Mutex::new(None);

/// A record claimed for one resume, given back when it is dropped, whether the resume started or
/// failed.
struct Claim((usize, String));

impl Claim {
    fn take(store: &Store, record_id: &str) -> IpcResult<Self> {
        let key = (std::ptr::from_ref(store) as usize, record_id.to_owned());
        let mut resuming = RESUMING.lock().unwrap_or_else(PoisonError::into_inner);
        if !resuming
            .get_or_insert_with(HashSet::new)
            .insert(key.clone())
        {
            return Err(IpcError::new(
                "cannot_continue",
                "This session is being resumed already.",
            ));
        }
        Ok(Self(key))
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut resuming = RESUMING.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(resuming) = resuming.as_mut() {
            resuming.remove(&self.0);
        }
    }
}

impl Launcher<'_> {
    /// Start a harness on an existing conversation, typing `prompt` into it once it is ready.
    ///
    /// A resume of a record that still says it is running settles that record first, should its
    /// terminal have gone: after the daemon or the whole computer stopped, nothing was there to
    /// hear the exit. See [`Self::settle_if_gone`]. Two resumes of one record at once in this
    /// process start one agent: the other is refused.
    pub fn continue_session(
        &self,
        record_id: &str,
        how: Continue,
        size: TermSize,
        prompt: Option<String>,
    ) -> IpcResult<SessionInfo> {
        // Held until the record names its new terminal; a fork makes a record of its own.
        let _claim = match how {
            Continue::Resume => Some(Claim::take(self.store, record_id)?),
            Continue::Fork => None,
        };
        let mut row = self.store.session(record_id)?.ok_or_else(|| {
            IpcError::new("unknown_session", "That session is no longer on record.")
        })?;
        if how == Continue::Resume && row.running {
            self.settle_if_gone(&row)?;
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

    /// End this one record, should its process be gone, as a start would: an exit the daemon kept
    /// in its spool first, so a code it recorded wins; then the host's own word, only when it says
    /// the terminal exited or is not there. When the host cannot be asked, nothing is settled,
    /// and the resume is refused rather than taken for a sign that nobody is running.
    fn settle_if_gone(&self, row: &SessionRow) -> IpcResult<()> {
        activity::import_spool(self.store, self.data_dir);
        let Some(pty) = self
            .store
            .session(&row.id)?
            .filter(|row| row.running)
            .and_then(|row| row.pty_session_id)
        else {
            return Ok(());
        };
        let exit = match self.host.info(&pty_host::SessionId(pty.clone())) {
            Ok(SessionInfo {
                state: SessionState::Exited { exit },
                ..
            }) => activity::ExitFacts::from(&exit),
            Ok(_) => return Ok(()),
            // Gone with the daemon that ran it: interrupted, with no code to give.
            Err(pty_host::HostError::UnknownSession(_) | pty_host::HostError::SessionExited(_)) => {
                activity::ExitFacts {
                    code: None,
                    success: false,
                    signal: None,
                    at: None,
                }
            }
            Err(error) => {
                return Err(IpcError::new(
                    "host_unreachable",
                    format!(
                        "Whether this session is still running could not be asked of the terminal \
                         daemon, so it was not resumed: {error}"
                    ),
                ))
            }
        };
        self.store.end_session_by_pty(&pty, exit.code)?;
        let via = if exit.code.is_some() {
            activity::Via::Settle
        } else {
            activity::Via::Reconcile
        };
        activity::record_exit(self.store, &pty, &exit, via);
        Ok(())
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

    /// A profile with one workspace, its records, and a stand-in harness whose resume exits 7, so
    /// the exit says a resume ran.
    struct Profile {
        dir: tempfile::TempDir,
        store: Store,
        ws: String,
        harnesses: [HarnessOverride; 1],
        env: std::sync::Arc<crate::env::ShellEnv>,
        activity: crate::settings::ActivitySettings,
    }

    impl Profile {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::in_memory();
            let path = dir.path().to_string_lossy().into_owned();
            store
                .add_project(&dir.path().file_name().unwrap().to_string_lossy(), &path)
                .unwrap();
            let ws = store.workspaces().unwrap()[0].id.clone();
            let (command, flag) = if cfg!(windows) {
                ("cmd.exe", "/C")
            } else {
                ("sh", "-c")
            };
            Self {
                dir,
                store,
                ws,
                harnesses: [HarnessOverride {
                    id: "stand-in".into(),
                    builtin: Some(false),
                    command: Some(command.into()),
                    prompt_args: Some(vec![flag.into(), "{prompt}".into()]),
                    // Lingering, then 7: a resume settles its record at once should the program
                    // have exited already, which an instant `exit 7` on a busy machine sometimes
                    // has — and the record is then rightly ended before a test can see it run.
                    resume_args: Some(vec![
                        flag.into(),
                        if cfg!(windows) {
                            "ping -n 3 127.0.0.1 >nul & exit 7".into()
                        } else {
                            "sleep 2; exit 7".into()
                        },
                    ]),
                    session_id_mode: Some(SessionIdMode::Assigned),
                    ..Default::default()
                }],
                env: std::sync::Arc::new(crate::env::ShellEnv {
                    vars: std::env::vars().collect(),
                    source: crate::env::EnvSource::Process,
                    warning: None,
                }),
                activity: crate::settings::ActivitySettings::default(),
            }
        }

        /// A record that says it runs in `pty`, with its run, as a launch leaves them.
        fn running(&self, id: &str, pty: &str) -> String {
            let recorder =
                activity::Recorder::new(&self.store, &self.activity, activity::LaunchedBy::Cli);
            let draft = RunDraft {
                workspace_id: self.ws.clone(),
                session_id: None,
                kind: RunKind::Harness,
                harness_id: Some("stand-in".into()),
                harness_session_id: Some(format!("{id}-own-id")),
                model: None,
                effort: None,
                program: "sh".into(),
                continuation: Continuation::Fresh,
            };
            let run = recorder.begin(&draft).unwrap();
            recorder.spawned(&run, &draft, pty, None);
            self.store
                .add_session(&NewSession {
                    id,
                    workspace_id: &self.ws,
                    harness_id: "stand-in",
                    harness_session_id: Some(&format!("{id}-own-id")),
                    title: id,
                    pty_session_id: pty,
                    ..Default::default()
                })
                .unwrap();
            recorder.link_session(&run, id);
            run
        }

        fn launcher<'a>(&'a self, host: &'a dyn pty_host::TerminalHost) -> Launcher<'a> {
            Launcher {
                store: &self.store,
                host,
                env: &self.env,
                harnesses: &self.harnesses,
                activity: &self.activity,
                launched_by: activity::LaunchedBy::Cli,
                data_dir: self.dir.path(),
            }
        }
    }

    fn real_host() -> (
        pty_host::PtyHost,
        std::sync::mpsc::Receiver<pty_host::HostEvent>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let host = pty_host::PtyHost::new(std::sync::Arc::new(move |event| {
            let _ = tx.lock().unwrap().send(event);
        }));
        (host, rx)
    }

    const SIZE: TermSize = TermSize { cols: 80, rows: 24 };

    /// A record that says it is running in a terminal the daemon no longer has, as after the daemon
    /// or the computer stopped with nobody to hear it: resuming settles that record, and that one
    /// only, then continues the conversation under it, in a real terminal.
    #[test]
    fn a_conversation_whose_terminal_went_with_the_daemon_is_resumed_under_its_record() {
        let profile = Profile::new();
        let first_run = profile.running("conversation", "a-terminal-the-daemon-took-with-it");
        // Another the host does not have either: not this resume's to settle.
        profile.running("neighbour", "another-terminal-it-does-not-have");
        let (host, events) = real_host();

        let before = list(&profile.store, &profile.harnesses, &profile.ws).unwrap();
        assert!(before.iter().all(|r| r.running && !r.resumable));

        let terminal = profile
            .launcher(&host)
            .continue_session("conversation", Continue::Resume, SIZE, None)
            .unwrap();
        assert_eq!(terminal.labels[RECORD_LABEL], "conversation");
        assert_eq!(
            terminal.labels[HARNESS_SESSION_LABEL],
            "conversation-own-id"
        );
        let row = profile.store.session("conversation").unwrap().unwrap();
        assert_eq!(row.pty_session_id.as_deref(), Some(terminal.id.0.as_str()));
        let neighbour = profile.store.session("neighbour").unwrap().unwrap();
        assert!(neighbour.running, "only the record resumed is settled");
        assert_eq!(
            neighbour.pty_session_id.as_deref(),
            Some("another-terminal-it-does-not-have")
        );
        let ended = profile.store.run(&first_run).unwrap().unwrap();
        assert_eq!(
            (ended.exit_code, ended.end_reason.as_deref()),
            (None, Some("interrupted"))
        );

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
    }

    /// An exit the daemon kept in its spool, with nobody connected to hear it, is the truth about how
    /// the conversation ended: not an interruption.
    #[test]
    fn an_exit_the_daemon_kept_settles_the_record_before_it_is_resumed() {
        let profile = Profile::new();
        let first_run = profile.running("conversation", "a-terminal-that-exited");
        pty_ipc::spool::Spool::new(activity::spool_dir(profile.dir.path()))
            .record(&pty_ipc::spool::SpoolEntry {
                version: pty_ipc::spool::SPOOL_VERSION,
                at_ms: 1,
                daemon_pid: 1,
                session: pty_host::SessionId("a-terminal-that-exited".into()),
                labels: Default::default(),
                exit: pty_host::ExitInfo {
                    code: 0,
                    success: true,
                    signal: None,
                },
            })
            .unwrap();
        let (host, _events) = real_host();

        profile
            .launcher(&host)
            .continue_session("conversation", Continue::Resume, SIZE, None)
            .unwrap();
        let ended = profile.store.run(&first_run).unwrap().unwrap();
        assert_eq!(
            (ended.exit_code, ended.end_reason.as_deref()),
            (Some(0), Some("exited"))
        );
    }

    /// A daemon that cannot be asked says nothing about what runs: nothing is settled, and the
    /// resume is refused with why.
    #[test]
    fn a_daemon_that_cannot_be_asked_settles_nothing() {
        let profile = Profile::new();
        let run = profile.running("conversation", "a-terminal-in-a-daemon-not-answering");
        profile.running("neighbour", "another-in-it");

        let error = profile
            .launcher(&NotAnswering)
            .continue_session("conversation", Continue::Resume, SIZE, None)
            .unwrap_err();
        assert_eq!(error.code, "host_unreachable", "{error:?}");
        for (id, pty) in [
            ("conversation", "a-terminal-in-a-daemon-not-answering"),
            ("neighbour", "another-in-it"),
        ] {
            let row = profile.store.session(id).unwrap().unwrap();
            assert!(row.running, "{id}");
            assert_eq!(row.pty_session_id.as_deref(), Some(pty), "{id}");
        }
        assert_eq!(profile.store.run(&run).unwrap().unwrap().end_reason, None);
    }

    /// A daemon whose socket answers nothing: every question fails, and it lists nobody, as the
    /// client does when it cannot ask.
    struct NotAnswering;

    impl pty_host::TerminalHost for NotAnswering {
        fn spawn(&self, _: pty_host::LaunchPlan) -> pty_host::Result<SessionInfo> {
            Err(gone())
        }
        fn attach(
            &self,
            _: &pty_host::SessionId,
            _: pty_host::OutputSink,
        ) -> pty_host::Result<pty_host::AttachmentId> {
            Err(gone())
        }
        fn detach(
            &self,
            _: &pty_host::SessionId,
            _: pty_host::AttachmentId,
        ) -> pty_host::Result<()> {
            Err(gone())
        }
        fn write(&self, _: &pty_host::SessionId, _: &[u8]) -> pty_host::Result<()> {
            Err(gone())
        }
        fn paste(&self, _: &pty_host::SessionId, _: &str) -> pty_host::Result<()> {
            Err(gone())
        }
        fn resize(&self, _: &pty_host::SessionId, _: TermSize) -> pty_host::Result<()> {
            Err(gone())
        }
        fn kill(&self, _: &pty_host::SessionId) -> pty_host::Result<()> {
            Err(gone())
        }
        fn remove(&self, _: &pty_host::SessionId) -> pty_host::Result<()> {
            Err(gone())
        }
        fn info(&self, _: &pty_host::SessionId) -> pty_host::Result<SessionInfo> {
            Err(gone())
        }
        fn list(&self) -> Vec<SessionInfo> {
            vec![]
        }
    }

    /// A real host whose spawns wait until the test says go, having said they are waiting.
    struct HeldAtSpawn<'a> {
        inner: &'a dyn pty_host::TerminalHost,
        spawning: std::sync::mpsc::Sender<()>,
        go: Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl pty_host::TerminalHost for HeldAtSpawn<'_> {
        fn spawn(&self, plan: pty_host::LaunchPlan) -> pty_host::Result<SessionInfo> {
            self.spawning.send(()).unwrap();
            self.go.lock().unwrap().recv().unwrap();
            self.inner.spawn(plan)
        }
        fn attach(
            &self,
            id: &pty_host::SessionId,
            sink: pty_host::OutputSink,
        ) -> pty_host::Result<pty_host::AttachmentId> {
            self.inner.attach(id, sink)
        }
        fn detach(
            &self,
            id: &pty_host::SessionId,
            attachment: pty_host::AttachmentId,
        ) -> pty_host::Result<()> {
            self.inner.detach(id, attachment)
        }
        fn write(&self, id: &pty_host::SessionId, bytes: &[u8]) -> pty_host::Result<()> {
            self.inner.write(id, bytes)
        }
        fn paste(&self, id: &pty_host::SessionId, text: &str) -> pty_host::Result<()> {
            self.inner.paste(id, text)
        }
        fn resize(&self, id: &pty_host::SessionId, size: TermSize) -> pty_host::Result<()> {
            self.inner.resize(id, size)
        }
        fn kill(&self, id: &pty_host::SessionId) -> pty_host::Result<()> {
            self.inner.kill(id)
        }
        fn remove(&self, id: &pty_host::SessionId) -> pty_host::Result<()> {
            self.inner.remove(id)
        }
        fn info(&self, id: &pty_host::SessionId) -> pty_host::Result<SessionInfo> {
            self.inner.info(id)
        }
        fn list(&self) -> Vec<SessionInfo> {
            self.inner.list()
        }
    }

    /// Two devices pressing Resume together: the second is refused while the first starts, and
    /// the record names the one agent that runs. Once it is done, the claim is given back.
    #[test]
    fn two_resumes_of_one_conversation_at_once_start_one_agent() {
        let profile = Profile::new();
        profile.running("held-conversation", "a-terminal-long-gone");
        let (host, _events) = real_host();
        let (spawning, spawned) = std::sync::mpsc::channel();
        let (go, gone_ahead) = std::sync::mpsc::channel();
        let held = HeldAtSpawn {
            inner: &host,
            spawning,
            go: Mutex::new(gone_ahead),
        };
        std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                profile.launcher(&held).continue_session(
                    "held-conversation",
                    Continue::Resume,
                    SIZE,
                    None,
                )
            });
            spawned.recv().unwrap();
            let second = profile
                .launcher(&host)
                .continue_session("held-conversation", Continue::Resume, SIZE, None)
                .unwrap_err();
            assert_eq!(second.code, "cannot_continue");
            assert_eq!(second.message, "This session is being resumed already.");
            go.send(()).unwrap();
            let started = first.join().unwrap().unwrap();
            let row = profile.store.session("held-conversation").unwrap().unwrap();
            assert_eq!(row.pty_session_id.as_deref(), Some(started.id.0.as_str()));
            let ours = host
                .list()
                .into_iter()
                .filter(|info| {
                    info.labels.get(RECORD_LABEL).map(String::as_str) == Some("held-conversation")
                })
                .count();
            assert_eq!(ours, 1);
        });
        // Given back: a later resume is judged on the record, which now runs.
        let after = profile
            .launcher(&host)
            .continue_session("held-conversation", Continue::Resume, SIZE, None)
            .unwrap_err();
        assert_eq!(after.message, "This session is already running.");
    }

    fn gone() -> pty_host::HostError {
        std::io::Error::from(std::io::ErrorKind::BrokenPipe).into()
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
