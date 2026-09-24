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
        ///
        /// A message is free text, so it may start with `-` — a bullet pasted out of a list is
        /// the usual way — and must not be read as an option.
        #[arg(allow_hyphen_values = true)]
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
    /// Remove a workspace's folder and forget it. The branch, and its commits, are kept.
    ///
    /// Uncommitted changes and untracked files are refused unless `--force` is given: they live
    /// only in the folder this removes, so there is no way back to them.
    Delete {
        /// Which one: a workspace name, or its id from `ys workspace list --json`.
        workspace: String,
        /// Delete even when the folder holds work that is not committed.
        ///
        /// That work cannot be recovered. The branch is still kept.
        #[arg(long)]
        force: bool,
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
        Command::Delete { workspace, force } => delete(ys, &workspace, force, out),
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Deleted {
    id: String,
    name: String,
    project: String,
    /// Kept on purpose. Commits are never thrown away by deleting a workspace.
    branch: Option<String>,
    path: String,
}

/// Remove the worktree and the record. A running process is left running: a harness deletes the
/// workspace it is standing in by calling this, and has to be able to finish afterwards.
fn delete(ys: &Yardsort, wanted: &str, force: bool, out: &Output) -> Result<(), Failure> {
    let workspace = find_workspace(ys, wanted)?;
    if workspace.kind != "worktree" {
        return Err(Failure::new(
            "The local workspace is the project's own checkout and cannot be deleted.".to_owned(),
        ));
    }

    let git = ys.git()?;
    let worktree_root = ys.worktree_root()?;
    let result = Workspaces {
        store: &ys.store,
        git: &git,
        worktree_root: &worktree_root,
        settings: &ys.settings.workspaces,
    }
    .delete(&workspace.id, force);

    if let Err(error) = result {
        if error.code == "worktree_dirty" {
            return Err(Failure::new(format!(
                "{} has uncommitted changes or untracked files, so it was not deleted.\n\
                 Pass --force to delete it anyway. That work is in no commit and cannot be \
                 recovered.\n\
                 The branch is kept either way.",
                workspace.name
            )));
        }
        return Err(error.into());
    }

    let projects = ys.store.projects()?;
    let project = projects
        .iter()
        .find(|p| p.id == workspace.project_id)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "?".to_owned());
    let deleted = Deleted {
        id: workspace.id,
        name: workspace.name,
        project,
        branch: workspace.branch,
        path: workspace.path,
    };
    out.emit(&deleted, || {
        println!("deleted  {}", deleted.name);
        if let Some(branch) = &deleted.branch {
            println!("  branch {branch} (kept)");
        }
        println!("  path   {}", deleted.path);
    })
}

/// A workspace by id, or by name when that is unambiguous. Archived ones are included: deleting
/// is how an archived workspace is removed for good.
fn find_workspace(ys: &Yardsort, wanted: &str) -> Result<WorkspaceRow, Failure> {
    let workspaces = ys.store.workspaces()?;
    if let Some(exact) = workspaces.iter().find(|w| w.id == wanted) {
        return Ok(exact.clone());
    }
    let matches: Vec<_> = workspaces
        .iter()
        .filter(|w| w.name.eq_ignore_ascii_case(wanted))
        .collect();
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(Failure::new(format!(
            "No workspace called {wanted:?}.{}",
            known_workspaces(ys, &workspaces)?
        ))),
        many => {
            let projects = ys.store.projects()?;
            let named = |id: &str| {
                projects
                    .iter()
                    .find(|p| p.id == id)
                    .map_or("?", |p| p.name.as_str())
            };
            Err(Failure::new(format!(
                "{} workspaces are called {wanted:?}. Use the id instead: {}.",
                many.len(),
                many.iter()
                    .map(|w| format!("{} ({})", w.id, named(&w.project_id)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        }
    }
}

fn known_workspaces(ys: &Yardsort, workspaces: &[WorkspaceRow]) -> Result<String, Failure> {
    if workspaces.is_empty() {
        return Ok(" There are no workspaces yet.".to_owned());
    }
    let projects = ys.store.projects()?;
    let named = |id: &str| {
        projects
            .iter()
            .find(|p| p.id == id)
            .map_or("?", |p| p.name.as_str())
    };
    Ok(format!(
        " Known: {}.",
        workspaces
            .iter()
            .map(|w| format!("{} ({})", w.name, named(&w.project_id)))
            .collect::<Vec<_>>()
            .join(", ")
    ))
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
