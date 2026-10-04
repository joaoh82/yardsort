//! `ys memory …` — a project's memory, as an agent sees it.
//!
//! `list` and `search` read the approved entries, unless the user has stopped the project sharing
//! them with its agents;
//! `propose` adds a candidate for the user to review in the app, shared or not. There is no approve, edit, reject or revoke here, on purpose: an agent in
//! a Yardsort terminal can run anything `ys` offers, and no agent may approve what it proposed.
//! The user decides in the app's Memory view. See `yardsort_core::memory`.

use serde::Serialize;
use yardsort_core::memory::{self, Proposed, Source};
use yardsort_core::store::MemoryRow;

use crate::{table, Failure, Output, Yardsort};

#[derive(clap::Subcommand)]
pub enum Command {
    /// The project's approved entries, newest first.
    List {
        /// The project, by name or id. Default: the one this is run in.
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
    },
    /// Approved entries containing every one of these words.
    Search {
        /// Words to look for, ignoring case.
        #[arg(required = true)]
        words: Vec<String>,
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
    },
    /// Suggest an entry. It waits for the user to approve it in Yardsort; until then no agent
    /// sees it.
    Propose {
        /// One short paragraph: something the next agent in this project should know.
        text: String,
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
    },
}

/// An entry as it is printed.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    id: String,
    text: String,
    from: String,
}

impl From<&MemoryRow> for Entry {
    fn from(row: &MemoryRow) -> Self {
        Self {
            id: memory::short_id(&row.id).to_owned(),
            text: row.text.clone(),
            from: memory::citation(row),
        }
    }
}

pub fn run(ys: &Yardsort, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::List { project } => {
            let (project, _) = locate(ys, project.as_deref())?;
            shared(ys, &project)?;
            print(
                &memory::approved(&ys.store, &project)?,
                out,
                "No approved entries.",
            )
        }
        Command::Search { words, project } => {
            let (project, _) = locate(ys, project.as_deref())?;
            shared(ys, &project)?;
            let found = memory::search(&ys.store, &project, &words.join(" "))?;
            print(&found, out, "Nothing approved matches.")
        }
        Command::Propose { text, project } => {
            let (project, source) = locate(ys, project.as_deref())?;
            let proposed = memory::propose(&ys.store, &project, &text, &source)?;
            #[derive(Serialize)]
            #[serde(rename_all = "camelCase")]
            struct Result {
                id: String,
                state: String,
                added: bool,
            }
            let (id, state, added) = match proposed {
                Proposed::New(id) => (id, "candidate".to_owned(), true),
                Proposed::Known { id, state } => (id, state, false),
            };
            let result = Result {
                id: memory::short_id(&id).to_owned(),
                state,
                added,
            };
            out.emit(&result, || {
                if result.added {
                    println!(
                        "Proposed ({}). It waits for the user to approve it in Yardsort; until \
                         then no agent sees it.",
                        result.id
                    );
                } else {
                    println!(
                        "Already there ({}), {}. Nothing was added.",
                        result.id, result.state
                    );
                }
            })
        }
    }
}

/// Reading memory is giving it to whoever runs this — most often an agent — so it follows the
/// project's switch, as a launch does. Proposing does not: a proposal reaches no agent.
fn shared(ys: &Yardsort, project: &str) -> Result<(), Failure> {
    if ys.store.memory_shared(project)? {
        return Ok(());
    }
    Err(Failure::new(
        "This project does not give its agents its memory: the user turned that off in \
         Yardsort (Memory… in the project's menu).",
    ))
}

fn print(rows: &[MemoryRow], out: &Output, empty: &str) -> Result<(), Failure> {
    let entries: Vec<Entry> = rows.iter().map(Entry::from).collect();
    out.emit(&entries, || {
        if entries.is_empty() {
            println!("{empty}");
            return;
        }
        let mut lines = vec![vec!["ID".to_owned(), "FROM".to_owned(), "ENTRY".to_owned()]];
        lines.extend(
            entries
                .iter()
                .map(|e| vec![e.id.clone(), e.from.clone(), e.text.clone()]),
        );
        table(&lines);
    })
}

/// The project a command is about, and where it came from: `--project` when given; otherwise
/// the launch environment of the agent running it, or the folder it runs in.
pub(super) fn locate(ys: &Yardsort, project: Option<&str>) -> Result<(String, Source), Failure> {
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let run = env(yardsort_core::activity::RUN_ENV);
    let workspace = env(yardsort_core::activity::WORKSPACE_ENV);
    let cwd = std::env::current_dir().map_err(|e| Failure::new(format!("cwd: {e}")))?;
    let located = memory::locate(&ys.store, run.as_deref(), workspace.as_deref(), &cwd)?;
    match project {
        Some(wanted) => {
            let project = ys.project(wanted)?;
            // The launch environment still says who is asking, when it is about this project.
            let source = located
                .filter(|(p, _)| *p == project.id)
                .map(|(_, source)| source)
                .unwrap_or_default();
            Ok((project.id, source))
        }
        None => located.ok_or_else(|| {
            Failure::new(
                "Not inside a Yardsort workspace. Run this in one, or name the project with \
                 --project.",
            )
        }),
    }
}
