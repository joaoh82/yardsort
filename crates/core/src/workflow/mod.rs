//! Workflows: named, reusable sequences of agent work, written in YAML.
//!
//! A workflow file says what to ask for before a run (its inputs) and what to do (its steps, a
//! graph through each step's `needs`). This module is the file: its types, the parser and
//! validator that turn text into them with a line number on every mistake, and the catalog —
//! the built-ins compiled in here, and the user's own files in `<data dir>/workflows/`.
//!
//! Workflows are the user's, never a repository's: a cloned project must not decide what gets
//! sent to the user's agents. That is the line project automation drew too. See
//! `docs/design/21-workflows.md`.

pub mod driver;
pub mod engine;
mod parse;
pub mod runs;
pub mod template;

use std::path::{Path, PathBuf};

use serde::Serialize;
use specta::Type;

use crate::error::{IpcError, IpcResult};

pub use parse::parse;

/// The schema version this build reads. A file asking for a later one is listed, not run.
pub const SCHEMA_VERSION: u32 = 1;

/// Files bigger than this are not read. A workflow is a page of text, not a data set.
pub const MAX_FILE_BYTES: u64 = 256 * 1024;

/// A workflow file, checked and ready to run.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub version: u32,
    pub trigger: Trigger,
    pub inputs: Vec<Input>,
    pub steps: Vec<Step>,
}

/// What starts a run, and what a run is about.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Trigger {
    pub kind: TriggerKind,
    pub context: RunContext,
}

/// Only a person starts a run in this version. Schedules and forge events come later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    Manual,
}

/// What a run is about. A workspace run knows the workspace, its branch and its pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RunContext {
    Workspace,
}

/// Something asked of the person starting a run, before it starts.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    pub id: String,
    pub kind: InputKind,
    /// What the question says. The id when the file gives none.
    pub label: String,
    pub required: bool,
    pub default: Option<String>,
    /// The choices of a `choice` input; empty for the others.
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    /// One of the harnesses set up in Yardsort, picked the way the composer picks one.
    Harness,
    Text,
    Choice,
}

/// One node of the graph.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: String,
    /// The steps that must have succeeded before this one starts. Steps whose needs are all met
    /// run together.
    pub needs: Vec<String>,
    /// Flattened, so a step reads `{ id, needs, action: "notify", title }` as the file does.
    #[serde(flatten)]
    pub action: Action,
}

/// What a step does. Every string here may hold `{{ variables }}`; see [`template`].
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Action {
    /// Start a harness in the run's workspace, with a first message.
    StartSession {
        harness: String,
        prompt: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        skip_memory: bool,
    },
    /// Wait for a session to settle or to end.
    WaitSession {
        session: SessionRef,
        until: Until,
        timeout_secs: Option<u32>,
    },
    /// Paste a message into a live session once it is quiet. A busy agent is never written to.
    SendToSession {
        session: SessionRef,
        prompt: String,
        timeout_secs: Option<u32>,
    },
    /// Wait for a review or a comment on the workspace's pull request, newer than the run.
    WaitPrActivity {
        kind: PrActivity,
        timeout_secs: Option<u32>,
    },
    /// Tell the person who started the run.
    Notify { title: String, body: Option<String> },
}

impl Action {
    /// The name a file uses for this action.
    pub fn name(&self) -> &'static str {
        match self {
            Action::StartSession { .. } => "start_session",
            Action::WaitSession { .. } => "wait_session",
            Action::SendToSession { .. } => "send_to_session",
            Action::WaitPrActivity { .. } => "wait_pr_activity",
            Action::Notify { .. } => "notify",
        }
    }

    /// What a finished step of this kind makes available as `{{ steps.<id>.<field> }}`.
    pub fn produces(name: &str) -> &'static [&'static str] {
        match name {
            "start_session" => &["session", "run"],
            "wait_session" => &["outcome"],
            "wait_pr_activity" => &["count", "latest_url"],
            _ => &[],
        }
    }
}

/// Which session a step means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SessionRef {
    /// The workspace's most recent live harness session: the agent whose work the run is about.
    Origin,
    /// The session an earlier `start_session` step started, by that step's id.
    Step(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Until {
    /// Quiet, or a finished turn where the harness reports turns.
    Settled,
    Exited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PrActivity {
    Review,
    Comment,
    Any,
}

/// One thing wrong with a file, where it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// 1-based. `None` when the problem is the file as a whole.
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
}

impl Problem {
    pub fn at(line: usize, column: usize, message: impl Into<String>) -> Self {
        Self {
            line: Some(line as u32),
            column: Some(column as u32),
            message: message.into(),
        }
    }

    pub fn whole(message: impl Into<String>) -> Self {
        Self {
            line: None,
            column: None,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => write!(f, "{line}:{column}: {}", self.message),
            (Some(line), None) => write!(f, "{line}: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

/// A file that did not check out. Whatever could be read of it comes along, so it can still be
/// listed by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    pub id: Option<String>,
    pub name: Option<String>,
    pub problems: Vec<Problem>,
}

/// Where a workflow came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum Source {
    /// Compiled into Yardsort. Read-only; duplicate it to change it.
    BuiltIn,
    /// A file in the user's workflows directory.
    File {
        path: String,
        /// This file's id is a built-in's, so it is used instead. Removing it brings the
        /// built-in back.
        replaces_built_in: bool,
    },
}

/// One workflow as the catalog lists it: runnable, or not, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub source: Source,
    /// Empty when the workflow can run.
    pub problems: Vec<Problem>,
    /// The checked workflow; `None` when there are problems.
    pub workflow: Option<Workflow>,
    /// The file exactly as written.
    pub text: String,
}

impl Entry {
    pub fn runnable(&self) -> bool {
        self.problems.is_empty() && self.workflow.is_some()
    }
}

/// The workflows that ship with Yardsort, as text.
pub const BUILT_IN: &[(&str, &str)] = &[("code-review", include_str!("builtin/code-review.yaml"))];

/// Where the user's own workflow files live.
pub fn user_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("workflows")
}

/// Every workflow: the built-ins, and the user's files, which replace a built-in with the same
/// id. Sorted by name.
///
/// A file that does not parse still takes the id of its file name, so a broken
/// `code-review.yaml` replaces the built-in and shows its problems, rather than the built-in
/// quietly running in its place.
pub fn catalog(data_dir: &Path) -> Vec<Entry> {
    load(data_dir).into_iter().map(|(entry, _)| entry).collect()
}

/// The workflow with this id, the way a run would find it: the first file, by name, to claim
/// the id, or else the built-in.
pub fn find(data_dir: &Path, id: &str) -> Option<Entry> {
    load(data_dir)
        .into_iter()
        .find(|(entry, duplicate)| entry.id == id && !duplicate)
        .map(|(entry, _)| entry)
}

/// The catalog, each entry marked when a file earlier by name already claimed its id.
fn load(data_dir: &Path) -> Vec<(Entry, bool)> {
    let mut entries: Vec<(Entry, bool)> = BUILT_IN
        .iter()
        .map(|(id, text)| {
            let entry = entry((*id).to_owned(), (*text).to_owned(), Source::BuiltIn);
            (entry, false)
        })
        .collect();
    let mut claimed: Vec<(String, PathBuf)> = Vec::new();
    for path in user_files(&user_dir(data_dir)) {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (id, mut found) = match read(&path) {
            Ok(text) => {
                let id = match parse(&text) {
                    Ok(workflow) => workflow.id,
                    Err(invalid) => invalid.id.unwrap_or_else(|| stem.clone()),
                };
                let source = Source::File {
                    path: path.to_string_lossy().into_owned(),
                    replaces_built_in: is_built_in(&id),
                };
                (id.clone(), entry(id, text, source))
            }
            Err(problem) => {
                let source = Source::File {
                    path: path.to_string_lossy().into_owned(),
                    replaces_built_in: is_built_in(&stem),
                };
                let entry = Entry {
                    id: stem.clone(),
                    name: stem.clone(),
                    description: None,
                    source,
                    problems: vec![problem],
                    workflow: None,
                    text: String::new(),
                };
                (stem, entry)
            }
        };
        let duplicate = match claimed.iter().find(|(other, _)| *other == id) {
            Some((_, first)) => {
                found.problems.insert(
                    0,
                    Problem::whole(format!(
                        "{} already has the id `{id}`, and ids must be unique. This file is \
                         ignored until one of them changes.",
                        first.display()
                    )),
                );
                found.workflow = None;
                true
            }
            None => {
                claimed.push((id.clone(), path.clone()));
                entries.retain(|(e, _)| !(e.id == id && e.source == Source::BuiltIn));
                false
            }
        };
        entries.push((found, duplicate));
    }
    entries.sort_by(|(a, _), (b, _)| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    entries
}

fn is_built_in(id: &str) -> bool {
    BUILT_IN.iter().any(|(built_in, _)| *built_in == id)
}

/// Copy a workflow into the user's folder as `<id>.yaml`, ready to edit, and say where.
///
/// With no `as_id`, the copy keeps the id, which only makes sense for a built-in: the copy is
/// then used instead of it. With `as_id`, the copy is a new workflow beside the original.
///
/// This is the only safe way to start from a built-in. A shell redirect into the folder,
/// `ys workflow show code-review > …/code-review.yaml`, makes the empty file before `ys` runs,
/// so `ys` finds that empty file claiming the id and prints it instead of the built-in; and
/// PowerShell's `>` writes UTF-16, which is not a workflow file. Never overwrites anything.
pub fn copy(data_dir: &Path, id: &str, as_id: Option<&str>) -> IpcResult<PathBuf> {
    let source = find(data_dir, id).ok_or_else(|| {
        IpcError::new(
            "workflow_not_found",
            format!("There is no workflow `{id}`."),
        )
    })?;
    let target = as_id.unwrap_or(id);
    if !parse::is_id(target) {
        return Err(IpcError::new(
            "workflow_invalid_id",
            format!(
                "`{target}` cannot be an id. Use lowercase letters, digits and `-`, like \
                 `my-review`."
            ),
        ));
    }
    if let Source::File { path, .. } = &source.source {
        if target == id {
            return Err(IpcError::new(
                "workflow_exists",
                format!("`{id}` is already your file, {path}. Give the copy an id of its own."),
            ));
        }
    }
    let claimed = load(data_dir)
        .into_iter()
        .find(|(entry, _)| entry.id == target && matches!(entry.source, Source::File { .. }));
    if let Some((entry, _)) = claimed {
        let Source::File { path, .. } = entry.source else {
            unreachable!("only files are looked for")
        };
        return Err(IpcError::new(
            "workflow_exists",
            format!("You already have a workflow `{target}`: {path}."),
        ));
    }

    let mut text = source.text.clone();
    if source.source == Source::BuiltIn {
        // The built-in's opening comment explains how to copy it, which a copy no longer needs.
        let body = text
            .lines()
            .skip_while(|line| line.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        text = format!("# Copied from Yardsort's built-in `{id}`.\n{body}\n");
    }
    if target != id {
        text = with_id(&text, target).ok_or_else(|| {
            IpcError::new(
                "workflow_invalid",
                format!(
                    "Cannot find the `id:` line of `{id}` to change it. Copy the file by hand."
                ),
            )
        })?;
    }

    let folder = user_dir(data_dir);
    std::fs::create_dir_all(&folder)
        .map_err(|e| IpcError::new("io", format!("Cannot create {}: {e}", folder.display())))?;
    let path = folder.join(format!("{target}.yaml"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => IpcError::new(
                "workflow_exists",
                format!("{} is already there. Nothing was copied.", path.display()),
            ),
            _ => IpcError::new("io", format!("Cannot write {}: {e}", path.display())),
        })?;
    std::io::Write::write_all(&mut file, text.as_bytes())
        .map_err(|e| IpcError::new("io", format!("Cannot write {}: {e}", path.display())))?;
    Ok(path)
}

/// Save a workflow file and return where it went.
///
/// `path` is the file being edited, which must be one of the user's own: it is replaced whole,
/// through a temporary file, so a crash never leaves half of it. With no `path` this is a new
/// workflow, written as `<id>.yaml` beside the others, and nothing already there is ever
/// overwritten. Either way the text must say its `id`, and no other file of the user's may
/// already claim it. A file with problems is saved as it is: the catalog lists it, with them.
pub fn save(data_dir: &Path, path: Option<&Path>, text: &str) -> IpcResult<PathBuf> {
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(IpcError::new(
            "workflow_too_big",
            format!("A workflow file may be at most {MAX_FILE_BYTES} bytes."),
        ));
    }
    let id = match parse(text) {
        Ok(workflow) => Some(workflow.id),
        Err(invalid) => invalid.id,
    }
    .filter(|id| parse::is_id(id))
    .ok_or_else(|| {
        IpcError::new(
            "workflow_invalid_id",
            "Give the workflow an `id:` first: lowercase letters, digits and `-`, like `fix-ci`.",
        )
    })?;
    let editing = match path {
        Some(path) => Some(own_file(data_dir, path)?),
        None => None,
    };
    let clash = load(data_dir)
        .into_iter()
        .find_map(|(entry, _)| match entry.source {
            Source::File { path, .. }
                if entry.id == id && editing.as_deref() != Some(Path::new(&path)) =>
            {
                Some(path)
            }
            _ => None,
        });
    if let Some(other) = clash {
        return Err(IpcError::new(
            "workflow_exists",
            format!("{other} already has the id `{id}`. Give this one an id of its own."),
        ));
    }
    let folder = user_dir(data_dir);
    std::fs::create_dir_all(&folder)
        .map_err(|e| IpcError::new("io", format!("Cannot create {}: {e}", folder.display())))?;
    let written = |path: &Path, e: std::io::Error| {
        IpcError::new("io", format!("Cannot write {}: {e}", path.display()))
    };
    match editing {
        Some(path) => {
            let temporary = path.with_extension("yaml.saving");
            std::fs::write(&temporary, text).map_err(|e| written(&temporary, e))?;
            std::fs::rename(&temporary, &path).map_err(|e| {
                let _ = std::fs::remove_file(&temporary);
                written(&path, e)
            })?;
            Ok(path)
        }
        None => {
            let path = folder.join(format!("{id}.yaml"));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|e| match e.kind() {
                    std::io::ErrorKind::AlreadyExists => IpcError::new(
                        "workflow_exists",
                        format!("{} is already there. Nothing was saved.", path.display()),
                    ),
                    _ => written(&path, e),
                })?;
            std::io::Write::write_all(&mut file, text.as_bytes()).map_err(|e| written(&path, e))?;
            Ok(path)
        }
    }
}

/// Delete one of the user's workflow files. A built-in it replaced is used again.
pub fn remove(data_dir: &Path, path: &Path) -> IpcResult<()> {
    let path = own_file(data_dir, path)?;
    std::fs::remove_file(&path)
        .map_err(|e| IpcError::new("io", format!("Cannot delete {}: {e}", path.display())))
}

/// `path`, when it is a workflow file directly in the user's folder, as the catalog lists it.
/// Nothing outside that folder is ever written or removed through a workflow.
fn own_file(data_dir: &Path, path: &Path) -> IpcResult<PathBuf> {
    let listed = user_files(&user_dir(data_dir));
    listed.into_iter().find(|own| own == path).ok_or_else(|| {
        IpcError::new(
            "workflow_not_yours",
            format!("{} is not one of your workflow files.", path.display()),
        )
    })
}

/// `text` with its top-level `id:` line saying `id`, or `None` when it has no such line.
fn with_id(text: &str, id: &str) -> Option<String> {
    let mut found = false;
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            if !found && line.starts_with("id:") {
                found = true;
                format!("id: {id}")
            } else {
                line.to_owned()
            }
        })
        .collect();
    found.then(|| lines.join("\n") + "\n")
}

/// Read a workflow file as text, refusing ones too big or not text.
pub fn read(path: &Path) -> Result<String, Problem> {
    let size = std::fs::metadata(path)
        .map_err(|e| Problem::whole(format!("Cannot read {}: {e}", path.display())))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(Problem::whole(format!(
            "{} is {size} bytes; a workflow file may be at most {MAX_FILE_BYTES}.",
            path.display()
        )));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| Problem::whole(format!("Cannot read {}: {e}", path.display())))?;
    String::from_utf8(bytes)
        .map_err(|_| Problem::whole(format!("{} is not UTF-8 text.", path.display())))
}

fn entry(id: String, text: String, source: Source) -> Entry {
    match parse(&text) {
        Ok(workflow) => Entry {
            id,
            name: workflow.name.clone(),
            description: workflow.description.clone(),
            source,
            problems: Vec::new(),
            workflow: Some(workflow),
            text,
        },
        Err(invalid) => Entry {
            name: invalid.name.unwrap_or_else(|| id.clone()),
            id,
            description: None,
            source,
            problems: invalid.problems,
            workflow: None,
            text,
        },
    }
}

/// `*.yaml` and `*.yml` directly in `dir`, by file name. A missing directory has none.
fn user_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("yaml" | "yml")
                )
        })
        .collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests;
