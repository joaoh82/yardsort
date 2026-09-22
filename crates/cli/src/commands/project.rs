//! `ys project …`

use serde::Serialize;

use crate::{table, Failure, Output, Yardsort};

#[derive(clap::Subcommand)]
pub enum Command {
    /// List the repositories Yardsort knows about.
    List,
}

#[derive(Serialize)]
struct Project {
    id: String,
    name: String,
    path: String,
    workspaces: usize,
}

pub fn run(ys: &Yardsort, command: Command, out: &Output) -> Result<(), Failure> {
    let Command::List = command;
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
            println!("No projects yet. Add one in the app.");
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
