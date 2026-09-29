//! `ys workflow …` — the workflows Yardsort knows, and checking a file before it is used.
//!
//! Workflows are YAML files in the profile's `workflows` folder, plus the ones built in. See
//! `yardsort_core::workflow` and docs/guide/workflows.md.

use std::io::Read;
use std::path::{Path, PathBuf};

use std::collections::BTreeMap;

use serde::Serialize;
use yardsort_core::memory;
use yardsort_core::presence;
use yardsort_core::store::{WorkflowRunRow, WorkspaceRow};
use yardsort_core::workflow::{self, runs, Entry, Problem, Source};

use super::activity::when;
use crate::{table, Failure, Output, Yardsort};

#[derive(clap::Subcommand)]
pub enum Command {
    /// The built-in workflows and yours, and whether each is ready to run.
    List,
    /// Print a workflow's file, and anything wrong with it.
    Show {
        /// The workflow's id, from `ys workflow list`.
        id: String,
    },
    /// Copy a workflow into your folder to edit. A built-in copied under its own id is then
    /// used instead of the built-in; delete the copy to go back.
    Copy {
        /// The workflow to copy, from `ys workflow list`.
        id: String,
        /// Give the copy an id of its own, so it sits beside the original.
        #[arg(long = "as", value_name = "NEW_ID")]
        as_id: Option<String>,
    },
    /// Run a workflow in a workspace. Yardsort must be open: the app carries runs out. Prints
    /// the run's id and returns at once; `ys workflow runs --run <id>` follows it.
    Run {
        /// The workflow, from `ys workflow list`.
        id: String,
        /// Which workspace, by name or id. Default: the one this is run in.
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
        /// An answer to one of the workflow's inputs. Repeat for each.
        #[arg(long = "input", value_name = "ID=VALUE")]
        inputs: Vec<String>,
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
    /// Stop a queued or running run. Agents it started keep running.
    Cancel {
        /// The run: its id, or the start of it.
        run: String,
    },
    /// Check a workflow file. Prints each problem with its line and column; exits 1 if there
    /// are any. Needs no Yardsort profile, so it works anywhere.
    Validate {
        /// The file to check, or `-` to read it from standard input.
        file: PathBuf,
    },
}

/// A workflow as `list` prints it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Listed<'a> {
    id: &'a str,
    name: &'a str,
    description: Option<&'a str>,
    source: &'a Source,
    runnable: bool,
    problems: &'a [Problem],
}

pub fn run(data_dir: Option<PathBuf>, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::Validate { file } => validate(&file, out),
        Command::List => {
            let ys = Yardsort::open(data_dir)?;
            list(&ys.data_dir, out)
        }
        Command::Show { id } => {
            let ys = Yardsort::open(data_dir)?;
            show(&ys.data_dir, &id, out)
        }
        Command::Run {
            id,
            workspace,
            inputs,
        } => {
            let ys = Yardsort::open(data_dir)?;
            start_run(&ys, &id, workspace.as_deref(), &inputs, out)
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
        Command::Cancel { run } => {
            let ys = Yardsort::open(data_dir)?;
            cancel(&ys, &run, out)
        }
        Command::Copy { id, as_id } => {
            let ys = Yardsort::open(data_dir)?;
            copy(&ys.data_dir, &id, as_id.as_deref(), out)
        }
    }
}

fn list(data_dir: &Path, out: &Output) -> Result<(), Failure> {
    let entries = workflow::catalog(data_dir);
    let listed: Vec<Listed> = entries
        .iter()
        .map(|e| Listed {
            id: &e.id,
            name: &e.name,
            description: e.description.as_deref(),
            source: &e.source,
            runnable: e.runnable(),
            problems: &e.problems,
        })
        .collect();
    out.emit(&listed, || {
        let mut rows = vec![vec![
            "ID".to_owned(),
            "NAME".to_owned(),
            "STATUS".to_owned(),
            "FROM".to_owned(),
        ]];
        rows.extend(
            entries
                .iter()
                .map(|e| vec![e.id.clone(), e.name.clone(), status(e), from(&e.source)]),
        );
        table(&rows);
        println!();
        println!(
            "Your workflows go in {}",
            workflow::user_dir(data_dir).display()
        );
    })
}

fn show(data_dir: &Path, id: &str, out: &Output) -> Result<(), Failure> {
    let Some(entry) = workflow::find(data_dir, id) else {
        let known: Vec<String> = workflow::catalog(data_dir)
            .into_iter()
            .map(|e| e.id)
            .collect();
        return Err(Failure::new(format!(
            "There is no workflow `{id}`. There are: {}.",
            known.join(", ")
        )));
    };
    out.emit(&entry, || {
        print!("{}", entry.text);
        if !entry.text.ends_with('\n') {
            println!();
        }
        if !entry.problems.is_empty() {
            let name = match &entry.source {
                Source::File { path, .. } => path.clone(),
                Source::BuiltIn => entry.id.clone(),
            };
            eprintln!();
            eprintln!("{} — it cannot run until it is fixed:", status(&entry));
            for problem in &entry.problems {
                eprintln!("{}", located(&name, problem));
            }
        }
    })
}

fn start_run(
    ys: &Yardsort,
    id: &str,
    workspace: Option<&str>,
    given: &[String],
    out: &Output,
) -> Result<(), Failure> {
    let workspace = match workspace {
        Some(wanted) => super::workspace::find_workspace(ys, wanted)?,
        None => here(ys)?,
    };
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
            "Yardsort is not running. Workflows run in the app: open it, then run this again. \
             Nothing was queued.",
        ));
    }
    let run_id = runs::queue(
        &ys.store,
        &ys.data_dir,
        &runs::Request {
            workflow_id: id,
            workspace_id: &workspace.id,
            inputs: &inputs,
            requested_by: "cli",
        },
        &Looking(ys),
    )?;

    // What the run will find nothing for: said, not refused.
    let empty = match (
        workflow::find(&ys.data_dir, id).and_then(|e| e.workflow),
        runs::Place::load(&ys.store, &workspace.id)?,
    ) {
        (Some(found), Some(place)) => {
            let shared = ys.store.memory_shared(&workspace.project_id)?;
            runs::empty_variables(&found, &place, shared)
        }
        _ => Vec::new(),
    };

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Queued<'a> {
        run: &'a str,
        workflow: &'a str,
        workspace: &'a str,
        /// Variables the run will render as nothing, and why.
        empty: &'a [String],
    }
    let queued = Queued {
        run: &run_id,
        workflow: id,
        workspace: &workspace.name,
        empty: &empty,
    };
    out.emit(&queued, || {
        let short = memory::short_id(&run_id);
        println!("Queued `{id}` in {} as run {short}.", workspace.name);
        for line in &empty {
            println!("Note: {line}");
        }
        println!("Follow it with: ys workflow runs --run {short}");
    })
}

/// What `ys` answers when a run is queued: agents from the settings and `PATH`, the pull
/// request from `gh`.
struct Looking<'a>(&'a Yardsort);

impl runs::Look for Looking<'_> {
    fn harness(&self, wanted: &str) -> Result<String, String> {
        super::workspace::choose_harness(self.0, Some(wanted)).map_err(Failure::into_message)
    }

    fn pull_request(
        &self,
        workspace: &WorkspaceRow,
    ) -> Result<Option<yardsort_core::forge::PullRequest>, String> {
        let git = self.0.git().map_err(Failure::into_message)?;
        let gh = yardsort_core::forge::Gh::find(self.0.env());
        runs::pull_request_of(gh.as_ref(), &git, workspace)
    }
}

/// The workspace this is run in: the agent's own, from its launch environment, or the folder.
fn here(ys: &Yardsort) -> Result<WorkspaceRow, Failure> {
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let cwd = std::env::current_dir().map_err(|e| Failure::new(format!("cwd: {e}")))?;
    let located = memory::locate(
        &ys.store,
        env(yardsort_core::activity::RUN_ENV).as_deref(),
        env(yardsort_core::activity::WORKSPACE_ENV).as_deref(),
        &cwd,
    )?;
    let id = located
        .and_then(|(_, source)| source.workspace_id)
        .ok_or_else(|| {
            Failure::new(
                "Not inside a Yardsort workspace. Run this in one, or name it with --workspace.",
            )
        })?;
    super::workspace::find_workspace(ys, &id)
}

/// A run as `runs` lists it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunListed {
    id: String,
    workflow: String,
    workspace: String,
    status: String,
    requested_by: String,
    created_at: i64,
    started_at: Option<i64>,
    ended_at: Option<i64>,
    error: Option<String>,
}

impl From<&WorkflowRunRow> for RunListed {
    fn from(run: &WorkflowRunRow) -> Self {
        Self {
            id: run.id.clone(),
            workflow: run.workflow_id.clone(),
            workspace: run.workspace_name.clone(),
            status: run.status.clone(),
            requested_by: run.requested_by.clone(),
            created_at: run.created_at,
            started_at: run.started_at,
            ended_at: run.ended_at,
            error: run.error.clone(),
        }
    }
}

fn runs(ys: &Yardsort, workspace: Option<&str>, limit: usize, out: &Output) -> Result<(), Failure> {
    let workspace_id = match workspace {
        Some(wanted) => Some(super::workspace::find_workspace(ys, wanted)?.id),
        None => None,
    };
    let rows = ys.store.workflow_runs(workspace_id.as_deref(), limit)?;
    let listed: Vec<RunListed> = rows.iter().map(RunListed::from).collect();
    out.emit(&listed, || {
        if listed.is_empty() {
            println!("No workflow runs yet.");
            return;
        }
        let mut lines = vec![vec![
            "RUN".to_owned(),
            "WORKFLOW".to_owned(),
            "WORKSPACE".to_owned(),
            "STATUS".to_owned(),
            "ASKED".to_owned(),
        ]];
        lines.extend(listed.iter().map(|run| {
            vec![
                memory::short_id(&run.id).to_owned(),
                run.workflow.clone(),
                run.workspace.clone(),
                run.status.clone(),
                when(run.created_at),
            ]
        }));
        table(&lines);
    })
}

fn show_run(ys: &Yardsort, wanted: &str, out: &Output) -> Result<(), Failure> {
    let run = find_run(ys, wanted)?;
    let steps = ys.store.workflow_steps(&run.id)?;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Step<'a> {
        id: &'a str,
        action: &'a str,
        status: &'a str,
        started_at: Option<i64>,
        ended_at: Option<i64>,
        outputs: serde_json::Value,
        note: Option<&'a str>,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Shown<'a> {
        #[serde(flatten)]
        run: RunListed,
        inputs: serde_json::Value,
        steps: Vec<Step<'a>>,
    }
    let shown = Shown {
        run: RunListed::from(&run),
        inputs: serde_json::from_str(&run.inputs).unwrap_or_default(),
        steps: steps
            .iter()
            .map(|s| Step {
                id: &s.step_id,
                action: &s.action,
                status: &s.status,
                started_at: s.started_at,
                ended_at: s.ended_at,
                outputs: serde_json::from_str(&s.outputs).unwrap_or_default(),
                note: s.note.as_deref(),
            })
            .collect(),
    };
    out.emit(&shown, || {
        println!(
            "Run {} of `{}` in {}: {}",
            memory::short_id(&run.id),
            run.workflow_id,
            run.workspace_name,
            run.status
        );
        if let Some(error) = &run.error {
            println!("{error}");
        }
        println!();
        let mut lines = vec![vec![
            "STEP".to_owned(),
            "ACTION".to_owned(),
            "STATUS".to_owned(),
            "NOTE".to_owned(),
        ]];
        lines.extend(steps.iter().map(|s| {
            vec![
                s.step_id.clone(),
                s.action.clone(),
                s.status.clone(),
                s.note.clone().unwrap_or_default(),
            ]
        }));
        table(&lines);
    })
}

fn cancel(ys: &Yardsort, wanted: &str, out: &Output) -> Result<(), Failure> {
    let run = find_run(ys, wanted)?;
    if !ys
        .store
        .cancel_workflow_run(&run.id, "Cancelled from ys.")?
    {
        return Err(Failure::new(format!(
            "Run {} has already ended: {}.",
            memory::short_id(&run.id),
            run.status
        )));
    }
    #[derive(Serialize)]
    struct Cancelled<'a> {
        run: &'a str,
    }
    out.emit(&Cancelled { run: &run.id }, || {
        println!(
            "Cancelled run {}. Agents it started keep running.",
            memory::short_id(&run.id)
        );
    })
}

/// A run by its id or the start of one, as `runs` prints it.
fn find_run(ys: &Yardsort, wanted: &str) -> Result<WorkflowRunRow, Failure> {
    let wanted = wanted.trim();
    let matches: Vec<WorkflowRunRow> = ys
        .store
        .workflow_runs(None, usize::MAX)?
        .into_iter()
        .filter(|run| !wanted.is_empty() && run.id.starts_with(wanted))
        .collect();
    match matches.len() {
        1 => Ok(matches.into_iter().next().expect("one")),
        0 => Err(Failure::new(format!(
            "No run {wanted:?}. `ys workflow runs` lists them."
        ))),
        n => Err(Failure::new(format!(
            "{n} runs start with {wanted:?}. Give more of the id."
        ))),
    }
}

fn copy(data_dir: &Path, id: &str, as_id: Option<&str>, out: &Output) -> Result<(), Failure> {
    let path = workflow::copy(data_dir, id, as_id)?;
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Copied {
        id: String,
        path: String,
    }
    let copied = Copied {
        id: as_id.unwrap_or(id).to_owned(),
        path: path.display().to_string(),
    };
    out.emit(&copied, || {
        println!("Copied to {}", copied.path);
        if as_id.is_none() {
            println!("It is used instead of the built-in. Delete it to go back.");
        }
        println!(
            "Check it after editing: ys workflow validate {}",
            copied.path
        );
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
        workflow::read(file).map_err(|problem| Failure::new(problem.message))?
    };

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Checked<'a> {
        valid: bool,
        id: Option<&'a str>,
        problems: &'a [Problem],
    }
    let (id, problems) = match workflow::parse(&text) {
        Ok(workflow) => (Some(workflow.id), Vec::new()),
        Err(invalid) => (invalid.id, invalid.problems),
    };
    let checked = Checked {
        valid: problems.is_empty(),
        id: id.as_deref(),
        problems: &problems,
    };
    if checked.valid {
        return out.emit(&checked, || {
            println!(
                "{name}: a valid workflow, `{}`.",
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

fn from(source: &Source) -> String {
    match source {
        Source::BuiltIn => "built in".to_owned(),
        Source::File {
            path,
            replaces_built_in: true,
        } => format!("{path} (instead of the built-in)"),
        Source::File { path, .. } => path.clone(),
    }
}
