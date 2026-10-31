//! `ys test …` — a workspace's smoke tests: checking a file, and running one.
//!
//! Smoke tests are YAML files in a repository's `.yardsort/tests` folder, so they belong to a
//! worktree, not to the profile. A run is queued here and carried out by the app, which must be
//! open; a file nobody has approved in the app waits there, and nothing here can approve it.
//! See `yardsort_core::smoke` and docs/guide/smoke-tests.md.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use yardsort_core::memory;
use yardsort_core::presence;
use yardsort_core::smoke::{self, report, runs, Entry, Problem};
use yardsort_core::store::{TestRunRow, TestStepRow, WorkspaceRow};

use super::activity::when;
use crate::{table, Failure, Output, Yardsort};

#[derive(clap::Subcommand)]
pub enum Command {
    /// The smoke tests in a workspace's `.yardsort/tests`, and whether each is ready to run.
    List {
        /// Which workspace, by name or id. Default: the one this is run in.
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
    },
    /// Print a test's file, and anything wrong with it.
    Show {
        /// The test's id, from `ys test list`.
        id: String,
        /// Which workspace, by name or id. Default: the one this is run in.
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
    },
    /// Check a smoke test file. Prints each problem with its line and column; exits 1 if there
    /// are any. Needs no Yardsort profile, so it works anywhere.
    Validate {
        /// The file to check, or `-` to read it from standard input.
        file: PathBuf,
    },
    /// Print a complete, valid smoke test to start from.
    Example,
    /// Print the file format: every action, key and variable, written for an agent.
    Guide,
    /// Run a test in a workspace. Yardsort must be open: the app carries runs out. A file the
    /// user has not approved in Yardsort waits there first. Prints the run's id and returns at
    /// once; `ys test wait <run>` follows it.
    Run {
        /// The test, from `ys test list`.
        id: String,
        /// Which workspace, by name or id. Default: the one this is run in.
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
        /// An answer to one of the test's inputs. Repeat for each.
        #[arg(long = "input", value_name = "ID=VALUE")]
        inputs: Vec<String>,
    },
    /// Wait for a run to end, then print the verdict: which steps failed and why. Exits 0 when
    /// it passed, 1 when it failed, 2 for anything else.
    Wait {
        /// The run: its id, or the start of it.
        run: String,
        /// How long to wait before giving up, like `20m` or `90s`.
        #[arg(long, default_value = "20m")]
        timeout: String,
    },
    /// Runs, newest first, or one run step by step.
    Runs {
        /// Only this workspace's runs, by name or id.
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
        /// Show this run's steps: its id, or the start of it.
        #[arg(long, value_name = "RUN")]
        run: Option<String>,
        /// How many runs to list.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// A step's output: what its process wrote, or the body an `http` step got. Default: the
    /// first step that failed, or the last one that ran.
    Log {
        /// The run: its id, or the start of it.
        run: String,
        /// Which step.
        #[arg(long, value_name = "STEP")]
        step: Option<String>,
        /// The step's stderr instead of its stdout.
        #[arg(long)]
        stderr: bool,
    },
    /// Stop a run. One waiting or queued ends at once; one running stops its step, runs the
    /// teardown, and ends as cancelled.
    Cancel {
        /// The run: its id, or the start of it.
        run: String,
    },
}

/// A test as `list` prints it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Listed<'a> {
    id: &'a str,
    name: &'a str,
    description: Option<&'a str>,
    path: &'a str,
    runnable: bool,
    uses_browser: bool,
    problems: &'a [Problem],
}

pub fn run(data_dir: Option<PathBuf>, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::Validate { file } => validate(&file, out),
        Command::Example => text(smoke::EXAMPLE, out),
        Command::Guide => text(smoke::GUIDE, out),
        Command::List { workspace } => {
            let ys = Yardsort::open(data_dir)?;
            let workspace = place(&ys, workspace.as_deref())?;
            list(&workspace, out)
        }
        Command::Show { id, workspace } => {
            let ys = Yardsort::open(data_dir)?;
            let workspace = place(&ys, workspace.as_deref())?;
            show(&workspace, &id, out)
        }
        Command::Run {
            id,
            workspace,
            inputs,
        } => {
            let ys = Yardsort::open(data_dir)?;
            start_run(&ys, &id, workspace.as_deref(), &inputs, out)
        }
        Command::Wait { run, timeout } => {
            let ys = Yardsort::open(data_dir)?;
            wait(&ys, &run, &timeout, out)
        }
        Command::Runs {
            workspace,
            run,
            limit,
        } => {
            let ys = Yardsort::open(data_dir)?;
            match run {
                Some(run) => show_run(&ys, &run, out),
                None => runs(&ys, workspace.as_deref(), limit, out),
            }
        }
        Command::Log { run, step, stderr } => {
            let ys = Yardsort::open(data_dir)?;
            log(&ys, &run, step.as_deref(), stderr, out)
        }
        Command::Cancel { run } => {
            let ys = Yardsort::open(data_dir)?;
            cancel(&ys, &run, out)
        }
    }
}

/// The workspace named, or the one this is run in.
fn place(ys: &Yardsort, workspace: Option<&str>) -> Result<WorkspaceRow, Failure> {
    match workspace {
        Some(wanted) => super::workspace::find_workspace(ys, wanted),
        None => super::workflow::here(ys),
    }
}

fn list(workspace: &WorkspaceRow, out: &Output) -> Result<(), Failure> {
    let worktree = Path::new(&workspace.path);
    let entries = smoke::catalog(worktree);
    let listed: Vec<Listed> = entries
        .iter()
        .map(|e| Listed {
            id: &e.id,
            name: &e.name,
            description: e.description.as_deref(),
            path: &e.path,
            runnable: e.runnable(),
            uses_browser: e.uses_browser(),
            problems: &e.problems,
        })
        .collect();
    out.emit(&listed, || {
        let folder = smoke::dir(worktree);
        if entries.is_empty() {
            sayln!(
                "No smoke tests in {}. `ys test example` prints one to start from; `ys test guide` \
                 explains the format.",
                folder.display()
            );
            return;
        }
        let mut rows = vec![vec![
            "ID".to_owned(),
            "NAME".to_owned(),
            "STATUS".to_owned(),
            "FILE".to_owned(),
        ]];
        rows.extend(entries.iter().map(|e| {
            let file = Path::new(&e.path)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| e.path.clone());
            vec![e.id.clone(), e.name.clone(), status(e), file]
        }));
        table(&rows);
        sayln!();
        sayln!(
            "The tests of {} are in {}",
            workspace.name,
            folder.display()
        );
    })
}

fn show(workspace: &WorkspaceRow, id: &str, out: &Output) -> Result<(), Failure> {
    let worktree = Path::new(&workspace.path);
    let Some(entry) = smoke::find(worktree, id) else {
        let known: Vec<String> = smoke::catalog(worktree).into_iter().map(|e| e.id).collect();
        return Err(Failure::new(if known.is_empty() {
            format!(
                "There is no smoke test `{id}`: {} has none.",
                smoke::dir(worktree).display()
            )
        } else {
            format!(
                "There is no smoke test `{id}`. There are: {}.",
                known.join(", ")
            )
        }));
    };
    out.emit(&entry, || {
        say!("{}", entry.text);
        if !entry.text.ends_with('\n') {
            sayln!();
        }
        if !entry.problems.is_empty() {
            eprintln!();
            eprintln!("{} — it cannot run until it is fixed:", status(&entry));
            for problem in &entry.problems {
                eprintln!("{}", located(&entry.path, problem));
            }
        }
    })
}

fn validate(file: &Path, out: &Output) -> Result<(), Failure> {
    let stdin = file == Path::new("-");
    let name = if stdin {
        "<stdin>".to_owned()
    } else {
        file.display().to_string()
    };
    let text = if stdin {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|e| Failure::new(format!("Cannot read standard input: {e}")))?;
        text
    } else {
        smoke::read(file).map_err(|problem| Failure::new(problem.message))?
    };

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Checked<'a> {
        valid: bool,
        id: Option<&'a str>,
        problems: &'a [Problem],
    }
    let (id, problems) = match smoke::parse(&text) {
        Ok(test) => (Some(test.id), Vec::new()),
        Err(invalid) => (invalid.id, invalid.problems),
    };
    let checked = Checked {
        valid: problems.is_empty(),
        id: id.as_deref(),
        problems: &problems,
    };
    if checked.valid {
        return out.emit(&checked, || {
            sayln!(
                "{name}: a valid smoke test, `{}`.",
                checked.id.unwrap_or_default()
            );
        });
    }
    if out.json {
        out.emit(&checked, || {})?;
        return Err(Failure::new(count(problems.len())));
    }
    let lines: Vec<String> = problems.iter().map(|p| located(&name, p)).collect();
    Err(Failure::new(lines.join("\n")))
}

/// A built-in text, printed whole; `{ "text" }` as JSON.
fn text(body: &str, out: &Output) -> Result<(), Failure> {
    #[derive(Serialize)]
    struct Text<'a> {
        text: &'a str,
    }
    out.emit(&Text { text: body }, || say!("{body}"))
}

fn start_run(
    ys: &Yardsort,
    id: &str,
    workspace: Option<&str>,
    given: &[String],
    out: &Output,
) -> Result<(), Failure> {
    let workspace = place(ys, workspace)?;
    let mut inputs = BTreeMap::new();
    for pair in given {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(Failure::new(format!(
                "`--input {pair}` needs an `=`: `--input id=value`."
            )));
        };
        inputs.insert(key.trim().to_owned(), value.to_owned());
    }
    // The app carries runs out. Queued with nobody to run it, a run would only sit there.
    if !presence::app_running(&ys.data_dir) {
        return Err(Failure::new(
            "Yardsort is not running. Smoke tests run in the app: open it, then run this again. \
             Nothing was queued.",
        ));
    }
    // From an agent's terminal, the launch environment says which conversation asked.
    let record = std::env::var(yardsort_core::activity::RECORD_ENV)
        .ok()
        .filter(|v| !v.is_empty());
    let queued = runs::queue(
        &ys.store,
        &runs::Request {
            test_id: id,
            workspace_id: &workspace.id,
            inputs: &inputs,
            requested_by: "cli",
            session_record_id: record.as_deref(),
        },
    )?;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Queued<'a> {
        run: &'a str,
        test: &'a str,
        workspace: &'a str,
        /// The file has not been approved in Yardsort: the run waits for the user there.
        awaiting_approval: bool,
    }
    let answer = Queued {
        run: &queued.id,
        test: id,
        workspace: &workspace.name,
        awaiting_approval: queued.awaiting_approval,
    };
    out.emit(&answer, || {
        let short = memory::short_id(&queued.id);
        if queued.awaiting_approval {
            sayln!(
                "Queued `{id}` in {} as run {short}: waiting for you to approve it in Yardsort.",
                workspace.name
            );
        } else {
            sayln!("Queued `{id}` in {} as run {short}.", workspace.name);
        }
        sayln!("Follow it with: ys test wait {short}");
    })
}

/// Block until the run ends, then say how it went. The exit code is the verdict.
fn wait(ys: &Yardsort, wanted: &str, timeout: &str, out: &Output) -> Result<(), Failure> {
    let limit = yardsort_core::yaml::duration(timeout, "--timeout")
        .map(Duration::from_secs)
        .map_err(Failure::new)?;
    let mut run = find_run(ys, wanted)?;
    let started = Instant::now();
    while run.active() {
        if started.elapsed() >= limit {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
        run = ys
            .store
            .test_run(&run.id)?
            .ok_or_else(|| Failure::new("The run is gone."))?;
    }
    let steps = ys.store.test_steps(&run.id)?;
    let summary = report::summary(&run, &steps);
    let timed_out = run.active();

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Waited<'a> {
        #[serde(flatten)]
        run: RunListed,
        summary: &'a str,
        /// Still going when `--timeout` passed.
        timed_out: bool,
        steps: Vec<Step<'a>>,
    }
    let waited = Waited {
        run: RunListed::from(&run),
        summary: &summary,
        timed_out,
        steps: steps.iter().map(Step::from).collect(),
    };
    out.emit(&waited, || {
        sayln!("{summary}");
        if timed_out {
            sayln!(
                "  still {} after {timeout}; `ys test wait {}` waits again",
                run.status.replace('_', " "),
                memory::short_id(&run.id)
            );
        }
    })?;
    match report::exit_code(&run.status) {
        0 => Ok(()),
        code => Err(Failure::with_code(code, "")),
    }
}

/// A run as `runs` lists it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunListed {
    id: String,
    test: String,
    test_name: String,
    workspace: String,
    status: String,
    requested_by: String,
    port: Option<u16>,
    error: Option<String>,
    output_dir: Option<String>,
    created_at: i64,
    started_at: Option<i64>,
    ended_at: Option<i64>,
}

impl From<&TestRunRow> for RunListed {
    fn from(run: &TestRunRow) -> Self {
        Self {
            id: run.id.clone(),
            test: run.test_id.clone(),
            test_name: run.test_name.clone(),
            workspace: run.workspace_name.clone(),
            status: run.status.clone(),
            requested_by: run.requested_by.clone(),
            port: run.port,
            error: run.error.clone(),
            output_dir: run.output_dir.clone(),
            created_at: run.created_at,
            started_at: run.started_at,
            ended_at: run.ended_at,
        }
    }
}

/// A step as `runs --run` and `wait` print it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Step<'a> {
    id: &'a str,
    phase: &'a str,
    action: &'a str,
    status: &'a str,
    started_at: Option<i64>,
    ended_at: Option<i64>,
    exit: Option<i64>,
    outputs: BTreeMap<String, String>,
    note: Option<&'a str>,
    screenshots: Vec<String>,
}

impl<'a> From<&'a TestStepRow> for Step<'a> {
    fn from(s: &'a TestStepRow) -> Self {
        Self {
            id: &s.step_id,
            phase: &s.phase,
            action: &s.action,
            status: &s.status,
            started_at: s.started_at,
            ended_at: s.ended_at,
            exit: s.exit,
            outputs: s.outputs(),
            note: s.note.as_deref(),
            screenshots: s.screenshots(),
        }
    }
}

fn runs(ys: &Yardsort, workspace: Option<&str>, limit: usize, out: &Output) -> Result<(), Failure> {
    let workspace_id = match workspace {
        Some(wanted) => Some(super::workspace::find_workspace(ys, wanted)?.id),
        None => None,
    };
    let rows = ys.store.test_runs(workspace_id.as_deref(), limit)?;
    let listed: Vec<RunListed> = rows.iter().map(RunListed::from).collect();
    out.emit(&listed, || {
        if listed.is_empty() {
            sayln!("No smoke test runs yet.");
            return;
        }
        let mut lines = vec![vec![
            "RUN".to_owned(),
            "TEST".to_owned(),
            "WORKSPACE".to_owned(),
            "STATUS".to_owned(),
            "ASKED".to_owned(),
        ]];
        lines.extend(listed.iter().map(|run| {
            vec![
                memory::short_id(&run.id).to_owned(),
                run.test.clone(),
                run.workspace.clone(),
                run.status.replace('_', " "),
                when(run.created_at),
            ]
        }));
        table(&lines);
    })
}

fn show_run(ys: &Yardsort, wanted: &str, out: &Output) -> Result<(), Failure> {
    let run = find_run(ys, wanted)?;
    let steps = ys.store.test_steps(&run.id)?;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Shown<'a> {
        #[serde(flatten)]
        run: RunListed,
        inputs: BTreeMap<String, String>,
        steps: Vec<Step<'a>>,
    }
    let shown = Shown {
        run: RunListed::from(&run),
        inputs: serde_json::from_str(&run.inputs).unwrap_or_default(),
        steps: steps.iter().map(Step::from).collect(),
    };
    out.emit(&shown, || {
        sayln!("{}", report::headline(&run, &steps));
        sayln!(
            "  in {}, asked {} by {}{}",
            run.workspace_name,
            when(run.created_at),
            run.requested_by,
            run.port
                .map(|port| format!(", on port {port}"))
                .unwrap_or_default()
        );
        if let Some(error) = &run.error {
            sayln!("  {error}");
        }
        sayln!();
        let mut lines = vec![vec![
            "STEP".to_owned(),
            "PHASE".to_owned(),
            "ACTION".to_owned(),
            "STATUS".to_owned(),
            "NOTE".to_owned(),
        ]];
        lines.extend(steps.iter().map(|s| {
            vec![
                s.step_id.clone(),
                s.phase.clone(),
                s.action.clone(),
                s.status.clone(),
                s.note
                    .as_deref()
                    .and_then(|note| note.lines().next())
                    .unwrap_or_default()
                    .to_owned(),
            ]
        }));
        table(&lines);
    })
}

fn log(
    ys: &Yardsort,
    wanted: &str,
    step: Option<&str>,
    stderr: bool,
    out: &Output,
) -> Result<(), Failure> {
    let run = find_run(ys, wanted)?;
    let steps = ys.store.test_steps(&run.id)?;
    let chosen = match step {
        Some(id) => steps.iter().find(|s| s.step_id == id).ok_or_else(|| {
            Failure::new(format!(
                "Run {} has no step `{id}`. It has: {}.",
                memory::short_id(&run.id),
                steps
                    .iter()
                    .map(|s| s.step_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?,
        None => steps
            .iter()
            .find(|s| s.status == "failed")
            .or_else(|| steps.iter().rfind(|s| s.started_at.is_some()))
            .ok_or_else(|| {
                Failure::new(format!(
                    "Run {} has not run any step yet: it is {}.",
                    memory::short_id(&run.id),
                    run.status.replace('_', " ")
                ))
            })?,
    };
    let outputs = chosen.outputs();
    let stream = if stderr { "stderr" } else { "stdout" };

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Logged<'a> {
        run: &'a str,
        step: &'a str,
        action: &'a str,
        status: &'a str,
        /// The output file read, when the step has one.
        path: Option<&'a str>,
        text: String,
        outputs: &'a BTreeMap<String, String>,
        screenshots: Vec<String>,
    }
    let (path, body) = if let Some(path) = outputs.get(stream) {
        (
            Some(path.as_str()),
            std::fs::read_to_string(path).unwrap_or_default(),
        )
    } else if chosen.action == "http" && !stderr {
        (None, outputs.get("body").cloned().unwrap_or_default())
    } else {
        return Err(Failure::new(format!(
            "`{}` is {} step; it has no {stream}.",
            chosen.step_id,
            article(&chosen.action)
        )));
    };
    let logged = Logged {
        run: &run.id,
        step: &chosen.step_id,
        action: &chosen.action,
        status: &chosen.status,
        path,
        text: body.clone(),
        outputs: &outputs,
        screenshots: chosen.screenshots(),
    };
    out.emit(&logged, || {
        say!("{body}");
        if !body.is_empty() && !body.ends_with('\n') {
            sayln!();
        }
    })
}

/// `a run` or `an http`.
fn article(action: &str) -> String {
    if action.starts_with(['a', 'e', 'i', 'o', 'u', 'h']) {
        format!("an `{action}`")
    } else {
        format!("a `{action}`")
    }
}

fn cancel(ys: &Yardsort, wanted: &str, out: &Output) -> Result<(), Failure> {
    let run = find_run(ys, wanted)?;
    let was_running = run.status == "running";
    if !ys.store.cancel_test_run(&run.id, "Cancelled from ys.")? {
        return Err(Failure::new(if run.cancel_requested_at.is_some() {
            format!(
                "Run {} is already stopping; its teardown is running.",
                memory::short_id(&run.id)
            )
        } else {
            format!(
                "Run {} has already ended: {}.",
                memory::short_id(&run.id),
                run.status
            )
        }));
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Cancelled<'a> {
        run: &'a str,
        /// It was running: the step it was on is stopped, and the teardown runs first.
        stopping: bool,
    }
    out.emit(
        &Cancelled {
            run: &run.id,
            stopping: was_running,
        },
        || {
            if was_running {
                sayln!(
                    "Stopping run {}: its step is ended, its teardown runs, then it is cancelled.",
                    memory::short_id(&run.id)
                );
            } else {
                sayln!("Cancelled run {}.", memory::short_id(&run.id));
            }
        },
    )
}

/// A run by its id or the start of one, as `runs` prints it.
fn find_run(ys: &Yardsort, wanted: &str) -> Result<TestRunRow, Failure> {
    let wanted = wanted.trim();
    let matches: Vec<TestRunRow> = ys
        .store
        .test_runs(None, usize::MAX)?
        .into_iter()
        .filter(|run| !wanted.is_empty() && run.id.starts_with(wanted))
        .collect();
    match matches.len() {
        1 => Ok(matches.into_iter().next().unwrap_or_else(|| unreachable!())),
        0 => Err(Failure::new(format!(
            "No smoke test run starts with `{wanted}`. `ys test runs` lists them."
        ))),
        n => Err(Failure::new(format!(
            "{n} runs start with `{wanted}`. Give more of the id."
        ))),
    }
}

/// `file:line:column: message`, the shape editors and terminals turn into links.
fn located(name: &str, problem: &Problem) -> String {
    match problem.line {
        Some(_) => format!("{name}:{problem}"),
        None => format!("{name}: {problem}"),
    }
}

fn status(entry: &Entry) -> String {
    if entry.runnable() {
        "ready".to_owned()
    } else {
        count(entry.problems.len())
    }
}

fn count(problems: usize) -> String {
    match problems {
        1 => "1 problem".to_owned(),
        n => format!("{n} problems"),
    }
}
