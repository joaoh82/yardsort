//! Turning "run this here" into a live PTY session.
//!
//! This is everything about launching that has nothing to do with a webview: choosing the
//! program, finding it on the *user's* `PATH`, labelling the session so it can be matched back
//! to its workspace, and writing the session record. The app and the `ys` CLI both come through
//! here, which is why it takes its collaborators explicitly rather than reaching for app state.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pty_host::{LaunchPlan, PendingPrompt, SessionInfo, TermSize, TerminalHost};
use serde::Deserialize;
use specta::Type;

use crate::activity::{self, Continuation, LaunchedBy, Recorder, RunDraft, RunKind};
use crate::env::{EnvInfo, ShellEnv};
use crate::error::{IpcError, IpcResult};
use crate::harness::{self, HarnessOverride, LaunchValues, PromptTransport, SessionIdMode};
use crate::settings::ActivitySettings;
use crate::store::Store;

/// Which harness to start, and with what.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessRequest {
    pub id: String,
    /// `None` or empty: let the harness use its own default.
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The first message. `None` or empty just opens the harness.
    pub prompt: Option<String>,
}

/// What to run in a workspace.
pub enum Launch {
    /// The user's shell.
    Shell,
    Program {
        program: String,
        args: Vec<String>,
    },
    Harness(HarnessRequest),
}

/// Label carrying the id of the workspace a session belongs to.
pub const WORKSPACE_LABEL: &str = "workspace";
/// Labels recording which harness a session runs and the harness's own session id (when we
/// assigned one), so the session can be resumed or forked later.
pub const HARNESS_LABEL: &str = "harness";
pub const HARNESS_SESSION_LABEL: &str = "harnessSession";
/// Label carrying the id of the session record (see `sessions.rs`) a PTY session belongs to.
pub const RECORD_LABEL: &str = "record";
/// Label carrying the id of the activity run (see [`crate::activity`]) a PTY session is.
pub const RUN_LABEL: &str = "run";

/// What launching something needs: where sessions are recorded, what runs them, the user's
/// environment, and the harness definitions in force.
///
/// Built per call rather than held: the environment is resolved lazily and the settings are a
/// snapshot, so a launcher is a borrow of both for as long as one launch takes.
pub struct Launcher<'a> {
    pub store: &'a Store,
    pub host: &'a dyn TerminalHost,
    pub env: &'a Arc<ShellEnv>,
    pub harnesses: &'a [HarnessOverride],
    /// Whether launches are recorded as activity, and who to say launched them.
    pub activity: &'a ActivitySettings,
    pub launched_by: LaunchedBy,
    /// This profile's data directory: where a harness's hooks are told to report to.
    pub data_dir: &'a Path,
}

impl Launcher<'_> {
    /// Start or focus the project's run command. Caller serializes concurrent starts.
    pub fn project_run(&self, workspace_id: &str, size: TermSize) -> IpcResult<SessionInfo> {
        let workspace = self.store.workspace(workspace_id)?.ok_or_else(|| {
            crate::error::IpcError::new("unknown_workspace", "That workspace no longer exists.")
        })?;
        if workspace.archived
            || workspace.forgotten
            || !std::path::Path::new(&workspace.path).is_dir()
        {
            return Err(IpcError::new(
                "workspace_missing",
                "Restore this workspace before running its command.",
            ));
        }
        if let Some(session) = self.host.list().into_iter().find(|session| {
            session.labels.get("projectRun").map(String::as_str) == Some(workspace_id)
                && matches!(session.state, pty_host::SessionState::Running)
        }) {
            return Ok(session);
        }
        let command =
            crate::project_automation::ProjectAutomation::load(self.store, &workspace.project_id)?
                .run
                .ok_or_else(|| {
                    crate::error::IpcError::new(
                        "no_run_command",
                        "Configure a run command in Project settings first.",
                    )
                })?;
        let mut resolved = resolve_launch(
            Launch::Program {
                program: command.program,
                args: command.args,
            },
            &[],
        )?;
        resolved
            .labels
            .insert("projectRun".into(), workspace_id.to_owned());
        resolved
            .labels
            .insert(WORKSPACE_LABEL.into(), workspace_id.to_owned());
        let draft = RunDraft {
            workspace_id: workspace_id.to_owned(),
            session_id: None,
            kind: RunKind::Program,
            harness_id: None,
            harness_session_id: None,
            model: None,
            effort: None,
            program: activity::program_name(resolved.program.as_deref()),
            continuation: Continuation::Fresh,
        };
        Ok(self
            .start_recorded(resolved, Some(workspace.path), size, &draft)?
            .0)
    }

    /// Start something in a workspace's folder, labelled so it can be matched back to it.
    pub fn in_workspace(
        &self,
        workspace_id: &str,
        launch: Launch,
        size: TermSize,
    ) -> IpcResult<SessionInfo> {
        let workspace = self.store.workspace(workspace_id)?.ok_or_else(|| {
            IpcError::new("unknown_workspace", "That workspace no longer exists.")
        })?;
        if !std::path::Path::new(&workspace.path).is_dir() {
            return Err(IpcError::new(
                "workspace_missing",
                format!("{} does not exist any more.", workspace.path),
            ));
        }
        let mut resolved = resolve_launch(launch, self.harnesses)?;
        resolved
            .labels
            .insert(WORKSPACE_LABEL.to_owned(), workspace_id.to_owned());
        // A harness conversation gets a record, so it can be resumed after its process is gone.
        let record_id = resolved
            .record
            .is_some()
            .then(|| uuid::Uuid::new_v4().to_string());
        if let Some(id) = &record_id {
            resolved.labels.insert(RECORD_LABEL.to_owned(), id.clone());
        }
        let draft = resolved.record.take();
        let run = RunDraft {
            workspace_id: workspace_id.to_owned(),
            // The record is written after the spawn, so the run is linked to it afterwards.
            session_id: None,
            kind: match (&resolved.program, &draft) {
                (None, _) => RunKind::Shell,
                (Some(_), Some(_)) => RunKind::Harness,
                (Some(_), None) => RunKind::Program,
            },
            harness_id: draft.as_ref().map(|d| d.harness_id.clone()),
            harness_session_id: draft.as_ref().and_then(|d| d.harness_session_id.clone()),
            model: draft.as_ref().and_then(|d| d.model.clone()),
            effort: draft.as_ref().and_then(|d| d.effort.clone()),
            program: activity::program_name(resolved.program.as_deref()),
            continuation: Continuation::Fresh,
        };
        let (session, run_id) = self.start_recorded(resolved, Some(workspace.path), size, &run)?;
        if let (Some(id), Some(draft)) = (record_id, draft) {
            self.store.add_session(&crate::store::NewSession {
                id: &id,
                workspace_id,
                harness_id: &draft.harness_id,
                model: draft.model.as_deref(),
                effort: draft.effort.as_deref(),
                harness_session_id: draft.harness_session_id.as_deref(),
                title: &draft.title,
                forked_from: draft.forked_from.as_deref(),
                pty_session_id: &session.id.0,
                prompt: draft.prompt.as_deref(),
            })?;
            if let Some(run_id) = &run_id {
                self.recorder().link_session(run_id, &id);
            }
            self.settle_record(&session);
        }
        Ok(session)
    }

    /// The activity recorder for this launcher's settings and client.
    pub fn recorder(&self) -> Recorder<'_> {
        Recorder::new(self.store, self.activity, self.launched_by)
    }

    /// Spawn a resolved launch as a recorded run: the run row is written before the spawn, the
    /// PTY id attached after, and a spawn that fails is recorded as such. Returns the session
    /// and the run id — `None` when recording is off, or could not be done; the launch is never
    /// the one to pay for that.
    pub fn start_recorded(
        &self,
        mut resolved: ResolvedLaunch,
        cwd: Option<String>,
        size: TermSize,
        draft: &RunDraft,
    ) -> IpcResult<(SessionInfo, Option<String>)> {
        let recorder = self.recorder();
        let run_id = recorder.begin(draft);
        if let Some(run_id) = &run_id {
            resolved.labels.insert(RUN_LABEL.to_owned(), run_id.clone());
        }
        // A recorded launch of a harness with an adapter, when asked, is given the means to
        // report what it does. Only a recorded one: a run is what the reports are linked to. A
        // refusal is a counted diagnostic, and the launch goes ahead without.
        let capture = run_id.as_ref().and_then(|_| {
            let armed = match draft.harness_id.as_deref() {
                Some(activity::claude::HARNESS_ID) if self.activity.capture_claude => (
                    activity::claude::arm(&mut resolved.args, self.data_dir).map(|_| ()),
                    activity::claude::METHOD,
                ),
                Some(activity::codex::HARNESS_ID) if self.activity.capture_codex => {
                    let home =
                        activity::codex::codex_home(&|name| self.env.vars.get(name).cloned());
                    (
                        activity::codex::arm(&mut resolved.args, self.data_dir, home.as_deref()),
                        activity::codex::METHOD_NOTIFY,
                    )
                }
                Some(activity::opencode::HARNESS_ID) if self.activity.capture_opencode => {
                    let existing = self.env.vars.get(activity::opencode::CONFIG_ENV).cloned();
                    (
                        activity::opencode::arm(&resolved.args, self.data_dir, existing.as_deref())
                            .map(|variable| resolved.env.push(variable)),
                        activity::opencode::METHOD,
                    )
                }
                _ => return None,
            };
            match armed {
                (Ok(()), method) => Some(method),
                (Err(why), _) => {
                    eprintln!("activity: {} not armed: {why}", draft.program);
                    let _ = self.store.bump_diagnostic("hooks_not_armed", Some(&why));
                    None
                }
            }
        });
        match self.start(resolved, cwd, size) {
            Ok(session) => {
                if let Some(run_id) = &run_id {
                    recorder.spawned(run_id, draft, &session.id.0, capture);
                    // The process may be gone already — a shell script that exits at once — in
                    // which case its exit was announced to a PTY id nothing was linked to yet.
                    // Ask now, for every kind of launch; the conversation's record gets the same
                    // treatment from `settle_record` once it exists.
                    if let Ok(SessionInfo {
                        state: pty_host::SessionState::Exited { exit },
                        ..
                    }) = self.host.info(&session.id)
                    {
                        activity::record_exit(
                            self.store,
                            &session.id.0,
                            &activity::ExitFacts::from(&exit),
                            activity::Via::Settle,
                        );
                    }
                }
                Ok((session, run_id))
            }
            Err(error) => {
                if let Some(run_id) = &run_id {
                    recorder.spawn_failed(run_id, draft, &error.message);
                }
                Err(error)
            }
        }
    }

    /// See [`settle_record`].
    pub fn settle_record(&self, session: &SessionInfo) {
        settle_record(self.store, self.host, session);
    }

    /// Spawn a resolved launch. A prompt that travels over stdin rides along in the plan, and the
    /// host types it in once the program is ready — nothing here has to stay alive to see it land.
    pub fn start(
        &self,
        resolved: ResolvedLaunch,
        cwd: Option<String>,
        size: TermSize,
    ) -> IpcResult<SessionInfo> {
        let mut plan = launch_plan(self.env, resolved.program, resolved.args, cwd, size)?;
        // A workspace launch is told which run, workspace and conversation it is, so that a
        // program — or, later, a hook it runs — can say so. Ids only, appended to the user's
        // environment; nothing is taken out of it.
        for (label, variable) in [
            (RUN_LABEL, activity::RUN_ENV),
            (WORKSPACE_LABEL, activity::WORKSPACE_ENV),
            (RECORD_LABEL, activity::RECORD_ENV),
        ] {
            if let Some(value) = resolved.labels.get(label) {
                plan.env.push((variable.to_owned(), value.clone()));
            }
        }
        plan.env.extend(resolved.env);
        plan.labels = resolved.labels;
        plan.prompt = resolved.paste_when_ready;
        Ok(self.host.spawn(plan)?)
    }
}

/// A program can exit before its record exists, in which case the exit event found nothing to
/// update. Call this once the record is written to catch up.
///
/// Free-standing, and not a [`Launcher`] method only: settling a record needs the store and the
/// host but never the environment, and resolving the environment is expensive.
pub fn settle_record(store: &Store, host: &dyn TerminalHost, session: &SessionInfo) {
    if let Ok(SessionInfo {
        state: pty_host::SessionState::Exited { exit },
        ..
    }) = host.info(&session.id)
    {
        let _ = store.end_session_by_pty(&session.id.0, Some(i64::from(exit.code)));
        activity::record_exit(
            store,
            &session.id.0,
            &activity::ExitFacts::from(&exit),
            activity::Via::Settle,
        );
    }
}

type Labels = std::collections::BTreeMap<String, String>;

/// A [`Launch`] made concrete.
pub struct ResolvedLaunch {
    /// `None` runs the user's shell.
    pub program: Option<String>,
    pub args: Vec<String>,
    pub labels: Labels,
    /// A prompt to paste once the program is up, instead of passing it as an argument.
    pub paste_when_ready: Option<PendingPrompt>,
    /// For a new harness conversation: what to remember about it.
    pub record: Option<RecordDraft>,
    /// Variables for this launch alone, appended to the user's environment: an adapter's
    /// configuration, for a harness that takes it that way.
    pub env: Vec<(String, String)>,
}

/// What a session record needs beyond the ids chosen at spawn time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordDraft {
    pub harness_id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub harness_session_id: Option<String>,
    pub title: String,
    pub forked_from: Option<String>,
    /// The whole first message, if there was one.
    pub prompt: Option<String>,
}

/// The start of the first message, as a one-line title.
pub fn title_from_prompt(prompt: Option<&str>) -> String {
    let line = prompt
        .and_then(|p| p.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or("");
    let mut title: String = line.chars().take(80).collect();
    if line.chars().count() > 80 {
        title.push('…');
    }
    title
}

/// The longest prompt we will put on a command line. Beyond this the OS may refuse to start the
/// process at all (Windows caps a whole command line near 32 K characters; Linux caps a single
/// argument at 128 KiB), so such prompts are pasted instead.
const MAX_ARGV_PROMPT: usize = if cfg!(windows) { 24_000 } else { 100_000 };

/// Turn a [`Launch`] into a program, its argv and the labels describing it.
pub fn resolve_launch(launch: Launch, overrides: &[HarnessOverride]) -> IpcResult<ResolvedLaunch> {
    let mut labels = Labels::new();
    Ok(match launch {
        Launch::Shell => ResolvedLaunch {
            program: None,
            args: vec![],
            labels,
            paste_when_ready: None,
            record: None,
            env: vec![],
        },
        Launch::Program { program, args } => ResolvedLaunch {
            program: Some(program),
            args,
            labels,
            paste_when_ready: None,
            record: None,
            env: vec![],
        },
        Launch::Harness(request) => {
            let mut def = harness::find(&request.id, overrides).ok_or_else(|| {
                IpcError::new("unknown_harness", "That harness is not configured.")
            })?;
            let prompt = request.prompt.filter(|p| !p.trim().is_empty());
            if prompt.as_ref().is_some_and(|p| p.len() > MAX_ARGV_PROMPT) {
                def.prompt_transport = PromptTransport::Stdin;
            }
            let session_id = (def.session_id_mode == SessionIdMode::Assigned)
                .then(|| uuid::Uuid::new_v4().to_string());
            let given = |value: Option<String>| value.filter(|v| !v.trim().is_empty());
            let (model, effort) = (given(request.model), given(request.effort));
            let args = def.start_args(&LaunchValues {
                prompt: prompt.clone(),
                model: model.clone(),
                effort: effort.clone(),
                session_id: session_id.clone(),
                new_session_id: None,
            });
            labels.insert(HARNESS_LABEL.to_owned(), def.id.clone());
            if let Some(session_id) = &session_id {
                labels.insert(HARNESS_SESSION_LABEL.to_owned(), session_id.clone());
            }
            let record = RecordDraft {
                harness_id: def.id.clone(),
                model,
                effort,
                harness_session_id: session_id,
                title: title_from_prompt(prompt.as_deref()),
                forked_from: None,
                prompt: prompt.clone(),
            };
            ResolvedLaunch {
                program: Some(def.command.clone()),
                args,
                labels,
                paste_when_ready: prompt
                    .filter(|_| def.prompt_transport == PromptTransport::Stdin)
                    .map(|text| PendingPrompt {
                        text,
                        quiet_ms: def.stdin_ready_ms,
                    }),
                record: Some(record),
                env: vec![],
            }
        }
    })
}

/// Turn a request into a concrete plan: pick the program, find it on the *user's* `PATH`, and
/// hand the process the user's environment.
fn launch_plan(
    env: &Arc<ShellEnv>,
    program: Option<String>,
    args: Vec<String>,
    cwd: Option<String>,
    size: TermSize,
) -> IpcResult<LaunchPlan> {
    let cwd = cwd
        .map(PathBuf::from)
        .or_else(|| env.home_dir())
        .filter(|dir| dir.is_dir())
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| IpcError::new("bad_cwd", "no usable working directory"))?;

    let (program, args) = match program {
        Some(program) => (program, args),
        None => env.default_shell(),
    };

    let resolved = env.find_program(&program, &cwd).ok_or_else(|| {
        IpcError::new(
            "program_not_found",
            format!(
                "`{program}` was not found on PATH ({} entries, from {:?}).",
                EnvInfo::from(&**env).path_entries,
                env.source
            ),
        )
    })?;
    let (program, args) = wrap_for_platform(resolved, args);

    Ok(LaunchPlan {
        program,
        args,
        cwd: Some(cwd),
        env: env
            .vars
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        // The resolved environment is complete; nothing from the GUI process should leak in.
        clear_env: env.replaces_inherited(),
        size,
        labels: Default::default(),
        prompt: None,
    })
}

/// npm-installed CLIs on Windows are `.cmd` shims, which only `cmd.exe` can run.
fn wrap_for_platform(program: PathBuf, args: Vec<String>) -> (String, Vec<String>) {
    let path = program.to_string_lossy().into_owned();
    let is_batch = cfg!(windows)
        && program
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"));
    if is_batch {
        let mut wrapped = vec!["/D".to_owned(), "/C".to_owned(), path];
        wrapped.extend(args);
        ("cmd.exe".to_owned(), wrapped)
    } else {
        (path, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::EnvSource;

    fn process_env() -> Arc<ShellEnv> {
        Arc::new(ShellEnv {
            vars: std::env::vars().collect(),
            source: EnvSource::Process,
            warning: None,
        })
    }

    const SIZE: TermSize = TermSize { cols: 80, rows: 24 };

    #[test]
    fn unknown_programs_fail_with_a_useful_error() {
        let err = launch_plan(
            &process_env(),
            Some("yardsort-no-such-program".into()),
            vec![],
            None,
            SIZE,
        )
        .unwrap_err();
        assert_eq!(err.code, "program_not_found");
        assert!(err.message.contains("yardsort-no-such-program"));
    }

    #[test]
    fn no_program_means_the_users_shell_in_a_real_directory() {
        let plan = launch_plan(&process_env(), None, vec![], None, SIZE).unwrap();
        assert!(PathBuf::from(&plan.program).is_absolute() || plan.program == "cmd.exe");
        assert!(plan.cwd.unwrap().is_dir());
        assert_eq!(
            plan.clear_env,
            cfg!(unix),
            "a process-sourced env replaces the inherited one on Unix, and is layered on Windows"
        );
    }

    #[test]
    fn a_missing_cwd_falls_back_instead_of_failing() {
        let cwd = Some("/definitely/not/a/real/dir".to_owned());
        let plan = launch_plan(&process_env(), None, vec![], cwd, SIZE).unwrap();
        assert!(plan.cwd.unwrap().is_dir());
    }

    #[test]
    fn a_harness_launch_is_labelled_and_gets_a_session_id_when_the_harness_takes_one() {
        let request = |id: &str| HarnessRequest {
            id: id.into(),
            model: Some("opus".into()),
            effort: None,
            prompt: Some("fix it".into()),
        };
        let claude = resolve_launch(Launch::Harness(request("claude")), &[]).unwrap();
        assert_eq!(claude.program.as_deref(), Some("claude"));
        let session = &claude.labels[HARNESS_SESSION_LABEL];
        assert_eq!(
            claude.args,
            ["--model", "opus", "--session-id", session, "--", "fix it"]
        );
        assert_eq!(claude.labels[HARNESS_LABEL], "claude");
        assert!(
            claude.paste_when_ready.is_none(),
            "argv harnesses take the prompt as an argument"
        );

        let codex = resolve_launch(Launch::Harness(request("codex")), &[]).unwrap();
        assert_eq!(codex.args, ["-m", "opus", "--", "fix it"]);
        assert!(
            !codex.labels.contains_key(HARNESS_SESSION_LABEL),
            "codex picks its own id"
        );

        let err = resolve_launch(Launch::Harness(request("nope")), &[])
            .err()
            .unwrap();
        assert_eq!(err.code, "unknown_harness");
    }

    #[test]
    fn stdin_harnesses_and_oversized_prompts_are_pasted_not_passed() {
        let request = |prompt: String| HarnessRequest {
            id: "claude".into(),
            model: None,
            effort: None,
            prompt: Some(prompt),
        };
        let stdin = [HarnessOverride {
            id: "claude".into(),
            prompt_transport: Some(PromptTransport::Stdin),
            stdin_ready_ms: Some(700),
            ..Default::default()
        }];
        let pasted = resolve_launch(Launch::Harness(request("hello".into())), &stdin).unwrap();
        assert!(!pasted.args.contains(&"hello".to_owned()));
        let pending = pasted.paste_when_ready.unwrap();
        assert_eq!((pending.text.as_str(), pending.quiet_ms), ("hello", 700));

        let huge = "x".repeat(MAX_ARGV_PROMPT + 1);
        let fallback = resolve_launch(Launch::Harness(request(huge.clone())), &[]).unwrap();
        assert!(
            fallback.args.iter().all(|arg| arg.len() < 100),
            "kept off the command line"
        );
        assert_eq!(fallback.paste_when_ready.unwrap().text, huge);

        let blank = resolve_launch(Launch::Harness(request("   ".into())), &stdin).unwrap();
        assert!(
            blank.paste_when_ready.is_none(),
            "nothing to say means nothing to paste"
        );
    }

    /// A host that spawns nothing and keeps the plans it was given, for looking at what a
    /// launch would have run with. With `exited` set, every session it is asked about has
    /// already ended that way — the process that was gone before the spawn even returned.
    #[derive(Default)]
    struct PlanCatcher {
        plans: std::sync::Mutex<Vec<LaunchPlan>>,
        exited: Option<pty_host::ExitInfo>,
    }

    impl TerminalHost for PlanCatcher {
        fn spawn(&self, plan: LaunchPlan) -> pty_host::Result<SessionInfo> {
            let info = SessionInfo {
                id: pty_host::SessionId(format!("fake-{}", self.plans.lock().unwrap().len())),
                program: plan.program.clone(),
                args: plan.args.clone(),
                cwd: plan.cwd.clone(),
                pid: None,
                size: plan.size,
                labels: plan.labels.clone(),
                state: pty_host::SessionState::Running,
                has_output: false,
                busy: false,
                idle_ms: 0,
            };
            self.plans.lock().unwrap().push(plan);
            Ok(info)
        }
        fn attach(
            &self,
            _: &pty_host::SessionId,
            _: pty_host::OutputSink,
        ) -> pty_host::Result<pty_host::AttachmentId> {
            unimplemented!()
        }
        fn detach(
            &self,
            _: &pty_host::SessionId,
            _: pty_host::AttachmentId,
        ) -> pty_host::Result<()> {
            unimplemented!()
        }
        fn write(&self, _: &pty_host::SessionId, _: &[u8]) -> pty_host::Result<()> {
            unimplemented!()
        }
        fn paste(&self, _: &pty_host::SessionId, _: &str) -> pty_host::Result<()> {
            unimplemented!()
        }
        fn resize(&self, _: &pty_host::SessionId, _: TermSize) -> pty_host::Result<()> {
            unimplemented!()
        }
        fn kill(&self, _: &pty_host::SessionId) -> pty_host::Result<()> {
            unimplemented!()
        }
        fn remove(&self, _: &pty_host::SessionId) -> pty_host::Result<()> {
            unimplemented!()
        }
        fn info(&self, id: &pty_host::SessionId) -> pty_host::Result<SessionInfo> {
            let Some(exit) = &self.exited else {
                return Err(pty_host::HostError::UnknownSession(id.clone()));
            };
            Ok(SessionInfo {
                id: id.clone(),
                program: String::new(),
                args: Vec::new(),
                cwd: None,
                pid: None,
                size: TermSize::DEFAULT,
                labels: Default::default(),
                state: pty_host::SessionState::Exited { exit: exit.clone() },
                has_output: true,
                busy: false,
                idle_ms: 0,
            })
        }
        fn list(&self) -> Vec<SessionInfo> {
            Vec::new()
        }
    }

    /// A store with one project whose `local` workspace is a real (temporary) directory.
    fn workspace_in(dir: &std::path::Path, store: &Store) -> String {
        store
            .add_project(
                &dir.file_name().unwrap().to_string_lossy(),
                &dir.to_string_lossy(),
            )
            .unwrap();
        store
            .workspaces()
            .unwrap()
            .into_iter()
            .find(|w| w.path == dir.to_string_lossy())
            .unwrap()
            .id
    }

    /// `sh -c <script>`, or `cmd.exe /C <script>`, as a program launch.
    fn script(script: &str) -> Launch {
        let (program, flag) = if cfg!(windows) {
            ("cmd.exe", "/C")
        } else {
            ("sh", "-c")
        };
        Launch::Program {
            program: program.into(),
            args: vec![flag.into(), script.into()],
        }
    }

    /// A harness that is really the shell, so a "conversation" can be started without an agent
    /// on the machine. Its prompt is the script to run.
    fn shell_harness() -> HarnessOverride {
        let (command, flag) = if cfg!(windows) {
            ("cmd.exe", "/C")
        } else {
            ("sh", "-c")
        };
        HarnessOverride {
            id: "sh-agent".into(),
            builtin: Some(false),
            command: Some(command.into()),
            prompt_args: Some(vec![flag.into(), "{prompt}".into()]),
            session_id_mode: Some(SessionIdMode::Assigned),
            ..Default::default()
        }
    }

    fn launcher<'a>(
        store: &'a Store,
        host: &'a dyn TerminalHost,
        env: &'a Arc<ShellEnv>,
        harnesses: &'a [HarnessOverride],
        activity: &'a ActivitySettings,
        data_dir: &'a Path,
    ) -> Launcher<'a> {
        Launcher {
            store,
            host,
            env,
            harnesses,
            activity,
            launched_by: LaunchedBy::Cli,
            data_dir,
        }
    }

    #[test]
    fn a_workspace_launch_is_a_recorded_run_and_tells_the_program_which_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace_in(dir.path(), &store);
        let host = PlanCatcher::default();
        let env = process_env();
        let harnesses = [shell_harness()];
        let activity = ActivitySettings::default();
        let launcher = launcher(&store, &host, &env, &harnesses, &activity, dir.path());

        let session = launcher
            .in_workspace(
                &ws,
                Launch::Harness(HarnessRequest {
                    id: "sh-agent".into(),
                    model: Some("m".into()),
                    effort: None,
                    prompt: Some("exit 0".into()),
                }),
                SIZE,
            )
            .unwrap();
        let run_id = session.labels[RUN_LABEL].clone();
        let record_id = session.labels[RECORD_LABEL].clone();

        let plan = host.plans.lock().unwrap().remove(0);
        let var = |name: &str| {
            plan.env
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(var(activity::RUN_ENV), Some(run_id.clone()));
        assert_eq!(var(activity::WORKSPACE_ENV), Some(ws.clone()));
        assert_eq!(var(activity::RECORD_ENV), Some(record_id.clone()));
        // `Path` on Windows, `PATH` elsewhere: the point is that it is there exactly once.
        assert!(
            plan.env
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("PATH"))
                .count()
                == 1,
            "appended to the user's environment, not replacing it"
        );

        let run = store.run(&run_id).unwrap().unwrap();
        assert_eq!(run.kind, "harness");
        assert_eq!(run.harness_id.as_deref(), Some("sh-agent"));
        assert_eq!(run.session_id.as_deref(), Some(record_id.as_str()));
        assert_eq!(run.pty_session_id.as_deref(), Some(session.id.0.as_str()));
        assert_eq!(run.launched_by, "cli");
        assert!(run.ended_at.is_none());
        let events = store.events(&ws, None, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "process.started");
        assert!(
            !events[0].payload.contains("exit 0"),
            "the prompt is never in an event: {}",
            events[0].payload
        );
        assert_eq!(events[0].session_id.as_deref(), Some(record_id.as_str()));
    }

    /// A shell or run command has no session record, so nothing called `settle_record` for it:
    /// a script that exited before the spawn returned stayed an open run until the next start
    /// called it interrupted. Every recorded launch is settled now.
    #[test]
    fn a_launch_that_has_already_ended_when_the_spawn_returns_is_settled_whatever_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace_in(dir.path(), &store);
        let host = PlanCatcher {
            exited: Some(pty_host::ExitInfo {
                code: 9,
                success: false,
                signal: None,
            }),
            ..Default::default()
        };
        let env = process_env();
        let activity = ActivitySettings::default();
        let launcher = launcher(&store, &host, &env, &[], &activity, dir.path());

        launcher.in_workspace(&ws, Launch::Shell, SIZE).unwrap();
        let run = &store.runs(&ws).unwrap()[0];
        assert_eq!(run.kind, "shell");
        assert_eq!(
            (run.exit_code, run.end_reason.as_deref()),
            (Some(9), Some("exited"))
        );
        let kinds: Vec<_> = store
            .events(&ws, None, 10)
            .unwrap()
            .into_iter()
            .map(|e| (e.kind, e.payload.contains("\"via\":\"settle\"")))
            .collect();
        assert_eq!(
            kinds,
            [
                ("process.exited".to_owned(), true),
                ("process.started".to_owned(), false)
            ]
        );
        assert_eq!(
            activity::end_interrupted(&store, &[]),
            0,
            "nothing left to call interrupted"
        );
    }

    #[test]
    fn a_shell_and_a_run_command_are_runs_too_but_a_launch_outside_a_workspace_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace_in(dir.path(), &store);
        let host = PlanCatcher::default();
        let env = process_env();
        let activity = ActivitySettings::default();
        let launcher = launcher(&store, &host, &env, &[], &activity, dir.path());

        launcher.in_workspace(&ws, Launch::Shell, SIZE).unwrap();
        launcher.in_workspace(&ws, script("exit 0"), SIZE).unwrap();
        let resolved = resolve_launch(script("exit 0"), &[]).unwrap();
        let outside = launcher.start(resolved, None, SIZE).unwrap();
        assert!(!outside.labels.contains_key(RUN_LABEL));

        let kinds: Vec<_> = store
            .runs(&ws)
            .unwrap()
            .into_iter()
            .map(|r| r.kind)
            .collect();
        assert_eq!(
            kinds,
            ["program", "shell"],
            "newest first, nothing for the outsider"
        );
        let plans = host.plans.lock().unwrap();
        assert!(plans[2].env.iter().all(|(k, _)| k != activity::RUN_ENV));
    }

    #[test]
    fn a_program_that_cannot_start_is_recorded_as_such_and_still_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace_in(dir.path(), &store);
        let host = PlanCatcher::default();
        let env = process_env();
        let activity = ActivitySettings::default();
        let launcher = launcher(&store, &host, &env, &[], &activity, dir.path());

        let error = launcher
            .in_workspace(
                &ws,
                Launch::Program {
                    program: "yardsort-no-such-program".into(),
                    args: vec![],
                },
                SIZE,
            )
            .unwrap_err();
        assert_eq!(error.code, "program_not_found");
        let run = &store.runs(&ws).unwrap()[0];
        assert_eq!(run.end_reason.as_deref(), Some("spawn_failed"));
        assert!(run.pty_session_id.is_none());
        let events = store.events(&ws, None, 10).unwrap();
        assert_eq!(events[0].kind, "process.spawn_failed");
        assert!(events[0].payload.contains("not found"));
    }

    #[test]
    fn recording_off_means_no_run_no_label_and_no_variable() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace_in(dir.path(), &store);
        let host = PlanCatcher::default();
        let env = process_env();
        let activity = ActivitySettings {
            record_lifecycle: false,
            ..Default::default()
        };
        let launcher = launcher(&store, &host, &env, &[], &activity, dir.path());
        let session = launcher.in_workspace(&ws, Launch::Shell, SIZE).unwrap();
        assert!(!session.labels.contains_key(RUN_LABEL));
        assert!(store.runs(&ws).unwrap().is_empty());
        let plan = host.plans.lock().unwrap().remove(0);
        assert!(plan.env.iter().all(|(k, _)| k != activity::RUN_ENV));
        assert!(
            plan.env.iter().any(|(k, _)| k == activity::WORKSPACE_ENV),
            "the workspace id is a fact of the launch, not of recording"
        );
    }

    /// Real processes in real PTYs from here: the exit path is where the platforms differ.
    fn real_host() -> (
        Arc<pty_host::PtyHost>,
        std::sync::mpsc::Receiver<pty_host::HostEvent>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let host = pty_host::PtyHost::new(Arc::new(move |event| {
            let _ = tx.lock().unwrap().send(event);
        }));
        (Arc::new(host), rx)
    }

    /// The exits of every session in `ids`, however they are ordered: two processes started
    /// together end in whichever order the platform pleases, and an exit that arrives while
    /// another is being waited for must not be thrown away.
    fn wait_for_exits(
        events: &std::sync::mpsc::Receiver<pty_host::HostEvent>,
        ids: &[&pty_host::SessionId],
    ) -> std::collections::HashMap<pty_host::SessionId, pty_host::ExitInfo> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut seen = std::collections::HashMap::new();
        while seen.len() < ids.len() {
            match events.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            {
                Ok(pty_host::HostEvent::Exited { id, exit }) if ids.contains(&&id) => {
                    seen.insert(id, exit);
                }
                Ok(_) => {}
                Err(_) => panic!("no exit for {ids:?}"),
            }
        }
        seen
    }

    fn wait_for_exit(
        events: &std::sync::mpsc::Receiver<pty_host::HostEvent>,
        id: &pty_host::SessionId,
    ) -> pty_host::ExitInfo {
        wait_for_exits(events, &[id]).remove(id).unwrap()
    }

    #[test]
    fn a_process_that_exits_at_once_is_settled_in_the_record_and_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace_in(dir.path(), &store);
        let (host, events) = real_host();
        let env = process_env();
        let harnesses = [shell_harness()];
        let activity = ActivitySettings::default();
        let launcher = launcher(
            &store,
            host.as_ref(),
            &env,
            &harnesses,
            &activity,
            dir.path(),
        );

        let session = launcher
            .in_workspace(
                &ws,
                Launch::Harness(HarnessRequest {
                    id: "sh-agent".into(),
                    model: None,
                    effort: None,
                    prompt: Some("exit 3".into()),
                }),
                SIZE,
            )
            .unwrap();
        let exit = wait_for_exit(&events, &session.id);
        assert_eq!(exit.code, 3);
        // What the app's event sink and the CLI both do on an exit, whichever arrives first.
        let _ = store.end_session_by_pty(&session.id.0, Some(3));
        activity::record_exit(
            &store,
            &session.id.0,
            &activity::ExitFacts::from(&exit),
            activity::Via::Live,
        );
        // `in_workspace` settled it too, if the exit beat the record; the two must agree.
        launcher.settle_record(&session);

        let record = store
            .session(&session.labels[RECORD_LABEL])
            .unwrap()
            .unwrap();
        assert_eq!((record.running, record.exit_code), (false, Some(3)));
        let run = store.run(&session.labels[RUN_LABEL]).unwrap().unwrap();
        assert_eq!(
            (run.exit_code, run.end_reason.as_deref()),
            (Some(3), Some("exited"))
        );
        let kinds: Vec<_> = store
            .events(&ws, None, 10)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert_eq!(kinds, ["process.exited", "process.started"], "once each");
    }

    #[test]
    fn two_workspaces_keep_their_runs_and_exits_apart() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let store = Store::in_memory();
        let (ws_a, ws_b) = (
            workspace_in(a.path(), &store),
            workspace_in(b.path(), &store),
        );
        let (host, events) = real_host();
        let env = process_env();
        let activity = ActivitySettings::default();
        let launcher = launcher(&store, host.as_ref(), &env, &[], &activity, a.path());

        let in_a = launcher
            .in_workspace(&ws_a, script("exit 1"), SIZE)
            .unwrap();
        let in_b = launcher
            .in_workspace(&ws_b, script("exit 2"), SIZE)
            .unwrap();
        for (id, exit) in wait_for_exits(&events, &[&in_a.id, &in_b.id]) {
            activity::record_exit(
                &store,
                &id.0,
                &activity::ExitFacts::from(&exit),
                activity::Via::Live,
            );
        }
        let code = |ws: &str| store.runs(ws).unwrap()[0].exit_code;
        assert_eq!((code(&ws_a), code(&ws_b)), (Some(1), Some(2)));
        assert_eq!(store.events(&ws_a, None, 10).unwrap().len(), 2);
        assert_eq!(store.events(&ws_b, None, 10).unwrap().len(), 2);
        assert_eq!(
            store.runs(&ws_a).unwrap()[0].pty_session_id.as_deref(),
            Some(in_a.id.0.as_str())
        );
    }

    /// The point of the recorder: a database that cannot take the row must not take the launch
    /// down with it.
    #[test]
    fn a_broken_activity_table_does_not_stop_a_launch() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace_in(dir.path(), &store);
        store.break_activity_tables();
        let (host, events) = real_host();
        let env = process_env();
        let harnesses = [shell_harness()];
        let activity = ActivitySettings::default();
        let launcher = launcher(
            &store,
            host.as_ref(),
            &env,
            &harnesses,
            &activity,
            dir.path(),
        );

        let session = launcher
            .in_workspace(
                &ws,
                Launch::Harness(HarnessRequest {
                    id: "sh-agent".into(),
                    model: None,
                    effort: None,
                    prompt: Some("exit 0".into()),
                }),
                SIZE,
            )
            .expect("the launch goes ahead");
        assert!(!session.labels.contains_key(RUN_LABEL));
        wait_for_exit(&events, &session.id);
        launcher.settle_record(&session);
        assert!(
            store
                .session(&session.labels[RECORD_LABEL])
                .unwrap()
                .is_some(),
            "the record is written as before"
        );
        let failed = store
            .diagnostics()
            .unwrap()
            .into_iter()
            .find(|d| d.name == "write_failed")
            .expect("the failure is counted, not raised");
        assert!(failed.count >= 1);
    }

    #[cfg(not(windows))]
    #[test]
    fn programs_run_directly_off_windows() {
        let (program, args) = wrap_for_platform("/usr/bin/tool.cmd".into(), vec!["a".into()]);
        assert_eq!(
            (program.as_str(), args),
            ("/usr/bin/tool.cmd", vec!["a".to_owned()])
        );
    }

    #[test]
    fn a_claude_launch_is_given_hooks_when_asked_and_only_then() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace_in(dir.path(), &store);
        let host = PlanCatcher::default();
        let env = process_env();
        // "Claude Code" is whatever the built-in is overridden to run; here, the shell.
        let (command, flag) = if cfg!(windows) {
            ("cmd.exe", "/C")
        } else {
            ("sh", "-c")
        };
        let claude = |base_args: Vec<&str>| HarnessOverride {
            id: "claude".into(),
            command: Some(command.into()),
            base_args: Some(base_args.into_iter().map(String::from).collect()),
            session_args: Some(vec![]),
            prompt_args: Some(vec![flag.into(), "{prompt}".into()]),
            ..Default::default()
        };
        let request = || {
            Launch::Harness(HarnessRequest {
                id: "claude".into(),
                model: None,
                effort: None,
                prompt: Some("exit 0".into()),
            })
        };
        let on = ActivitySettings {
            capture_claude: true,
            ..Default::default()
        };

        let harnesses = [claude(vec![])];
        let armed = launcher(&store, &host, &env, &harnesses, &on, dir.path());
        let session = armed.in_workspace(&ws, request(), SIZE).unwrap();
        let plan = host.plans.lock().unwrap().remove(0);
        let at = plan
            .args
            .iter()
            .position(|arg| arg == "--settings")
            .expect("--settings added");
        let settings = PathBuf::from(&plan.args[at + 1]);
        assert_eq!(settings, activity::claude::settings_path(dir.path()));
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
        assert_eq!(
            written["hooks"]["PreToolUse"][0]["hooks"][0]["args"],
            serde_json::json!([
                activity::hook::HOOK_FLAG,
                "claude",
                activity::inbox_dir(dir.path()).to_string_lossy()
            ])
        );
        let started = store
            .all_events(Some(&ws))
            .unwrap()
            .into_iter()
            .find(|event| event.kind == "process.started")
            .unwrap();
        assert_eq!(
            started.run_id.as_deref(),
            Some(session.labels[RUN_LABEL].as_str())
        );
        assert!(
            started.payload.contains(r#""capture":"hook""#),
            "{}",
            started.payload
        );

        // The switch off: the same launch, untouched.
        let off = ActivitySettings::default();
        let plain = launcher(&store, &host, &env, &harnesses, &off, dir.path());
        plain.in_workspace(&ws, request(), SIZE).unwrap();
        let plan = host.plans.lock().unwrap().remove(0);
        assert!(!plan.args.iter().any(|arg| arg == "--settings"));

        // Another harness is never touched, whatever the switch says.
        let others = [shell_harness()];
        let other = launcher(&store, &host, &env, &others, &on, dir.path());
        other
            .in_workspace(
                &ws,
                Launch::Harness(HarnessRequest {
                    id: "sh-agent".into(),
                    model: None,
                    effort: None,
                    prompt: Some("exit 0".into()),
                }),
                SIZE,
            )
            .unwrap();
        let plan = host.plans.lock().unwrap().remove(0);
        assert!(!plan.args.iter().any(|arg| arg == "--settings"));

        // The user's own `--settings` wins, and the refusal is counted, not fatal.
        let theirs = [claude(vec!["--settings", "mine.json"])];
        let overridden = launcher(&store, &host, &env, &theirs, &on, dir.path());
        overridden.in_workspace(&ws, request(), SIZE).unwrap();
        let plan = host.plans.lock().unwrap().remove(0);
        assert_eq!(
            plan.args.iter().filter(|arg| *arg == "--settings").count(),
            1
        );
        assert!(store
            .diagnostics()
            .unwrap()
            .iter()
            .any(|d| d.name == "hooks_not_armed"));
    }

    #[test]
    fn a_codex_launch_is_given_notify_when_asked_and_only_then() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace_in(dir.path(), &store);
        let host = PlanCatcher::default();
        let env = process_env();
        let (command, flag) = if cfg!(windows) {
            ("cmd.exe", "/C")
        } else {
            ("sh", "-c")
        };
        let harnesses = [HarnessOverride {
            id: "codex".into(),
            command: Some(command.into()),
            prompt_args: Some(vec![flag.into(), "{prompt}".into()]),
            ..Default::default()
        }];
        let request = || {
            Launch::Harness(HarnessRequest {
                id: "codex".into(),
                model: None,
                effort: Some("high".into()),
                prompt: Some("exit 0".into()),
            })
        };
        let on = ActivitySettings {
            capture_codex: true,
            ..Default::default()
        };
        let armed = launcher(&store, &host, &env, &harnesses, &on, dir.path());
        armed.in_workspace(&ws, request(), SIZE).unwrap();
        let plan = host.plans.lock().unwrap().remove(0);
        let notify = plan
            .args
            .iter()
            .find(|arg| arg.starts_with("notify="))
            .expect("a notify override");
        let value: toml::Value = toml::from_str(notify).unwrap();
        let argv = value["notify"].as_array().unwrap();
        assert_eq!(argv[1].as_str(), Some(activity::hook::HOOK_FLAG));
        assert_eq!(argv[2].as_str(), Some("codex"));
        assert_eq!(
            argv[3].as_str(),
            Some(activity::inbox_dir(dir.path()).to_string_lossy().as_ref())
        );
        let at = plan.args.iter().position(|a| a == notify).unwrap();
        assert_eq!(plan.args[at - 1], "-c");
        assert!(
            plan.args
                .contains(&"model_reasoning_effort=\"high\"".to_owned()),
            "the effort override is still there: {:?}",
            plan.args
        );
        let started = store
            .all_events(Some(&ws))
            .unwrap()
            .into_iter()
            .find(|event| event.kind == "process.started")
            .unwrap();
        assert!(
            started.payload.contains(r#""capture":"notify""#),
            "{}",
            started.payload
        );

        // Only Codex, and only when asked.
        let claude_only = ActivitySettings {
            capture_claude: true,
            ..Default::default()
        };
        let plain = launcher(&store, &host, &env, &harnesses, &claude_only, dir.path());
        plain.in_workspace(&ws, request(), SIZE).unwrap();
        let plan = host.plans.lock().unwrap().remove(0);
        assert!(!plan.args.iter().any(|arg| arg.starts_with("notify=")));
    }

    #[test]
    fn an_opencode_launch_is_given_its_plugin_through_the_environment_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let ws = workspace_in(dir.path(), &store);
        let host = PlanCatcher::default();
        let env = process_env();
        let (command, flag) = if cfg!(windows) {
            ("cmd.exe", "/C")
        } else {
            ("sh", "-c")
        };
        let opencode = |base_args: Vec<&str>| HarnessOverride {
            id: "opencode".into(),
            command: Some(command.into()),
            base_args: Some(base_args.into_iter().map(String::from).collect()),
            prompt_args: Some(vec![flag.into(), "{prompt}".into()]),
            ..Default::default()
        };
        let request = || {
            Launch::Harness(HarnessRequest {
                id: "opencode".into(),
                model: None,
                effort: None,
                prompt: Some("exit 0".into()),
            })
        };
        let on = ActivitySettings {
            capture_opencode: true,
            ..Default::default()
        };
        let harnesses = [opencode(vec![])];
        let armed = launcher(&store, &host, &env, &harnesses, &on, dir.path());
        armed.in_workspace(&ws, request(), SIZE).unwrap();
        let plan = host.plans.lock().unwrap().remove(0);
        let (_, content) = plan
            .env
            .iter()
            .find(|(k, _)| k == activity::opencode::CONFIG_ENV)
            .expect("the configuration variable");
        let config: serde_json::Value = serde_json::from_str(content).unwrap();
        assert_eq!(
            config["plugin"][0],
            activity::opencode::file_url(&activity::opencode::plugin_path(dir.path()))
        );
        assert!(activity::opencode::plugin_path(dir.path()).is_file());
        assert!(
            !plan.args.iter().any(|a| a.contains("plugin")),
            "nothing on the command line: {:?}",
            plan.args
        );
        let started = store
            .all_events(Some(&ws))
            .unwrap()
            .into_iter()
            .find(|event| event.kind == "process.started")
            .unwrap();
        assert!(
            started.payload.contains(r#""capture":"plugin""#),
            "{}",
            started.payload
        );

        // Off, or `--pure`: nothing given.
        let off = ActivitySettings::default();
        let plain = launcher(&store, &host, &env, &harnesses, &off, dir.path());
        plain.in_workspace(&ws, request(), SIZE).unwrap();
        let plan = host.plans.lock().unwrap().remove(0);
        assert!(!plan
            .env
            .iter()
            .any(|(k, _)| k == activity::opencode::CONFIG_ENV));
        let pure = [opencode(vec!["--pure"])];
        let refused = launcher(&store, &host, &env, &pure, &on, dir.path());
        refused.in_workspace(&ws, request(), SIZE).unwrap();
        let plan = host.plans.lock().unwrap().remove(0);
        assert!(!plan
            .env
            .iter()
            .any(|(k, _)| k == activity::opencode::CONFIG_ENV));
        assert!(store
            .diagnostics()
            .unwrap()
            .iter()
            .any(|d| d.name == "hooks_not_armed"));
    }
}
