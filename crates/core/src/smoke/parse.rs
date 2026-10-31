//! From YAML text to a [`Test`], with a line and column on every mistake.
//!
//! Two passes, as the workflow parser has: serde's first, for shape and types, stopping at the
//! first mistake; then ours, over a raw form that kept each value's position — names, each
//! action's keys, expectations, the browser vocabulary, and every `{{ variable }}` — reporting
//! everything it finds, sorted by where it is.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Deserialize;
use serde_saphyr::{Location, Spanned};

use super::{
    Action, BrowserAction, ExitExpect, HeaderExpect, HttpExpect, HttpMethod, Input, InputKind,
    Invalid, JsonExpect, Phase, Problem, ProcessExpect, Screenshots, StatusExpect, Step, Test,
    Viewport, WaitTarget, DEFAULT_RUN_TIMEOUT_SECS, SCHEMA_VERSION,
};
use crate::template::{self, Placeholder};
use crate::yaml::{self, at, is_id, is_name, list, suggestion};

/// What a `{{ … }}` may name, besides `inputs.*`, `env.*` and `steps.*`.
const FIELDS: &[(&str, &[&str])] = &[
    ("project", &["name", "root"]),
    ("workspace", &["name", "branch", "path"]),
    ("run", &["id", "port", "dir"]),
];

const ACTIONS: &[&str] = &[
    "run", "shell", "start", "wait_for", "http", "browser", "manual",
];

const METHODS: &[&str] = &["get", "post", "put", "delete", "patch"];

const BROWSER_ACTIONS: &[&str] = &[
    "goto",
    "click",
    "fill",
    "press",
    "select",
    "wait_for",
    "expect_text",
    "expect_visible",
    "expect_hidden",
    "expect_url",
    "expect_title",
    "screenshot",
];

/// The keys a step of each action may have, besides `id` and `action`.
fn allowed(action: &str) -> &'static [&'static str] {
    match action {
        "run" => &[
            "program", "args", "cwd", "env", "stdin", "expect", "timeout",
        ],
        "shell" => &["line", "cwd", "env", "expect", "timeout"],
        "start" => &["program", "args", "cwd", "env"],
        "wait_for" => &[
            "port", "url", "file", "program", "args", "interval", "timeout",
        ],
        "http" => &[
            "get", "post", "put", "delete", "patch", "headers", "body", "json", "expect", "timeout",
        ],
        "browser" => &["do", "screenshots", "viewport", "headed", "timeout"],
        "manual" => &["prompt"],
        _ => &[],
    }
}

/// The expectations a `run` or `shell` step may have, and the ones an `http` step may have.
const PROCESS_EXPECT: &[&str] = &[
    "exit",
    "stdout_contains",
    "stdout_matches",
    "stderr_empty",
    "stdout_json",
];
const HTTP_EXPECT: &[&str] = &["status", "json", "body_contains", "body_matches", "header"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTest {
    id: Spanned<String>,
    name: Spanned<String>,
    #[serde(default)]
    description: Option<String>,
    version: Spanned<u32>,
    #[serde(default)]
    inputs: Vec<RawInput>,
    #[serde(default)]
    timeout: Option<Spanned<String>>,
    #[serde(default)]
    setup: Vec<RawStep>,
    steps: Spanned<Vec<RawStep>>,
    #[serde(default)]
    teardown: Vec<RawStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInput {
    id: Spanned<String>,
    kind: Spanned<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    default: Option<Spanned<String>>,
    #[serde(default)]
    options: Option<Spanned<Vec<Spanned<String>>>>,
}

/// A number or a piece of text, where the file may give either: `exit: 0` or `exit: nonzero`,
/// `port: 8080` or `port: "{{ run.port }}"`.
#[derive(Deserialize)]
#[serde(untagged)]
enum NumberOrText {
    Number(i64),
    Text(String),
}

impl NumberOrText {
    fn text(&self) -> String {
        match self {
            NumberOrText::Number(n) => n.to_string(),
            NumberOrText::Text(t) => t.clone(),
        }
    }
}

/// Every key any action takes, all optional here: which belong to which action is checked
/// afterwards, so that a misplaced key is named as misplaced rather than unknown.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    id: Spanned<String>,
    action: Spanned<String>,
    #[serde(default)]
    program: Option<Spanned<String>>,
    #[serde(default)]
    args: Option<Spanned<Vec<Spanned<String>>>>,
    #[serde(default)]
    cwd: Option<Spanned<String>>,
    #[serde(default)]
    env: Option<Spanned<BTreeMap<String, Spanned<String>>>>,
    #[serde(default)]
    stdin: Option<Spanned<String>>,
    #[serde(default)]
    expect: Option<Spanned<RawExpect>>,
    #[serde(default)]
    timeout: Option<Spanned<String>>,
    #[serde(default)]
    line: Option<Spanned<String>>,
    #[serde(default)]
    port: Option<Spanned<NumberOrText>>,
    #[serde(default)]
    url: Option<Spanned<String>>,
    #[serde(default)]
    file: Option<Spanned<String>>,
    #[serde(default)]
    interval: Option<Spanned<String>>,
    #[serde(default)]
    get: Option<Spanned<String>>,
    #[serde(default)]
    post: Option<Spanned<String>>,
    #[serde(default)]
    put: Option<Spanned<String>>,
    #[serde(default)]
    delete: Option<Spanned<String>>,
    #[serde(default)]
    patch: Option<Spanned<String>>,
    #[serde(default)]
    headers: Option<Spanned<BTreeMap<String, Spanned<String>>>>,
    #[serde(default)]
    body: Option<Spanned<String>>,
    #[serde(default)]
    json: Option<Spanned<serde_json::Value>>,
    #[serde(default, rename = "do")]
    actions: Option<Spanned<Vec<BrowserItem>>>,
    #[serde(default)]
    screenshots: Option<Spanned<String>>,
    #[serde(default)]
    viewport: Option<Spanned<String>>,
    #[serde(default)]
    headed: Option<Spanned<bool>>,
    #[serde(default)]
    prompt: Option<Spanned<String>>,
}

impl RawStep {
    /// The action-specific keys this step has, with where each is.
    fn present(&self) -> Vec<(&'static str, &Location)> {
        fn at<T>(field: &Option<Spanned<T>>) -> Option<&Location> {
            field.as_ref().map(|s| &s.referenced)
        }
        [
            ("program", at(&self.program)),
            ("args", at(&self.args)),
            ("cwd", at(&self.cwd)),
            ("env", at(&self.env)),
            ("stdin", at(&self.stdin)),
            ("expect", at(&self.expect)),
            ("timeout", at(&self.timeout)),
            ("line", at(&self.line)),
            ("port", at(&self.port)),
            ("url", at(&self.url)),
            ("file", at(&self.file)),
            ("interval", at(&self.interval)),
            ("get", at(&self.get)),
            ("post", at(&self.post)),
            ("put", at(&self.put)),
            ("delete", at(&self.delete)),
            ("patch", at(&self.patch)),
            ("headers", at(&self.headers)),
            ("body", at(&self.body)),
            ("json", at(&self.json)),
            ("do", at(&self.actions)),
            ("screenshots", at(&self.screenshots)),
            ("viewport", at(&self.viewport)),
            ("headed", at(&self.headed)),
            ("prompt", at(&self.prompt)),
        ]
        .into_iter()
        .filter_map(|(key, at)| at.map(|at| (key, at)))
        .collect()
    }
}

/// Every expectation any step takes, all optional; which belong to which action is checked
/// afterwards.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExpect {
    #[serde(default)]
    exit: Option<Spanned<NumberOrText>>,
    #[serde(default)]
    stdout_contains: Option<Spanned<String>>,
    #[serde(default)]
    stdout_matches: Option<Spanned<String>>,
    #[serde(default)]
    stderr_empty: Option<Spanned<bool>>,
    #[serde(default)]
    stdout_json: Option<Spanned<RawJsonExpect>>,
    #[serde(default)]
    status: Option<Spanned<NumberOrText>>,
    #[serde(default)]
    json: Option<Spanned<serde_json::Value>>,
    #[serde(default)]
    body_contains: Option<Spanned<String>>,
    #[serde(default)]
    body_matches: Option<Spanned<String>>,
    #[serde(default)]
    header: Option<Spanned<RawHeaderExpect>>,
}

impl RawExpect {
    fn present(&self) -> Vec<(&'static str, &Location)> {
        fn at<T>(field: &Option<Spanned<T>>) -> Option<&Location> {
            field.as_ref().map(|s| &s.referenced)
        }
        [
            ("exit", at(&self.exit)),
            ("stdout_contains", at(&self.stdout_contains)),
            ("stdout_matches", at(&self.stdout_matches)),
            ("stderr_empty", at(&self.stderr_empty)),
            ("stdout_json", at(&self.stdout_json)),
            ("status", at(&self.status)),
            ("json", at(&self.json)),
            ("body_contains", at(&self.body_contains)),
            ("body_matches", at(&self.body_matches)),
            ("header", at(&self.header)),
        ]
        .into_iter()
        .filter_map(|(key, at)| at.map(|at| (key, at)))
        .collect()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJsonExpect {
    path: Spanned<String>,
    /// Any JSON value, kept as one: `1` and `"1"` are different answers.
    equals: Spanned<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHeaderExpect {
    name: Spanned<String>,
    contains: Spanned<String>,
}

/// One item of a `do` list, at its position: one key and what follows it.
type BrowserItem = Spanned<BTreeMap<String, BrowserArg>>;

/// What follows a browser action's key: `goto: <url>`, or `fill: { selector, text }`.
#[derive(Deserialize)]
#[serde(untagged)]
enum BrowserArg {
    Text(String),
    Map(BTreeMap<String, String>),
}

/// Check `text` and build the test it describes, or say everything that is wrong with it.
pub fn parse(text: &str) -> Result<Test, Invalid> {
    let heading = yaml::heading(text, "smoke test", SCHEMA_VERSION);
    let invalid = |problems| Invalid {
        id: heading.id.clone(),
        name: heading.name.clone(),
        problems,
    };
    if let Some(later) = heading.later_version.clone() {
        return Err(invalid(vec![later]));
    }
    let raw: RawTest = match serde_saphyr::from_str(text) {
        Ok(raw) => raw,
        Err(error) => return Err(invalid(vec![yaml::from_serde(&error)])),
    };
    let mut check = Checker {
        source: text,
        problems: Vec::new(),
    };
    let test = check.test(raw);
    if check.problems.is_empty() {
        Ok(test)
    } else {
        let mut problems = check.problems;
        problems.sort_by_key(|p| (p.line.unwrap_or(0), p.column.unwrap_or(0)));
        problems.dedup();
        Err(Invalid {
            id: Some(test.id),
            name: Some(test.name),
            problems,
        })
    }
}

/// What the variable check needs to know about the file around one field.
struct Scope<'a> {
    inputs: &'a HashMap<String, InputKind>,
    /// Step id to its action, for every step in the file.
    actions: &'a HashMap<String, String>,
    /// The steps that run before this one: everything earlier in the file.
    before: &'a BTreeSet<String>,
}

struct Checker<'a> {
    source: &'a str,
    problems: Vec<Problem>,
}

impl Checker<'_> {
    fn problem(&mut self, location: &Location, message: impl Into<String>) {
        self.problems.push(at(location, message));
    }

    fn test(&mut self, raw: RawTest) -> Test {
        if !is_id(&raw.id.value) {
            self.problem(
                &raw.id.referenced,
                format!(
                    "`{}` cannot be an id. Use lowercase letters, digits and `-`, like `login`.",
                    raw.id.value
                ),
            );
        }
        if raw.name.value.trim().is_empty() {
            self.problem(&raw.name.referenced, "The name is empty.");
        }
        if raw.version.value != SCHEMA_VERSION {
            self.problem(
                &raw.version.referenced,
                format!("`version` is {SCHEMA_VERSION} for this Yardsort."),
            );
        }
        let timeout_secs = match &raw.timeout {
            Some(field) => self
                .duration(field, "a timeout")
                .unwrap_or(DEFAULT_RUN_TIMEOUT_SECS),
            None => DEFAULT_RUN_TIMEOUT_SECS,
        };
        let inputs = self.inputs(&raw.inputs);
        if raw.steps.value.is_empty() {
            self.problem(
                &raw.steps.referenced,
                "A smoke test needs at least one step.",
            );
        }
        let phases = [
            (Phase::Setup, &raw.setup),
            (Phase::Steps, &raw.steps.value),
            (Phase::Teardown, &raw.teardown),
        ];
        // Ids are one namespace across the three lists: `{{ steps.<id>.<field> }}` names a step
        // wherever it is.
        let mut actions: HashMap<String, String> = HashMap::new();
        let mut first_line: HashMap<&str, u64> = HashMap::new();
        for (_, steps) in &phases {
            for step in steps.iter() {
                let id = &step.id.value;
                if !is_name(id) {
                    self.problem(
                        &step.id.referenced,
                        format!(
                            "`{id}` cannot be a step's id. Start with a letter; use lowercase \
                             letters, digits, `_` and `-`."
                        ),
                    );
                }
                match first_line.get(id.as_str()) {
                    Some(line) => self.problem(
                        &step.id.referenced,
                        format!("There is already a step `{id}`, on line {line}."),
                    ),
                    None => {
                        first_line.insert(id, step.id.referenced.line());
                        actions.insert(id.clone(), step.action.value.clone());
                    }
                }
            }
        }
        // The first input with an id is the one a variable means; a second is already reported.
        let mut input_kinds: HashMap<String, InputKind> = HashMap::new();
        for input in &inputs {
            input_kinds.entry(input.id.clone()).or_insert(input.kind);
        }
        let mut before: BTreeSet<String> = BTreeSet::new();
        let mut checked: [Vec<Step>; 3] = Default::default();
        for (index, (_, steps)) in phases.iter().enumerate() {
            for step in steps.iter() {
                let scope = Scope {
                    inputs: &input_kinds,
                    actions: &actions,
                    before: &before,
                };
                checked[index].push(self.step(step, &scope));
                before.insert(step.id.value.clone());
            }
        }
        let [setup, steps, teardown] = checked;
        Test {
            id: raw.id.value,
            name: raw.name.value.trim().to_owned(),
            description: raw
                .description
                .map(|d| d.trim().to_owned())
                .filter(|d| !d.is_empty()),
            version: raw.version.value,
            inputs,
            timeout_secs,
            setup,
            steps,
            teardown,
        }
    }

    fn inputs(&mut self, raw: &[RawInput]) -> Vec<Input> {
        let mut seen: HashMap<&str, &Location> = HashMap::new();
        let mut inputs = Vec::new();
        for input in raw {
            let id = &input.id.value;
            if !is_name(id) {
                self.problem(
                    &input.id.referenced,
                    format!(
                        "`{id}` cannot be an input's id. Start with a letter; use lowercase \
                         letters, digits, `_` and `-`."
                    ),
                );
            }
            if let Some(first) = seen.insert(id, &input.id.referenced) {
                self.problem(
                    &input.id.referenced,
                    format!(
                        "There is already an input `{id}`, on line {}.",
                        first.line()
                    ),
                );
            }
            let kind = match input.kind.value.as_str() {
                "text" => InputKind::Text,
                "choice" => InputKind::Choice,
                "harness" => {
                    self.problem(
                        &input.kind.referenced,
                        "A smoke test has no `harness` input: it starts no agent. Use `text` or \
                         `choice`.",
                    );
                    InputKind::Text
                }
                other => {
                    self.problem(
                        &input.kind.referenced,
                        format!("`{other}` is not a kind of input. Use `text` or `choice`."),
                    );
                    InputKind::Text
                }
            };
            let mut options = Vec::new();
            match (&input.options, kind) {
                (Some(given), InputKind::Choice) => {
                    let mut unique = BTreeSet::new();
                    for option in &given.value {
                        if !unique.insert(option.value.as_str()) {
                            self.problem(
                                &option.referenced,
                                format!("`{}` is listed twice.", option.value),
                            );
                        }
                        options.push(option.value.clone());
                    }
                    if options.is_empty() {
                        self.problem(&given.referenced, "A choice needs at least one option.");
                    }
                }
                (None, InputKind::Choice) => self.problem(
                    &input.kind.referenced,
                    format!("The choice `{id}` needs `options`: the answers to choose from."),
                ),
                (Some(given), _) => {
                    self.problem(&given.referenced, "Only a `choice` input has `options`.")
                }
                (None, _) => {}
            }
            if let Some(default) = &input.default {
                if kind == InputKind::Choice
                    && !options.is_empty()
                    && !options.contains(&default.value)
                {
                    self.problem(
                        &default.referenced,
                        format!("`{}` is not one of the options.", default.value),
                    );
                }
            }
            let label = input
                .label
                .as_deref()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .unwrap_or(id)
                .to_owned();
            inputs.push(Input {
                id: id.clone(),
                kind,
                label,
                required: input.required,
                default: input.default.as_ref().map(|d| d.value.clone()),
                options,
            });
        }
        inputs
    }

    fn step(&mut self, raw: &RawStep, scope: &Scope) -> Step {
        let action_name = raw.action.value.as_str();
        let id = raw.id.value.clone();
        if !ACTIONS.contains(&action_name) {
            self.problem(
                &raw.action.referenced,
                format!(
                    "`{action_name}` is not an action. The actions are {}.",
                    list(ACTIONS.iter())
                ),
            );
            return Step {
                id,
                action: Action::Manual {
                    prompt: String::new(),
                },
            };
        }
        let allowed = allowed(action_name);
        for (key, location) in raw.present() {
            if !allowed.contains(&key) {
                self.problem(
                    location,
                    format!("`{key}` is not something a `{action_name}` step takes."),
                );
            }
        }
        let action = match action_name {
            "run" => Action::Run {
                program: self.required(&raw.program, raw, "program", "what to run", scope),
                args: self.args(raw, scope),
                cwd: self.cwd(raw),
                env: self.env(&raw.env, scope),
                stdin: self.text(&raw.stdin, scope),
                expect: self.process_expect(raw, scope),
                timeout_secs: self.timeout(raw),
            },
            "shell" => Action::Shell {
                line: self.line(raw),
                cwd: self.cwd(raw),
                env: self.env(&raw.env, scope),
                expect: self.process_expect(raw, scope),
                timeout_secs: self.timeout(raw),
            },
            "start" => Action::Start {
                program: self.required(&raw.program, raw, "program", "what to start", scope),
                args: self.args(raw, scope),
                cwd: self.cwd(raw),
                env: self.env(&raw.env, scope),
            },
            "wait_for" => Action::WaitFor {
                target: self.wait_target(raw, scope),
                interval_secs: match &raw.interval {
                    Some(field) => self.duration(field, "an interval").unwrap_or(1),
                    None => 1,
                },
                timeout_secs: self.timeout(raw),
            },
            "http" => self.http(raw, scope),
            "browser" => self.browser(raw, scope),
            _ => Action::Manual {
                prompt: self.required(
                    &raw.prompt,
                    raw,
                    "prompt",
                    "what the person should check",
                    scope,
                ),
            },
        };
        Step { id, action }
    }

    /// A key the action cannot do without.
    fn required(
        &mut self,
        field: &Option<Spanned<String>>,
        step: &RawStep,
        key: &str,
        what: &str,
        scope: &Scope,
    ) -> String {
        match field {
            Some(given) => {
                if given.value.trim().is_empty() {
                    self.problem(&given.referenced, format!("`{key}` is empty."));
                }
                self.text(field, scope).unwrap_or_default()
            }
            None => {
                self.problem(
                    &step.action.referenced,
                    format!("A `{}` step needs `{key}`: {what}.", step.action.value),
                );
                String::new()
            }
        }
    }

    fn args(&mut self, raw: &RawStep, scope: &Scope) -> Vec<String> {
        let Some(args) = &raw.args else {
            return Vec::new();
        };
        args.value
            .iter()
            .map(|arg| {
                self.placeholders(&arg.referenced, &arg.value, scope);
                arg.value.clone()
            })
            .collect()
    }

    /// Relative to the workspace and inside it: no absolute path, no `..`, no variables.
    fn cwd(&mut self, raw: &RawStep) -> Option<String> {
        let field = raw.cwd.as_ref()?;
        let value = field.value.trim();
        let path = std::path::Path::new(value);
        let escapes = path.is_absolute()
            || value.starts_with(['/', '\\'])
            || path.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::Prefix(_)
                )
            });
        if value.is_empty() || escapes || value.contains("{{") {
            self.problem(
                &field.referenced,
                "`cwd` is a folder inside the workspace, relative to it, like `web` or \
                 `packages/api`. No `..`, no absolute path, no variables.",
            );
        }
        Some(value.to_owned())
    }

    fn env(
        &mut self,
        field: &Option<Spanned<BTreeMap<String, Spanned<String>>>>,
        scope: &Scope,
    ) -> BTreeMap<String, String> {
        let Some(given) = field else {
            return BTreeMap::new();
        };
        let mut env = BTreeMap::new();
        for (name, value) in &given.value {
            if !is_env_name(name) {
                self.problem(
                    &value.referenced,
                    format!(
                        "`{name}` is not an environment variable name. Use letters, digits and \
                         `_`, not starting with a digit."
                    ),
                );
            }
            self.placeholders(&value.referenced, &value.value, scope);
            env.insert(name.clone(), value.value.clone());
        }
        env
    }

    /// A `shell` line: text for the shell, with no `{{ }}` in it. Variables reach a shell as
    /// environment variables, never by being pasted into the line.
    fn line(&mut self, raw: &RawStep) -> String {
        let Some(field) = &raw.line else {
            self.problem(
                &raw.action.referenced,
                "A `shell` step needs `line`: the command line to run.",
            );
            return String::new();
        };
        if field.value.trim().is_empty() {
            self.problem(&field.referenced, "`line` is empty.");
        }
        if field.value.contains("{{") {
            self.problem(
                &field.referenced,
                "A `shell` line cannot hold `{{ … }}`: a value pasted into a shell line could \
                 become a command. The run's variables are in the environment instead: \
                 `$YS_RUN_PORT`, `$YS_INPUT_<ID>`, `$YS_ENV_<NAME>`, `$YS_WORKSPACE_PATH`. Or \
                 use a `run` step, whose `args` are each one value.",
            );
        }
        field.value.clone()
    }

    fn wait_target(&mut self, raw: &RawStep, scope: &Scope) -> WaitTarget {
        let given: Vec<(&str, &Location)> = [
            ("port", raw.port.as_ref().map(|f| &f.referenced)),
            ("url", raw.url.as_ref().map(|f| &f.referenced)),
            ("file", raw.file.as_ref().map(|f| &f.referenced)),
            ("program", raw.program.as_ref().map(|f| &f.referenced)),
        ]
        .into_iter()
        .filter_map(|(key, at)| at.map(|at| (key, at)))
        .collect();
        let usage = "A `wait_for` step waits for one thing: `port`, `url`, `file`, or `program` \
                     with `args`.";
        if let (Some(args), None) = (&raw.args, &raw.program) {
            self.problem(
                &args.referenced,
                "`args` goes with `program`. A `wait_for` on a port, an address or a file takes \
                 none.",
            );
        }
        match given.as_slice() {
            [] => {
                self.problem(&raw.action.referenced, usage);
                WaitTarget::Url { url: String::new() }
            }
            [_, second, ..] => {
                self.problem(
                    second.1,
                    format!("`{}` is a second thing to wait for. {usage}", second.0),
                );
                WaitTarget::Url { url: String::new() }
            }
            [("port", _)] => {
                let field = raw.port.as_ref().expect("given");
                let text = field.value.text();
                match &field.value {
                    NumberOrText::Number(n) if (1..=65535).contains(n) => {}
                    NumberOrText::Text(t) if t.contains("{{") => {
                        self.placeholders(&field.referenced, t, scope);
                    }
                    _ => self.problem(
                        &field.referenced,
                        format!(
                            "`{text}` is not a port. Use a number from 1 to 65535, or \
                             `{{{{ run.port }}}}`."
                        ),
                    ),
                }
                WaitTarget::Port { port: text }
            }
            [("url", _)] => WaitTarget::Url {
                url: self.required(&raw.url, raw, "url", "the address", scope),
            },
            [("file", _)] => WaitTarget::File {
                file: self.required(&raw.file, raw, "file", "the path", scope),
            },
            _ => WaitTarget::Command {
                program: self.required(&raw.program, raw, "program", "what to run", scope),
                args: self.args(raw, scope),
            },
        }
    }

    fn http(&mut self, raw: &RawStep, scope: &Scope) -> Action {
        let methods: Vec<(HttpMethod, &Spanned<String>)> = [
            (HttpMethod::Get, &raw.get),
            (HttpMethod::Post, &raw.post),
            (HttpMethod::Put, &raw.put),
            (HttpMethod::Delete, &raw.delete),
            (HttpMethod::Patch, &raw.patch),
        ]
        .into_iter()
        .filter_map(|(method, field)| field.as_ref().map(|f| (method, f)))
        .collect();
        let (method, url) = match methods.as_slice() {
            [] => {
                self.problem(
                    &raw.action.referenced,
                    format!(
                        "An `http` step needs the request: one of {}, with the address.",
                        list(METHODS.iter())
                    ),
                );
                (HttpMethod::Get, String::new())
            }
            [(method, field)] => {
                if field.value.trim().is_empty() {
                    self.problem(&field.referenced, format!("`{}` is empty.", method.name()));
                }
                self.placeholders(&field.referenced, &field.value, scope);
                (*method, field.value.clone())
            }
            [(method, field), (second, _), ..] => {
                self.problem(
                    &field.referenced,
                    format!(
                        "An `http` step makes one request: `{}` and `{}` are two.",
                        method.name(),
                        second.name()
                    ),
                );
                (*method, field.value.clone())
            }
        };
        let mut headers = BTreeMap::new();
        if let Some(given) = &raw.headers {
            for (name, value) in &given.value {
                if name.trim().is_empty() || name.chars().any(|c| c.is_whitespace() || c == ':') {
                    self.problem(&value.referenced, format!("`{name}` is not a header name."));
                }
                self.placeholders(&value.referenced, &value.value, scope);
                headers.insert(name.clone(), value.value.clone());
            }
        }
        if let (Some(_), Some(json)) = (&raw.body, &raw.json) {
            self.problem(
                &json.referenced,
                "A request has one body: `body` as text, or `json`. Not both.",
            );
        }
        let json = raw.json.as_ref().map(|field| {
            let text = compact_json(&field.value);
            self.placeholders(&field.referenced, &text, scope);
            text
        });
        Action::Http {
            method,
            url,
            headers,
            body: self.text(&raw.body, scope),
            json,
            expect: self.http_expect(raw, scope),
            timeout_secs: self.timeout(raw),
        }
    }

    fn process_expect(&mut self, raw: &RawStep, scope: &Scope) -> ProcessExpect {
        let mut expect = ProcessExpect::default();
        let Some(given) = &raw.expect else {
            return expect;
        };
        for (key, location) in given.value.present() {
            if HTTP_EXPECT.contains(&key) {
                self.problem(
                    location,
                    format!(
                        "`{key}` is an expectation for an `http` step. A `{}` step expects {}.",
                        raw.action.value,
                        list(PROCESS_EXPECT.iter())
                    ),
                );
            }
        }
        let e = &given.value;
        if let Some(exit) = &e.exit {
            expect.exit = match &exit.value {
                NumberOrText::Number(n) => match i32::try_from(*n) {
                    Ok(code) => ExitExpect::Code(code),
                    Err(_) => {
                        self.problem(&exit.referenced, format!("`{n}` is not an exit code."));
                        ExitExpect::Code(0)
                    }
                },
                NumberOrText::Text(t) if t.trim() == "nonzero" => ExitExpect::Nonzero,
                NumberOrText::Text(t) => {
                    self.problem(
                        &exit.referenced,
                        format!("`{t}` is not an exit code. Use a number, or `nonzero`."),
                    );
                    ExitExpect::Code(0)
                }
            };
        }
        expect.stdout_contains = self.text(&e.stdout_contains, scope);
        expect.stdout_matches = self.regex(&e.stdout_matches);
        expect.stderr_empty = e.stderr_empty.as_ref().is_some_and(|s| s.value);
        if let Some(json) = &e.stdout_json {
            if json.value.path.value.trim().is_empty() {
                self.problem(&json.value.path.referenced, "`path` is empty.");
            }
            let equals = compact_json(&json.value.equals.value);
            self.placeholders(&json.value.equals.referenced, &equals, scope);
            expect.stdout_json = Some(JsonExpect {
                path: json.value.path.value.trim().to_owned(),
                equals,
            });
        }
        expect
    }

    fn http_expect(&mut self, raw: &RawStep, scope: &Scope) -> HttpExpect {
        let mut expect = HttpExpect::default();
        let Some(given) = &raw.expect else {
            return expect;
        };
        for (key, location) in given.value.present() {
            if PROCESS_EXPECT.contains(&key) {
                self.problem(
                    location,
                    format!(
                        "`{key}` is an expectation for a `run` or `shell` step. An `http` step \
                         expects {}.",
                        list(HTTP_EXPECT.iter())
                    ),
                );
            }
        }
        let e = &given.value;
        if let Some(status) = &e.status {
            expect.status = match &status.value {
                NumberOrText::Number(n) if (100..=599).contains(n) => {
                    StatusExpect::Exact(*n as u16)
                }
                NumberOrText::Text(t)
                    if t.len() == 3
                        && t.ends_with("xx")
                        && t.starts_with(|c: char| ('1'..='5').contains(&c)) =>
                {
                    StatusExpect::Class(t.as_bytes()[0] - b'0')
                }
                other => {
                    self.problem(
                        &status.referenced,
                        format!(
                            "`{}` is not a status. Use a number from 100 to 599, or a class \
                             like `2xx`.",
                            other.text()
                        ),
                    );
                    StatusExpect::Class(2)
                }
            };
        }
        expect.json = e.json.as_ref().map(|field| {
            let text = compact_json(&field.value);
            self.placeholders(&field.referenced, &text, scope);
            text
        });
        expect.body_contains = self.text(&e.body_contains, scope);
        expect.body_matches = self.regex(&e.body_matches);
        if let Some(header) = &e.header {
            if header.value.name.value.trim().is_empty() {
                self.problem(&header.value.name.referenced, "`name` is empty.");
            }
            self.placeholders(
                &header.value.contains.referenced,
                &header.value.contains.value,
                scope,
            );
            expect.header = Some(HeaderExpect {
                name: header.value.name.value.trim().to_owned(),
                contains: header.value.contains.value.clone(),
            });
        }
        expect
    }

    fn browser(&mut self, raw: &RawStep, scope: &Scope) -> Action {
        let mut actions = Vec::new();
        match &raw.actions {
            None => self.problem(
                &raw.action.referenced,
                "A `browser` step needs `do`: the list of actions on the page.",
            ),
            Some(given) if given.value.is_empty() => {
                self.problem(&given.referenced, "`do` is empty.");
            }
            Some(given) => {
                for item in &given.value {
                    if let Some(action) = self.browser_action(item, scope) {
                        actions.push(action);
                    }
                }
            }
        }
        let screenshots = match raw
            .screenshots
            .as_ref()
            .map(|s| (s.value.as_str(), &s.referenced))
        {
            None | Some(("on_failure", _)) => Screenshots::OnFailure,
            Some(("every_step", _)) => Screenshots::EveryStep,
            Some(("never", _)) => Screenshots::Never,
            Some((other, location)) => {
                self.problem(
                    location,
                    format!(
                        "`{other}` is not a time to take screenshots. Use `on_failure`, \
                         `every_step` or `never`."
                    ),
                );
                Screenshots::OnFailure
            }
        };
        let viewport = raw.viewport.as_ref().and_then(|field| {
            let parsed = field.value.trim().split_once('x').and_then(|(w, h)| {
                Some(Viewport {
                    width: w.parse().ok().filter(|w| *w > 0)?,
                    height: h.parse().ok().filter(|h| *h > 0)?,
                })
            });
            if parsed.is_none() {
                self.problem(
                    &field.referenced,
                    format!(
                        "`{}` is not a viewport. Write it as width by height, like `1280x800`.",
                        field.value
                    ),
                );
            }
            parsed
        });
        Action::Browser {
            actions,
            screenshots,
            viewport,
            headed: raw.headed.as_ref().is_some_and(|h| h.value),
            timeout_secs: self.timeout(raw),
        }
    }

    /// One item of a `do` list: exactly one key, from the vocabulary, with the shape it takes.
    fn browser_action(&mut self, item: &BrowserItem, scope: &Scope) -> Option<BrowserAction> {
        let at = &item.referenced;
        let mut keys = item.value.keys();
        let (Some(key), None) = (keys.next(), keys.next()) else {
            self.problem(
                at,
                format!(
                    "Each action in `do` is one key and its value, like `click: \"#save\"`. The \
                     actions are {}.",
                    list(BROWSER_ACTIONS.iter())
                ),
            );
            return None;
        };
        let arg = &item.value[key];
        if !BROWSER_ACTIONS.contains(&key.as_str()) {
            self.problem(
                at,
                format!(
                    "`{key}` is not a browser action{}",
                    suggestion(
                        key,
                        BROWSER_ACTIONS
                            .iter()
                            .map(|a| (*a).to_owned())
                            .collect::<Vec<_>>()
                            .iter()
                    )
                ),
            );
            return None;
        }
        // A string where one is wanted, checked for variables.
        let text = |this: &mut Self, what: &str| -> Option<String> {
            match arg {
                BrowserArg::Text(text) if !text.trim().is_empty() => {
                    this.placeholders(at, text, scope);
                    Some(text.clone())
                }
                _ => {
                    this.problem(at, format!("`{key}` takes {what}, like `{key}: \"…\"`."));
                    None
                }
            }
        };
        // A mapping with exactly these keys. The ones in `data` may be empty: filling an input
        // with nothing clears it, and selecting `value=""` is choosing the blank option.
        let fields = |this: &mut Self, wanted: &[&str], data: &[&str]| -> Option<Vec<String>> {
            let BrowserArg::Map(map) = arg else {
                this.problem(at, format!("`{key}` takes {}.", mapping_usage(key, wanted)));
                return None;
            };
            let mut ok = true;
            for name in map.keys() {
                if !wanted.contains(&name.as_str()) {
                    this.problem(
                        at,
                        format!(
                            "`{key}` has no `{name}`. It takes {}.",
                            mapping_usage(key, wanted)
                        ),
                    );
                    ok = false;
                }
            }
            // A wrong key is usually a right one misnamed; saying it is also missing adds nothing.
            if !ok {
                return None;
            }
            let mut values = Vec::new();
            for name in wanted {
                match map.get(*name) {
                    Some(value) if data.contains(name) || !value.trim().is_empty() => {
                        this.placeholders(at, value, scope);
                        values.push(value.clone());
                    }
                    _ => {
                        this.problem(
                            at,
                            format!("`{key}` needs `{name}`: {}.", mapping_usage(key, wanted)),
                        );
                        ok = false;
                    }
                }
            }
            ok.then_some(values)
        };
        Some(match key.as_str() {
            "goto" => BrowserAction::Goto {
                url: text(self, "an address")?,
            },
            "click" => BrowserAction::Click {
                selector: text(self, "a selector")?,
            },
            "fill" => {
                let mut values = fields(self, &["selector", "text"], &["text"])?.into_iter();
                BrowserAction::Fill {
                    selector: values.next()?,
                    text: values.next()?,
                }
            }
            "press" => BrowserAction::Press {
                key: text(self, "a key, like `Enter`")?,
            },
            "select" => {
                let mut values = fields(self, &["selector", "value"], &["value"])?.into_iter();
                BrowserAction::Select {
                    selector: values.next()?,
                    value: values.next()?,
                }
            }
            "wait_for" => match arg {
                BrowserArg::Text(_) => BrowserAction::WaitFor {
                    selector: Some(text(self, "a selector, or `{ text: … }`")?),
                    text: None,
                },
                BrowserArg::Map(_) => {
                    let mut values = fields(self, &["text"], &[])?.into_iter();
                    BrowserAction::WaitFor {
                        selector: None,
                        text: values.next(),
                    }
                }
            },
            "expect_text" => BrowserAction::ExpectText {
                text: text(self, "the text to find on the page")?,
            },
            "expect_visible" => BrowserAction::ExpectVisible {
                selector: text(self, "a selector")?,
            },
            "expect_hidden" => BrowserAction::ExpectHidden {
                selector: text(self, "a selector")?,
            },
            "expect_url" => BrowserAction::ExpectUrl {
                pattern: text(self, "a pattern over the address, like `**/dashboard`")?,
            },
            "expect_title" => BrowserAction::ExpectTitle {
                title: text(self, "the page title")?,
            },
            _ => BrowserAction::Screenshot {
                name: text(self, "a name for the file")?,
            },
        })
    }

    fn regex(&mut self, field: &Option<Spanned<String>>) -> Option<String> {
        let field = field.as_ref()?;
        if let Err(error) = regex::Regex::new(&field.value) {
            let reason = error.to_string();
            let reason = reason.lines().last().unwrap_or("").trim();
            self.problem(
                &field.referenced,
                format!("`{}` is not a regular expression: {reason}.", field.value),
            );
        }
        Some(field.value.clone())
    }

    fn timeout(&mut self, step: &RawStep) -> Option<u32> {
        let field = step.timeout.as_ref()?;
        self.duration(field, "a timeout")
    }

    fn duration(&mut self, field: &Spanned<String>, what: &str) -> Option<u32> {
        // At most a week, so it always fits.
        match yaml::duration(&field.value, what) {
            Ok(secs) => u32::try_from(secs).ok(),
            Err(message) => {
                self.problem(&field.referenced, message);
                None
            }
        }
    }

    /// Template text: every placeholder well-formed and naming something that exists.
    fn text(&mut self, field: &Option<Spanned<String>>, scope: &Scope) -> Option<String> {
        let field = field.as_ref()?;
        self.placeholders(&field.referenced, &field.value, scope);
        Some(field.value.clone())
    }

    /// Check every `{{ … }}` in `value`, a piece of text written at `field`.
    fn placeholders(&mut self, field: &Location, value: &str, scope: &Scope) {
        match template::parse(value) {
            Ok(_) => {
                for var in template::placeholders(value) {
                    self.variable(&var, field, value, scope);
                }
            }
            Err(malformed) => {
                for bad in malformed {
                    let (line, column) = yaml::find(self.source, field, value, &bad.raw, bad.start);
                    self.problems.push(Problem::at(line, column, bad.message));
                }
            }
        }
    }

    /// Report `var` if it names nothing. `value` is the text it was found in.
    fn variable(&mut self, var: &Placeholder, field: &Location, value: &str, scope: &Scope) {
        let Err(message) = resolve(var, scope) else {
            return;
        };
        let (line, column) = yaml::find(self.source, field, value, &var.raw, var.start);
        self.problems.push(Problem::at(line, column, message));
    }
}

/// `{ selector: "…", text: "…" }`, for a message.
fn mapping_usage(key: &str, wanted: &[&str]) -> String {
    format!(
        "`{key}: {{ {} }}`",
        wanted
            .iter()
            .map(|w| format!("{w}: \"…\""))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// A YAML value as one line of JSON text.
fn compact_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// An environment variable name: letters, digits and `_`, not starting with a digit.
pub fn is_env_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// What is wrong with `var` in this scope, if anything.
fn resolve(var: &Placeholder, scope: &Scope) -> Result<(), String> {
    let raw = &var.raw;
    let path: Vec<&str> = var.path.iter().map(String::as_str).collect();
    let fields_of = |namespace: &str| {
        FIELDS
            .iter()
            .find(|(name, _)| *name == namespace)
            .map(|(_, fields)| *fields)
    };
    match path.as_slice() {
        [namespace, field] if fields_of(namespace).is_some() => {
            let fields = fields_of(namespace).unwrap_or_default();
            if fields.contains(field) {
                Ok(())
            } else {
                Err(format!(
                    "`{raw}`: `{namespace}` has no `{field}`. It has {}.",
                    list(fields.iter().copied())
                ))
            }
        }
        [namespace] if fields_of(namespace).is_some() => Err(format!(
            "`{raw}` needs a field: {}.",
            list(
                fields_of(namespace)
                    .unwrap_or_default()
                    .iter()
                    .map(|f| format!("{namespace}.{f}"))
            )
        )),
        ["inputs", id] => {
            if scope.inputs.contains_key(*id) {
                Ok(())
            } else {
                Err(format!(
                    "`{raw}`: there is no input `{id}`{}",
                    suggestion(id, scope.inputs.keys())
                ))
            }
        }
        ["env", name] => {
            if is_env_name(name) {
                Ok(())
            } else {
                Err(format!(
                    "`{raw}`: `{name}` is not an environment variable name. Use letters, digits \
                     and `_`, like `{{{{ env.TEST_PASSWORD }}}}`."
                ))
            }
        }
        ["steps", id, field] => {
            let Some(action) = scope.actions.get(*id) else {
                return Err(format!(
                    "`{raw}`: there is no step `{id}`{}",
                    suggestion(id, scope.actions.keys())
                ));
            };
            if !scope.before.contains(*id) {
                return Err(format!(
                    "`{raw}` is used before step `{id}` has run. Only steps earlier in the file \
                     have results."
                ));
            }
            let produced = Action::produces(action);
            if produced.contains(field) {
                Ok(())
            } else {
                Err(format!(
                    "`{raw}`: a `{action}` step leaves {}.",
                    list(produced.iter().copied())
                ))
            }
        }
        ["inputs", ..] => Err(format!(
            "`{raw}`: write an input as `{{{{ inputs.<id> }}}}`."
        )),
        ["env", ..] => Err(format!(
            "`{raw}`: write an environment value as `{{{{ env.<NAME> }}}}`."
        )),
        ["steps", ..] => Err(format!(
            "`{raw}`: write a step's result as `{{{{ steps.<id>.<field> }}}}`."
        )),
        [namespace, ..] => Err(format!(
            "`{raw}`: there is no `{namespace}`. Variables start with {}.",
            list(
                FIELDS
                    .iter()
                    .map(|(n, _)| *n)
                    .chain(["inputs", "env", "steps"])
            )
        )),
        [] => Err(format!("`{raw}` is empty.")),
    }
}
