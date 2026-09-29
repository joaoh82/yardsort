//! From YAML text to a [`Workflow`], with a line and column on every mistake.
//!
//! Two passes. The first is serde's: shape and types, and it stops at the first mistake, as
//! serde does. The second is ours, over a raw form that kept each value's position: names,
//! references, the graph, and every `{{ variable }}`. It reports everything it finds, sorted by
//! where it is, because a file written by a model or by hand usually has more than one thing
//! wrong and fixing them one run at a time is tedious.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Deserialize;
use serde_saphyr::{Location, Spanned};

use super::template::{self, Placeholder};
use super::{
    Action, Input, InputKind, Invalid, PrActivity, Problem, RunContext, SessionRef, Step, Trigger,
    TriggerKind, Until, Workflow, SCHEMA_VERSION,
};

/// The longest a timeout may be. A run waiting a week is a run somebody forgot.
const MAX_TIMEOUT_SECS: u64 = 7 * 24 * 60 * 60;

/// What a `{{ … }}` may name, besides `inputs.*` and `steps.*`.
const FIELDS: &[(&str, &[&str])] = &[
    ("project", &["name", "root"]),
    (
        "workspace",
        &["name", "branch", "base_branch", "path", "task"],
    ),
    ("pr", &["number", "url", "title"]),
];

/// Variables that are a whole value, with no fields.
const WHOLE: &[&str] = &["memory", "handoff"];

const ACTIONS: &[&str] = &[
    "start_session",
    "wait_session",
    "send_to_session",
    "wait_pr_activity",
    "notify",
];

/// The keys a step of each action may have, besides `id`, `action` and `needs`.
fn allowed(action: &str) -> &'static [&'static str] {
    match action {
        "start_session" => &["harness", "prompt", "model", "effort", "skip_memory"],
        "wait_session" => &["session", "until", "timeout"],
        "send_to_session" => &["session", "prompt", "timeout"],
        "wait_pr_activity" => &["kind", "timeout"],
        "notify" => &["title", "body"],
        _ => &[],
    }
}

/// Just enough to name a file that does not parse, and to see that it is from a later version
/// before its unknown keys are blamed on it.
#[derive(Deserialize)]
struct Head {
    #[serde(default)]
    id: Option<serde_saphyr::Spanned<HeadValue>>,
    #[serde(default)]
    name: Option<serde_saphyr::Spanned<HeadValue>>,
    #[serde(default)]
    version: Option<serde_saphyr::Spanned<HeadValue>>,
}

/// Anything at all; only a string or a number is kept.
#[derive(Deserialize)]
#[serde(untagged)]
enum HeadValue {
    Number(u64),
    Text(String),
    #[allow(dead_code)]
    Other(serde::de::IgnoredAny),
}

impl HeadValue {
    fn text(&self) -> Option<String> {
        match self {
            HeadValue::Text(text) => Some(text.clone()),
            HeadValue::Number(n) => Some(n.to_string()),
            HeadValue::Other(_) => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkflow {
    id: Spanned<String>,
    name: Spanned<String>,
    #[serde(default)]
    description: Option<String>,
    version: Spanned<u32>,
    trigger: Spanned<RawTrigger>,
    #[serde(default)]
    inputs: Vec<RawInput>,
    steps: Spanned<Vec<RawStep>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTrigger {
    kind: Spanned<String>,
    #[serde(default)]
    context: Option<Spanned<String>>,
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

/// Every key any action takes, all optional here: which belong to which action is checked
/// afterwards, so that a misplaced key is named as misplaced rather than unknown.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    id: Spanned<String>,
    action: Spanned<String>,
    #[serde(default)]
    needs: Vec<Spanned<String>>,
    #[serde(default)]
    harness: Option<Spanned<String>>,
    #[serde(default)]
    prompt: Option<Spanned<String>>,
    #[serde(default)]
    model: Option<Spanned<String>>,
    #[serde(default)]
    effort: Option<Spanned<String>>,
    #[serde(default)]
    skip_memory: Option<Spanned<bool>>,
    #[serde(default)]
    session: Option<Spanned<String>>,
    #[serde(default)]
    until: Option<Spanned<String>>,
    #[serde(default)]
    timeout: Option<Spanned<String>>,
    #[serde(default)]
    kind: Option<Spanned<String>>,
    #[serde(default)]
    title: Option<Spanned<String>>,
    #[serde(default)]
    body: Option<Spanned<String>>,
}

impl RawStep {
    /// The action-specific keys this step has, with where each is.
    fn present(&self) -> Vec<(&'static str, &Location)> {
        fn at<T>(field: &Option<Spanned<T>>) -> Option<&Location> {
            field.as_ref().map(|s| &s.referenced)
        }
        [
            ("harness", at(&self.harness)),
            ("prompt", at(&self.prompt)),
            ("model", at(&self.model)),
            ("effort", at(&self.effort)),
            ("skip_memory", at(&self.skip_memory)),
            ("session", at(&self.session)),
            ("until", at(&self.until)),
            ("timeout", at(&self.timeout)),
            ("kind", at(&self.kind)),
            ("title", at(&self.title)),
            ("body", at(&self.body)),
        ]
        .into_iter()
        .filter_map(|(key, at)| at.map(|at| (key, at)))
        .collect()
    }
}

/// Source lines for chart navigation, including files with semantic errors. Read YAML rather
/// than searching text: prompts can themselves contain `- id:`, and keys may be quoted or reordered.
pub fn step_lines(text: &str) -> BTreeMap<String, u32> {
    #[derive(Deserialize)]
    struct Source {
        steps: Vec<SourceStep>,
    }
    #[derive(Deserialize)]
    struct SourceStep {
        id: Spanned<String>,
    }
    let Ok(source) = serde_saphyr::from_str::<Source>(text) else {
        return BTreeMap::new();
    };
    let mut lines = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for step in source.steps {
        let id = step.id.value;
        if lines
            .insert(id.clone(), step.id.referenced.line() as u32)
            .is_some()
        {
            duplicates.insert(id);
        }
    }
    // A duplicated id has no unambiguous node to point at.
    for id in duplicates {
        lines.remove(&id);
    }
    lines
}

/// Check `text` and build the workflow it describes, or say everything that is wrong with it.
pub fn parse(text: &str) -> Result<Workflow, Invalid> {
    let head = serde_saphyr::from_str::<Head>(text).ok();
    let head_id = head
        .as_ref()
        .and_then(|h| h.id.as_ref())
        .and_then(|v| v.value.text());
    let head_name = head
        .as_ref()
        .and_then(|h| h.name.as_ref())
        .and_then(|v| v.value.text());
    let invalid = |problems| Invalid {
        id: head_id.clone(),
        name: head_name.clone(),
        problems,
    };
    if let Some(version) = head.as_ref().and_then(|h| h.version.as_ref()) {
        if let HeadValue::Number(n) = version.value {
            if n > u64::from(SCHEMA_VERSION) {
                return Err(invalid(vec![at(
                    &version.referenced,
                    format!(
                        "This file is for a later Yardsort (workflow version {n}); this one reads \
                         version {SCHEMA_VERSION}. Update Yardsort to run it."
                    ),
                )]));
            }
        }
    }
    let raw: RawWorkflow = match serde_saphyr::from_str(text) {
        Ok(raw) => raw,
        Err(error) => return Err(invalid(vec![from_serde(&error)])),
    };
    let mut check = Checker {
        source: text,
        problems: Vec::new(),
    };
    let workflow = check.workflow(raw);
    if check.problems.is_empty() {
        Ok(workflow)
    } else {
        let mut problems = check.problems;
        problems.sort_by_key(|p| (p.line.unwrap_or(0), p.column.unwrap_or(0)));
        problems.dedup();
        Err(Invalid {
            id: Some(workflow.id),
            name: Some(workflow.name),
            problems,
        })
    }
}

/// serde's message, without its trailing "at line N, column M" — the position is ours to show.
fn from_serde(error: &serde_saphyr::Error) -> Problem {
    let plain = error.without_snippet().to_string();
    let message = match plain.rfind(" at line ") {
        Some(cut) => plain[..cut].to_owned(),
        None => plain,
    };
    let message = match message.strip_prefix("duplicate mapping key: ") {
        Some(rest) => {
            let key = rest.split(',').next().unwrap_or(rest).trim();
            format!("`{key}` is given twice.")
        }
        None => capitalise(&message),
    };
    match error.location() {
        Some(location) => at(&location, message),
        None => Problem::whole(message),
    }
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn at(location: &Location, message: impl Into<String>) -> Problem {
    Problem::at(
        location.line() as usize,
        location.column() as usize,
        message,
    )
}

/// A name for an input or a step: what a `{{ … }}` path segment can hold, starting with a letter.
fn is_name(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A workflow's id: what its file is called.
pub(super) fn is_id(text: &str) -> bool {
    text.len() <= 64
        && text.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `45m`, `30s`, `2h`: a whole number and one unit.
fn duration(text: &str) -> Result<u64, String> {
    let text = text.trim();
    let usage = "Write a timeout as a whole number and s, m or h, like `45m`.";
    let not_a_timeout = || format!("`{text}` is not a timeout. {usage}");
    // Matched as suffixes, never sliced by byte count: the last character of a file's text can be
    // any width, and a slice through the middle of one panics.
    let (number, scale) = [("s", 1), ("m", 60), ("h", 60 * 60)]
        .into_iter()
        .find_map(|(unit, scale)| text.strip_suffix(unit).map(|number| (number, scale)))
        .ok_or_else(not_a_timeout)?;
    // Digits only: `u64::from_str` would also take a leading `+`.
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Err(not_a_timeout());
    }
    let count: u64 = number.parse().map_err(|_| not_a_timeout())?;
    let secs = count.saturating_mul(scale);
    if secs == 0 {
        return Err("A timeout of zero would fail the step at once.".into());
    }
    if secs > MAX_TIMEOUT_SECS {
        return Err("A timeout may be at most a week (`168h`).".into());
    }
    Ok(secs)
}

/// What the variable check needs to know about the file around one field.
struct Scope<'a> {
    inputs: &'a HashMap<String, InputKind>,
    /// Step id to its action, for every step in the file.
    actions: &'a HashMap<String, String>,
    /// The steps certain to have finished before this one: its needs, transitively.
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

    fn workflow(&mut self, raw: RawWorkflow) -> Workflow {
        if !is_id(&raw.id.value) {
            self.problem(
                &raw.id.referenced,
                format!(
                    "`{}` cannot be an id. Use lowercase letters, digits and `-`, like \
                     `code-review`.",
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
        let trigger = self.trigger(&raw.trigger.value);
        let inputs = self.inputs(&raw.inputs);
        let steps = self.steps(&raw.steps, &inputs);
        Workflow {
            id: raw.id.value,
            name: raw.name.value.trim().to_owned(),
            description: raw
                .description
                .map(|d| d.trim().to_owned())
                .filter(|d| !d.is_empty()),
            version: raw.version.value,
            trigger,
            inputs,
            steps,
        }
    }

    fn trigger(&mut self, raw: &RawTrigger) -> Trigger {
        if raw.kind.value != "manual" {
            self.problem(
                &raw.kind.referenced,
                format!(
                    "`{}` is not a trigger this Yardsort has. Only `manual` for now.",
                    raw.kind.value
                ),
            );
        }
        if let Some(context) = &raw.context {
            if context.value != "workspace" {
                self.problem(
                    &context.referenced,
                    format!(
                        "`{}` is not a context this Yardsort has. Only `workspace` for now.",
                        context.value
                    ),
                );
            }
        }
        Trigger {
            kind: TriggerKind::Manual,
            context: RunContext::Workspace,
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
                "harness" => InputKind::Harness,
                "text" => InputKind::Text,
                "choice" => InputKind::Choice,
                other => {
                    self.problem(
                        &input.kind.referenced,
                        format!(
                            "`{other}` is not a kind of input. Use `harness`, `text` or `choice`."
                        ),
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

    fn steps(&mut self, raw: &Spanned<Vec<RawStep>>, inputs: &[Input]) -> Vec<Step> {
        if raw.value.is_empty() {
            self.problem(&raw.referenced, "A workflow needs at least one step.");
            return Vec::new();
        }
        let mut actions: HashMap<String, String> = HashMap::new();
        let mut first_line: HashMap<&str, u64> = HashMap::new();
        for step in &raw.value {
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
        let graph = self.graph(&raw.value, &actions);
        // The first input with an id is the one a variable means; a second is already reported.
        let mut input_kinds: HashMap<String, InputKind> = HashMap::new();
        for input in inputs {
            input_kinds.entry(input.id.clone()).or_insert(input.kind);
        }
        let empty = BTreeSet::new();
        raw.value
            .iter()
            .map(|step| {
                let scope = Scope {
                    inputs: &input_kinds,
                    actions: &actions,
                    before: graph.get(&step.id.value).unwrap_or(&empty),
                };
                self.step(step, &scope)
            })
            .collect()
    }

    /// Check every `needs`, find any circle, and work out what comes before each step.
    fn graph(
        &mut self,
        steps: &[RawStep],
        actions: &HashMap<String, String>,
    ) -> BTreeMap<String, BTreeSet<String>> {
        let mut edges: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for step in steps {
            let id = step.id.value.as_str();
            let mut listed = BTreeSet::new();
            for need in &step.needs {
                let name = need.value.as_str();
                if name == id {
                    self.problem(&need.referenced, format!("`{id}` cannot need itself."));
                } else if !actions.contains_key(name) {
                    self.problem(
                        &need.referenced,
                        format!(
                            "There is no step `{name}`{}",
                            suggestion(name, actions.keys())
                        ),
                    );
                } else if !listed.insert(name) {
                    self.problem(&need.referenced, format!("`{name}` is listed twice."));
                } else {
                    edges.entry(id).or_default().push(name);
                }
            }
        }
        if let Some(circle) = circle(steps, &edges) {
            let step = steps
                .iter()
                .find(|s| s.id.value == circle[0])
                .expect("a circle is made of steps");
            let at = step
                .needs
                .first()
                .map(|n| &n.referenced)
                .unwrap_or(&step.id.referenced);
            self.problem(
                at,
                format!(
                    "These steps wait for each other, so none of them could ever start: {}.",
                    circle.join(" → ")
                ),
            );
        }
        let mut before = BTreeMap::new();
        for step in steps {
            let mut reached = BTreeSet::new();
            let mut stack: Vec<&str> = edges
                .get(step.id.value.as_str())
                .cloned()
                .unwrap_or_default();
            while let Some(next) = stack.pop() {
                if reached.insert(next.to_owned()) {
                    stack.extend(edges.get(next).into_iter().flatten().copied());
                }
            }
            before.insert(step.id.value.clone(), reached);
        }
        before
    }

    fn step(&mut self, raw: &RawStep, scope: &Scope) -> Step {
        let action_name = raw.action.value.as_str();
        let needs: Vec<String> = raw.needs.iter().map(|n| n.value.clone()).collect();
        let id = raw.id.value.clone();
        if !ACTIONS.contains(&action_name) {
            self.problem(
                &raw.action.referenced,
                format!(
                    "`{action_name}` is not an action. The actions are {}.",
                    ACTIONS
                        .iter()
                        .map(|a| format!("`{a}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
            let title = String::new();
            return Step {
                id,
                needs,
                action: Action::Notify { title, body: None },
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
            "start_session" => Action::StartSession {
                harness: self.harness(raw, scope),
                prompt: self.text(&raw.prompt, scope),
                model: self.text(&raw.model, scope),
                effort: self.text(&raw.effort, scope),
                skip_memory: raw.skip_memory.as_ref().is_some_and(|s| s.value),
            },
            "wait_session" => {
                Action::WaitSession {
                    session: self.session(raw, scope),
                    until: match raw
                        .until
                        .as_ref()
                        .map(|u| (u.value.as_str(), &u.referenced))
                    {
                        None | Some(("settled", _)) => Until::Settled,
                        Some(("exited", _)) => Until::Exited,
                        Some((other, location)) => {
                            self.problem(
                            location,
                            format!("`{other}` is not a thing to wait for. Use `settled` or `exited`."),
                        );
                            Until::Settled
                        }
                    },
                    timeout_secs: self.timeout(raw),
                }
            }
            "send_to_session" => Action::SendToSession {
                session: self.session(raw, scope),
                prompt: self.required(&raw.prompt, raw, "prompt", "the message to send", scope),
                timeout_secs: self.timeout(raw),
            },
            "wait_pr_activity" => Action::WaitPrActivity {
                kind: match raw.kind.as_ref().map(|k| (k.value.as_str(), &k.referenced)) {
                    None | Some(("any", _)) => PrActivity::Any,
                    Some(("review", _)) => PrActivity::Review,
                    Some(("comment", _)) => PrActivity::Comment,
                    Some((other, location)) => {
                        self.problem(
                            location,
                            format!(
                                "`{other}` is not a kind of pull request activity. Use `review`, \
                                 `comment` or `any`."
                            ),
                        );
                        PrActivity::Any
                    }
                },
                timeout_secs: self.timeout(raw),
            },
            _ => Action::Notify {
                title: self.required(
                    &raw.title,
                    raw,
                    "title",
                    "what the notification says",
                    scope,
                ),
                body: self.text(&raw.body, scope),
            },
        };
        Step { id, needs, action }
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
            Some(_) => self.text(field, scope).unwrap_or_default(),
            None => {
                self.problem(
                    &step.action.referenced,
                    format!("A `{}` step needs `{key}`: {what}.", step.action.value),
                );
                String::new()
            }
        }
    }

    /// A harness id as written, or exactly one `{{ inputs.<harness input> }}`.
    fn harness(&mut self, step: &RawStep, scope: &Scope) -> String {
        let Some(field) = &step.harness else {
            self.problem(
                &step.action.referenced,
                "A `start_session` step needs `harness`: which agent to start, like `claude` or \
                 `{{ inputs.reviewer }}`.",
            );
            return String::new();
        };
        let value = field.value.trim();
        if !value.contains("{{") {
            if !is_name(value) {
                self.problem(
                    &field.referenced,
                    format!("`{value}` is not a harness id, like `claude` or `codex`."),
                );
            }
            return value.to_owned();
        }
        let only = match template::parse(value) {
            Ok(parts) if parts.len() == 1 => match &parts[0] {
                template::Part::Var(var) => Some(var.clone()),
                template::Part::Text(_) => None,
            },
            _ => None,
        };
        let usable = only.as_ref().is_some_and(|var| {
            var.path.len() == 2
                && var.path[0] == "inputs"
                && scope.inputs.get(&var.path[1]) == Some(&InputKind::Harness)
        });
        if !usable {
            self.problem(
                &field.referenced,
                "`harness` is a harness id, or one `{{ inputs.<id> }}` naming an input of kind \
                 `harness`, and nothing else.",
            );
        }
        value.to_owned()
    }

    /// `origin`, or exactly `{{ steps.<id>.session }}` for an earlier `start_session` step.
    fn session(&mut self, step: &RawStep, scope: &Scope) -> SessionRef {
        let usage = "`session` is `origin` — the workspace's own agent — or \
                     `{{ steps.<id>.session }}` for a session this workflow started.";
        let Some(field) = &step.session else {
            self.problem(
                &step.action.referenced,
                format!("A `{}` step needs `session`. {usage}", step.action.value),
            );
            return SessionRef::Origin;
        };
        let value = field.value.trim();
        if value == "origin" {
            return SessionRef::Origin;
        }
        let parts = template::parse(value).unwrap_or_default();
        let var = match parts.as_slice() {
            [template::Part::Var(var)] if var.path.len() == 3 && var.path[0] == "steps" => var,
            _ => {
                self.problem(&field.referenced, usage);
                return SessionRef::Origin;
            }
        };
        if var.path[2] != "session" {
            self.problem(&field.referenced, usage);
        } else if !scope.actions.contains_key(&var.path[1]) {
            self.problem(
                &field.referenced,
                format!(
                    "There is no step `{}`{}",
                    var.path[1],
                    suggestion(&var.path[1], scope.actions.keys())
                ),
            );
        } else if scope.actions.get(&var.path[1]).map(String::as_str) != Some("start_session") {
            self.problem(
                &field.referenced,
                format!(
                    "`{}` is not a `start_session` step, so it has no session.",
                    var.path[1]
                ),
            );
        } else {
            self.variable(var, &field.referenced, value, scope);
        }
        SessionRef::Step(var.path[1].clone())
    }

    fn timeout(&mut self, step: &RawStep) -> Option<u32> {
        let field = step.timeout.as_ref()?;
        // At most a week, so it always fits.
        match duration(&field.value) {
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
        match template::parse(&field.value) {
            Ok(_) => {
                for var in template::placeholders(&field.value) {
                    self.variable(&var, &field.referenced, &field.value, scope);
                }
            }
            Err(malformed) => {
                for bad in malformed {
                    let (line, column) =
                        self.find(&field.referenced, &field.value, &bad.raw, bad.start);
                    self.problems.push(Problem::at(line, column, bad.message));
                }
            }
        }
        Some(field.value.clone())
    }

    /// Report `var` if it names nothing. `value` is the text it was found in.
    fn variable(&mut self, var: &Placeholder, field: &Location, value: &str, scope: &Scope) {
        let Err(message) = resolve(var, scope) else {
            return;
        };
        let (line, column) = self.find(field, value, &var.raw, var.start);
        self.problems.push(Problem::at(line, column, message));
    }

    /// Where in the file the placeholder `raw`, at byte `start` of `value`, is written.
    ///
    /// YAML may fold, indent or unescape a value, so an offset in the value is not an offset in
    /// the file; the placeholder's own text is the same in both. The same text can appear more
    /// than once, so this finds the same *occurrence*: the second `{{ pr.nubmer }}` in the value
    /// is the second in the file. Escaped ones, `\{{`, are literals and are counted in neither.
    fn find(&self, field: &Location, value: &str, raw: &str, start: usize) -> (usize, usize) {
        let fallback = (field.line() as usize, field.column() as usize);
        let Some(from) = offset_of(self.source, field.line() as usize, field.column() as usize)
        else {
            return fallback;
        };
        let nth = unescaped(value, raw).take_while(|&at| at < start).count();
        match unescaped(&self.source[from..], raw).nth(nth) {
            Some(found) => position_of(self.source, from + found),
            None => fallback,
        }
    }
}

/// Where `needle` occurs in `hay`, leaving out occurrences escaped with a backslash.
fn unescaped<'a>(hay: &'a str, needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    hay.match_indices(needle)
        .map(|(at, _)| at)
        .filter(move |&at| !hay[..at].ends_with('\\'))
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
        [whole] if WHOLE.contains(whole) => Ok(()),
        [whole, ..] if WHOLE.contains(whole) => Err(format!(
            "`{raw}`: `{whole}` is used whole, as `{{{{ {whole} }}}}`."
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
        ["steps", id, field] => {
            let Some(action) = scope.actions.get(*id) else {
                return Err(format!(
                    "`{raw}`: there is no step `{id}`{}",
                    suggestion(id, scope.actions.keys())
                ));
            };
            if !scope.before.contains(*id) {
                return Err(format!(
                    "`{raw}` is used before step `{id}` is sure to have run. Add `{id}` to this \
                     step's `needs`."
                ));
            }
            let produced = Action::produces(action);
            if produced.contains(field) {
                Ok(())
            } else if produced.is_empty() {
                Err(format!("`{raw}`: a `{action}` step leaves nothing to use."))
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
        ["steps", ..] => Err(format!(
            "`{raw}`: write a step's result as `{{{{ steps.<id>.<field> }}}}`."
        )),
        [namespace, ..] => Err(format!(
            "`{raw}`: there is no `{namespace}`. Variables start with {}.",
            list(
                FIELDS
                    .iter()
                    .map(|(n, _)| *n)
                    .chain(["inputs", "steps"])
                    .chain(WHOLE.iter().copied())
            )
        )),
        [] => Err(format!("`{raw}` is empty.")),
    }
}

fn list<T: std::fmt::Display>(items: impl IntoIterator<Item = T>) -> String {
    items
        .into_iter()
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// ". Did you mean `x`?" when one name is close, else ".".
fn suggestion<'a>(name: &str, known: impl Iterator<Item = &'a String>) -> String {
    let close = known
        .filter(|k| distance(name, k) <= 2)
        .min_by_key(|k| distance(name, k));
    match close {
        Some(k) => format!(". Did you mean `{k}`?"),
        None => ".".to_owned(),
    }
}

/// Edit distance, for "did you mean". Names are short.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let here = row[j + 1];
            row[j + 1] = if ca == *cb {
                prev
            } else {
                1 + prev.min(row[j]).min(here)
            };
            prev = here;
        }
    }
    row[b.len()]
}

/// One circle in the graph, as step ids in order with the first repeated at the end.
fn circle(steps: &[RawStep], edges: &BTreeMap<&str, Vec<&str>>) -> Option<Vec<String>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Fresh,
        Open,
        Done,
    }
    fn visit<'a>(
        node: &'a str,
        edges: &BTreeMap<&'a str, Vec<&'a str>>,
        marks: &mut HashMap<&'a str, Mark>,
        path: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        marks.insert(node, Mark::Open);
        path.push(node);
        for next in edges.get(node).into_iter().flatten() {
            match marks.get(next).copied().unwrap_or(Mark::Fresh) {
                Mark::Open => {
                    let from = path.iter().position(|n| n == next).unwrap_or(0);
                    let mut found: Vec<String> =
                        path[from..].iter().map(|n| (*n).to_owned()).collect();
                    found.push((*next).to_owned());
                    return Some(found);
                }
                Mark::Fresh => {
                    if let Some(found) = visit(next, edges, marks, path) {
                        return Some(found);
                    }
                }
                Mark::Done => {}
            }
        }
        path.pop();
        marks.insert(node, Mark::Done);
        None
    }
    let mut marks = HashMap::new();
    for step in steps {
        let id = step.id.value.as_str();
        if marks.get(id).copied().unwrap_or(Mark::Fresh) == Mark::Fresh {
            if let Some(found) = visit(id, edges, &mut marks, &mut Vec::new()) {
                return Some(found);
            }
        }
    }
    None
}

/// The byte offset of a 1-based line and character column.
fn offset_of(source: &str, line: usize, column: usize) -> Option<usize> {
    let mut start = 0;
    for _ in 1..line {
        start += source[start..].find('\n')? + 1;
    }
    let text = &source[start..];
    let within = text
        .char_indices()
        .nth(column.saturating_sub(1))
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    Some(start + within)
}

/// The 1-based line and character column of a byte offset.
fn position_of(source: &str, offset: usize) -> (usize, usize) {
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, source[line_start..offset].chars().count() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_take_one_unit() {
        assert_eq!(duration("45m"), Ok(45 * 60));
        assert_eq!(duration("30s"), Ok(30));
        assert_eq!(duration(" 2h "), Ok(7200));
        for bad in [
            "45", "m", "1h30m", "-5m", "+5m", "4.5h", "10d", "", "1秒", "秒", "5 m",
        ] {
            assert!(duration(bad).is_err(), "{bad}");
        }
        assert!(duration("0m").unwrap_err().contains("zero"));
        assert!(duration("169h").unwrap_err().contains("week"));
    }

    #[test]
    fn positions_count_characters_not_bytes() {
        let source = "a: é\nb: {{ x }}\n";
        let offset = offset_of(source, 2, 4).unwrap();
        assert_eq!(&source[offset..offset + 2], "{{");
        assert_eq!(position_of(source, offset), (2, 4));
        assert_eq!(offset_of("é{{", 1, 2), Some(2));
    }

    #[test]
    fn close_names_are_suggested() {
        let known = ["review".to_owned(), "notify".to_owned()];
        assert_eq!(
            suggestion("reveiw", known.iter()),
            ". Did you mean `review`?"
        );
        assert_eq!(suggestion("zzz", known.iter()), ".");
    }
}
