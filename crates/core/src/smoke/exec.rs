//! Carrying out one step: a process, a shell line, a server, a wait, a request. What a step
//! leaves behind, and every expectation it failed, in words.
//!
//! Nothing here touches the store. The runner claims a step, calls [`carry_out`], and writes
//! what comes back; what the runner wants to know meanwhile — stop now? — comes in through
//! [`Watch`], asked every quarter second while a process runs.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{
    Action, ExitExpect, HttpExpect, HttpMethod, JsonExpect, ProcessExpect, StatusExpect,
    WaitTarget, DEFAULT_STEP_TIMEOUT_SECS,
};
use crate::env::ShellEnv;
use crate::process::Group;
use crate::template;
use crate::workflow::runs::Place;

/// A process's stdout or stderr may grow to this; past it the step fails and says so. A dev
/// server that logs a line per millisecond for an hour must not fill a disk.
pub const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;

/// How much of a step's stdout a later step's `{{ steps.<id>.stdout }}` gets, from the start.
pub const MAX_VARIABLE_BYTES: usize = 64 * 1024;

/// How much of a response body is kept with the step. The rest is dropped, and the step says.
pub const MAX_BODY_BYTES: usize = 256 * 1024;

/// How often a running process is looked at: for its exit, the stop signal and its output.
const POLL: Duration = Duration::from_millis(100);

/// How often the window is told that a step's output grew, at most.
const OUTPUT_NOTICE_EVERY: Duration = Duration::from_millis(500);

/// Why a step should stop before it is done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// `ys test cancel`, or the app.
    Cancelled,
    /// The whole run's limit passed.
    RunTimedOut,
    /// A `start` process ended before the run did.
    ServerExited { step_id: String, exit: String },
}

/// What the runner tells a step while it runs.
pub trait Watch {
    /// Whether to stop now, and why.
    fn stop(&self) -> Option<Stop>;
    /// A step's output file grew: whoever shows it should look again.
    fn output_grew(&self, step_id: &str);
}

/// What a step came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepEnd {
    /// `passed`, `failed` or `cancelled`.
    pub status: &'static str,
    pub exit: Option<i64>,
    /// The fields the action produces, for `{{ steps.<id>.<field> }}` and the row.
    pub outputs: BTreeMap<String, String>,
    pub note: Option<String>,
    /// Why it stopped early, when it did.
    pub stopped: Option<Stop>,
}

impl StepEnd {
    fn passed(outputs: BTreeMap<String, String>) -> Self {
        Self {
            status: "passed",
            exit: None,
            outputs,
            note: None,
            stopped: None,
        }
    }

    fn failed(note: impl Into<String>) -> Self {
        Self {
            status: "failed",
            exit: None,
            outputs: BTreeMap::new(),
            note: Some(note.into()),
            stopped: None,
        }
    }

    fn stopped(stop: Stop, outputs: BTreeMap<String, String>) -> Self {
        let (status, note) = match &stop {
            Stop::Cancelled => ("cancelled", "Cancelled while this ran.".to_owned()),
            Stop::RunTimedOut => (
                "failed",
                "The run's time limit passed while this ran.".to_owned(),
            ),
            Stop::ServerExited { step_id, exit } => (
                "failed",
                format!("`{step_id}` exited with {exit} while this ran."),
            ),
        };
        Self {
            status,
            exit: None,
            outputs,
            note: Some(note),
            stopped: Some(stop),
        }
    }
}

/// A `start` step's process, kept until the run ends.
#[derive(Debug)]
pub struct Started {
    pub step_id: String,
    pub group: Group,
    pub stdout: PathBuf,
    pub stderr: PathBuf,
}

/// What a run's variables mean: `{{ project.… }}`, `{{ workspace.… }}`, `{{ run.… }}`,
/// `{{ inputs.… }}`, `{{ env.… }}` and `{{ steps.… }}`.
#[derive(Debug)]
pub struct Vars {
    pub place: Place,
    pub run_id: String,
    pub port: u16,
    /// The run's scratch folder.
    pub dir: PathBuf,
    pub inputs: BTreeMap<String, String>,
    /// The project's test environment.
    pub env: BTreeMap<String, String>,
    /// What each finished step left, by step id.
    pub steps: BTreeMap<String, BTreeMap<String, String>>,
}

impl Vars {
    pub fn render(&self, text: &str) -> String {
        template::render(text, |path| self.lookup(path))
    }

    pub fn lookup(&self, path: &[String]) -> Option<String> {
        let parts: Vec<&str> = path.iter().map(String::as_str).collect();
        match parts.as_slice() {
            ["run", "id"] => Some(self.run_id.clone()),
            ["run", "port"] => Some(self.port.to_string()),
            ["run", "dir"] => Some(self.dir.to_string_lossy().into_owned()),
            ["inputs", id] => self.inputs.get(*id).cloned(),
            ["env", name] => self.env.get(*name).cloned(),
            ["steps", id, field] => {
                let outputs = self.steps.get(*id)?;
                let value = outputs.get(*field)?;
                // `stdout` and `stderr` are kept as files; the variable is their text.
                if matches!(*field, "stdout" | "stderr") {
                    let text = read_capped(Path::new(value), MAX_VARIABLE_BYTES);
                    Some(text.trim_end_matches(['\n', '\r']).to_owned())
                } else {
                    Some(value.clone())
                }
            }
            _ => self.place.var(path),
        }
    }

    /// What every process gets in its environment: the run's variables, for a `shell` line
    /// that cannot have them in its text, and for any program that wants them.
    pub fn process_env(&self) -> Vec<(String, String)> {
        let mut env = vec![
            ("YS_RUN_ID".to_owned(), self.run_id.clone()),
            ("YS_RUN_PORT".to_owned(), self.port.to_string()),
            (
                "YS_RUN_DIR".to_owned(),
                self.dir.to_string_lossy().into_owned(),
            ),
            (
                "YS_WORKSPACE_PATH".to_owned(),
                self.place.workspace.path.clone(),
            ),
        ];
        for (id, value) in &self.inputs {
            env.push((format!("YS_INPUT_{}", env_suffix(id)), value.clone()));
        }
        for (name, value) in &self.env {
            env.push((format!("YS_ENV_{name}"), value.clone()));
        }
        env
    }
}

/// An input id as an environment variable suffix: `user-name` is `USER_NAME`.
fn env_suffix(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Where a step runs.
pub struct Context<'a> {
    pub env: &'a ShellEnv,
    pub worktree: &'a Path,
    pub output_dir: &'a Path,
    pub vars: &'a Vars,
}

impl Context<'_> {
    /// The folder a step runs in: the workspace, or `cwd` under it.
    fn cwd(&self, cwd: Option<&str>) -> Result<PathBuf, String> {
        let Some(cwd) = cwd else {
            return Ok(self.worktree.to_path_buf());
        };
        let path = self.worktree.join(cwd);
        if !path.is_dir() {
            return Err(format!("`cwd: {cwd}` is not a folder in the workspace."));
        }
        Ok(path)
    }

    /// A program by name, as the user's shell would find it.
    fn program(&self, name: &str, cwd: &Path) -> Result<PathBuf, String> {
        self.env
            .find_program(name, cwd)
            .ok_or_else(|| format!("`{name}` is not on your PATH."))
    }

    /// A command for `program`, with the run's environment and the step's on top, and its
    /// output going to the step's files.
    fn command(
        &self,
        program: &Path,
        args: &[String],
        cwd: &Path,
        env: &BTreeMap<String, String>,
        files: &Files,
        stdin: bool,
    ) -> Result<Command, String> {
        let mut command = Command::new(program);
        if self.env.replaces_inherited() {
            command.env_clear();
        }
        command
            .args(args)
            .current_dir(cwd)
            .envs(self.env.vars.iter())
            .envs(self.vars.process_env())
            .envs(env.iter().map(|(k, v)| (k, self.vars.render(v))))
            .stdin(if stdin { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::from(files.open(&files.stdout)?))
            .stderr(Stdio::from(files.open(&files.stderr)?));
        Ok(command)
    }

    /// The user's shell and how to give it one line.
    fn shell(&self, cwd: &Path) -> Result<(PathBuf, Vec<String>), String> {
        #[cfg(unix)]
        {
            let shell = self
                .env
                .get("SHELL")
                .map(str::to_owned)
                .or_else(|| std::env::var("SHELL").ok())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "/bin/sh".to_owned());
            let path = if shell.contains('/') {
                PathBuf::from(&shell)
            } else {
                self.program(&shell, cwd)?
            };
            Ok((path, vec!["-l".to_owned(), "-c".to_owned()]))
        }
        #[cfg(windows)]
        {
            let path = self
                .program("pwsh", cwd)
                .or_else(|_| self.program("powershell", cwd))
                .map_err(|_| "Neither `pwsh` nor `powershell` is on your PATH.".to_owned())?;
            Ok((
                path,
                vec![
                    "-NoProfile".to_owned(),
                    "-NonInteractive".to_owned(),
                    "-Command".to_owned(),
                ],
            ))
        }
    }
}

/// A step's output files.
struct Files {
    stdout: PathBuf,
    stderr: PathBuf,
}

impl Files {
    fn for_step(output_dir: &Path, step_id: &str) -> Self {
        Self {
            stdout: output_dir.join(format!("{step_id}.stdout")),
            stderr: output_dir.join(format!("{step_id}.stderr")),
        }
    }

    fn open(&self, path: &Path) -> Result<File, String> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("Cannot write {}: {e}", path.display()))
    }

    fn outputs(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            (
                "stdout".to_owned(),
                self.stdout.to_string_lossy().into_owned(),
            ),
            (
                "stderr".to_owned(),
                self.stderr.to_string_lossy().into_owned(),
            ),
        ])
    }

    fn size(&self) -> (u64, u64) {
        let size = |path: &Path| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        (size(&self.stdout), size(&self.stderr))
    }
}

/// How a process came to its end.
enum Waited {
    Exited(std::process::ExitStatus),
    TimedOut,
    OutputCapped,
    Stopped(Stop),
}

/// Wait for a process, looking at it every [`POLL`]: its exit, the deadline, the stop signal
/// and its output. It is killed, with everything it started, when it must stop.
fn wait_for_process(
    group: &mut Group,
    deadline: Option<Instant>,
    watch: &dyn Watch,
    step_id: &str,
    files: &Files,
) -> Waited {
    let mut last_size = files.size();
    let mut last_notice = Instant::now();
    loop {
        match group.try_wait() {
            Ok(Some(status)) => {
                if files.size() != last_size {
                    watch.output_grew(step_id);
                }
                return Waited::Exited(status);
            }
            Ok(None) => {}
            Err(_) => return Waited::Exited(Default::default()),
        }
        if let Some(stop) = watch.stop() {
            group.kill_tree();
            return Waited::Stopped(stop);
        }
        if deadline.is_some_and(|at| Instant::now() >= at) {
            group.kill_tree();
            return Waited::TimedOut;
        }
        let size = files.size();
        if size.0 > MAX_OUTPUT_BYTES || size.1 > MAX_OUTPUT_BYTES {
            group.kill_tree();
            return Waited::OutputCapped;
        }
        if size != last_size && last_notice.elapsed() >= OUTPUT_NOTICE_EVERY {
            last_size = size;
            last_notice = Instant::now();
            watch.output_grew(step_id);
        }
        std::thread::sleep(POLL);
    }
}

fn timeout(timeout_secs: Option<u32>) -> u64 {
    u64::from(timeout_secs.unwrap_or(DEFAULT_STEP_TIMEOUT_SECS))
}

/// Carry out `action` as step `step_id`. A `start` step hands back its process in `started`,
/// to be kept until the run ends. `manual` and `browser` steps are not carried out here.
pub fn carry_out(
    ctx: &Context<'_>,
    watch: &dyn Watch,
    step_id: &str,
    action: &Action,
    started: &mut Vec<Started>,
) -> StepEnd {
    match action {
        Action::Run {
            program,
            args,
            cwd,
            env,
            stdin,
            expect,
            timeout_secs,
        } => {
            let files = Files::for_step(ctx.output_dir, step_id);
            let (cwd, program) = match prepare(ctx, cwd.as_deref(), &ctx.vars.render(program)) {
                Ok(found) => found,
                Err(why) => return StepEnd::failed(why),
            };
            let args: Vec<String> = args.iter().map(|a| ctx.vars.render(a)).collect();
            let mut command = match ctx.command(&program, &args, &cwd, env, &files, stdin.is_some())
            {
                Ok(command) => command,
                Err(why) => return StepEnd::failed(why),
            };
            let stdin = stdin.as_deref().map(|text| ctx.vars.render(text));
            run_process(
                &mut command,
                stdin.as_deref(),
                timeout(*timeout_secs),
                watch,
                step_id,
                &files,
                &rendered(expect, ctx.vars),
            )
        }
        Action::Shell {
            line,
            cwd,
            env,
            expect,
            timeout_secs,
        } => {
            let files = Files::for_step(ctx.output_dir, step_id);
            let cwd = match ctx.cwd(cwd.as_deref()) {
                Ok(cwd) => cwd,
                Err(why) => return StepEnd::failed(why),
            };
            let (shell, mut args) = match ctx.shell(&cwd) {
                Ok(shell) => shell,
                Err(why) => return StepEnd::failed(why),
            };
            // The line goes to the shell as written: no variable is ever rendered into it.
            args.push(line.clone());
            let mut command = match ctx.command(&shell, &args, &cwd, env, &files, false) {
                Ok(command) => command,
                Err(why) => return StepEnd::failed(why),
            };
            run_process(
                &mut command,
                None,
                timeout(*timeout_secs),
                watch,
                step_id,
                &files,
                &rendered(expect, ctx.vars),
            )
        }
        Action::Start {
            program,
            args,
            cwd,
            env,
        } => {
            let files = Files::for_step(ctx.output_dir, step_id);
            let (cwd, program) = match prepare(ctx, cwd.as_deref(), &ctx.vars.render(program)) {
                Ok(found) => found,
                Err(why) => return StepEnd::failed(why),
            };
            let args: Vec<String> = args.iter().map(|a| ctx.vars.render(a)).collect();
            let mut command = match ctx.command(&program, &args, &cwd, env, &files, false) {
                Ok(command) => command,
                Err(why) => return StepEnd::failed(why),
            };
            let group = match Group::spawn(&mut command) {
                Ok(group) => group,
                Err(error) => {
                    return StepEnd::failed(format!(
                        "`{}` could not start: {error}",
                        program.display()
                    ))
                }
            };
            let mut outputs = files.outputs();
            outputs.insert("pid".to_owned(), group.id().to_string());
            if let Some(print) = group.fingerprint() {
                outputs.insert("process".to_owned(), print);
            }
            started.push(Started {
                step_id: step_id.to_owned(),
                group,
                stdout: files.stdout,
                stderr: files.stderr,
            });
            StepEnd::passed(outputs)
        }
        Action::WaitFor {
            target,
            interval_secs,
            timeout_secs,
        } => wait_for(ctx, watch, step_id, target, *interval_secs, *timeout_secs),
        Action::Http {
            method,
            url,
            headers,
            body,
            json,
            expect,
            timeout_secs,
        } => http(
            ctx,
            *method,
            url,
            headers,
            body.as_deref(),
            json.as_deref(),
            expect,
            *timeout_secs,
        ),
        Action::Browser { .. } => {
            StepEnd::failed("This version of Yardsort cannot run browser steps.")
        }
        Action::Manual { .. } => StepEnd::failed("A manual step is the runner's to wait on."),
    }
}

/// A `run` or `shell` step's expectations with their variables filled in: what is looked for
/// may name the port, or what an earlier step left.
fn rendered(expect: &ProcessExpect, vars: &Vars) -> ProcessExpect {
    ProcessExpect {
        exit: expect.exit,
        stdout_contains: expect.stdout_contains.as_deref().map(|t| vars.render(t)),
        stdout_matches: expect.stdout_matches.clone(),
        stderr_empty: expect.stderr_empty,
        stdout_json: expect.stdout_json.as_ref().map(|json| JsonExpect {
            path: json.path.clone(),
            equals: vars.render(&json.equals),
        }),
    }
}

/// The folder and the program a `run` or `start` step needs.
fn prepare(
    ctx: &Context<'_>,
    cwd: Option<&str>,
    program: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let cwd = ctx.cwd(cwd)?;
    let program = ctx.program(program, &cwd)?;
    Ok((cwd, program))
}

/// Start a process, feed it `stdin` when there is one, wait, and check what the step expects.
fn run_process(
    command: &mut Command,
    stdin: Option<&str>,
    timeout_secs: u64,
    watch: &dyn Watch,
    step_id: &str,
    files: &Files,
    expect: &ProcessExpect,
) -> StepEnd {
    let mut group = match Group::spawn(command) {
        Ok(group) => group,
        Err(error) => {
            return StepEnd::failed(format!(
                "`{}` could not start: {error}",
                command.get_program().to_string_lossy()
            ))
        }
    };
    if let Some(text) = stdin {
        if let Some(mut pipe) = group.stdin() {
            // A program that never reads its input is not a failure of the test.
            let _ = pipe.write_all(text.as_bytes());
        }
    }
    let outputs = files.outputs();
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    match wait_for_process(&mut group, Some(deadline), watch, step_id, files) {
        Waited::Exited(status) => {
            let exit = status.code().map(i64::from);
            let notes = check_process(expect, &status, files);
            StepEnd {
                status: if notes.is_empty() { "passed" } else { "failed" },
                exit,
                outputs: with_exit(outputs, exit),
                note: (!notes.is_empty()).then(|| notes.join("\n")),
                stopped: None,
            }
        }
        Waited::TimedOut => StepEnd {
            status: "failed",
            exit: None,
            outputs,
            note: Some(format!(
                "Still running after {}, so it was stopped.",
                duration_words(timeout_secs)
            )),
            stopped: None,
        },
        Waited::OutputCapped => StepEnd {
            status: "failed",
            exit: None,
            outputs,
            note: Some(format!(
                "It wrote more than {} MiB of output, so it was stopped.",
                MAX_OUTPUT_BYTES / (1024 * 1024)
            )),
            stopped: None,
        },
        Waited::Stopped(stop) => StepEnd::stopped(stop, outputs),
    }
}

fn with_exit(mut outputs: BTreeMap<String, String>, exit: Option<i64>) -> BTreeMap<String, String> {
    outputs.insert(
        "exit".to_owned(),
        exit.map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_owned()),
    );
    outputs
}

/// Every expectation of a `run` or `shell` step that the process failed, in words.
fn check_process(
    expect: &ProcessExpect,
    status: &std::process::ExitStatus,
    files: &Files,
) -> Vec<String> {
    let mut notes = Vec::new();
    let exit = status.code();
    let exit_words = || match exit {
        Some(code) => code.to_string(),
        None => "a signal".to_owned(),
    };
    match expect.exit {
        ExitExpect::Code(wanted) if exit != Some(wanted) => {
            let mut note = format!("exit: expected {wanted}, got {}.", exit_words());
            let stderr = read_capped(&files.stderr, MAX_VARIABLE_BYTES);
            let tail = last_lines(&stderr, 5);
            if !tail.is_empty() {
                note.push_str("\nlast lines of stderr:\n");
                note.push_str(&indent(&tail, "  "));
            }
            notes.push(note);
        }
        ExitExpect::Nonzero if exit == Some(0) => {
            notes.push("exit: expected nonzero, got 0.".to_owned());
        }
        _ => {}
    }
    let stdout = read_capped(&files.stdout, MAX_OUTPUT_BYTES as usize);
    if let Some(wanted) = &expect.stdout_contains {
        if !stdout.contains(wanted.as_str()) {
            notes.push(format!("stdout_contains {wanted:?}: not found in stdout."));
        }
    }
    if let Some(pattern) = &expect.stdout_matches {
        match regex::Regex::new(pattern) {
            Ok(re) if !re.is_match(&stdout) => {
                notes.push(format!("stdout_matches {pattern:?}: no match in stdout."));
            }
            Ok(_) => {}
            Err(error) => notes.push(format!("stdout_matches {pattern:?}: {error}")),
        }
    }
    if expect.stderr_empty {
        let stderr = read_capped(&files.stderr, MAX_VARIABLE_BYTES);
        if !stderr.trim().is_empty() {
            notes.push(format!(
                "stderr_empty: stderr has {}.",
                describe_lines(&stderr)
            ));
        }
    }
    if let Some(json) = &expect.stdout_json {
        if let Some(note) = check_json(json, &stdout, "stdout") {
            notes.push(note);
        }
    }
    notes
}

/// `stdout_json: { path, equals }` against `text`.
fn check_json(json: &JsonExpect, text: &str, what: &str) -> Option<String> {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(error) => return Some(format!("stdout_json: {what} is not JSON ({error}).")),
    };
    let wanted: serde_json::Value =
        serde_json::from_str(&json.equals).unwrap_or(serde_json::Value::Null);
    match at_path(&value, &json.path) {
        Some(found) if *found == wanted => None,
        Some(found) => Some(format!(
            "stdout_json {}: expected {}, got {}.",
            json.path, json.equals, found
        )),
        None => Some(format!("stdout_json {}: nothing at that path.", json.path)),
    }
}

/// The value at a dotted path, where a segment that is a number indexes an array.
fn at_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    path.split('.')
        .try_fold(value, |current, segment| match current {
            serde_json::Value::Object(map) => map.get(segment),
            serde_json::Value::Array(items) => {
                segment.parse::<usize>().ok().and_then(|i| items.get(i))
            }
            _ => None,
        })
}

/// Wait for a port, an address, a file or a command, trying every `interval_secs`.
fn wait_for(
    ctx: &Context<'_>,
    watch: &dyn Watch,
    step_id: &str,
    target: &WaitTarget,
    interval_secs: u32,
    timeout_secs: Option<u32>,
) -> StepEnd {
    let started = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(timeout(timeout_secs));
    let interval = Duration::from_secs(u64::from(interval_secs.max(1)));
    let mut tries = 0u32;
    loop {
        tries += 1;
        let what: String;
        let ready: Result<(), String> = match target {
            WaitTarget::Port { port } => {
                let port = ctx.vars.render(port);
                what = format!("port {port}");
                match port.parse::<u16>() {
                    Ok(port) => {
                        let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
                        std::net::TcpStream::connect_timeout(&address, Duration::from_secs(1))
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    }
                    Err(_) => {
                        return StepEnd::failed(format!("`port: {port}` is not a port number."))
                    }
                }
            }
            WaitTarget::Url { url } => {
                let url = ctx.vars.render(url);
                what = url.clone();
                match client(Duration::from_secs(5)) {
                    Ok(client) => match client.get(&url).send() {
                        Ok(response) if response.status().is_success() => Ok(()),
                        Ok(response) => Err(format!("it answered {}", response.status())),
                        Err(error) => Err(plain_error(&error)),
                    },
                    Err(why) => return StepEnd::failed(why),
                }
            }
            WaitTarget::File { file } => {
                let file = ctx.vars.render(file);
                what = file.clone();
                if ctx.worktree.join(&file).exists() {
                    Ok(())
                } else {
                    Err("it is not there".to_owned())
                }
            }
            WaitTarget::Command { program, args } => {
                let program = ctx.vars.render(program);
                what = format!("`{program}` exiting 0");
                let args: Vec<String> = args.iter().map(|a| ctx.vars.render(a)).collect();
                match command_once(ctx, watch, step_id, &program, &args, deadline) {
                    Once::Ready => Ok(()),
                    Once::NotYet(reason) => Err(reason),
                    Once::Stopped(stop) => return StepEnd::stopped(stop, BTreeMap::new()),
                    Once::Broken(why) => return StepEnd::failed(why),
                }
            }
        };
        let last_reason = match ready {
            Ok(()) => {
                let waited = started.elapsed();
                return StepEnd::passed(BTreeMap::from([(
                    "waited".to_owned(),
                    format!("{:.1}", waited.as_secs_f64()),
                )]));
            }
            Err(reason) => reason,
        };
        if let Some(stop) = watch.stop() {
            return StepEnd::stopped(stop, BTreeMap::new());
        }
        if Instant::now() >= deadline {
            return StepEnd::failed(format!(
                "{what}: not ready after {} ({tries} tries; last: {last_reason}).",
                duration_words(started.elapsed().as_secs())
            ));
        }
        // Sleep the interval, but look for a stop every quarter second of it.
        let until = Instant::now() + interval;
        while Instant::now() < until {
            if let Some(stop) = watch.stop() {
                return StepEnd::stopped(stop, BTreeMap::new());
            }
            std::thread::sleep(POLL.min(until.saturating_duration_since(Instant::now())));
        }
    }
}

/// One try of a `wait_for` on a command.
enum Once {
    Ready,
    /// Not this time, and why.
    NotYet(String),
    Stopped(Stop),
    /// It cannot be tried at all: the program is not there.
    Broken(String),
}

fn command_once(
    ctx: &Context<'_>,
    watch: &dyn Watch,
    step_id: &str,
    program: &str,
    args: &[String],
    deadline: Instant,
) -> Once {
    let cwd = ctx.worktree.to_path_buf();
    let path = match ctx.program(program, &cwd) {
        Ok(path) => path,
        Err(why) => return Once::Broken(why),
    };
    let files = Files::for_step(ctx.output_dir, step_id);
    let mut command = match ctx.command(&path, args, &cwd, &BTreeMap::new(), &files, false) {
        Ok(command) => command,
        Err(why) => return Once::Broken(why),
    };
    let mut group = match Group::spawn(&mut command) {
        Ok(group) => group,
        Err(error) => return Once::Broken(format!("`{program}` could not start: {error}")),
    };
    match wait_for_process(&mut group, Some(deadline), watch, step_id, &files) {
        Waited::Exited(status) if status.success() => Once::Ready,
        Waited::Exited(status) => Once::NotYet(format!("it exited with {status}")),
        Waited::TimedOut => Once::NotYet("it was still running at the limit".to_owned()),
        Waited::OutputCapped => Once::NotYet("it wrote too much output".to_owned()),
        Waited::Stopped(stop) => Once::Stopped(stop),
    }
}

/// An `http` step: the request, and every expectation the response failed.
#[allow(clippy::too_many_arguments)]
fn http(
    ctx: &Context<'_>,
    method: HttpMethod,
    url: &str,
    headers: &BTreeMap<String, String>,
    body: Option<&str>,
    json: Option<&str>,
    expect: &HttpExpect,
    timeout_secs: Option<u32>,
) -> StepEnd {
    let url = ctx.vars.render(url);
    let timeout = Duration::from_secs(u64::from(timeout_secs.unwrap_or(DEFAULT_STEP_TIMEOUT_SECS)));
    let client = match client(timeout) {
        Ok(client) => client,
        Err(why) => return StepEnd::failed(why),
    };
    let verb = match method {
        HttpMethod::Get => reqwest::Method::GET,
        HttpMethod::Post => reqwest::Method::POST,
        HttpMethod::Put => reqwest::Method::PUT,
        HttpMethod::Delete => reqwest::Method::DELETE,
        HttpMethod::Patch => reqwest::Method::PATCH,
    };
    let mut request = client.request(verb, &url);
    for (name, value) in headers {
        request = request.header(name, ctx.vars.render(value));
    }
    if let Some(body) = body {
        request = request.body(ctx.vars.render(body));
    } else if let Some(json) = json {
        request = request
            .header("content-type", "application/json")
            .body(ctx.vars.render(json));
    }
    let described = format!("{} {url}", method.name().to_uppercase());
    let response = match request.send() {
        Ok(response) => response,
        Err(error) => return StepEnd::failed(format!("{described}: {}.", plain_error(&error))),
    };
    let status = response.status();
    let mut header_map: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in response.headers() {
        let entry = header_map.entry(name.as_str().to_lowercase()).or_default();
        if !entry.is_empty() {
            entry.push_str(", ");
        }
        entry.push_str(&String::from_utf8_lossy(value.as_bytes()));
    }
    let bytes = match response.bytes() {
        Ok(bytes) => bytes,
        Err(error) => {
            return StepEnd::failed(format!(
                "{described}: reading the response: {}.",
                plain_error(&error)
            ))
        }
    };
    let body_text = String::from_utf8_lossy(&bytes).into_owned();
    let mut notes = check_http(expect, status, &header_map, &body_text, ctx);
    let mut kept = body_text;
    if kept.len() > MAX_BODY_BYTES {
        let mut end = MAX_BODY_BYTES;
        while !kept.is_char_boundary(end) {
            end -= 1;
        }
        kept.truncate(end);
        notes.push(format!(
            "The body is longer than {} KiB; only the start is kept.",
            MAX_BODY_BYTES / 1024
        ));
        // A long body is not a failure by itself.
        if notes.len() == 1 {
            return StepEnd {
                status: "passed",
                exit: None,
                outputs: http_outputs(status, &header_map, kept),
                note: notes.pop(),
                stopped: None,
            };
        }
    }
    StepEnd {
        status: if notes.is_empty() { "passed" } else { "failed" },
        exit: None,
        outputs: http_outputs(status, &header_map, kept),
        note: (!notes.is_empty()).then(|| notes.join("\n")),
        stopped: None,
    }
}

fn http_outputs(
    status: reqwest::StatusCode,
    headers: &BTreeMap<String, String>,
    body: String,
) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("status".to_owned(), status.as_u16().to_string()),
        ("body".to_owned(), body),
        (
            "headers".to_owned(),
            serde_json::to_string(headers).unwrap_or_else(|_| "{}".to_owned()),
        ),
    ])
}

/// Every expectation of an `http` step that the response failed.
fn check_http(
    expect: &HttpExpect,
    status: reqwest::StatusCode,
    headers: &BTreeMap<String, String>,
    body: &str,
    ctx: &Context<'_>,
) -> Vec<String> {
    let mut notes = Vec::new();
    let code = status.as_u16();
    match expect.status {
        StatusExpect::Exact(wanted) if code != wanted => {
            notes.push(format!("status: expected {wanted}, got {code}."));
        }
        StatusExpect::Class(class) if code / 100 != u16::from(class) => {
            notes.push(format!("status: expected {class}xx, got {code}."));
        }
        _ => {}
    }
    if let Some(wanted) = &expect.json {
        let wanted = ctx.vars.render(wanted);
        match (
            serde_json::from_str::<serde_json::Value>(&wanted),
            serde_json::from_str::<serde_json::Value>(body),
        ) {
            (Ok(wanted), Ok(actual)) => {
                if let Some(why) = json_includes(&wanted, &actual, "") {
                    notes.push(format!("json: {why}"));
                }
            }
            (Ok(_), Err(error)) => notes.push(format!("json: the body is not JSON ({error}).")),
            (Err(error), _) => notes.push(format!("json: the expectation is not JSON ({error}).")),
        }
    }
    if let Some(wanted) = &expect.body_contains {
        let wanted = ctx.vars.render(wanted);
        if !body.contains(&wanted) {
            notes.push(format!("body_contains {wanted:?}: not found in the body."));
        }
    }
    if let Some(pattern) = &expect.body_matches {
        match regex::Regex::new(pattern) {
            Ok(re) if !re.is_match(body) => {
                notes.push(format!("body_matches {pattern:?}: no match in the body."));
            }
            Ok(_) => {}
            Err(error) => notes.push(format!("body_matches {pattern:?}: {error}")),
        }
    }
    if let Some(header) = &expect.header {
        let wanted = ctx.vars.render(&header.contains);
        match headers.get(&header.name.to_lowercase()) {
            Some(value) if value.contains(&wanted) => {}
            Some(value) => notes.push(format!(
                "header {}: expected it to contain {wanted:?}, it is {value:?}.",
                header.name
            )),
            None => notes.push(format!("header {}: not in the response.", header.name)),
        }
    }
    notes
}

/// Whether every key of `wanted` appears in `actual` with its value, object by object. Arrays
/// and scalars must be equal. The first difference, in words, or `None`.
fn json_includes(
    wanted: &serde_json::Value,
    actual: &serde_json::Value,
    at: &str,
) -> Option<String> {
    use serde_json::Value;
    let here = |key: &str| {
        if at.is_empty() {
            key.to_owned()
        } else {
            format!("{at}.{key}")
        }
    };
    match (wanted, actual) {
        (Value::Object(wanted), Value::Object(actual)) => {
            for (key, value) in wanted {
                match actual.get(key) {
                    Some(found) => {
                        if let Some(why) = json_includes(value, found, &here(key)) {
                            return Some(why);
                        }
                    }
                    None => return Some(format!("`{}` is not in the response.", here(key))),
                }
            }
            None
        }
        (Value::Object(_), _) => Some(format!(
            "`{}` is not an object in the response.",
            if at.is_empty() { "the body" } else { at }
        )),
        (wanted, actual) if wanted == actual => None,
        (wanted, actual) => Some(format!(
            "`{}`: expected {wanted}, got {actual}.",
            if at.is_empty() { "the body" } else { at }
        )),
    }
}

/// A blocking HTTP client. reqwest brings rustls without a crypto provider; the first client
/// installs the one the app's updater uses as well.
fn client(timeout: Duration) -> Result<reqwest::blocking::Client, String> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("Yardsort/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("Cannot make an HTTP client: {error}"))
}

/// A reqwest error without the chain of wrappers: the cause, which is what a person needs.
fn plain_error(error: &reqwest::Error) -> String {
    let mut source: &dyn std::error::Error = error;
    while let Some(next) = source.source() {
        source = next;
    }
    let text = source.to_string();
    if error.is_timeout() {
        "it did not answer in time".to_owned()
    } else if text.is_empty() {
        error.to_string()
    } else {
        text
    }
}

/// The first `max` bytes of a file as text, or nothing when it cannot be read.
pub fn read_capped(path: &Path, max: usize) -> String {
    use std::io::Read;
    let Ok(file) = File::open(path) else {
        return String::new();
    };
    let mut bytes = Vec::new();
    let _ = file.take(max as u64).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The last `n` lines of `text`, trailing blank lines dropped.
pub fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(0, |i| i + 1);
    let start = end.saturating_sub(n);
    lines[start..end].join("\n")
}

pub fn indent(text: &str, by: &str) -> String {
    text.lines()
        .map(|line| format!("{by}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn describe_lines(text: &str) -> String {
    let count = text.lines().filter(|l| !l.trim().is_empty()).count();
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    match count {
        1 => format!("one line: {first:?}"),
        n => format!("{n} lines, starting {first:?}"),
    }
}

/// `1m 12s`, `31s`, `2h 5m`.
pub fn duration_words(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    match (h, m, s) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_inclusion_walks_objects_and_compares_the_rest_whole() {
        let actual: serde_json::Value =
            serde_json::json!({ "status": "ok", "db": { "up": true, "lag": 3 }, "tags": [1, 2] });
        let ok = |wanted: serde_json::Value| json_includes(&wanted, &actual, "");
        assert_eq!(ok(serde_json::json!({ "status": "ok" })), None);
        assert_eq!(ok(serde_json::json!({ "db": { "up": true } })), None);
        assert_eq!(
            ok(serde_json::json!({ "db": { "lag": 4 } })),
            Some("`db.lag`: expected 4, got 3.".to_owned())
        );
        assert_eq!(
            ok(serde_json::json!({ "tags": [1] })),
            Some("`tags`: expected [1], got [1,2].".to_owned())
        );
        assert_eq!(
            ok(serde_json::json!({ "missing": 1 })),
            Some("`missing` is not in the response.".to_owned())
        );
        assert_eq!(
            ok(serde_json::json!({ "status": { "x": 1 } })),
            Some("`status` is not an object in the response.".to_owned())
        );
    }

    #[test]
    fn a_dotted_path_reaches_into_arrays_and_keeps_types_apart() {
        let text = r#"{"items":[{"id":1},{"id":"2"}],"ok":true}"#;
        let check = |path: &str, equals: &str| {
            check_json(
                &JsonExpect {
                    path: path.into(),
                    equals: equals.into(),
                },
                text,
                "stdout",
            )
        };
        assert_eq!(check("ok", "true"), None);
        assert_eq!(check("items.0.id", "1"), None);
        assert_eq!(check("items.1.id", "\"2\""), None);
        assert_eq!(
            check("items.1.id", "2"),
            Some("stdout_json items.1.id: expected 2, got \"2\".".to_owned())
        );
        assert_eq!(
            check("items.5.id", "1"),
            Some("stdout_json items.5.id: nothing at that path.".to_owned())
        );
        assert!(check_json(
            &JsonExpect {
                path: "a".into(),
                equals: "1".into()
            },
            "not json",
            "stdout"
        )
        .unwrap()
        .contains("is not JSON"));
    }

    #[test]
    fn words_for_durations_and_tails() {
        assert_eq!(duration_words(31), "31s");
        assert_eq!(duration_words(72), "1m 12s");
        assert_eq!(duration_words(7500), "2h 5m");
        assert_eq!(last_lines("a\nb\nc\n\n\n", 2), "b\nc");
        assert_eq!(last_lines("", 2), "");
        assert_eq!(env_suffix("user-name"), "USER_NAME");
    }
}
