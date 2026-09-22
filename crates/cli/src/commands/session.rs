//! `ys session …`

use pty_host::TerminalHost;
use serde::Serialize;

use crate::commands::workspace::short;
use crate::{table, Failure, Output, Yardsort};

#[derive(clap::Subcommand)]
pub enum Command {
    /// List agent conversations.
    List {
        /// Only this workspace's, by name or id.
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
        /// Include conversations that have ended.
        #[arg(long)]
        all: bool,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    id: String,
    workspace: String,
    harness: String,
    title: String,
    /// What the record says.
    running: bool,
    /// Whether a process is actually there, which only a daemon can answer. `None` when there is
    /// no daemon to ask.
    alive: Option<bool>,
}

pub fn run(ys: &Yardsort, command: Command, out: &Output) -> Result<(), Failure> {
    let Command::List { workspace, all } = command;

    let workspaces = ys.store.workspaces()?;
    let wanted = match &workspace {
        None => None,
        Some(name) => {
            let found = workspaces
                .iter()
                .find(|w| w.id == *name || w.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| Failure::new(format!("No workspace called {name:?}.")))?;
            Some(found.id.clone())
        }
    };

    // Only a daemon knows which processes are really there; a record can outlive its process if
    // nothing was connected to hear the exit — which is exactly what happens when the app is
    // closed and `ys` started the agent. The daemon keeps exited sessions in its list so their
    // last screen can still be read, so the state is what matters here, not the id being there.
    let live: Option<Vec<String>> = ys.daemon().map(|client| {
        client
            .list()
            .into_iter()
            .filter(|s| s.state == pty_host::SessionState::Running)
            .map(|s| s.id.0)
            .collect()
    });

    let mut sessions = Vec::new();
    for workspace in &workspaces {
        if wanted.as_ref().is_some_and(|id| *id != workspace.id) {
            continue;
        }
        for row in ys.store.sessions(&workspace.id)? {
            if !all && !row.running {
                continue;
            }
            sessions.push(Session {
                alive: live.as_ref().map(|live| {
                    row.pty_session_id
                        .as_ref()
                        .is_some_and(|pty| live.contains(pty))
                }),
                id: row.id,
                workspace: workspace.name.clone(),
                harness: row.harness_id,
                title: row.title,
                running: row.running,
            });
        }
    }

    out.emit(&sessions, || {
        if sessions.is_empty() {
            println!("No {}sessions.", if all { "" } else { "running " });
            return;
        }
        let mut rows = vec![vec![
            "ID".to_owned(),
            "WORKSPACE".to_owned(),
            "HARNESS".to_owned(),
            "STATE".to_owned(),
            "TITLE".to_owned(),
        ]];
        rows.extend(sessions.iter().map(|s| {
            vec![
                short(&s.id),
                s.workspace.clone(),
                s.harness.clone(),
                state_of(s),
                s.title.clone(),
            ]
        }));
        table(&rows);
    })
}

/// What to show in the state column. The record and the daemon can disagree — say so rather than
/// pick one, because it is the interesting case: the process is gone but nothing settled the row.
fn state_of(session: &Session) -> String {
    match (session.running, session.alive) {
        (true, Some(true)) => "running".to_owned(),
        (true, Some(false)) => "gone".to_owned(),
        (true, None) => "running?".to_owned(),
        (false, _) => "ended".to_owned(),
    }
}
