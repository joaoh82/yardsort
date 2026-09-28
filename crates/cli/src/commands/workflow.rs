//! `ys workflow …` — the workflows Yardsort knows, and checking a file before it is used.
//!
//! Workflows are YAML files in the profile's `workflows` folder, plus the ones built in. See
//! `yardsort_core::workflow` and docs/guide/workflows.md.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use yardsort_core::workflow::{self, Entry, Problem, Source};

use crate::{table, Failure, Output, Yardsort};

#[derive(clap::Subcommand)]
pub enum Command {
    /// The built-in workflows and yours, and whether each is ready to run.
    List,
    /// Print a workflow's file, and anything wrong with it.
    Show {
        /// The workflow's id, from `ys workflow list`.
        id: String,
    },
    /// Copy a workflow into your folder to edit. A built-in copied under its own id is then
    /// used instead of the built-in; delete the copy to go back.
    Copy {
        /// The workflow to copy, from `ys workflow list`.
        id: String,
        /// Give the copy an id of its own, so it sits beside the original.
        #[arg(long = "as", value_name = "NEW_ID")]
        as_id: Option<String>,
    },
    /// Check a workflow file. Prints each problem with its line and column; exits 1 if there
    /// are any. Needs no Yardsort profile, so it works anywhere.
    Validate {
        /// The file to check, or `-` to read it from standard input.
        file: PathBuf,
    },
}

/// A workflow as `list` prints it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Listed<'a> {
    id: &'a str,
    name: &'a str,
    description: Option<&'a str>,
    source: &'a Source,
    runnable: bool,
    problems: &'a [Problem],
}

pub fn run(data_dir: Option<PathBuf>, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::Validate { file } => validate(&file, out),
        Command::List => {
            let ys = Yardsort::open(data_dir)?;
            list(&ys.data_dir, out)
        }
        Command::Show { id } => {
            let ys = Yardsort::open(data_dir)?;
            show(&ys.data_dir, &id, out)
        }
        Command::Copy { id, as_id } => {
            let ys = Yardsort::open(data_dir)?;
            copy(&ys.data_dir, &id, as_id.as_deref(), out)
        }
    }
}

fn list(data_dir: &Path, out: &Output) -> Result<(), Failure> {
    let entries = workflow::catalog(data_dir);
    let listed: Vec<Listed> = entries
        .iter()
        .map(|e| Listed {
            id: &e.id,
            name: &e.name,
            description: e.description.as_deref(),
            source: &e.source,
            runnable: e.runnable(),
            problems: &e.problems,
        })
        .collect();
    out.emit(&listed, || {
        let mut rows = vec![vec![
            "ID".to_owned(),
            "NAME".to_owned(),
            "STATUS".to_owned(),
            "FROM".to_owned(),
        ]];
        rows.extend(
            entries
                .iter()
                .map(|e| vec![e.id.clone(), e.name.clone(), status(e), from(&e.source)]),
        );
        table(&rows);
        println!();
        println!(
            "Your workflows go in {}",
            workflow::user_dir(data_dir).display()
        );
    })
}

fn show(data_dir: &Path, id: &str, out: &Output) -> Result<(), Failure> {
    let Some(entry) = workflow::find(data_dir, id) else {
        let known: Vec<String> = workflow::catalog(data_dir)
            .into_iter()
            .map(|e| e.id)
            .collect();
        return Err(Failure::new(format!(
            "There is no workflow `{id}`. There are: {}.",
            known.join(", ")
        )));
    };
    out.emit(&entry, || {
        print!("{}", entry.text);
        if !entry.text.ends_with('\n') {
            println!();
        }
        if !entry.problems.is_empty() {
            let name = match &entry.source {
                Source::File { path, .. } => path.clone(),
                Source::BuiltIn => entry.id.clone(),
            };
            eprintln!();
            eprintln!("{} — it cannot run until it is fixed:", status(&entry));
            for problem in &entry.problems {
                eprintln!("{}", located(&name, problem));
            }
        }
    })
}

fn copy(data_dir: &Path, id: &str, as_id: Option<&str>, out: &Output) -> Result<(), Failure> {
    let path = workflow::copy(data_dir, id, as_id)?;
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Copied {
        id: String,
        path: String,
    }
    let copied = Copied {
        id: as_id.unwrap_or(id).to_owned(),
        path: path.display().to_string(),
    };
    out.emit(&copied, || {
        println!("Copied to {}", copied.path);
        if as_id.is_none() {
            println!("It is used instead of the built-in. Delete it to go back.");
        }
        println!(
            "Check it after editing: ys workflow validate {}",
            copied.path
        );
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
        workflow::read(file).map_err(|problem| Failure::new(problem.message))?
    };

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Checked<'a> {
        valid: bool,
        id: Option<&'a str>,
        problems: &'a [Problem],
    }
    let (id, problems) = match workflow::parse(&text) {
        Ok(workflow) => (Some(workflow.id), Vec::new()),
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
                "{name}: a valid workflow, `{}`.",
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

fn from(source: &Source) -> String {
    match source {
        Source::BuiltIn => "built in".to_owned(),
        Source::File {
            path,
            replaces_built_in: true,
        } => format!("{path} (instead of the built-in)"),
        Source::File { path, .. } => path.clone(),
    }
}
