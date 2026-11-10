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
    artifact, Action, ArtifactDecl, ArtifactSource, Input, InputKind, Invalid, PrActivity, Problem,
    RunContext, SessionRef, Step, Trigger, TriggerKind, Until, Workflow, SCHEMA_VERSION,
};
use crate::yaml::{self, at, is_id, is_name, list, suggestion};

/// What a `{{ … }}` may name, besides `inputs.*` and `steps.*`.
const FIELDS: &[(&str, &[&str])] = &[
    ("project", &["name", "root"]),
    (
        "workspace",
        &["name", "branch", "base_branch", "path", "task"],
    ),
    ("pr", &["number", "url", "title", "repo", "clone"]),
    // What the event that started the run left. Empty when a person started it.
    (
        "event",
        &[
            "kind", "note", "head", "summary", "verdict", "url", "number", "title", "count",
            "authors", "name",
        ],
    ),
];

/// Variables that are a whole value, with no fields.
const WHOLE: &[&str] = &["memory", "handoff"];

const ACTIONS: &[&str] = &[
    "start_session",
    "wait_session",
    "send_to_session",
    "wait_pr_activity",
    "run_test",
    "notify",
];

/// The keys a step of each action may have, besides `id`, `action` and `needs`.
fn allowed(action: &str) -> &'static [&'static str] {
    match action {
        "start_session" => &["harness", "prompt", "model", "effort", "skip_memory"],
        "wait_session" => &["session", "until", "timeout"],
        "send_to_session" => &["session", "prompt", "timeout"],
        "wait_pr_activity" => &["kind", "timeout"],
        "run_test" => &["test", "inputs", "timeout", "gate"],
        "notify" => &["title", "body"],
        _ => &[],
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
    test: Option<Spanned<String>>,
    #[serde(default)]
    inputs: Option<Spanned<BTreeMap<String, Spanned<String>>>>,
    #[serde(default)]
    title: Option<Spanned<String>>,
    #[serde(default)]
    body: Option<Spanned<String>>,
    #[serde(default)]
    gate: Option<Spanned<bool>>,
    /// Named documents this step leaves. Not an action key: every action may hand one on.
    #[serde(default)]
    produces: Option<Spanned<Vec<Spanned<RawProduce>>>>,
    /// Named documents from earlier steps. Not an action key either.
    #[serde(default)]
    consumes: Option<Spanned<Vec<Spanned<String>>>>,
}

/// One named document, as the file writes it. `from` may be left out when only one source key
/// is set: `path` is a file, `text` is text, `field` is one of the step's own outputs.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProduce {
    name: Spanned<String>,
    #[serde(default)]
    from: Option<Spanned<String>>,
    #[serde(default)]
    path: Option<Spanned<String>>,
    #[serde(default)]
    text: Option<Spanned<String>>,
    #[serde(default)]
    field: Option<Spanned<String>>,
    #[serde(default)]
    session: Option<Spanned<String>>,
    #[serde(default)]
    raw: Option<Spanned<bool>>,
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
            ("test", at(&self.test)),
            ("inputs", at(&self.inputs)),
            ("title", at(&self.title)),
            ("body", at(&self.body)),
            ("gate", at(&self.gate)),
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
    let heading = yaml::heading(text, "workflow", SCHEMA_VERSION);
    let invalid = |problems| Invalid {
        id: heading.id.clone(),
        name: heading.name.clone(),
        problems,
    };
    if let Some(later) = heading.later_version.clone() {
        return Err(invalid(vec![later]));
    }
    let raw: RawWorkflow = match serde_saphyr::from_str(text) {
        Ok(raw) => raw,
        Err(error) => return Err(invalid(vec![yaml::from_serde(&error)])),
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

/// What the variable check needs to know about the file around one field.
struct Scope<'a> {
    inputs: &'a HashMap<String, InputKind>,
    /// Step id to its action, for every step in the file.
    actions: &'a HashMap<String, String>,
    /// The steps certain to have finished before this one: its needs, transitively.
    before: &'a BTreeSet<String>,
    /// What the run is about: `pr.repo` and `pr.clone` exist only for a tracked pull request.
    context: RunContext,
    /// Artifact name to the step that produces it.
    artifacts: &'a HashMap<String, String>,
    /// Names this step lists under `consumes`, in file order.
    consumes: &'a [String],
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
        let steps = self.steps(&raw.steps, &inputs, trigger.context);
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
        let context = match raw.context.as_ref().map(|c| c.value.as_str()) {
            None | Some("workspace") => RunContext::Workspace,
            Some("tracked_pull_request") => RunContext::TrackedPullRequest,
            Some(other) => {
                let context = raw.context.as_ref().expect("matched above");
                self.problem(
                    &context.referenced,
                    format!(
                        "`{other}` is not a context this Yardsort has: `workspace`, or \
                         `tracked_pull_request`."
                    ),
                );
                RunContext::Workspace
            }
        };
        Trigger {
            kind: TriggerKind::Manual,
            context,
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
                "test" => InputKind::Test,
                "text" => InputKind::Text,
                "choice" => InputKind::Choice,
                other => {
                    self.problem(
                        &input.kind.referenced,
                        format!(
                            "`{other}` is not a kind of input. Use `harness`, `test`, `text` or \
                             `choice`."
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

    fn steps(
        &mut self,
        raw: &Spanned<Vec<RawStep>>,
        inputs: &[Input],
        context: RunContext,
    ) -> Vec<Step> {
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
        let producers = self.producers(&raw.value);
        let empty = BTreeSet::new();
        raw.value
            .iter()
            .map(|step| {
                let before = graph.get(&step.id.value).unwrap_or(&empty);
                let consumes = self.consumes(step, &producers, before);
                let scope = Scope {
                    inputs: &input_kinds,
                    actions: &actions,
                    before,
                    context,
                    artifacts: &producers,
                    consumes: &consumes,
                };
                self.step(step, &scope)
            })
            .collect()
    }

    /// Every artifact name in the file, and which step produces it. A name used twice is reported
    /// and kept for the first step, so a later consumer still points at a real producer.
    fn producers(&mut self, steps: &[RawStep]) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for step in steps {
            let Some(list) = &step.produces else {
                continue;
            };
            if list.value.is_empty() {
                self.problem(&list.referenced, "`produces` needs at least one artifact.");
                continue;
            }
            let mut seen = BTreeSet::new();
            for item in &list.value {
                let name = item.value.name.value.trim();
                if !is_name(name) {
                    self.problem(
                        &item.value.name.referenced,
                        format!(
                            "`{name}` cannot be an artifact's name. Start with a letter; use \
                             lowercase letters, digits, `_` and `-`."
                        ),
                    );
                    continue;
                }
                if !seen.insert(name.to_owned()) {
                    self.problem(
                        &item.value.name.referenced,
                        format!("`{name}` is listed twice."),
                    );
                    continue;
                }
                if let Some(other) = map.get(name) {
                    self.problem(
                        &item.value.name.referenced,
                        format!("Step `{other}` already produces `{name}`."),
                    );
                    continue;
                }
                map.insert(name.to_owned(), step.id.value.clone());
            }
        }
        map
    }

    /// The artifacts this step says it reads. Each must be produced by a step in `needs`.
    fn consumes(
        &mut self,
        step: &RawStep,
        producers: &HashMap<String, String>,
        before: &BTreeSet<String>,
    ) -> Vec<String> {
        let Some(list) = &step.consumes else {
            return Vec::new();
        };
        if list.value.is_empty() {
            self.problem(&list.referenced, "`consumes` needs at least one artifact.");
            return Vec::new();
        }
        let mut names = Vec::new();
        for item in &list.value {
            let name = item.value.trim();
            if names.iter().any(|listed: &String| listed == name) {
                self.problem(&item.referenced, format!("`{name}` is listed twice."));
                continue;
            }
            match producers.get(name) {
                None => self.problem(
                    &item.referenced,
                    format!(
                        "There is no artifact `{name}`{}",
                        suggestion(name, producers.keys())
                    ),
                ),
                Some(producer) if producer == &step.id.value => self.problem(
                    &item.referenced,
                    format!("A step cannot consume `{name}`, which it produces itself."),
                ),
                Some(producer) if !before.contains(producer) => self.problem(
                    &item.referenced,
                    format!(
                        "Artifact `{name}` comes from step `{producer}`, which is not sure to \
                         have run. Add `{producer}` to this step's `needs`."
                    ),
                ),
                Some(_) => {}
            }
            names.push(name.to_owned());
        }
        names
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
                gate: false,
                produces: Vec::new(),
                consumes: scope.consumes.to_vec(),
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
            "run_test" => Action::RunTest {
                test: self.test(raw, scope),
                inputs: self.test_inputs(raw, scope),
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
        // `allowed` already rejects `gate` on an action that does not take it. Only
        // `run_test` does, today, so a later action can become a gate without a new field.
        let gate = action_name == "run_test" && raw.gate.as_ref().is_some_and(|gate| gate.value);
        let produces = self.artifact_decls(raw, action_name, &action, scope);
        Step {
            id,
            needs,
            gate,
            produces,
            consumes: scope.consumes.to_vec(),
            action,
        }
    }

    /// The documents this step leaves, checked against the action it has.
    fn artifact_decls(
        &mut self,
        raw: &RawStep,
        action_name: &str,
        action: &Action,
        scope: &Scope,
    ) -> Vec<ArtifactDecl> {
        let Some(list) = &raw.produces else {
            return Vec::new();
        };
        list.value
            .iter()
            .filter_map(|item| self.artifact_decl(&item.value, action_name, action, scope))
            .collect()
    }

    fn artifact_decl(
        &mut self,
        item: &RawProduce,
        action_name: &str,
        action: &Action,
        scope: &Scope,
    ) -> Option<ArtifactDecl> {
        let name = item.name.value.trim();
        if !is_name(name) {
            return None;
        }
        let kind = self.artifact_kind(item);
        let source = match kind {
            "file" => ArtifactSource::File {
                path: self.artifact_path(item, scope)?,
            },
            "text" => ArtifactSource::Text {
                text: self.artifact_text(item, scope)?,
            },
            "output" => ArtifactSource::Output {
                field: self.artifact_field(item, action_name)?,
            },
            "transcript" => ArtifactSource::Transcript {
                session: self.artifact_session(item, action, scope),
                raw: item.raw.as_ref().is_some_and(|raw| raw.value),
            },
            _ => return None,
        };
        Some(ArtifactDecl {
            name: name.to_owned(),
            source,
        })
    }

    /// `file`, `text`, `output` or `transcript`, once the keys agree.
    fn artifact_kind(&mut self, item: &RawProduce) -> &'static str {
        // `raw` and `session` do not pick a source by themselves when a file, text or field
        // is also set: those keys are then the mistake, not an ambiguous source.
        let guessed = match (
            item.path.is_some(),
            item.text.is_some(),
            item.field.is_some(),
        ) {
            (true, false, false) => Some("file"),
            (false, true, false) => Some("text"),
            (false, false, true) => Some("output"),
            (false, false, false) if item.session.is_some() || item.raw.is_some() => {
                Some("transcript")
            }
            _ => None,
        };
        let kind = match item.from.as_ref() {
            Some(from) => match from.value.trim() {
                "file" => "file",
                "text" => "text",
                "output" => "output",
                "transcript" => "transcript",
                other => {
                    self.problem(
                        &from.referenced,
                        format!(
                            "`{other}` is not a place an artifact comes from. Use `file`, \
                             `text`, `output` or `transcript`."
                        ),
                    );
                    return "";
                }
            },
            None => guessed.unwrap_or_else(|| {
                self.problem(
                    &item.name.referenced,
                    format!(
                        "Say where `{}` comes from: `file` with `path`, `text`, `output` with \
                         `field`, or `transcript`.",
                        item.name.value.trim()
                    ),
                );
                ""
            }),
        };
        if kind.is_empty() {
            return "";
        }
        self.artifact_keys(item, kind);
        kind
    }

    /// Keys that belong to a different source, and `raw` anywhere but a transcript.
    fn artifact_keys(&mut self, item: &RawProduce, kind: &str) {
        if kind != "file" {
            self.reject_key(&item.path, "path", kind);
        }
        if kind != "text" {
            self.reject_key(&item.text, "text", kind);
        }
        if kind != "output" {
            self.reject_key(&item.field, "field", kind);
        }
        if kind != "transcript" {
            self.reject_key(&item.session, "session", kind);
            if let Some(raw) = &item.raw {
                self.problem(&raw.referenced, "`raw` is only for a transcript.");
            }
        }
    }

    fn reject_key<T>(&mut self, field: &Option<Spanned<T>>, key: &str, kind: &str) {
        if let Some(field) = field {
            self.problem(
                &field.referenced,
                format!("`{key}` is not part of a `{kind}` artifact."),
            );
        }
    }

    fn artifact_path(&mut self, item: &RawProduce, scope: &Scope) -> Option<String> {
        let Some(path) = self.text(&item.path, scope) else {
            self.problem(
                &item.name.referenced,
                "A file artifact needs `path`: a file inside the workspace.",
            );
            return None;
        };
        if let Err(message) = artifact::check_path(&path) {
            let at = item
                .path
                .as_ref()
                .map(|path| &path.referenced)
                .unwrap_or(&item.name.referenced);
            self.problem(at, message);
        }
        Some(path)
    }

    fn artifact_text(&mut self, item: &RawProduce, scope: &Scope) -> Option<String> {
        let Some(text) = self.text(&item.text, scope) else {
            self.problem(&item.name.referenced, "A text artifact needs `text`.");
            return None;
        };
        if text.trim().is_empty() {
            self.problem(
                item.text
                    .as_ref()
                    .map(|text| &text.referenced)
                    .unwrap_or(&item.name.referenced),
                "`text` is empty.",
            );
        }
        Some(text)
    }

    fn artifact_field(&mut self, item: &RawProduce, action_name: &str) -> Option<String> {
        let Some(field) = item.field.as_ref() else {
            self.problem(
                &item.name.referenced,
                "An output artifact needs `field`: one this step leaves, such as `summary`.",
            );
            return None;
        };
        let name = field.value.trim();
        if name.is_empty() {
            self.problem(&field.referenced, "`field` is empty.");
            return None;
        }
        let produced = Action::produces(action_name);
        if !produced.contains(&name) {
            self.problem(
                &field.referenced,
                if produced.is_empty() {
                    format!(
                        "A `{action_name}` step leaves nothing a later step can read. Use a \
                         file or text for `{name}`."
                    )
                } else {
                    format!(
                        "A `{action_name}` step leaves {}. `{name}` is not one of them.",
                        list(produced.iter().copied())
                    )
                },
            );
        }
        Some(name.to_owned())
    }

    /// A transcript's session: the one this step already names, or the one written here.
    fn artifact_session(
        &mut self,
        item: &RawProduce,
        action: &Action,
        scope: &Scope,
    ) -> SessionRef {
        if let Some(field) = &item.session {
            return self.session_value(field.value.trim(), &field.referenced, scope);
        }
        match action {
            Action::WaitSession { session, .. } | Action::SendToSession { session, .. } => {
                session.clone()
            }
            _ => {
                self.problem(
                    &item.name.referenced,
                    "A transcript needs `session`: `origin`, or `{{ steps.<id>.session }}` for \
                     a session this workflow started.",
                );
                SessionRef::Origin
            }
        }
    }

    /// `origin`, or exactly `{{ steps.<id>.session }}` for an earlier `start_session` step.
    fn session_value(&mut self, value: &str, location: &Location, scope: &Scope) -> SessionRef {
        let usage = "`session` is `origin`, the workspace's own agent, or \
                     `{{ steps.<id>.session }}` for a session this workflow started.";
        if value == "origin" {
            return SessionRef::Origin;
        }
        let parts = template::parse(value).unwrap_or_default();
        let var = match parts.as_slice() {
            [template::Part::Var(var)] if var.path.len() == 3 && var.path[0] == "steps" => var,
            _ => {
                self.problem(location, usage);
                return SessionRef::Origin;
            }
        };
        if var.path[2] != "session" {
            self.problem(location, usage);
        } else if !scope.actions.contains_key(&var.path[1]) {
            self.problem(
                location,
                format!(
                    "There is no step `{}`{}",
                    var.path[1],
                    suggestion(&var.path[1], scope.actions.keys())
                ),
            );
        } else if scope.actions.get(&var.path[1]).map(String::as_str) != Some("start_session") {
            self.problem(
                location,
                format!(
                    "`{}` is not a `start_session` step, so it has no session.",
                    var.path[1]
                ),
            );
        } else if !scope.before.contains(&var.path[1]) {
            self.problem(
                location,
                format!(
                    "`{{{{ steps.{}.session }}}}` is used before step `{}` is sure to have run. \
                     Add `{}` to this step's `needs`.",
                    var.path[1], var.path[1], var.path[1]
                ),
            );
        }
        SessionRef::Step(var.path[1].clone())
    }

    /// A test id as written, or exactly one `{{ inputs.<test input> }}`.
    fn test(&mut self, step: &RawStep, scope: &Scope) -> String {
        let Some(field) = &step.test else {
            self.problem(
                &step.action.referenced,
                "A `run_test` step needs `test`: which smoke test to run, like `login` or \
                 `{{ inputs.suite }}`.",
            );
            return String::new();
        };
        let value = field.value.trim();
        if !value.contains("{{") {
            if value.is_empty() {
                self.problem(&field.referenced, "`test` is empty.");
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
                && scope.inputs.get(&var.path[1]) == Some(&InputKind::Test)
        });
        if !usable {
            self.problem(
                &field.referenced,
                "`test` is a test id, or one `{{ inputs.<id> }}` naming an input of kind `test`, \
                 and nothing else.",
            );
        }
        value.to_owned()
    }

    /// The `inputs` map of a `run_test` step: each value may hold `{{ variables }}`.
    fn test_inputs(&mut self, step: &RawStep, scope: &Scope) -> BTreeMap<String, String> {
        let Some(field) = &step.inputs else {
            return BTreeMap::new();
        };
        field
            .value
            .iter()
            .map(|(name, value)| {
                self.text(&Some(value.clone()), scope);
                (name.clone(), value.value.clone())
            })
            .collect()
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
        match yaml::duration(&field.value, "a timeout") {
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
    fn find(&self, field: &Location, value: &str, raw: &str, start: usize) -> (usize, usize) {
        yaml::find(self.source, field, value, raw, start)
    }
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
        ["pr", field @ ("repo" | "clone")] if scope.context == RunContext::Workspace => {
            Err(format!(
                "`{raw}`: `pr.{field}` is for a pull request a workspace tracks in another \
             repository, and this workflow is about the workspace's own. Add `context: \
             tracked_pull_request` to its trigger for one elsewhere."
            ))
        }
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
        ["artifacts", name] => {
            let Some(producer) = scope.artifacts.get(*name) else {
                return Err(format!(
                    "`{raw}`: there is no artifact `{name}`{}",
                    suggestion(name, scope.artifacts.keys())
                ));
            };
            if !scope.before.contains(producer.as_str()) {
                return Err(format!(
                    "`{raw}` is used before step `{producer}` is sure to have run. Add \
                     `{producer}` to this step's `needs`."
                ));
            }
            if !scope.consumes.iter().any(|listed| listed == *name) {
                return Err(format!(
                    "`{raw}` is used here, but this step does not list `{name}` under `consumes`."
                ));
            }
            Ok(())
        }
        ["artifacts"] | ["artifacts", _, ..] => Err(format!(
            "`{raw}`: write an artifact as `{{{{ artifacts.<name> }}}}`."
        )),
        [namespace, ..] => Err(format!(
            "`{raw}`: there is no `{namespace}`. Variables start with {}.",
            list(
                FIELDS
                    .iter()
                    .map(|(n, _)| *n)
                    .chain(["inputs", "steps", "artifacts"])
                    .chain(WHOLE.iter().copied())
            )
        )),
        [] => Err(format!("`{raw}` is empty.")),
    }
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
