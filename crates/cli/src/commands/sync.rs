//! `ys sync …`: how sync stands here, read from the records. Nothing more: turning sync on,
//! linking, stopping and removing are the person's, in the app or with `yardsort-host sync` at a
//! terminal of their own, since an agent in a Yardsort terminal can run anything `ys` offers.

use serde::Serialize;
use yardsort_core::sync::{project_sync, ProjectSync};

use crate::{table, Failure, Output, Yardsort};

#[derive(clap::Subcommand)]
pub enum Command {
    /// Which projects sync with your other computers, and how the last pass went.
    Status,
}

pub fn run(ys: &Yardsort, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::Status => status(ys, out),
    }
}

#[derive(Serialize)]
struct Status {
    remote: bool,
    synced_at: Option<i64>,
    problem: Option<String>,
    waiting: usize,
    projects: Vec<Project>,
}

#[derive(Serialize)]
struct Project {
    id: String,
    name: String,
    /// `syncing`, `waiting`, `removing` or `refused`.
    state: &'static str,
    refused: Option<String>,
}

fn status(ys: &Yardsort, out: &Output) -> Result<(), Failure> {
    let remote = ys.store.remote_host()?.is_some();
    let state = ys.store.sync_state()?;
    let projects = ys.store.projects()?;
    let mut shown = Vec::new();
    for row in ys.store.synced_projects()? {
        let Some(project) = projects.iter().find(|p| p.id == row.local_id) else {
            continue;
        };
        let ProjectSync::On {
            reached,
            removing,
            refused,
        } = project_sync(&ys.store, &project.id, remote, || unreachable!())?
        else {
            continue;
        };
        shown.push(Project {
            id: project.id.clone(),
            name: project.name.clone(),
            state: match (&refused, removing, reached) {
                (Some(_), _, _) => "refused",
                (None, true, _) => "removing",
                (None, false, false) => "waiting",
                (None, false, true) => "syncing",
            },
            refused,
        });
    }
    let status = Status {
        remote,
        synced_at: state.synced_at,
        problem: state.problem,
        waiting: ys.store.sync_waiting()?,
        projects: shown,
    };
    out.emit(&status, || {
        if !status.remote {
            sayln!("Remote is off here, so nothing syncs.");
            return;
        }
        if status.projects.is_empty() {
            sayln!("No project syncs here. Turn one on in the app's Project settings, or with `yardsort-host sync on`.");
        } else {
            let mut rows = vec![vec!["NAME".to_owned(), "SYNC".to_owned()]];
            rows.extend(status.projects.iter().map(|p| {
                vec![
                    p.name.clone(),
                    match &p.refused {
                        Some(why) => format!("refused: {why}"),
                        None => p.state.to_owned(),
                    },
                ]
            }));
            table(&rows);
        }
        match status.synced_at {
            Some(at) => sayln!("Last synced: {}", ago(at)),
            None => sayln!("Last synced: never"),
        }
        if let Some(problem) = &status.problem {
            sayln!("Last problem: {problem}");
        }
        if status.waiting > 0 {
            sayln!("Waiting to go up: {}", status.waiting);
        }
    })
}

/// How long ago `at` was, roughly.
fn ago(at: i64) -> String {
    let seconds = (yardsort_core::store::now_ms() - at).max(0) / 1000;
    match seconds {
        0..=59 => format!("{seconds}s ago"),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86_399 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}
