//! Terminal commands: a thin adapter between the webview and the PTY host.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pty_host::{AttachmentId, HostError, HostEvent, LaunchPlan, SessionId, SessionInfo, TermSize};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::ipc::{Channel, InvokeResponseBody, IpcResponse};
use tauri::{AppHandle, State};

use crate::env::{EnvSource, ShellEnv};
use crate::error::{IpcError, IpcResult};
use crate::harness::{self, HarnessOverride, LaunchValues, PromptTransport, SessionIdMode};
use crate::state::{blocking, AppState};

/// Emitted for every [`HostEvent`].
#[derive(Debug, Clone, Serialize, Type, tauri_specta::Event)]
pub struct PtyHostEvent(pub HostEvent);

/// Terminal output, sent over the channel as raw bytes rather than JSON: the webview receives
/// an `ArrayBuffer` it can hand straight to xterm.js.
///
/// The generated binding types this as `number[]` because specta only sees the `Vec<u8>`;
/// `src/lib/ipc.ts` corrects that in one place.
#[derive(Debug, Clone, Type)]
#[specta(transparent)]
pub struct RawBytes(Vec<u8>);

impl IpcResponse for RawBytes {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        Ok(InvokeResponseBody::Raw(self.0))
    }
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SpawnRequest {
    /// Program to run. `None` starts the user's shell.
    pub program: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory. `None` means the home directory. Ignored when `workspace_id` is set.
    pub cwd: Option<String>,
    /// Run inside this workspace: the core looks up its folder (the webview never supplies
    /// paths for this) and labels the session so it can be matched back to the workspace.
    pub workspace_id: Option<String>,
    /// Run a harness instead of `program`. Requires `workspace_id`.
    pub harness: Option<HarnessRequest>,
    pub size: TermSize,
}

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

/// What the launch environment looks like, for the status bar and for bug reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnvInfo {
    pub source: EnvSource,
    pub shell: String,
    pub path_entries: u32,
    pub warning: Option<String>,
}

impl From<&ShellEnv> for EnvInfo {
    fn from(env: &ShellEnv) -> Self {
        let entries = env
            .get("PATH")
            .map(|path| std::env::split_paths(path).count())
            .unwrap_or(0);
        Self {
            source: env.source,
            shell: env.default_shell().0,
            path_entries: u32::try_from(entries).unwrap_or(u32::MAX),
            warning: env.warning.clone(),
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn pty_spawn(app: AppHandle, request: SpawnRequest) -> IpcResult<SessionInfo> {
    // Resolving the environment and forking are both blocking.
    blocking(app, move |state| {
        let launch = match (request.harness, request.program) {
            (Some(harness), _) => Launch::Harness(harness),
            (None, Some(program)) => Launch::Program {
                program,
                args: request.args,
            },
            (None, None) => Launch::Shell,
        };
        match request.workspace_id {
            Some(workspace_id) => spawn_in_workspace(state, &workspace_id, launch, request.size),
            None => {
                let resolved = resolve_launch(launch, &state.settings.get().harnesses)?;
                start(state, resolved, request.cwd, request.size)
            }
        }
    })
    .await
}

/// Start something in a workspace's folder, labelled so it can be matched back to it.
pub fn spawn_in_workspace(
    state: &AppState,
    workspace_id: &str,
    launch: Launch,
    size: TermSize,
) -> IpcResult<SessionInfo> {
    let workspace = state
        .store
        .workspace(workspace_id)?
        .ok_or_else(|| IpcError::new("unknown_workspace", "That workspace no longer exists."))?;
    if !std::path::Path::new(&workspace.path).is_dir() {
        return Err(IpcError::new(
            "workspace_missing",
            format!("{} does not exist any more.", workspace.path),
        ));
    }
    let mut resolved = resolve_launch(launch, &state.settings.get().harnesses)?;
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
    let session = start(state, resolved, Some(workspace.path), size)?;
    if let (Some(id), Some(draft)) = (record_id, draft) {
        state.store.add_session(&crate::store::NewSession {
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
        settle_record(state, &session);
    }
    Ok(session)
}

/// A program can exit before its record exists, in which case the exit event found nothing to
/// update. Call this once the record is written to catch up.
pub fn settle_record(state: &AppState, session: &SessionInfo) {
    if let Ok(SessionInfo {
        state: pty_host::SessionState::Exited { exit },
        ..
    }) = state.host.info(&session.id)
    {
        let _ = state
            .store
            .end_session_by_pty(&session.id.0, Some(i64::from(exit.code)));
    }
}

/// Spawn a resolved launch and, if its prompt travels over stdin, arrange for the delivery.
pub fn start(
    state: &AppState,
    resolved: ResolvedLaunch,
    cwd: Option<String>,
    size: TermSize,
) -> IpcResult<SessionInfo> {
    let mut plan = launch_plan(&state.env(), resolved.program, resolved.args, cwd, size)?;
    plan.labels = resolved.labels;
    let session = state.host.spawn(plan)?;
    if let Some(prompt) = resolved.paste_when_ready {
        deliver_prompt(Arc::clone(&state.host), session.id.clone(), prompt);
    }
    Ok(session)
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

pub struct PendingPrompt {
    pub text: String,
    pub quiet_ms: u32,
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

/// How long to wait for a harness to become ready before pasting anyway.
const READY_TIMEOUT: Duration = Duration::from_secs(20);
const READY_POLL: Duration = Duration::from_millis(100);
/// Pause between pasting and pressing Enter; some TUIs drop an Enter that arrives with the paste.
const SUBMIT_DELAY: Duration = Duration::from_millis(150);

#[derive(Debug, PartialEq, Eq)]
enum Readiness {
    Ready,
    /// Still running at the timeout; we paste anyway rather than lose the prompt.
    TimedOut,
    Gone,
}

/// Wait until the program has printed something and then stayed quiet for `quiet` — a TUI that
/// has finished drawing itself and is waiting for input. We never parse what it printed.
/// `observe` reports `(has_output, idle)` or `None` once the session is over.
fn wait_until_ready(
    quiet: Duration,
    timeout: Duration,
    poll: Duration,
    mut observe: impl FnMut() -> Option<(bool, Duration)>,
) -> Readiness {
    let started = Instant::now();
    loop {
        match observe() {
            None => return Readiness::Gone,
            Some((true, idle)) if idle >= quiet => return Readiness::Ready,
            Some(_) if started.elapsed() >= timeout => return Readiness::TimedOut,
            Some(_) => std::thread::sleep(poll),
        }
    }
}

/// Paste `prompt` into a session once it is ready, then submit it. Runs on its own thread.
fn deliver_prompt(host: Arc<pty_host::PtyHost>, id: SessionId, prompt: PendingPrompt) {
    let spawned = std::thread::Builder::new()
        .name("prompt-delivery".into())
        .spawn(move || {
            let quiet = Duration::from_millis(u64::from(prompt.quiet_ms));
            let readiness = wait_until_ready(quiet, READY_TIMEOUT, READY_POLL, || {
                let info = host.info(&id).ok()?;
                matches!(info.state, pty_host::SessionState::Running).then(|| {
                    (
                        info.has_output,
                        Duration::from_millis(u64::from(info.idle_ms)),
                    )
                })
            });
            if readiness == Readiness::Gone || host.paste(&id, &prompt.text).is_err() {
                return;
            }
            std::thread::sleep(SUBMIT_DELAY);
            let _ = host.write(&id, b"\r");
        });
    if let Err(error) = spawned {
        eprintln!("could not start prompt delivery: {error}");
    }
}

/// Stream a session into `output`: first a snapshot that repaints the terminal, then live bytes.
#[tauri::command]
#[specta::specta]
pub async fn pty_attach(
    state: State<'_, AppState>,
    id: SessionId,
    output: Channel<RawBytes>,
) -> IpcResult<AttachmentId> {
    let sink = Box::new(move |bytes: &[u8]| {
        // A failed send means the webview side is gone; returning false detaches us.
        output.send(RawBytes(bytes.to_vec())).is_ok()
    });
    Ok(state.host.attach(&id, sink)?)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_detach(
    state: State<'_, AppState>,
    id: SessionId,
    attachment: AttachmentId,
) -> IpcResult<()> {
    Ok(state.host.detach(&id, attachment)?)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_write(state: State<'_, AppState>, id: SessionId, data: String) -> IpcResult<()> {
    Ok(state.host.write(&id, data.as_bytes())?)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_resize(
    state: State<'_, AppState>,
    id: SessionId,
    size: TermSize,
) -> IpcResult<()> {
    Ok(state.host.resize(&id, size)?)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_kill(state: State<'_, AppState>, id: SessionId) -> IpcResult<()> {
    match state.host.kill(&id) {
        // Killing something that already ended is not a failure worth reporting.
        Ok(()) | Err(HostError::SessionExited(_)) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Kill (if needed) and forget a session.
#[tauri::command]
#[specta::specta]
pub async fn pty_close(state: State<'_, AppState>, id: SessionId) -> IpcResult<()> {
    Ok(state.host.remove(&id)?)
}

#[tauri::command]
#[specta::specta]
pub async fn pty_list(state: State<'_, AppState>) -> IpcResult<Vec<SessionInfo>> {
    Ok(state.host.list())
}

#[tauri::command]
#[specta::specta]
pub async fn env_info(app: AppHandle, reload: bool) -> IpcResult<EnvInfo> {
    blocking(app, move |state| {
        let env = if reload {
            state.reload_env()
        } else {
            state.env()
        };
        Ok(EnvInfo::from(&*env))
    })
    .await
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
            ["--model", "opus", "--session-id", session, "fix it"]
        );
        assert_eq!(claude.labels[HARNESS_LABEL], "claude");
        assert!(
            claude.paste_when_ready.is_none(),
            "argv harnesses take the prompt as an argument"
        );

        let codex = resolve_launch(Launch::Harness(request("codex")), &[]).unwrap();
        assert_eq!(codex.args, ["-m", "opus", "fix it"]);
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

    /// The whole path, against a real program in a real PTY: a "harness" that draws a prompt,
    /// then reads a line. The message must arrive only after it has gone quiet, and be submitted.
    #[cfg(unix)]
    #[test]
    fn a_pasted_prompt_reaches_the_program_and_is_submitted() {
        use std::sync::Mutex;
        let host = Arc::new(pty_host::PtyHost::new(Arc::new(|_| {})));
        let session = host
            .spawn(LaunchPlan {
                program: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    "printf 'starting'; sleep 0.3; printf ' > '; read line; echo \"got:[$line]\""
                        .into(),
                ],
                cwd: None,
                env: vec![],
                clear_env: false,
                size: SIZE,
                labels: Default::default(),
            })
            .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        host.attach(
            &session.id,
            Box::new(move |bytes| {
                sink.lock().unwrap().extend_from_slice(bytes);
                true
            }),
        )
        .unwrap();

        let prompt = PendingPrompt {
            text: "fix the bug".into(),
            quiet_ms: 600,
        };
        deliver_prompt(Arc::clone(&host), session.id.clone(), prompt);

        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let text = String::from_utf8_lossy(&seen.lock().unwrap()).into_owned();
            if text.contains("got:[fix the bug]") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the prompt never arrived: {text:?}"
            );
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    #[test]
    fn readiness_is_output_followed_by_quiet() {
        let ms = Duration::from_millis;
        let run = |script: Vec<Option<(bool, u64)>>, timeout: u64| {
            let mut steps = script.into_iter();
            let mut last = None;
            wait_until_ready(ms(500), ms(timeout), ms(1), move || {
                last = steps.next().or(last);
                last.flatten().map(|(out, idle)| (out, ms(idle)))
            })
        };
        // Silent at first, then drawing (idle resets), then quiet long enough.
        let drawing = vec![
            Some((false, 900)),
            Some((true, 10)),
            Some((true, 200)),
            Some((true, 600)),
        ];
        assert_eq!(run(drawing, 5_000), Readiness::Ready);
        // Quiet from the start does not count: nothing was ever printed.
        assert_eq!(run(vec![Some((false, 10_000))], 30), Readiness::TimedOut);
        // Never settles: give up waiting rather than lose the prompt.
        assert_eq!(run(vec![Some((true, 5))], 30), Readiness::TimedOut);
        assert_eq!(run(vec![Some((true, 5)), None], 5_000), Readiness::Gone);
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
