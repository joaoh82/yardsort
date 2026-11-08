//! `ys event …` — custom events: a name raised in a workspace, which a trigger on
//! `custom.<name>` there hears. For what no built-in event covers: an agent's own milestone, a
//! smoke test's `run` step, a script.
//!
//! Raising one writes a row; the app, when it is open, hears it within a second, and one that
//! opens later hears it then. See `yardsort_core::trigger` and docs/guide/triggers.md.

use std::path::Path;

use serde::Serialize;
use yardsort_core::store::{NewCustomEvent, WorkspaceRow};
use yardsort_core::trigger::{self, CUSTOM_PREFIX};

use crate::{Failure, Output, Yardsort};

/// What a smoke test run's `run` step is told its run is.
const TEST_RUN_ENV: &str = "YS_RUN_ID";

#[derive(clap::Subcommand)]
pub enum Command {
    /// Raise a custom event in this workspace. A trigger on `custom.<name>` there starts what
    /// it starts.
    ///
    /// For what no built-in event covers: an agent's own milestone ("the migration is
    /// written"), a smoke test's `run` step, a script. Names are lowercase letters, digits, `-`
    /// and `_`.
    Emit {
        /// The event's name, without `custom.`: `migration-written`.
        name: String,
        /// The workspace, by name or id. Default: the one this is run in.
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
        /// Anything what it starts should know, in a sentence or two: `{{ event.note }}`.
        #[arg(long)]
        note: Option<String>,
    },
}

pub fn run(ys: &Yardsort, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::Emit {
            name,
            workspace,
            note,
        } => emit(ys, &name, workspace.as_deref(), note, out),
    }
}

/// What `emit` said.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Raised {
    event: String,
    workspace: String,
    workspace_id: String,
    head: Option<String>,
    note: Option<String>,
    /// Whether an enabled trigger there listens for it.
    heard: bool,
    /// The custom events triggers there do listen for, when this is not one of them.
    listened_for: Vec<String>,
}

fn emit(
    ys: &Yardsort,
    name: &str,
    workspace: Option<&str>,
    note: Option<String>,
    out: &Output,
) -> Result<(), Failure> {
    // `custom.x` is what a trigger is written with; the name alone is what is raised.
    let name = name.strip_prefix(CUSTOM_PREFIX).unwrap_or(name);
    if let Some(why) = trigger::custom_name_problem(name) {
        return Err(Failure::new(why));
    }
    let env = |variable: &str| std::env::var(variable).ok().filter(|v| !v.is_empty());
    // From a smoke test's `run` step: that run, which also says where.
    let test_run = env(TEST_RUN_ENV)
        .map(|id| ys.store.test_run(&id))
        .transpose()?
        .flatten();
    let workspace: WorkspaceRow = match (workspace, &test_run) {
        (Some(wanted), _) => super::workspace::find_workspace(ys, wanted)?,
        (None, Some(run)) if run.workspace_id.is_some() => {
            super::workspace::find_workspace(ys, run.workspace_id.as_deref().unwrap_or_default())?
        }
        (None, _) => super::workflow::here(ys)?,
    };
    let run_id = test_run
        .filter(|run| run.workspace_id.as_deref() == Some(workspace.id.as_str()))
        .map(|run| run.id);
    // From an agent's terminal, the launch environment says which conversation raised it.
    let record = env(yardsort_core::activity::RECORD_ENV).filter(|record| {
        ys.store
            .session(record)
            .ok()
            .flatten()
            .is_some_and(|session| session.workspace_id == workspace.id)
    });
    let head = ys.git().ok().and_then(|git| {
        git.commit_id(Path::new(&workspace.path), "HEAD")
            .ok()
            .flatten()
    });
    let note = note.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
    ys.store.add_custom_event(&NewCustomEvent {
        workspace_id: &workspace.id,
        session_id: record.as_deref(),
        run_id: run_id.as_deref(),
        name,
        head: head.as_deref(),
        note: note.as_deref(),
    })?;
    let heard_names = trigger::custom_names_heard(&ys.store, &workspace.id)?;
    let heard = heard_names.contains(name);
    let event = format!("{CUSTOM_PREFIX}{name}");
    let raised = Raised {
        event: event.clone(),
        workspace: workspace.name.clone(),
        workspace_id: workspace.id.clone(),
        head,
        note,
        heard,
        listened_for: if heard {
            Vec::new()
        } else {
            heard_names
                .iter()
                .map(|name| format!("{CUSTOM_PREFIX}{name}"))
                .collect()
        },
    };
    out.emit(&raised, || {
        sayln!("Raised `{event}` in {}.", workspace.name);
        if !heard {
            // A name nobody declared is the usual mistake: say what is listened for instead.
            match raised.listened_for.as_slice() {
                [] => sayln!(
                    "No trigger in {} listens for it, so nothing starts.",
                    workspace.name
                ),
                names => sayln!(
                    "No trigger in {} listens for it, so nothing starts. They listen for: {}.",
                    workspace.name,
                    names
                        .iter()
                        .map(|name| format!("`{name}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        } else if !yardsort_core::presence::app_running(&ys.data_dir) {
            sayln!("Yardsort is not running. What listens for it starts when it opens.");
        }
    })
}
