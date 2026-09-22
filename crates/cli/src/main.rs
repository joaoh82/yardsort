//! `ys` — Yardsort from a terminal.
//!
//! A second client of the same core as the desktop app: the same SQLite database, the same git
//! rules, the same harness definitions, and the same terminal daemon. It does not talk to the
//! app and does not need it running — an agent started here belongs to the daemon, so it keeps
//! working after `ys` exits and appears in the app the next time it looks.

mod commands;
mod target;
mod yardsort;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::yardsort::Yardsort;

/// Something the user needs to read, rather than a bug. Printed to stderr; exit status 1.
pub struct Failure(String);

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl From<yardsort_core::error::IpcError> for Failure {
    fn from(error: yardsort_core::error::IpcError) -> Self {
        Self(error.message)
    }
}

impl From<yardsort_core::store::StoreError> for Failure {
    fn from(error: yardsort_core::store::StoreError) -> Self {
        Self(error.to_string())
    }
}

#[derive(Parser)]
#[command(
    name = "ys",
    version,
    about = "Run Yardsort's agents from a terminal",
    long_about = "Run Yardsort's agents from a terminal.\n\n\
                  Reads the same database as the desktop app, so a workspace made here shows up \
                  there and the other way round. An agent started here is owned by the Yardsort \
                  daemon, not by this command, so it carries on after `ys` returns."
)]
struct Cli {
    /// Which Yardsort profile to use. Defaults to the one the app uses.
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Print JSON instead of a table, for scripts.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Repositories Yardsort knows about.
    #[command(subcommand)]
    Project(commands::project::Command),
    /// Branches with a worktree of their own.
    #[command(subcommand)]
    Workspace(commands::workspace::Command),
    /// Agent conversations and the terminals running them.
    #[command(subcommand)]
    Session(commands::session::Command),
    /// Put a running session on this terminal. Detaching leaves it running.
    Attach {
        /// Which one: a workspace name, or the start of an id from `ys session list`. With one
        /// session running, it can be left out.
        target: Option<String>,
        /// Leave the session at the size it already has, instead of matching this terminal.
        #[arg(long)]
        no_resize: bool,
    },
    /// Print what a session has on its screen, finished ones included.
    Logs {
        /// Which one: a workspace name, or the start of an id from `ys session list`.
        target: Option<String>,
        /// Emit the screen exactly as the daemon holds it, escape sequences and colours and all.
        #[arg(long)]
        raw: bool,
        /// Only the last so many lines.
        #[arg(long, value_name = "N")]
        lines: Option<usize>,
    },
    /// Where everything is, and whether it can be reached.
    Doctor,
}

fn main() {
    // This binary can also be the daemon, exactly as the app's can: whichever process needs one
    // first starts it from its own executable, and both are the same build.
    yardsort_core::daemon::run_daemon_and_exit_if_asked();

    let cli = Cli::parse();
    let out = Output { json: cli.json };
    let result =
        match cli.command {
            Command::Doctor => commands::doctor::run(cli.data_dir, &out),
            Command::Attach { target, no_resize } => Yardsort::open(cli.data_dir)
                .and_then(|ys| commands::attach::run(&ys, target, no_resize, &out)),
            Command::Logs { target, raw, lines } => Yardsort::open(cli.data_dir)
                .and_then(|ys| commands::logs::run(&ys, target, raw, lines, &out)),
            Command::Project(command) => Yardsort::open(cli.data_dir)
                .and_then(|ys| commands::project::run(&ys, command, &out)),
            Command::Workspace(command) => Yardsort::open(cli.data_dir)
                .and_then(|ys| commands::workspace::run(&ys, command, &out)),
            Command::Session(command) => Yardsort::open(cli.data_dir)
                .and_then(|ys| commands::session::run(&ys, command, &out)),
        };
    if let Err(Failure(message)) = result {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

/// How to print. Every command supports both shapes: a table for a person, JSON for a script.
pub struct Output {
    pub json: bool,
}

impl Output {
    /// Print `value` as JSON, or hand `rows` to the plain-text printer.
    pub fn emit<T: serde::Serialize>(
        &self,
        value: &T,
        plain: impl FnOnce(),
    ) -> Result<(), Failure> {
        if self.json {
            let text = serde_json::to_string_pretty(value)
                .map_err(|e| Failure::new(format!("cannot render JSON: {e}")))?;
            println!("{text}");
        } else {
            plain();
        }
        Ok(())
    }

    /// A message for a person, skipped entirely when the output is JSON.
    pub fn note(&self, message: impl std::fmt::Display) {
        if !self.json {
            println!("{message}");
        }
    }
}

/// Left-align `rows` into columns. Empty input prints nothing, so callers say so themselves.
pub fn table(rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = Vec::new();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            let width = cell.chars().count();
            match widths.get_mut(i) {
                Some(slot) => *slot = (*slot).max(width),
                None => widths.push(width),
            }
        }
    }
    for row in rows {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            let last = i + 1 == row.len();
            line.push_str(cell);
            if !last {
                let pad = widths[i].saturating_sub(cell.chars().count()) + 2;
                line.extend(std::iter::repeat_n(' ', pad));
            }
        }
        println!("{}", line.trim_end());
    }
}
