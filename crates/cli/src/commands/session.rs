//! `ys session …`

use pty_host::TerminalHost;
use serde::Serialize;
use yardsort_core::launch::Launcher;
use yardsort_core::sessions::Continue;
use yardsort_core::store::SessionRow;

use crate::commands::workspace::{short, DETACHED_SIZE};
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
    /// Continue a conversation whose agent has ended, in a new terminal: after a restart ended it,
    /// say. It keeps running after this command returns.
    Resume {
        /// The session, by its id or the start of it, as `ys session list --all` prints it.
        session: String,
    },
    /// Start a copy of a conversation, running or not, that goes its own way from here.
    Fork {
        /// The session, by its id or the start of it, as `ys session list --all` prints it.
        session: String,
    },
}

/// A conversation continued: the record it is under, and the terminal it runs in.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Continued {
    id: String,
    workspace: String,
    harness: String,
    title: String,
    terminal: String,
    /// For a fork: the session it was copied from.
    forked_from: Option<String>,
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
    match command {
        Command::List { workspace, all } => list(ys, workspace, all, out),
        Command::Resume { session } => continued(ys, &session, Continue::Resume, out),
        Command::Fork { session } => continued(ys, &session, Continue::Fork, out),
    }
}

fn continued(ys: &Yardsort, wanted: &str, how: Continue, out: &Output) -> Result<(), Failure> {
    let row = session(ys, wanted)?;
    let connected = ys.daemon_or_start();
    if !connected.status.running {
        // Without a daemon the agent would belong to this process and end when it returns.
        return Err(Failure::new(format!(
            "No terminal daemon could be started, so the agent would stop the moment this \
             command returned.{}\nSee {}.",
            connected
                .status
                .problem
                .map(|p| format!("\n  {p}"))
                .unwrap_or_default(),
            connected.status.log_path,
        )));
    }
    let terminal = Launcher {
        store: &ys.store,
        host: connected.host.as_ref(),
        env: ys.env(),
        harnesses: &ys.settings.harnesses,
        activity: &ys.settings.activity,
        launched_by: yardsort_core::activity::LaunchedBy::Cli,
        data_dir: &ys.data_dir,
    }
    .continue_session(&row.id, how, DETACHED_SIZE, None)?;

    // A fork's record is the newest one copied from this session.
    let id = match how {
        Continue::Resume => row.id.clone(),
        Continue::Fork => ys
            .store
            .sessions(&row.workspace_id)?
            .into_iter()
            .find(|s| s.pty_session_id.as_deref() == Some(terminal.id.0.as_str()))
            .map_or_else(|| row.id.clone(), |s| s.id),
    };
    let workspace = ys
        .store
        .workspace(&row.workspace_id)?
        .map_or_else(|| row.workspace_id.clone(), |w| w.name);
    let continued = Continued {
        id,
        workspace,
        harness: row.harness_id,
        title: row.title,
        terminal: terminal.id.0,
        forked_from: (how == Continue::Fork).then_some(row.id),
    };
    out.emit(&continued, || {
        let verb = match how {
            Continue::Resume => "resumed",
            Continue::Fork => "forked ",
        };
        sayln!("{verb}  {}", continued.title);
        if let Some(from) = &continued.forked_from {
            sayln!("  from     {}", short(from));
        }
        sayln!("  session  {}", short(&continued.id));
        sayln!(
            "  agent    {} in {}",
            continued.harness,
            continued.workspace
        );
        sayln!(
            "\nIt keeps working after this command returns. `ys attach {}` watches it here.",
            short(&continued.id)
        );
    })
}

/// A session by its id, or the start of one that only it has.
fn session(ys: &Yardsort, wanted: &str) -> Result<SessionRow, Failure> {
    let wanted = wanted.trim();
    let all = ys.store.all_sessions()?;
    if let Some(exact) = all.iter().find(|s| s.id == wanted) {
        return Ok(exact.clone());
    }
    let mut matching: Vec<SessionRow> = if wanted.is_empty() {
        vec![]
    } else {
        all.into_iter()
            .filter(|s| s.id.starts_with(wanted))
            .collect()
    };
    match matching.len() {
        0 => Err(Failure::new(format!(
            "No session starts with {wanted:?}. `ys session list --all` lists them."
        ))),
        1 => Ok(matching.remove(0)),
        n => Err(Failure::new(format!(
            "{n} sessions start with {wanted:?}: give more of the id.\n{}",
            matching
                .iter()
                .map(|s| format!("  {}  {}", s.id, s.title))
                .collect::<Vec<_>>()
                .join("\n")
        ))),
    }
}

fn list(ys: &Yardsort, workspace: Option<String>, all: bool, out: &Output) -> Result<(), Failure> {
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
            sayln!("No {}sessions.", if all { "" } else { "running " });
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
