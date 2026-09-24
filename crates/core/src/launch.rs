//! Turning "run this here" into a live PTY session.
//!
//! This is everything about launching that has nothing to do with a webview: choosing the
//! program, finding it on the *user's* `PATH`, labelling the session so it can be matched back
//! to its workspace, and writing the session record. The app and the `ys` CLI both come through
//! here, which is why it takes its collaborators explicitly rather than reaching for app state.

use std::path::PathBuf;
use std::sync::Arc;

use pty_host::{LaunchPlan, PendingPrompt, SessionInfo, TermSize, TerminalHost};
use serde::Deserialize;
use specta::Type;

use crate::env::{EnvInfo, ShellEnv};
use crate::error::{IpcError, IpcResult};
use crate::harness::{self, HarnessOverride, LaunchValues, PromptTransport, SessionIdMode};
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
        self.start(resolved, Some(workspace.path), size)
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
        let session = self.start(resolved, Some(workspace.path), size)?;
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
            self.settle_record(&session);
        }
        Ok(session)
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
        },
        Launch::Program { program, args } => ResolvedLaunch {
            program: Some(program),
            args,
            labels,
            paste_when_ready: None,
            record: None,
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

    #[cfg(not(windows))]
    #[test]
    fn programs_run_directly_off_windows() {
        let (program, args) = wrap_for_platform("/usr/bin/tool.cmd".into(), vec!["a".into()]);
        assert_eq!(
            (program.as_str(), args),
            ("/usr/bin/tool.cmd", vec!["a".to_owned()])
        );
    }
}
