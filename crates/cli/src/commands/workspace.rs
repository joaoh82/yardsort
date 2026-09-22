//! `ys workspace …`

use pty_host::TermSize;
use serde::Serialize;
use yardsort_core::launch::{HarnessRequest, Launch, Launcher};
use yardsort_core::store::WorkspaceRow;
use yardsort_core::workspaces::Workspaces;

use crate::{table, Failure, Output, Yardsort};

/// A workspace started here has no window attached, so the size is only what the program is told
/// on its first draw. Wide enough that an agent's own formatting is not wrapped oddly before
/// anybody attaches; the app resizes it the moment somebody opens the tab.
const DETACHED_SIZE: TermSize = TermSize {
    cols: 120,
    rows: 30,
};

#[derive(clap::Subcommand)]
pub enum Command {
    /// List workspaces.
    List {
        /// Only this project's, by name or id.
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
        /// Include archived ones.
        #[arg(long)]
        all: bool,
    },
    /// Create a workspace — a branch and a worktree — and start an agent in it.
    New {
        /// The project to branch from, by name or id.
        project: String,
        /// The first message for the agent. Also names the branch.
        prompt: String,
        /// Branch to start from. Defaults to the repository's default branch.
        #[arg(long, value_name = "BRANCH")]
        base: Option<String>,
        /// Which harness to run. Defaults to the first one installed.
        #[arg(long, value_name = "ID")]
        harness: Option<String>,
        /// Model for the harness, if it takes one.
        #[arg(long, value_name = "MODEL")]
        model: Option<String>,
        /// Effort or thinking level, if the harness takes one.
        #[arg(long, value_name = "LEVEL")]
        effort: Option<String>,
        /// Only create the branch and worktree; start nothing.
        #[arg(long)]
        no_agent: bool,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Workspace {
    id: String,
    name: String,
    project: String,
    /// `local` is the project's own checkout, which every project has and nobody created;
    /// `worktree` is one Yardsort made. Scripts want to tell them apart.
    kind: String,
    branch: Option<String>,
    path: String,
    archived: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Created {
    id: String,
    name: String,
    branch: Option<String>,
    path: String,
    /// The PTY session running the agent, when one was started.
    session: Option<String>,
    harness: Option<String>,
}

pub fn run(ys: &Yardsort, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::List { project, all } => list(ys, project, all, out),
        Command::New {
            project,
            prompt,
            base,
            harness,
            model,
            effort,
            no_agent,
        } => new(
            ys, project, prompt, base, harness, model, effort, no_agent, out,
        ),
    }
}

fn list(ys: &Yardsort, project: Option<String>, all: bool, out: &Output) -> Result<(), Failure> {
    let only = project.map(|p| ys.project(&p)).transpose()?;
    let projects = ys.store.projects()?;
    let named = |id: &str| {
        projects
            .iter()
            .find(|p| p.id == id)
            .map_or("?", |p| p.name.as_str())
            .to_owned()
    };
    let workspaces: Vec<Workspace> = ys
        .store
        .workspaces()?
        .into_iter()
        .filter(|w| only.as_ref().is_none_or(|p| w.project_id == p.id))
        .filter(|w| all || !w.archived)
        .map(|w| Workspace {
            project: named(&w.project_id),
            id: w.id,
            name: w.name,
            kind: w.kind,
            branch: w.branch,
            path: w.path,
            archived: w.archived,
        })
        .collect();

    out.emit(&workspaces, || {
        if workspaces.is_empty() {
            println!("No workspaces.");
            return;
        }
        let mut rows = vec![vec![
            "PROJECT".to_owned(),
            "NAME".to_owned(),
            "BRANCH".to_owned(),
            "PATH".to_owned(),
        ]];
        rows.extend(workspaces.iter().map(|w| {
            vec![
                w.project.clone(),
                if w.archived {
                    format!("{} (archived)", w.name)
                } else {
                    w.name.clone()
                },
                // `local` is whatever branch you happen to have out there, which this does not
                // ask git for; the app's bottom bar is where that is shown.
                w.branch.clone().unwrap_or_else(|| "—".to_owned()),
                w.path.clone(),
            ]
        }));
        table(&rows);
    })
}

#[allow(clippy::too_many_arguments)]
fn new(
    ys: &Yardsort,
    project: String,
    prompt: String,
    base: Option<String>,
    harness: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    no_agent: bool,
    out: &Output,
) -> Result<(), Failure> {
    let project = ys.project(&project)?;
    // Pick the harness before making anything: a typo in `--harness` should not leave a branch
    // and a worktree behind.
    let harness = if no_agent {
        None
    } else {
        Some(choose_harness(ys, harness.as_deref())?)
    };

    let git = ys.git()?;
    let worktree_root = ys.worktree_root()?;
    let workspace: WorkspaceRow = Workspaces {
        store: &ys.store,
        git: &git,
        worktree_root: &worktree_root,
        settings: &ys.settings.workspaces,
    }
    .create(&project.id, base.as_deref(), &prompt)?;

    let mut created = Created {
        id: workspace.id.clone(),
        name: workspace.name.clone(),
        branch: workspace.branch.clone(),
        path: workspace.path.clone(),
        session: None,
        harness: harness.clone(),
    };

    if let Some(harness) = harness {
        let connected = ys.daemon_or_start();
        if !connected.status.running {
            // Without a daemon the agent would belong to this process and die when it returns,
            // which is the opposite of what this command is for.
            return Err(Failure::new(format!(
                "The workspace is ready at {}, but no terminal daemon could be started, so an \
                 agent would stop the moment this command returned.{}\nStart the agent from the \
                 app, or see {}.",
                workspace.path,
                connected
                    .status
                    .problem
                    .map(|p| format!("\n  {p}"))
                    .unwrap_or_default(),
                connected.status.log_path,
            )));
        }
        let session = Launcher {
            store: &ys.store,
            host: connected.host.as_ref(),
            env: ys.env(),
            harnesses: &ys.settings.harnesses,
        }
        .in_workspace(
            &workspace.id,
            Launch::Harness(HarnessRequest {
                id: harness,
                model,
                effort,
                prompt: Some(prompt),
            }),
            DETACHED_SIZE,
        )?;
        created.session = Some(session.id.0.clone());
    }

    out.emit(&created, || {
        println!("created  {}", created.name);
        if let Some(branch) = &created.branch {
            println!("  branch {branch}");
        }
        println!("  path   {}", created.path);
        match (&created.harness, &created.session) {
            (Some(harness), Some(session)) => {
                println!("  agent  {harness} ({})", short(session));
                println!("\nIt keeps working after this command returns. Open Yardsort to watch.");
            }
            _ => println!("\nNothing started. Open it in Yardsort to start an agent."),
        }
    })
}

/// The harness to run: the one asked for, or the first that is actually installed.
fn choose_harness(ys: &Yardsort, wanted: Option<&str>) -> Result<String, Failure> {
    let all = yardsort_core::harness::resolve_all(&ys.settings.harnesses);
    let cwd = std::env::current_dir().unwrap_or_default();
    let installed = |command: &str| ys.env().find_program(command, &cwd).is_some();

    if let Some(wanted) = wanted {
        let found = all
            .iter()
            .find(|r| r.def.id.eq_ignore_ascii_case(wanted))
            .ok_or_else(|| {
                Failure::new(format!(
                    "No harness called {wanted:?}. Configured: {}.",
                    all.iter()
                        .map(|r| r.def.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
        if !installed(&found.def.command) {
            return Err(Failure::new(format!(
                "{} is configured but `{}` is not on your PATH.",
                found.def.id, found.def.command
            )));
        }
        return Ok(found.def.id.clone());
    }

    all.iter()
        .find(|r| installed(&r.def.command))
        .map(|r| r.def.id.clone())
        .ok_or_else(|| {
            Failure::new(
                "No agent was found on your PATH. Install one, or name it with --harness."
                    .to_owned(),
            )
        })
}

/// Session ids are uuids; the first characters are enough to tell them apart when reading.
pub fn short(id: &str) -> String {
    id.chars().take(8).collect()
}
