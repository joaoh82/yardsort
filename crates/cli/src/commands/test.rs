//! `ys test …` — a workspace's smoke tests, and checking a file before it is used.
//!
//! Smoke tests are YAML files in a repository's `.yardsort/tests` folder, so they belong to a
//! worktree, not to the profile. See `yardsort_core::smoke` and docs/guide/smoke-tests.md.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use yardsort_core::smoke::{self, Entry, Problem};
use yardsort_core::store::WorkspaceRow;

use crate::{table, Failure, Output, Yardsort};

#[derive(clap::Subcommand)]
pub enum Command {
    /// The smoke tests in a workspace's `.yardsort/tests`, and whether each is ready to run.
    List {
        /// Which workspace, by name or id. Default: the one this is run in.
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
    },
    /// Print a test's file, and anything wrong with it.
    Show {
        /// The test's id, from `ys test list`.
        id: String,
        /// Which workspace, by name or id. Default: the one this is run in.
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
    },
    /// Check a smoke test file. Prints each problem with its line and column; exits 1 if there
    /// are any. Needs no Yardsort profile, so it works anywhere.
    Validate {
        /// The file to check, or `-` to read it from standard input.
        file: PathBuf,
    },
    /// Print a complete, valid smoke test to start from.
    Example,
    /// Print the file format: every action, key and variable, written for an agent.
    Guide,
}

/// A test as `list` prints it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Listed<'a> {
    id: &'a str,
    name: &'a str,
    description: Option<&'a str>,
    path: &'a str,
    runnable: bool,
    uses_browser: bool,
    problems: &'a [Problem],
}

pub fn run(data_dir: Option<PathBuf>, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::Validate { file } => validate(&file, out),
        Command::Example => text(smoke::EXAMPLE, out),
        Command::Guide => text(smoke::GUIDE, out),
        Command::List { workspace } => {
            let ys = Yardsort::open(data_dir)?;
            let workspace = place(&ys, workspace.as_deref())?;
            list(&workspace, out)
        }
        Command::Show { id, workspace } => {
            let ys = Yardsort::open(data_dir)?;
            let workspace = place(&ys, workspace.as_deref())?;
            show(&workspace, &id, out)
        }
    }
}

/// The workspace named, or the one this is run in.
fn place(ys: &Yardsort, workspace: Option<&str>) -> Result<WorkspaceRow, Failure> {
    match workspace {
        Some(wanted) => super::workspace::find_workspace(ys, wanted),
        None => super::workflow::here(ys),
    }
}

fn list(workspace: &WorkspaceRow, out: &Output) -> Result<(), Failure> {
    let worktree = Path::new(&workspace.path);
    let entries = smoke::catalog(worktree);
    let listed: Vec<Listed> = entries
        .iter()
        .map(|e| Listed {
            id: &e.id,
            name: &e.name,
            description: e.description.as_deref(),
            path: &e.path,
            runnable: e.runnable(),
            uses_browser: e.uses_browser(),
            problems: &e.problems,
        })
        .collect();
    out.emit(&listed, || {
        let folder = smoke::dir(worktree);
        if entries.is_empty() {
            println!(
                "No smoke tests in {}. `ys test example` prints one to start from; `ys test guide` \
                 explains the format.",
                folder.display()
            );
            return;
        }
        let mut rows = vec![vec![
            "ID".to_owned(),
            "NAME".to_owned(),
            "STATUS".to_owned(),
            "FILE".to_owned(),
        ]];
        rows.extend(entries.iter().map(|e| {
            let file = Path::new(&e.path)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| e.path.clone());
            vec![e.id.clone(), e.name.clone(), status(e), file]
        }));
        table(&rows);
        println!();
        println!(
            "The tests of {} are in {}",
            workspace.name,
            folder.display()
        );
    })
}

fn show(workspace: &WorkspaceRow, id: &str, out: &Output) -> Result<(), Failure> {
    let worktree = Path::new(&workspace.path);
    let Some(entry) = smoke::find(worktree, id) else {
        let known: Vec<String> = smoke::catalog(worktree).into_iter().map(|e| e.id).collect();
        return Err(Failure::new(if known.is_empty() {
            format!(
                "There is no smoke test `{id}`: {} has none.",
                smoke::dir(worktree).display()
            )
        } else {
            format!(
                "There is no smoke test `{id}`. There are: {}.",
                known.join(", ")
            )
        }));
    };
    out.emit(&entry, || {
        print!("{}", entry.text);
        if !entry.text.ends_with('\n') {
            println!();
        }
        if !entry.problems.is_empty() {
            eprintln!();
            eprintln!("{} — it cannot run until it is fixed:", status(&entry));
            for problem in &entry.problems {
                eprintln!("{}", located(&entry.path, problem));
            }
        }
    })
}

fn validate(file: &Path, out: &Output) -> Result<(), Failure> {
    let stdin = file == Path::new("-");
    let name = if stdin {
        "<stdin>".to_owned()
    } else {
        file.display().to_string()
    };
    let text = if stdin {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|e| Failure::new(format!("Cannot read standard input: {e}")))?;
        text
    } else {
        smoke::read(file).map_err(|problem| Failure::new(problem.message))?
    };

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Checked<'a> {
        valid: bool,
        id: Option<&'a str>,
        problems: &'a [Problem],
    }
    let (id, problems) = match smoke::parse(&text) {
        Ok(test) => (Some(test.id), Vec::new()),
        Err(invalid) => (invalid.id, invalid.problems),
    };
    let checked = Checked {
        valid: problems.is_empty(),
        id: id.as_deref(),
        problems: &problems,
    };
    if checked.valid {
        return out.emit(&checked, || {
            println!(
                "{name}: a valid smoke test, `{}`.",
                checked.id.unwrap_or_default()
            );
        });
    }
    if out.json {
        out.emit(&checked, || {})?;
        return Err(Failure::new(count(problems.len())));
    }
    let lines: Vec<String> = problems.iter().map(|p| located(&name, p)).collect();
    Err(Failure::new(lines.join("\n")))
}

/// A built-in text, printed whole; `{ "text" }` as JSON.
fn text(body: &str, out: &Output) -> Result<(), Failure> {
    #[derive(Serialize)]
    struct Text<'a> {
        text: &'a str,
    }
    out.emit(&Text { text: body }, || print!("{body}"))
}

/// `file:line:column: message`, the shape editors and terminals turn into links.
fn located(name: &str, problem: &Problem) -> String {
    match problem.line {
        Some(_) => format!("{name}:{problem}"),
        None => format!("{name}: {problem}"),
    }
}

fn status(entry: &Entry) -> String {
    if entry.runnable() {
        "ready".to_owned()
    } else {
        count(entry.problems.len())
    }
}

fn count(problems: usize) -> String {
    match problems {
        1 => "1 problem".to_owned(),
        n => format!("{n} problems"),
    }
}
