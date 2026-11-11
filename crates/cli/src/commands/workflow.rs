//! `ys workflow …` — the workflows Yardsort knows, and checking a file before it is used.
//!
//! Workflows are YAML files in the profile's `workflows` folder, plus the ones built in. See
//! `yardsort_core::workflow` and docs/guide/workflows.md.

use std::io::{BufRead, IsTerminal, Read};
use std::path::{Path, PathBuf};

use std::collections::BTreeMap;

use serde::Serialize;
use yardsort_core::memory;
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
    /// Approve a step that is waiting for you. The app applies it on its next tick.
    Approve {
        /// The run: its id, or the start of it.
        run: String,
        /// The step's id.
        step: String,
        /// A note stored with the decision.
        #[arg(long)]
        feedback: Option<String>,
        /// The code Yardsort showed you. Leave this out to type it with echo off, in a real
        /// console (cmd, PowerShell, or Windows Terminal on Windows). `-` reads one line
        /// from standard input. Passing the code here puts it in your shell history and in
        /// the process list. Not printed by `ys workflow runs`.
        #[arg(long, value_name = "CODE")]
        code: Option<String>,
    },
    /// Reject a step that is waiting for you. The run stops. This is not an approval.
    Reject {
        /// The run: its id, or the start of it.
        run: String,
        /// The step's id.
        step: String,
        /// A note stored with the decision.
        #[arg(long)]
        feedback: Option<String>,
        /// The code Yardsort showed you. Leave this out to type it with echo off, in a real
        /// console (cmd, PowerShell, or Windows Terminal on Windows). `-` reads one line
        /// from standard input. Passing the code here puts it in your shell history and in
        /// the process list. Not printed by `ys workflow runs`.
        #[arg(long, value_name = "CODE")]
        code: Option<String>,
    },
    /// Ask for changes on a step that is waiting for you. The run stops, and the note is kept
    /// for a later revision. `--feedback` is required.
    RequestChanges {
        /// The run: its id, or the start of it.
        run: String,
        /// The step's id.
        step: String,
        /// What should change. Required.
        #[arg(long)]
        feedback: Option<String>,
        /// The code Yardsort showed you. Leave this out to type it with echo off, in a real
        /// console (cmd, PowerShell, or Windows Terminal on Windows). `-` reads one line
        /// from standard input. Passing the code here puts it in your shell history and in
        /// the process list. Not printed by `ys workflow runs`.
        #[arg(long, value_name = "CODE")]
        code: Option<String>,
    },
    /// Ask the person a question from inside a workflow session. This is how an agent asks.
    /// A question only printed in the terminal is not asked.
    Ask {
        /// The workflow run, when this is not the session the run started. The start of the id.
        #[arg(long, value_name = "RUN")]
        run: Option<String>,
        /// The clarifying step, when more than one session could match.
        #[arg(long, value_name = "STEP")]
        step: Option<String>,
        /// One question. Repeat for each question in this round.
        #[arg(long = "question", required = true)]
        questions: Vec<String>,
    },
    /// Record the outcome of a clarifying step. The next phase reads this in the brief.
    /// Text printed in the terminal is not the summary.
    Summary {
        /// The workflow run, when this is not the session the run started.
        #[arg(long, value_name = "RUN")]
        run: Option<String>,
        /// The clarifying step, when more than one session could match.
        #[arg(long, value_name = "STEP")]
        step: Option<String>,
        /// The outcome, in the agent's own words.
        #[arg(long)]
        text: String,
    },
    /// Answer a clarifying step that is waiting. A session Yardsort started cannot answer.
    /// The one-time code is required, the same way an approval is decided.
    Answer {
        /// The run: its id, or the start of it.
        run: String,
        /// The step's id.
        step: String,
        /// The answer. One reply covers every question in the round.
        #[arg(long)]
        text: String,
        /// The code Yardsort showed you. Leave this out to type it with echo off, in a real
        /// console (cmd, PowerShell, or Windows Terminal on Windows). `-` reads one line
        /// from standard input. Passing the code here puts it in your shell history and in
        /// the process list. Not printed by `ys workflow runs`.
        #[arg(long, value_name = "CODE")]
        code: Option<String>,
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
        Command::Approve {
            run,
            step,
            feedback,
            code,
        } => {
            let ys = Yardsort::open(data_dir)?;
            decide(
                &ys,
                &run,
                &step,
                "approved",
                feedback.as_deref(),
                code.as_deref(),
                out,
            )
        }
        Command::Reject {
            run,
            step,
            feedback,
            code,
        } => {
            let ys = Yardsort::open(data_dir)?;
            decide(
                &ys,
                &run,
                &step,
                "rejected",
                feedback.as_deref(),
                code.as_deref(),
                out,
            )
        }
        Command::Ask {
            run,
            step,
            questions,
        } => {
            let ys = Yardsort::open(data_dir)?;
            ask(&ys, run.as_deref(), step.as_deref(), &questions, out)
        }
        Command::Summary { run, step, text } => {
            let ys = Yardsort::open(data_dir)?;
            summarise(&ys, run.as_deref(), step.as_deref(), &text, out)
        }
        Command::Answer {
            run,
            step,
            text,
            code,
        } => {
            let ys = Yardsort::open(data_dir)?;
            answer(&ys, &run, &step, &text, code.as_deref(), out)
        }
        Command::RequestChanges {
            run,
            step,
            feedback,
            code,
        } => {
            let ys = Yardsort::open(data_dir)?;
            decide(
                &ys,
                &run,
                &step,
                "changes_requested",
                feedback.as_deref(),
                code.as_deref(),
                out,
            )
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
        sayln!();
        sayln!(
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
        say!("{}", entry.text);
        if !entry.text.ends_with('\n') {
            sayln!();
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
    super::someone_carries_it_out(
        ys,
        "Yardsort is not running. Workflows run in the app: open it, then run this again. \
             Nothing was queued.",
    )?;
    let run_id = runs::queue(
        &ys.store,
        &ys.data_dir,
        &runs::Request {
            workflow_id: id,
            workspace_id: &workspace.id,
            inputs: &inputs,
            requested_by: "cli",
            unattended: false,
            origin_record: None,
            depth: 0,
            event: None,
            tracked: None,
        },
        &Looking(ys),
    )?;

    // What the run will find nothing for: said, not refused.
    let empty = match (
        workflow::find(&ys.data_dir, id).and_then(|e| e.workflow),
        runs::Place::load(&ys.store, &workspace.id, &ys.git()?)?,
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
        sayln!("Queued `{id}` in {} as run {short}.", workspace.name);
        for line in &empty {
            sayln!("Note: {line}");
        }
        sayln!("Follow it with: ys workflow runs --run {short}");
    })
}

/// What `ys` answers when a run is queued: agents from the settings and `PATH`, the pull
/// request from `gh`.
pub(crate) struct Looking<'a>(pub(crate) &'a Yardsort);

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

    fn watch_pull_request(
        &self,
        workspace: &WorkspaceRow,
        number: Option<u32>,
    ) -> Result<Option<yardsort_core::forge::PrLook>, String> {
        let git = self.0.git().map_err(Failure::into_message)?;
        let gh = yardsort_core::forge::Gh::find(self.0.env());
        yardsort_core::trigger::watch::watch_pull_request_of(gh.as_ref(), &git, workspace, number)
    }

    fn watch_tracked_pull_request(
        &self,
        workspace: &WorkspaceRow,
        tracked: &yardsort_core::store::TrackedPullRequestRow,
    ) -> Result<yardsort_core::forge::PrLook, String> {
        let gh = yardsort_core::forge::Gh::find(self.0.env());
        yardsort_core::trigger::watch::watch_tracked_of(
            gh.as_ref(),
            &self.0.store,
            workspace,
            tracked,
        )
    }
}

/// The workspace this is run in: the agent's own, from its launch environment, or the folder.
pub(crate) fn here(ys: &Yardsort) -> Result<WorkspaceRow, Failure> {
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
    /// Why the run is blocked on a person, as in "your approval". Absent when it is not.
    #[serde(skip_serializing_if = "Option::is_none")]
    waiting_on: Option<String>,
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
            waiting_on: None,
        }
    }
}

fn runs(ys: &Yardsort, workspace: Option<&str>, limit: usize, out: &Output) -> Result<(), Failure> {
    let workspace_id = match workspace {
        Some(wanted) => Some(super::workspace::find_workspace(ys, wanted)?.id),
        None => None,
    };
    let rows = ys.store.workflow_runs(workspace_id.as_deref(), limit)?;
    let open = workflow::waits::open_by_run(&ys.store).unwrap_or_default();
    let listed: Vec<RunListed> = rows
        .iter()
        .map(|run| {
            let mut listed = RunListed::from(run);
            listed.waiting_on = workflow::waits::label_for(&ys.store, &run.id, &open)
                .ok()
                .flatten();
            listed
        })
        .collect();
    out.emit(&listed, || {
        if listed.is_empty() {
            sayln!("No workflow runs yet.");
            return;
        }
        let mut lines = vec![vec![
            "RUN".to_owned(),
            "WORKFLOW".to_owned(),
            "WORKSPACE".to_owned(),
            "STATUS".to_owned(),
            "WAITING ON".to_owned(),
            "ASKED".to_owned(),
        ]];
        lines.extend(listed.iter().map(|run| {
            vec![
                memory::short_id(&run.id).to_owned(),
                run.workflow.clone(),
                run.workspace.clone(),
                run.status.clone(),
                run.waiting_on.clone().unwrap_or_default(),
                when(run.created_at),
            ]
        }));
        table(&lines);
        for run in listed.iter().filter(|run| run.status == "failed") {
            let Some(error) = run.error.as_deref().filter(|text| !text.is_empty()) else {
                continue;
            };
            sayln!("{}: {error}", memory::short_id(&run.id));
        }
    })
}

fn show_run(ys: &Yardsort, wanted: &str, out: &Output) -> Result<(), Failure> {
    let run = find_run(ys, wanted)?;
    let steps = ys.store.workflow_steps(&run.id)?;
    let artifacts = ys.store.workflow_artifacts(&run.id)?;

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
        /// Passed or failed, with a reason when it failed. Absent when the step judged nothing.
        #[serde(skip_serializing_if = "Option::is_none")]
        outcome: Option<serde_json::Value>,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Artifact<'a> {
        name: &'a str,
        step_id: &'a str,
        source: &'a str,
        detail: Option<&'a str>,
        body: &'a str,
        truncated: bool,
        /// Which entry of the producing step wrote this body. Older entries stay stored.
        attempt: u32,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Pending<'a> {
        step: &'a str,
        prompt: Option<String>,
        artifacts: Vec<String>,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Decision<'a> {
        step: &'a str,
        decision: String,
        feedback: Option<String>,
        actor: String,
        decided_at: i64,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Shown<'a> {
        #[serde(flatten)]
        run: RunListed,
        inputs: serde_json::Value,
        steps: Vec<Step<'a>>,
        artifacts: Vec<Artifact<'a>>,
        /// Approval steps still waiting, with no decision recorded yet.
        pending_approvals: Vec<Pending<'a>>,
        /// Decisions recorded on this run, including one the app has not applied yet.
        decisions: Vec<Decision<'a>>,
    }
    let parsed = workflow::parse(&run.definition).ok();
    let consumes_of = |step_id: &str| -> Vec<String> {
        parsed
            .as_ref()
            .and_then(|workflow| workflow.steps.iter().find(|step| step.id == step_id))
            .map(|step| step.consumes.clone())
            .unwrap_or_default()
    };
    let mut pending_approvals = Vec::new();
    let mut decisions = Vec::new();
    for step in &steps {
        let recorded = ys.store.workflow_approval(&run.id, &step.step_id)?;
        if step.action == "approve" && step.status == "waiting" && recorded.is_none() {
            let prompt = yardsort_core::workflow::engine::parse_outputs(&step.outputs)
                .get("prompt")
                .cloned();
            pending_approvals.push(Pending {
                step: &step.step_id,
                prompt,
                artifacts: consumes_of(&step.step_id),
            });
        }
        if let Some(recorded) = recorded {
            decisions.push(Decision {
                step: &step.step_id,
                decision: recorded.decision,
                feedback: recorded.feedback,
                actor: recorded.actor,
                decided_at: recorded.decided_at,
            });
        }
    }
    let pending_lines: Vec<(String, Option<String>, String)> = pending_approvals
        .iter()
        .map(|pending| {
            (
                pending.step.to_owned(),
                pending.prompt.clone(),
                pending.artifacts.join(", "),
            )
        })
        .collect();
    let decision_lines: Vec<(String, String, String, Option<String>, i64)> = decisions
        .iter()
        .map(|decision| {
            (
                decision.step.to_owned(),
                decision.decision.clone(),
                decision.actor.clone(),
                decision.feedback.clone(),
                decision.decided_at,
            )
        })
        .collect();
    let mut listed = RunListed::from(&run);
    listed.waiting_on = workflow::waits::waiting_on(&ys.store, &run.id)
        .ok()
        .flatten();
    let shown = Shown {
        run: listed,
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
                outcome: s
                    .outcome
                    .as_deref()
                    .and_then(|raw| serde_json::from_str(raw).ok()),
            })
            .collect(),
        pending_approvals,
        decisions,
        artifacts: artifacts
            .iter()
            .map(|artifact| Artifact {
                name: &artifact.name,
                step_id: &artifact.step_id,
                source: &artifact.source,
                detail: artifact.detail.as_deref(),
                body: &artifact.body,
                truncated: artifact.truncated,
                attempt: artifact.attempt,
            })
            .collect(),
    };
    let questions: Vec<(String, Option<String>)> = steps
        .iter()
        .filter_map(|step| {
            let wait = ys.store.workflow_wait(&run.id, &step.step_id).ok()??;
            if wait.kind != workflow::waits::KIND_QUESTION
                || wait.answer.is_some()
                || wait.applied_at.is_some()
            {
                return None;
            }
            Some((step.step_id.clone(), wait.prompt))
        })
        .collect();
    out.emit(&shown, || {
        sayln!(
            "Run {} of `{}` in {}: {}",
            memory::short_id(&run.id),
            run.workflow_id,
            run.workspace_name,
            run.status
        );
        if let Some(error) = &run.error {
            sayln!("{error}");
        }
        sayln!();
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
                note_line(s.note.as_deref()),
            ]
        }));
        table(&lines);
        // The table keeps one line. A report that continues past it is printed in full,
        // so `ys workflow runs --run` still shows what the step recorded.
        for step in &steps {
            let Some(note) = step.note.as_deref() else {
                continue;
            };
            if note.lines().nth(1).is_some() {
                sayln!();
                sayln!("{}:", step.step_id);
                sayln!("{note}");
            }
        }
        // Only when the run handed something on, so a run with none prints what it did before.
        if !artifacts.is_empty() {
            sayln!();
            sayln!("Artifacts");
            for artifact in &artifacts {
                let cut = if artifact.truncated {
                    ", cut short"
                } else {
                    ""
                };
                match &artifact.detail {
                    Some(detail) => {
                        sayln!(
                            "{} from {} ({}, attempt {}, {detail}{cut})",
                            artifact.name,
                            artifact.step_id,
                            artifact.source,
                            artifact.attempt
                        );
                    }
                    None => sayln!(
                        "{} from {} ({}, attempt {}{cut})",
                        artifact.name,
                        artifact.step_id,
                        artifact.source,
                        artifact.attempt
                    ),
                }
                sayln!("{}", artifact.body);
            }
        }
        if !pending_lines.is_empty() {
            sayln!();
            sayln!("Blocked: waiting for your approval.");
            for (step, prompt, artifacts) in &pending_lines {
                sayln!("  {step}");
                if let Some(prompt) = prompt {
                    sayln!("  {prompt}");
                }
                if !artifacts.is_empty() {
                    sayln!("  Artifacts: {artifacts}");
                }
                sayln!("  Approve, reject, or request changes.");
                sayln!(
                    "  Decide in Yardsort, or with `ys workflow approve`, `reject` or \
                     `request-changes` and the code Yardsort showed you."
                );
            }
        }
        for (step_id, prompt) in &questions {
            sayln!();
            sayln!("Blocked: waiting for your answer.");
            sayln!("  {step_id}");
            if let Some(prompt) = prompt {
                sayln!("  {prompt}");
            }
            sayln!(
                "  Answer in Yardsort, or with `ys workflow answer {} {step_id} --text` \
                 and the code Yardsort showed you.",
                memory::short_id(&run.id)
            );
        }
        if !decision_lines.is_empty() {
            sayln!();
            sayln!("Decisions");
            for (step, decision, actor, feedback, at) in &decision_lines {
                let word = match decision.as_str() {
                    "approved" => "Approved",
                    "rejected" => "Rejected",
                    "changes_requested" => "Changes requested",
                    other => other,
                };
                sayln!("  {step}: {word} by {actor} at {}.", when(*at));
                if let Some(feedback) = feedback {
                    sayln!("  {feedback}");
                }
            }
        }
    })
}

/// The NOTE cell: the first line only, so a multi-line report does not break the table.
fn note_line(note: Option<&str>) -> String {
    note.and_then(|text| text.lines().next())
        .unwrap_or("")
        .to_owned()
}

fn ask(
    ys: &Yardsort,
    run: Option<&str>,
    step: Option<&str>,
    questions: &[String],
    out: &Output,
) -> Result<(), Failure> {
    let (run_id, step_id) = clarifying_step(ys, run, step)?;
    let written = workflow::clarify::ask(&ys.store, &run_id, &step_id, questions)?;
    if let Some(message) =
        workflow::clarify::ask_refusal(&written, memory::short_id(&run_id), &step_id)
    {
        return Err(Failure::new(message));
    }
    let round = match written {
        workflow::clarify::AskWrite::Recorded { round } => round,
        _ => 0,
    };
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Asked<'a> {
        run: &'a str,
        step: &'a str,
        round: u32,
    }
    out.emit(
        &Asked {
            run: &run_id,
            step: &step_id,
            round,
        },
        || sayln!("Asked {step_id} (round {round}). The run waits for an answer."),
    )
}

fn summarise(
    ys: &Yardsort,
    run: Option<&str>,
    step: Option<&str>,
    text: &str,
    out: &Output,
) -> Result<(), Failure> {
    let (run_id, step_id) = clarifying_step(ys, run, step)?;
    let written = workflow::clarify::summary(&ys.store, &run_id, &step_id, text)?;
    if let Some(message) =
        workflow::clarify::summary_refusal(&written, memory::short_id(&run_id), &step_id)
    {
        return Err(Failure::new(message));
    }
    #[derive(Serialize)]
    struct Recorded<'a> {
        run: &'a str,
        step: &'a str,
    }
    out.emit(
        &Recorded {
            run: &run_id,
            step: &step_id,
        },
        || sayln!("Recorded the summary for {step_id}."),
    )
}

fn answer(
    ys: &Yardsort,
    wanted: &str,
    step: &str,
    text: &str,
    code: Option<&str>,
    out: &Output,
) -> Result<(), Failure> {
    if let Some(why) = launched_here() {
        return Err(Failure::new(why));
    }
    let code = read_one_time_code(code, "Answer code: ")?;
    let run = find_run(ys, wanted)?;
    let written =
        workflow::waits::answer_with_code(&ys.store, &run.id, step, text, &cli_actor(), &code)?;
    if let Some(message) =
        workflow::clarify::answer_refusal(&written, memory::short_id(&run.id), step)
    {
        return Err(Failure::new(message));
    }
    #[derive(Serialize)]
    struct Answered<'a> {
        run: &'a str,
        step: &'a str,
    }
    out.emit(&Answered { run: &run.id, step }, || {
        sayln!("Answered {step}. The same session receives it.")
    })
}

/// The clarifying step: the one this session was started for, or `--run` and `--step`.
fn clarifying_step(
    ys: &Yardsort,
    run: Option<&str>,
    step: Option<&str>,
) -> Result<(String, String), Failure> {
    if let Some(run) = run {
        let found = find_run(ys, run)?;
        let step = step.ok_or_else(|| Failure::new("Name the step with --step."))?;
        return Ok((found.id, step.to_owned()));
    }
    let activity = std::env::var("YARDSORT_RUN_ID").unwrap_or_default();
    match workflow::clarify::locate(&ys.store, activity.trim(), step)? {
        Ok(found) => Ok((found.run_id, found.step_id)),
        Err(workflow::clarify::LocateMiss::None) => Err(Failure::new(
            "This is not a workflow session that is asking. Pass --run and --step.",
        )),
        Err(workflow::clarify::LocateMiss::Many(n)) => Err(Failure::new(format!(
            "{n} steps could be this session. Pass --step."
        ))),
    }
}

fn decide(
    ys: &Yardsort,
    wanted: &str,
    step: &str,
    decision: &str,
    feedback: Option<&str>,
    code: Option<&str>,
    out: &Output,
) -> Result<(), Failure> {
    if let Some(why) = launched_here() {
        return Err(Failure::new(why));
    }
    let code = read_one_time_code(code, "Approval code: ")?;
    let run = find_run(ys, wanted)?;
    let feedback = feedback.map(str::trim).filter(|text| !text.is_empty());
    let actor = cli_actor();
    let written = workflow::waits::decide_with_code(
        &ys.store, &run.id, step, decision, feedback, &actor, &code,
    )?;
    if let Some(message) = workflow::approval_refusal(&written, memory::short_id(&run.id), step) {
        return Err(Failure::new(message));
    }
    #[derive(Serialize)]
    struct Decided<'a> {
        run: &'a str,
        step: &'a str,
        decision: &'a str,
    }
    let word = match decision {
        "approved" => "Approved",
        "rejected" => "Rejected",
        _ => "Recorded the requested changes for",
    };
    out.emit(
        &Decided {
            run: &run.id,
            step,
            decision,
        },
        || {
            sayln!("{word} `{step}` on run {}.", memory::short_id(&run.id));
        },
    )
}

/// The code for a decision or an answer. Left out, it is typed at the terminal with echo
/// off. `-` reads one line from standard input. Anything else was passed on the command
/// line, which keeps the code in shell history and in the process list, so that form warns.
fn read_one_time_code(code: Option<&str>, prompt: &str) -> Result<String, Failure> {
    match code.map(str::trim).filter(|text| !text.is_empty()) {
        None => prompt_one_time_code(prompt),
        Some("-") => stdin_decision_code(),
        Some(given) => {
            eprintln!(
                "warning: --code puts the code in your shell history and in the \
                 process list. Leave --code out, or pass --code - and the code on standard \
                 input."
            );
            Ok(given.to_owned())
        }
    }
}

fn prompt_one_time_code(prompt: &str) -> Result<String, Failure> {
    // Both streams: rpassword opens the console (CONIN$ on Windows), not stdin, so a
    // script or a scheduled task that merely has a console would wait forever.
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        return Err(Failure::new(
            "No terminal to ask for the code. Pass --code - and pipe it in.",
        ));
    }
    match rpassword::prompt_password(prompt) {
        Ok(code) => {
            let code = code.trim().to_owned();
            if code.is_empty() {
                Err(Failure::new("Say the code Yardsort showed you."))
            } else {
                Ok(code)
            }
        }
        Err(_) => Err(Failure::new(
            "No terminal to ask for the code. Pass --code - and pipe it in.",
        )),
    }
}

fn stdin_decision_code() -> Result<String, Failure> {
    let mut raw = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut raw)
        .map_err(|error| Failure::new(error.to_string()))?;
    let code = raw.trim_start_matches('\u{FEFF}').trim().to_owned();
    if code.is_empty() {
        Err(Failure::new(
            "Say the code Yardsort showed you on standard input.",
        ))
    } else {
        Ok(code)
    }
}

/// A session Yardsort started, or the Floor Manager, must not decide. Unsetting one variable
/// is not enough: any of these means this process was launched from inside Yardsort.
fn launched_here() -> Option<&'static str> {
    const NAMES: &[&str] = &[
        yardsort_core::coordinator::ENV,
        yardsort_core::activity::RUN_ENV,
        yardsort_core::activity::WORKSPACE_ENV,
        yardsort_core::activity::RECORD_ENV,
    ];
    let inside = NAMES.iter().any(|name| {
        std::env::var(name)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
    });
    inside.then_some(
        "This ys is running inside a session Yardsort started, so it cannot decide an \
         approval or answer a question. Do that in the app, or from a terminal Yardsort did \
         not start.",
    )
}

/// The account `ys` is running as. The window records `app` instead.
fn cli_actor() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "cli".to_owned())
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
        sayln!(
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
        sayln!("Copied to {}", copied.path);
        if as_id.is_none() {
            sayln!("It is used instead of the built-in. Delete it to go back.");
        }
        sayln!(
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
            sayln!(
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

#[cfg(test)]
mod tests {
    use super::note_line;

    #[test]
    fn the_note_cell_is_the_first_line_of_the_report() {
        assert_eq!(note_line(None), "");
        assert_eq!(note_line(Some("The test failed.")), "The test failed.");
        assert_eq!(
            note_line(Some("The test failed:\nwanted 200, got 500")),
            "The test failed:"
        );
    }
}
