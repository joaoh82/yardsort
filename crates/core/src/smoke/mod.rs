//! Smoke tests: a short, structured list of steps that checks a piece of software from the
//! outside — start the server, wait for it, hit an endpoint, open a page, look for a word.
//!
//! A test is a YAML file in the repository, `.yardsort/tests/<id>.yaml`, so an agent writes one
//! for the feature it built the way it writes any other file, and the test travels with the
//! branch. This module is the file: its types, the parser and validator that turn text into them
//! with a line and column on every mistake, and the catalog of a worktree's tests. Running one is
//! the runner's business, and a file from a repository never runs without the user having seen
//! it. See `docs/design/32-smoke-tests.md`.

pub mod browser;
pub mod describe;
mod parse;

pub mod deliver;
pub mod exec;
pub mod report;
pub mod runner;
pub mod runs;
pub mod words;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use specta::Type;

pub use crate::yaml::{Invalid, Problem};
pub use parse::{is_env_name, parse};

/// The schema version this build reads. A file asking for a later one is listed, not run.
pub const SCHEMA_VERSION: u32 = 1;

/// Where a repository keeps its smoke tests, relative to the worktree, as the docs name it.
/// Paths are built with [`dir`], which joins the two folders the way the platform does.
pub const DIR: &str = ".yardsort/tests";

/// Files bigger than this are not read. A smoke test is a page of text, not a data set.
pub const MAX_FILE_BYTES: u64 = 256 * 1024;

/// How long a whole run may take when the file does not say.
pub const DEFAULT_RUN_TIMEOUT_SECS: u32 = 30 * 60;

/// How long one step may take when it does not say. A `manual` step has no limit.
pub const DEFAULT_STEP_TIMEOUT_SECS: u32 = 5 * 60;

/// A complete, valid test to start from: what `ys test example` prints.
pub const EXAMPLE: &str = include_str!("example.yaml");

/// The file format, written for an agent: every action, key and variable, with whole files as
/// examples. What `ys test guide` prints. Every YAML block in it is checked by a test to be a
/// valid file, so what an agent is told and what the validator accepts cannot drift.
pub const GUIDE: &str = include_str!("guide.md");

/// A smoke test file, checked and ready to run.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Test {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub version: u32,
    pub inputs: Vec<Input>,
    /// For the whole run, setup to teardown.
    pub timeout_secs: u32,
    /// Run first; a failure here fails the run before any step.
    pub setup: Vec<Step>,
    /// The test itself, in order, stopping at the first failure.
    pub steps: Vec<Step>,
    /// Always run, however the run ended; a failure here is reported but changes no verdict.
    pub teardown: Vec<Step>,
}

impl Test {
    /// Every step in the order it runs, with its phase.
    pub fn all_steps(&self) -> impl Iterator<Item = (Phase, &Step)> {
        self.setup
            .iter()
            .map(|s| (Phase::Setup, s))
            .chain(self.steps.iter().map(|s| (Phase::Steps, s)))
            .chain(self.teardown.iter().map(|s| (Phase::Teardown, s)))
    }
}

/// Which list a step is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Setup,
    Steps,
    Teardown,
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Phase::Setup => "setup",
            Phase::Steps => "steps",
            Phase::Teardown => "teardown",
        }
    }
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

/// No `harness` here, unlike a workflow's inputs: a test starts no agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    Text,
    Choice,
}

/// One step of a test.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: String,
    /// Flattened, so a step reads `{ id, action: "run", program, … }` as the file does.
    #[serde(flatten)]
    pub action: Action,
}

/// What a step does. Every string here may hold `{{ variables }}` — except a `shell` line,
/// which gets them as environment variables instead. See [`crate::template`].
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Action {
    /// A program and its arguments, each argument its own value. Ends when it exits.
    Run {
        program: String,
        args: Vec<String>,
        /// Relative to the workspace, and inside it.
        cwd: Option<String>,
        env: BTreeMap<String, String>,
        stdin: Option<String>,
        expect: ProcessExpect,
        timeout_secs: Option<u32>,
    },
    /// One line for the user's shell, for pipes. The login shell on Unix, PowerShell on Windows.
    Shell {
        line: String,
        cwd: Option<String>,
        env: BTreeMap<String, String>,
        expect: ProcessExpect,
        timeout_secs: Option<u32>,
    },
    /// A program kept running for the rest of the test — a dev server — and stopped at the end.
    Start {
        program: String,
        args: Vec<String>,
        cwd: Option<String>,
        env: BTreeMap<String, String>,
    },
    /// Wait until something is ready: a port answers, an address returns 2xx, a file exists, a
    /// command exits 0.
    WaitFor {
        target: WaitTarget,
        interval_secs: u32,
        timeout_secs: Option<u32>,
    },
    /// A request, with expectations about the response.
    Http {
        method: HttpMethod,
        url: String,
        headers: BTreeMap<String, String>,
        /// The body as text.
        body: Option<String>,
        /// The body as JSON text, sent with `Content-Type: application/json`.
        json: Option<String>,
        expect: HttpExpect,
        timeout_secs: Option<u32>,
    },
    /// A short list of actions on a page, in a Chromium found on the machine.
    Browser {
        actions: Vec<BrowserAction>,
        screenshots: Screenshots,
        viewport: Option<Viewport>,
        /// Show the browser window rather than running it headless.
        headed: bool,
        timeout_secs: Option<u32>,
    },
    /// A step a person does. The run waits for **Pass** or **Fail** in the app.
    Manual { prompt: String },
}

impl Action {
    /// The name a file uses for this action.
    pub fn name(&self) -> &'static str {
        match self {
            Action::Run { .. } => "run",
            Action::Shell { .. } => "shell",
            Action::Start { .. } => "start",
            Action::WaitFor { .. } => "wait_for",
            Action::Http { .. } => "http",
            Action::Browser { .. } => "browser",
            Action::Manual { .. } => "manual",
        }
    }

    /// What a finished step of this kind makes available as `{{ steps.<id>.<field> }}`.
    pub fn produces(name: &str) -> &'static [&'static str] {
        match name {
            "run" | "shell" => &["exit", "stdout", "stderr"],
            "start" => &["pid", "stdout", "stderr"],
            "wait_for" => &["waited"],
            "http" => &["status", "body", "headers"],
            "browser" => &["url", "title"],
            "manual" => &["note"],
            _ => &[],
        }
    }
}

/// What a `run` or `shell` step expects of its process. Every one given is checked, and the
/// step's note lists each that failed.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProcessExpect {
    /// Exit 0 unless the file says otherwise.
    pub exit: ExitExpect,
    pub stdout_contains: Option<String>,
    /// A regular expression.
    pub stdout_matches: Option<String>,
    pub stderr_empty: bool,
    pub stdout_json: Option<JsonExpect>,
}

impl Default for ProcessExpect {
    fn default() -> Self {
        Self {
            exit: ExitExpect::Code(0),
            stdout_contains: None,
            stdout_matches: None,
            stderr_empty: false,
            stdout_json: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ExitExpect {
    Code(i32),
    Nonzero,
}

/// A value at a dotted path in the JSON output: `{ path: status, equals: ok }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JsonExpect {
    pub path: String,
    /// The expected value as JSON text, so its type is kept: `ok` is `"ok"`, `1` is `1`, `true`
    /// is `true`, and `"1"` stays a string.
    pub equals: String,
}

/// What an `http` step expects of the response.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HttpExpect {
    /// Any 2xx unless the file says otherwise.
    pub status: StatusExpect,
    /// JSON text whose every key must appear, with its value, in the response.
    pub json: Option<String>,
    pub body_contains: Option<String>,
    /// A regular expression.
    pub body_matches: Option<String>,
    pub header: Option<HeaderExpect>,
}

impl Default for HttpExpect {
    fn default() -> Self {
        Self {
            status: StatusExpect::Class(2),
            json: None,
            body_contains: None,
            body_matches: None,
            header: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum StatusExpect {
    /// One status, like 200.
    Exact(u16),
    /// A hundred of them, like `2xx`: the first digit.
    Class(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HeaderExpect {
    pub name: String,
    pub contains: String,
}

/// What a `wait_for` step waits on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WaitTarget {
    /// A TCP port on localhost accepts a connection. A number, or `{{ run.port }}`.
    Port { port: String },
    /// An address answers 2xx.
    Url { url: String },
    /// A path, relative to the workspace, exists.
    File { file: String },
    /// A program exits 0.
    Command { program: String, args: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
}

impl HttpMethod {
    pub fn name(self) -> &'static str {
        match self {
            HttpMethod::Get => "get",
            HttpMethod::Post => "post",
            HttpMethod::Put => "put",
            HttpMethod::Delete => "delete",
            HttpMethod::Patch => "patch",
        }
    }
}

/// When a `browser` step takes a screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Screenshots {
    OnFailure,
    EveryStep,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

/// One action in a `browser` step's `do` list. Selectors are CSS, or `text=…` for an element by
/// its text. Every action waits for its selector to exist and be visible before acting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum BrowserAction {
    Goto {
        url: String,
    },
    Click {
        selector: String,
    },
    Fill {
        selector: String,
        text: String,
    },
    /// A key, to the focused element: `Enter`, `Tab`, `Escape`.
    Press {
        key: String,
    },
    Select {
        selector: String,
        value: String,
    },
    /// A selector, or text, to appear.
    WaitFor {
        selector: Option<String>,
        text: Option<String>,
    },
    ExpectText {
        text: String,
    },
    ExpectVisible {
        selector: String,
    },
    ExpectHidden {
        selector: String,
    },
    /// A glob over the page's address, like `**/dashboard`.
    ExpectUrl {
        pattern: String,
    },
    ExpectTitle {
        title: String,
    },
    /// Saved under this name in the run's output.
    Screenshot {
        name: String,
    },
}

/// One test as the catalog lists it: runnable, or not, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// The file, as an absolute path.
    pub path: String,
    /// Empty when the test can run.
    pub problems: Vec<Problem>,
    /// The checked test; `None` when there are problems.
    pub test: Option<Test>,
    /// The file exactly as written.
    pub text: String,
}

impl Entry {
    pub fn runnable(&self) -> bool {
        self.problems.is_empty() && self.test.is_some()
    }

    /// Whether the test has a `browser` step, which needs a browser on the machine.
    pub fn uses_browser(&self) -> bool {
        self.test.as_ref().is_some_and(|test| {
            test.all_steps()
                .any(|(_, step)| matches!(step.action, Action::Browser { .. }))
        })
    }
}

/// The first-message section that tells a new agent this project has smoke tests, the way
/// `ys memory` and `ys task` are introduced. The composer adds it when the worktree has a
/// `.yardsort/tests` folder or the project has a test environment set. Framed as a pointer, not
/// an instruction, and it names approval so an agent does not run a file the user has not seen.
pub const LAUNCH_SECTION: &str = "## Smoke tests\n\n\
    This project has smoke tests in `.yardsort/tests/`: `ys test list` lists them, and \
    `ys test run <id>` runs one in this workspace, with the result from `ys test wait <run>`. \
    To write one for what you build, `ys test guide` prints the format; put it beside the others, \
    check it with `ys test validate`, and ask before running a new file — the user approves each \
    new test in Yardsort.";

/// Where a worktree keeps its smoke tests.
pub fn dir(worktree: &Path) -> PathBuf {
    worktree.join(".yardsort").join("tests")
}

/// Write a test file into a worktree's `.yardsort/tests`, at `<id>.yaml` for the file's own id,
/// and return where it went. This is how **Describe it** saves what a model wrote; the usual way
/// a test gets here is an agent writing the file itself. The file then is an ordinary uncommitted
/// change in the worktree, like any the agent makes — nothing is approved or run by saving.
///
/// The id comes from the text, valid or not, so a draft with problems can still be saved and
/// fixed in place. A different file in the folder already holding that id is a clash, refused
/// rather than left as two files one id, which neither would then run under.
///
/// Saving only ever *creates*: a path already there — a test with this id above all — is refused,
/// never overwritten, so an agent's uncommitted edits are safe. The folder must resolve inside
/// the worktree, and the write goes through an exclusively created temporary, so a planted
/// symlink cannot send a save out to some other file the app happens to be able to reach.
pub fn save(worktree: &Path, text: &str) -> crate::error::IpcResult<PathBuf> {
    use crate::error::IpcError;
    use std::io::Write;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(IpcError::new(
            "test_too_big",
            format!("A smoke test file may be at most {MAX_FILE_BYTES} bytes."),
        ));
    }
    let id = match parse(text) {
        Ok(test) => Some(test.id),
        Err(invalid) => invalid.id,
    }
    .filter(|id| crate::yaml::is_id(id))
    .ok_or_else(|| {
        IpcError::new(
            "test_invalid_id",
            "Give the test an `id:` first: lowercase letters, digits and `-`, like `login`.",
        )
    })?;
    let folder = dir(worktree);
    let path = folder.join(format!("{id}.yaml"));
    // Another file, by any name, already holding this id: two files one id, neither would run.
    if let Some(other) = catalog(worktree)
        .into_iter()
        .find(|entry| entry.id == id && Path::new(&entry.path) != path)
    {
        return Err(IpcError::new(
            "test_exists",
            format!(
                "{} already has the id `{id}`. Give this one an id of its own.",
                other.path
            ),
        ));
    }
    std::fs::create_dir_all(&folder)
        .map_err(|e| IpcError::new("io", format!("Cannot create {}: {e}", folder.display())))?;
    // The folder must resolve inside the worktree. A `.yardsort` or `.yardsort/tests` symlink
    // that escaped it would let a save land on a file the user never authorised, anywhere the
    // app can reach; refuse rather than follow it out.
    let contained = match (folder.canonicalize(), worktree.canonicalize()) {
        (Ok(resolved), Ok(root)) => resolved.starts_with(&root),
        _ => false,
    };
    if !contained {
        return Err(IpcError::new(
            "test_outside_worktree",
            format!(
                "{} does not resolve inside the workspace, so a test cannot be saved there.",
                folder.display()
            ),
        ));
    }
    // Create-only: never replace a test already there. Describe it writes new tests, and silently
    // overwriting one would destroy its file — uncommitted edits and all. A symlink at the path
    // counts as present (via `symlink_metadata`, which does not follow it) and is refused too.
    if path.symlink_metadata().is_ok() {
        return Err(IpcError::new(
            "test_exists",
            format!(
                "{} already exists. Give this test an id of its own, or edit that file in the \
                 worktree.",
                path.display()
            ),
        ));
    }
    let written = |path: &Path, e: std::io::Error| {
        IpcError::new("io", format!("Cannot write {}: {e}", path.display()))
    };
    // Write through a uniquely named temporary, created exclusively (`create_new`, so an existing
    // file or symlink at the path makes it fail rather than be followed), then rename it over the
    // destination — rename replaces a symlink at the target instead of following it. So a
    // half-written file never sits where a run would read it, and neither step follows a link out.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temporary = folder.join(format!(".{id}.{}-{stamp}.saving", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| written(&temporary, e))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| {
            let _ = std::fs::remove_file(&temporary);
            written(&temporary, e)
        })?;
    drop(file);
    std::fs::rename(&temporary, &path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        written(&path, e)
    })?;
    Ok(path)
}

/// Every test in a worktree's `.yardsort/tests`, sorted by name. A missing folder has none.
///
/// A file that does not parse still takes the id of its file name, so it is listed with its
/// problems rather than vanishing. Two files with one id: the first by file name is used, and
/// the second is listed with the clash.
pub fn catalog(worktree: &Path) -> Vec<Entry> {
    load(worktree).into_iter().map(|(entry, _)| entry).collect()
}

/// The test with this id, the way a run would find it: the first file, by name, to claim it.
pub fn find(worktree: &Path, id: &str) -> Option<Entry> {
    load(worktree)
        .into_iter()
        .find(|(entry, duplicate)| entry.id == id && !duplicate)
        .map(|(entry, _)| entry)
}

/// The catalog, each entry marked when a file earlier by name already claimed its id.
fn load(worktree: &Path) -> Vec<(Entry, bool)> {
    let mut entries: Vec<(Entry, bool)> = Vec::new();
    let mut claimed: Vec<(String, PathBuf)> = Vec::new();
    for path in files(&dir(worktree)) {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut found = match read(&path) {
            Ok(text) => entry(&path, &stem, text),
            Err(problem) => Entry {
                id: stem.clone(),
                name: stem.clone(),
                description: None,
                path: path.to_string_lossy().into_owned(),
                problems: vec![problem],
                test: None,
                text: String::new(),
            },
        };
        let duplicate = match claimed.iter().find(|(other, _)| *other == found.id) {
            Some((_, first)) => {
                found.problems.insert(
                    0,
                    Problem::whole(format!(
                        "{} already has the id `{}`, and ids must be unique. This file is \
                         ignored until one of them changes.",
                        first.display(),
                        found.id
                    )),
                );
                found.test = None;
                true
            }
            None => {
                claimed.push((found.id.clone(), path.clone()));
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

fn entry(path: &Path, stem: &str, text: String) -> Entry {
    let path = path.to_string_lossy().into_owned();
    match parse(&text) {
        Ok(test) => Entry {
            id: test.id.clone(),
            name: test.name.clone(),
            description: test.description.clone(),
            path,
            problems: Vec::new(),
            test: Some(test),
            text,
        },
        Err(invalid) => {
            let id = invalid.id.unwrap_or_else(|| stem.to_owned());
            Entry {
                name: invalid.name.unwrap_or_else(|| id.clone()),
                id,
                description: None,
                path,
                problems: invalid.problems,
                test: None,
                text,
            }
        }
    }
}

/// Read a test file as text, refusing ones too big or not text.
pub fn read(path: &Path) -> Result<String, Problem> {
    let size = std::fs::metadata(path)
        .map_err(|e| Problem::whole(format!("Cannot read {}: {e}", path.display())))?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(Problem::whole(format!(
            "{} is {size} bytes; a smoke test file may be at most {MAX_FILE_BYTES}.",
            path.display()
        )));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| Problem::whole(format!("Cannot read {}: {e}", path.display())))?;
    String::from_utf8(bytes)
        .map_err(|_| Problem::whole(format!("{} is not UTF-8 text.", path.display())))
}

/// `*.yaml` and `*.yml` directly in `dir`, by file name. A missing directory has none.
fn files(dir: &Path) -> Vec<PathBuf> {
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

/// Tests that launch a real browser take turns, and run apart from those that start a server.
/// Several browsers starting at once have kept a small CI runner busy for half a minute, so a
/// server beside them missed its time to come up, in tests that were not about that.
#[cfg(test)]
pub(crate) mod test_turns {
    use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

    static TURNS: RwLock<()> = RwLock::new(());

    /// For a test that launches a browser: held, nothing else that takes a turn runs.
    pub fn browser() -> RwLockWriteGuard<'static, ()> {
        TURNS.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// For a test that starts a server: these run together, but never beside a browser.
    pub fn server() -> RwLockReadGuard<'static, ()> {
        TURNS.read().unwrap_or_else(PoisonError::into_inner)
    }
}
