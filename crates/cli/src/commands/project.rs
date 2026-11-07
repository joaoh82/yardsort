//! `ys project …`

use std::path::PathBuf;

use serde::Serialize;
use yardsort_core::projects::Projects;

use crate::{table, Failure, Output, Yardsort};

#[derive(clap::Subcommand)]
pub enum Command {
    /// List the repositories Yardsort knows about.
    List,
    /// Add a git repository as a project, as the app's Open project does: a folder inside one
    /// adds the repository's root. A folder that is not a repository is refused.
    Add {
        /// The repository, or a folder inside it.
        path: PathBuf,
    },
}

#[derive(Serialize)]
struct Project {
    id: String,
    name: String,
    path: String,
    workspaces: usize,
}

pub fn run(ys: &Yardsort, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::List => list(ys, out),
        Command::Add { path } => add(ys, &path, out),
    }
}

#[derive(Serialize)]
struct Added {
    id: String,
    name: String,
    path: String,
    already_known: bool,
}

fn add(ys: &Yardsort, path: &std::path::Path, out: &Output) -> Result<(), Failure> {
    // Relative to where `ys` was run, not to wherever the core resolves it.
    let path =
        std::path::absolute(path).map_err(|e| Failure::new(format!("{}: {e}", path.display())))?;
    let git = ys.git()?;
    let added = Projects {
        store: &ys.store,
        git: &git,
    }
    .open(&path, false)?;
    let shown = Added {
        id: added.project.id.clone(),
        name: added.project.name.clone(),
        path: added.project.root_path.clone(),
        already_known: added.already_known,
    };
    out.emit(&shown, || {
        if added.already_known {
            sayln!("{} is already a project, at {}.", shown.name, shown.path);
        } else {
            if added.opened_root_instead {
                sayln!(
                    "That folder is inside {}; added the repository.",
                    shown.path
                );
            }
            sayln!("Added {}, at {}.", shown.name, shown.path);
        }
    })
}

fn list(ys: &Yardsort, out: &Output) -> Result<(), Failure> {
    let workspaces = ys.store.workspaces()?;
    let projects: Vec<Project> = ys
        .store
        .projects()?
        .into_iter()
        .map(|p| Project {
            workspaces: workspaces
                .iter()
                .filter(|w| w.project_id == p.id && w.kind != "local")
                .count(),
            id: p.id,
            name: p.name,
            path: p.root_path,
        })
        .collect();

    out.emit(&projects, || {
        if projects.is_empty() {
            sayln!("No projects yet. Add one in the app, or with `ys project add <path>`.");
            return;
        }
        let mut rows = vec![vec![
            "NAME".to_owned(),
            "WORKSPACES".to_owned(),
            "PATH".to_owned(),
        ]];
        rows.extend(
            projects
                .iter()
                .map(|p| vec![p.name.clone(), p.workspaces.to_string(), p.path.clone()]),
        );
        table(&rows);
    })
}
