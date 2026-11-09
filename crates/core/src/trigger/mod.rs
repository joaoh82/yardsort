//! Triggers: a standing instruction to start a workflow or a smoke test in a workspace when
//! something happens there. See `docs/design/36-triggers.md`.
//!
//! A trigger adds nothing to a run. It is a row that says what to listen for, where, and what
//! to start, and when what it listens for happens it goes through the door a person's click
//! goes through — [`crate::workflow::runs`] or [`crate::smoke::runs`] — so everything those
//! doors check is checked for it too. What is here is the part before a run: what a trigger may
//! hold ([`add`], [`change`]), and, in [`fire`], seeing that something happened, gathering it,
//! and deciding whether it starts a run.
//!
//! Nothing here reads the clock. Every rule takes the time, so every rule is tested with a
//! made-up one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{IpcError, IpcResult};
use crate::schedule::{ScheduleNotify, ScheduleTarget};
use crate::store::{Store, TriggerFields, TriggerFireRow, TriggerRow, WorkspaceRow};
use crate::workflow::runs::Look;

pub mod fire;
pub mod presets;
pub mod sources;
pub mod turns;
pub mod watch;

/// The longest a trigger waits after the last event before it fires.
pub const MOST_QUIET_MS: i64 = 60 * 60_000;

/// However many events keep coming, a fire is decided this long after its first.
pub const MOST_GATHER_MS: i64 = 10 * 60_000;

/// The most runs a trigger may start for one subject before it stops and asks.
pub const MOST_CAP: i64 = 100;

/// How many runs a trigger may start for one subject when it is not told otherwise.
pub const DEFAULT_CAP: i64 = 3;

/// How many runs a trigger may start in an hour, whatever their subjects: what backs the cap
/// up.
pub const RATE_PER_HOUR: i64 = 10;

/// How deep a chain of triggered runs may go: a run started by an event from a run of depth
/// _n_ has depth _n_ + 1, and one past this is refused.
pub const MOST_DEPTH: i64 = 3;

/// What a trigger listens for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
pub enum TriggerEvent {
    /// An agent ran `ys workspace done`.
    #[serde(rename = "workspace.done")]
    WorkspaceDone,
    /// An agent went quiet after real work, or its harness reported a finished turn. One per
    /// turn.
    #[serde(rename = "agent.settled")]
    AgentSettled,
    /// An agent's program ended.
    #[serde(rename = "agent.exited")]
    AgentExited,
    /// A smoke test run in the workspace reached a verdict, or could not.
    #[serde(rename = "test.finished")]
    TestFinished,
    /// A workflow run in the workspace ended.
    #[serde(rename = "workflow.finished")]
    WorkflowFinished,
    /// The workspace's branch has an open pull request it did not have before.
    #[serde(rename = "pr.opened")]
    PrOpened,
    /// A review was submitted on the workspace's pull request.
    #[serde(rename = "pr.review")]
    PrReview,
    /// A comment on its conversation, or a reply on a review thread.
    #[serde(rename = "pr.comment")]
    PrComment,
    /// Its checks finished on its newest commit, and one failed.
    #[serde(rename = "pr.checks_failed")]
    PrChecksFailed,
    /// Its checks finished on its newest commit, and none failed.
    #[serde(rename = "pr.checks_passed")]
    PrChecksPassed,
    /// GitHub says it conflicts with its base.
    #[serde(rename = "pr.conflicted")]
    PrConflicted,
    #[serde(rename = "pr.merged")]
    PrMerged,
    /// It was closed without being merged.
    #[serde(rename = "pr.closed")]
    PrClosed,
    /// Someone ran `ys event emit <name>` in the workspace: an agent, a smoke test's `run`
    /// step, a script. Which names a trigger listens for are its [`Filters::names`]; it is
    /// `custom.<name>` wherever a person reads or writes it.
    #[serde(rename = "custom")]
    Custom,
}

/// What a custom event's name is written after: `custom.migration-written`.
pub const CUSTOM_PREFIX: &str = "custom.";

/// The longest a custom event's name may be.
pub const MOST_CUSTOM_NAME: usize = 40;

/// Why `name` cannot be a custom event's name, when it cannot: lowercase letters, digits, `-`
/// and `_`, starting with a letter or a digit, so it reads the same in a file, a flag and a
/// sentence.
pub fn custom_name_problem(name: &str) -> Option<String> {
    let fits = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
    if name.is_empty() {
        return Some("Name the event: `custom.<name>`.".to_owned());
    }
    if name.len() > MOST_CUSTOM_NAME {
        return Some(format!(
            "`{name}` is longer than {MOST_CUSTOM_NAME} characters, the most an event's name may be."
        ));
    }
    let first = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if first && name.chars().all(fits) {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    let hint = if lower != name && custom_name_problem(&lower).is_none() {
        format!(" `{lower}` would do.")
    } else {
        String::new()
    };
    Some(format!(
        "`{name}` cannot be an event's name: lowercase letters, digits, `-` and `_`, starting \
         with a letter or a digit.{hint}"
    ))
}

/// What a person writes for an event: `pr.review`, or `custom.<name>` for a custom one.
/// `None` for one this Yardsort does not know, or a custom one with no name or a bad one.
pub fn parse_event(text: &str) -> Option<(TriggerEvent, Option<String>)> {
    match text.strip_prefix(CUSTOM_PREFIX) {
        Some(name) => custom_name_problem(name)
            .is_none()
            .then(|| (TriggerEvent::Custom, Some(name.to_owned()))),
        None => TriggerEvent::parse(text)
            .filter(|event| *event != TriggerEvent::Custom)
            .map(|event| (event, None)),
    }
}

/// What a trigger counts its runs for, and so what its events must agree on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    /// One agent session.
    Session,
    /// One pull request: a new one on the same branch starts again.
    PullRequest,
    /// The workspace.
    Workspace,
}

impl TriggerEvent {
    pub const ALL: [Self; 14] = [
        Self::WorkspaceDone,
        Self::AgentSettled,
        Self::AgentExited,
        Self::TestFinished,
        Self::WorkflowFinished,
        Self::PrOpened,
        Self::PrReview,
        Self::PrComment,
        Self::PrChecksFailed,
        Self::PrChecksPassed,
        Self::PrConflicted,
        Self::PrMerged,
        Self::PrClosed,
        Self::Custom,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceDone => "workspace.done",
            Self::AgentSettled => "agent.settled",
            Self::AgentExited => "agent.exited",
            Self::TestFinished => "test.finished",
            Self::WorkflowFinished => "workflow.finished",
            Self::PrOpened => "pr.opened",
            Self::PrReview => "pr.review",
            Self::PrComment => "pr.comment",
            Self::PrChecksFailed => "pr.checks_failed",
            Self::PrChecksPassed => "pr.checks_passed",
            Self::PrConflicted => "pr.conflicted",
            Self::PrMerged => "pr.merged",
            Self::PrClosed => "pr.closed",
            Self::Custom => "custom",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.as_str() == text)
    }

    /// What happened, as the end of "When …".
    pub fn words(self) -> &'static str {
        match self {
            Self::WorkspaceDone => "an agent says its work is done",
            Self::AgentSettled => "an agent finishes a turn",
            Self::AgentExited => "an agent's program ends",
            Self::TestFinished => "a smoke test run ends",
            Self::WorkflowFinished => "a workflow run ends",
            Self::PrOpened => "the workspace's pull request opens",
            Self::PrReview => "a review comes in on the pull request",
            Self::PrComment => "someone comments on the pull request",
            Self::PrChecksFailed => "the pull request's checks fail",
            Self::PrChecksPassed => "the pull request's checks pass",
            Self::PrConflicted => "the pull request conflicts with its base",
            Self::PrMerged => "the pull request is merged",
            Self::PrClosed => "the pull request is closed without merging",
            Self::Custom => "a custom event is raised",
        }
    }

    /// Whether it is about one agent session, rather than the whole workspace: what its fires'
    /// subject is, and what a session trigger narrows.
    pub fn of_a_session(self) -> bool {
        self.subject() == Subject::Session
    }

    /// Whether it is about the workspace's pull request, which the forge is asked about.
    pub fn of_a_pull_request(self) -> bool {
        self.subject() == Subject::PullRequest
    }

    pub fn subject(self) -> Subject {
        match self {
            Self::AgentSettled | Self::AgentExited => Subject::Session,
            Self::PrOpened
            | Self::PrReview
            | Self::PrComment
            | Self::PrChecksFailed
            | Self::PrChecksPassed
            | Self::PrConflicted
            | Self::PrMerged
            | Self::PrClosed => Subject::PullRequest,
            Self::WorkspaceDone | Self::TestFinished | Self::WorkflowFinished | Self::Custom => {
                Subject::Workspace
            }
        }
    }

    /// The quiet time a trigger on it has when none is given: two minutes for reviews and
    /// comments, which come in bursts; none for the rest.
    pub fn default_quiet_ms(self) -> i64 {
        match self {
            Self::PrReview | Self::PrComment => 2 * 60_000,
            _ => 0,
        }
    }

    /// What it leaves for `{{ event.* }}`, besides `kind` and `count`, which every fire leaves.
    pub fn leaves(self) -> &'static [&'static str] {
        match self {
            Self::WorkspaceDone => &["note", "head"],
            Self::AgentSettled => &["head"],
            Self::AgentExited => &["summary"],
            Self::TestFinished => &["verdict", "summary"],
            Self::WorkflowFinished => &["summary"],
            Self::PrOpened | Self::PrMerged | Self::PrClosed => &["url", "number", "title"],
            Self::PrReview | Self::PrComment => &["url", "number", "title", "authors", "summary"],
            Self::PrChecksFailed => &["url", "number", "title", "head", "summary"],
            Self::PrChecksPassed | Self::PrConflicted => &["url", "number", "title", "head"],
            Self::Custom => &["name", "note", "head"],
        }
    }
}

/// Whose reviews and comments count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FromFilter {
    /// Accounts other than the one `gh` posts as, which is the user's and every agent's: a
    /// teammate, a review bot.
    #[default]
    Others,
    /// The user's account too, except while a run this trigger started is going: what is posted
    /// then is the agent's own answers.
    Anyone,
}

/// What a review concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStateFilter {
    Approved,
    ChangesRequested,
    Commented,
}

impl ReviewStateFilter {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::ChangesRequested => "changes_requested",
            Self::Commented => "commented",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Approved, Self::ChangesRequested, Self::Commented]
            .into_iter()
            .find(|state| state.as_str() == text)
    }
}

/// How an agent's program must have ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExitFilter {
    Any,
    /// With a code that is not 0, or a signal.
    Failed,
}

/// How a smoke test run must have ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum VerdictFilter {
    Any,
    Passed,
    /// Failed, or could not finish.
    Failed,
}

/// How a workflow run must have ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeFilter {
    Any,
    Succeeded,
    /// Failed or cancelled.
    Failed,
}

/// What the events a trigger listens for must also be. Each applies to some of the events;
/// one given for an event the trigger does not listen for is refused when it is saved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Filters {
    /// `agent.settled`: the worktree's HEAD moved since this trigger last started a run for
    /// the session — or, before it ever has, the branch has commits its base does not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub new_commits: bool,
    /// `agent.settled`: nothing in the worktree is uncommitted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clean: bool,
    /// `agent.exited`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<ExitFilter>,
    /// `test.finished`: only this test.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<String>,
    /// `test.finished`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<VerdictFilter>,
    /// `workflow.finished`: only this workflow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<String>,
    /// `workflow.finished`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<OutcomeFilter>,
    /// `pr.opened`: a draft counts as soon as it opens. Without it a draft counts once it is
    /// marked ready for review.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub drafts: bool,
    /// `pr.review`, `pr.comment`: whose posts count. Others', when not given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<FromFilter>,
    /// `pr.review`: only reviews that concluded one of these. Any, when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<ReviewStateFilter>,
    /// `pr.review`, `pr.comment`: leave out accounts whose login ends in `[bot]`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_bots: bool,
    /// `custom`: the names it listens for, without `custom.`. At least one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
}

impl Filters {
    /// The filters a row holds; none for ones this Yardsort cannot read.
    pub fn parse(json: &str) -> Self {
        serde_json::from_str(json).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    /// Each filter given, with the events it is for.
    fn given(&self) -> Vec<(&'static str, &'static [TriggerEvent])> {
        const POSTS: &[TriggerEvent] = &[TriggerEvent::PrReview, TriggerEvent::PrComment];
        let mut given: Vec<(&'static str, &'static [TriggerEvent])> = Vec::new();
        if self.new_commits {
            given.push(("new commits", &[TriggerEvent::AgentSettled]));
        }
        if self.clean {
            given.push(("clean", &[TriggerEvent::AgentSettled]));
        }
        if self.exit.is_some() {
            given.push(("exit", &[TriggerEvent::AgentExited]));
        }
        if self.test.is_some() {
            given.push(("test", &[TriggerEvent::TestFinished]));
        }
        if self.verdict.is_some() {
            given.push(("verdict", &[TriggerEvent::TestFinished]));
        }
        if self.workflow.is_some() {
            given.push(("workflow", &[TriggerEvent::WorkflowFinished]));
        }
        if self.outcome.is_some() {
            given.push(("outcome", &[TriggerEvent::WorkflowFinished]));
        }
        if self.drafts {
            given.push(("drafts", &[TriggerEvent::PrOpened]));
        }
        if self.from.is_some() {
            given.push(("from", POSTS));
        }
        if !self.states.is_empty() {
            given.push(("state", &[TriggerEvent::PrReview]));
        }
        if self.no_bots {
            given.push(("no bots", POSTS));
        }
        if !self.names.is_empty() {
            given.push(("names", &[TriggerEvent::Custom]));
        }
        given
    }
}

/// The events a trigger listens for as a person writes them, in order: `pr.review`, and
/// `custom.<name>` for each name a custom one has.
pub fn event_names(events: &[TriggerEvent], filters: &Filters) -> Vec<String> {
    events
        .iter()
        .flat_map(|event| match event {
            TriggerEvent::Custom => filters
                .names
                .iter()
                .map(|name| format!("{CUSTOM_PREFIX}{name}"))
                .collect(),
            event => vec![event.as_str().to_owned()],
        })
        .collect()
}

/// The events a row holds, in order, leaving out any this Yardsort does not know.
pub fn events_of(row: &TriggerRow) -> Vec<TriggerEvent> {
    let names: Vec<String> = serde_json::from_str(&row.events).unwrap_or_default();
    names
        .iter()
        .filter_map(|name| TriggerEvent::parse(name))
        .collect()
}

/// A trigger as it is asked for, before anything is checked.
#[derive(Debug, Clone)]
pub struct Draft<'a> {
    /// What lists call it. The target's own name when not given.
    pub name: Option<&'a str>,
    pub preset: Option<&'a str>,
    pub workspace_id: &'a str,
    /// The conversation record, for a trigger that belongs to one agent session.
    pub session_id: Option<&'a str>,
    pub events: &'a [TriggerEvent],
    pub filters: &'a Filters,
    pub target: ScheduleTarget,
    pub target_id: &'a str,
    /// As given, by input id. Checked against what the target asks for.
    pub inputs: &'a BTreeMap<String, String>,
    pub quiet_ms: i64,
    pub cap: i64,
    pub notify: ScheduleNotify,
}

/// A draft that passed every check, ready to be written.
struct Checked {
    name: String,
    workspace: WorkspaceRow,
    events: String,
    filters: String,
    inputs: String,
}

/// Everything that can be known to be wrong before anything happens: the workspace and the
/// session are there, the events and their filters make sense together, the target is there and
/// ready, its inputs fit, it uses no `{{ event.* }}` its events do not leave, and nothing in
/// it would wait for a person for ever. The doors check most of it again at each fire; this is
/// so a mistake is told to the person making it.
fn check_draft(
    store: &Store,
    data_dir: &Path,
    look: &dyn Look,
    draft: &Draft<'_>,
) -> IpcResult<Checked> {
    let workspace = store
        .workspace(draft.workspace_id)?
        .filter(|w| !w.forgotten)
        .ok_or_else(|| IpcError::new("workspace_not_found", "That workspace is not there."))?;
    if workspace.archived {
        return Err(IpcError::new(
            "workspace_archived",
            format!(
                "`{}` is archived. Restore it before adding a trigger to it.",
                workspace.name
            ),
        ));
    }
    if let Some(session_id) = draft.session_id {
        store
            .session(session_id)?
            .filter(|session| session.workspace_id == workspace.id)
            .ok_or_else(|| {
                IpcError::new(
                    "session_not_found",
                    format!("`{}` has no such agent session.", workspace.name),
                )
            })?;
    }
    let mut events: Vec<TriggerEvent> = draft.events.to_vec();
    events.sort();
    events.dedup();
    if events.is_empty() {
        return Err(IpcError::new(
            "trigger_events",
            "Say what it listens for: at least one event.",
        ));
    }
    if events.iter().any(|e| e.subject() != events[0].subject()) {
        return Err(IpcError::new(
            "trigger_events",
            "An agent's events, the pull request's and the workspace's cannot share a trigger: \
             each counts its runs for something different. Make two triggers.",
        ));
    }
    if events.contains(&TriggerEvent::Custom) {
        if draft.filters.names.is_empty() {
            return Err(IpcError::new(
                "trigger_events",
                "Name the custom event it listens for: `custom.<name>`.",
            ));
        }
        if let Some(why) = draft
            .filters
            .names
            .iter()
            .find_map(|name| custom_name_problem(name))
        {
            return Err(IpcError::new("trigger_events", why));
        }
    }
    for (filter, about) in draft.filters.given() {
        if !about.iter().any(|event| events.contains(event)) {
            let names: Vec<String> = about
                .iter()
                .map(|event| format!("`{}`", event.as_str()))
                .collect();
            return Err(IpcError::new(
                "trigger_filters",
                format!(
                    "`{filter}` is about {}, which this trigger does not listen for.",
                    names.join(" or ")
                ),
            ));
        }
    }
    if !(0..=MOST_QUIET_MS).contains(&draft.quiet_ms) {
        return Err(IpcError::new(
            "trigger_quiet",
            "The longest a trigger waits after the last event is an hour.",
        ));
    }
    if !(1..=MOST_CAP).contains(&draft.cap) {
        return Err(IpcError::new(
            "trigger_cap",
            format!("A trigger starts from 1 to {MOST_CAP} runs before it stops and asks."),
        ));
    }
    let target_name = match draft.target {
        ScheduleTarget::Workflow => {
            let entry = crate::workflow::find(data_dir, draft.target_id).ok_or_else(|| {
                IpcError::new(
                    "workflow_not_found",
                    format!("There is no workflow `{}`.", draft.target_id),
                )
            })?;
            let Some(workflow) = entry.workflow.as_ref().filter(|_| entry.runnable()) else {
                return Err(IpcError::new(
                    "workflow_invalid",
                    format!("`{}` has problems and cannot run.", entry.id),
                ));
            };
            let harness = |wanted: &str| look.harness(wanted);
            crate::workflow::runs::check_inputs(workflow, draft.inputs, &harness)
                .map_err(|why| IpcError::new("workflow_inputs", why))?;
            if let Some(why) = workflow_waits_for_ever(&workspace, workflow) {
                return Err(IpcError::new("trigger_needs_person", why));
            }
            check_event_variables(workflow, &events)?;
            check_pull_request_gone(workflow, &events)?;
            workflow.name.clone()
        }
        ScheduleTarget::Test => {
            let test = test_of(&workspace, draft.target_id)?;
            if let Some(why) = crate::schedule::needs_a_person(&test) {
                return Err(IpcError::new("trigger_needs_person", why));
            }
            crate::smoke::runs::check_inputs(&test, draft.inputs)
                .map_err(|why| IpcError::new("test_inputs", why))?;
            test.name.clone()
        }
    };
    let name = draft
        .name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or(target_name);
    let names: Vec<&str> = events.iter().map(|event| event.as_str()).collect();
    let mut filters = draft.filters.clone();
    filters.names.sort();
    filters.names.dedup();
    Ok(Checked {
        name,
        workspace,
        events: serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_owned()),
        filters: filters.to_json(),
        inputs: serde_json::to_string(draft.inputs).unwrap_or_else(|_| "{}".to_owned()),
    })
}

/// The smoke test `id` in the workspace's folder, ready to run.
fn test_of(workspace: &WorkspaceRow, id: &str) -> IpcResult<crate::smoke::Test> {
    let entry = crate::smoke::find(Path::new(&workspace.path), id).ok_or_else(|| {
        IpcError::new(
            "test_not_found",
            format!("There is no smoke test `{id}` in `{}`.", workspace.name),
        )
    })?;
    let runnable = entry.runnable();
    match entry.test.filter(|_| runnable) {
        Some(test) => Ok(test),
        None => Err(IpcError::new(
            "test_invalid",
            format!("`{}` has problems and cannot run.", entry.id),
        )),
    }
}

/// Why a workflow could wait for ever when a trigger starts it, when it could. A triggered run
/// may wait for the user to approve a smoke test's file, but nothing else a person does: a
/// `run_test` step with no timeout could wait for an approval that never comes, and one whose
/// test has a step a person answers would wait for nobody.
///
/// A step whose test is a `{{ variable }}` is not known until it runs; its timeout ends a wait
/// for a person there, which is why every `run_test` step must have one.
pub(crate) fn workflow_waits_for_ever(
    workspace: &WorkspaceRow,
    workflow: &crate::workflow::Workflow,
) -> Option<String> {
    for step in &workflow.steps {
        let crate::workflow::Action::RunTest {
            test, timeout_secs, ..
        } = &step.action
        else {
            continue;
        };
        if timeout_secs.is_none() {
            return Some(format!(
                "`{}` cannot be started by a trigger: its step `{}` runs a smoke test with no \
                 `timeout`, and could wait for ever for an approval nobody gives. Give the step \
                 a timeout.",
                workflow.id, step.id
            ));
        }
        if test.contains("{{") {
            continue;
        }
        let needs = crate::smoke::find(Path::new(&workspace.path), test)
            .and_then(|entry| entry.test)
            .and_then(|test| crate::schedule::needs_a_person(&test));
        if let Some(why) = needs {
            return Some(format!(
                "`{}` cannot be started by a trigger: its step `{}` runs a smoke test that \
                 would wait for you. {why}",
                workflow.id, step.id
            ));
        }
    }
    None
}

/// Refuse a workflow that uses an `{{ event.* }}` some of the events would not leave.
fn check_event_variables(
    workflow: &crate::workflow::Workflow,
    events: &[TriggerEvent],
) -> IpcResult<()> {
    for used in crate::workflow::runs::variables(workflow) {
        let Some(field) = used.strip_prefix("event.") else {
            continue;
        };
        // Every fire leaves these, whatever its events.
        if field == "kind" || field == "count" {
            continue;
        }
        if let Some(event) = events.iter().find(|event| !event.leaves().contains(&field)) {
            let which = match event {
                TriggerEvent::Custom => "a custom event".to_owned(),
                event => format!("`{}`", event.as_str()),
            };
            return Err(IpcError::new(
                "trigger_variables",
                format!(
                    "`{}` uses `{{{{ event.{field} }}}}`, which {which} does not leave.",
                    workflow.id
                ),
            ));
        }
    }
    Ok(())
}

/// Refuse a workflow that needs the workspace's open pull request on an event that comes after
/// it stopped being open: the door would refuse it at every fire.
fn check_pull_request_gone(
    workflow: &crate::workflow::Workflow,
    events: &[TriggerEvent],
) -> IpcResult<()> {
    let gone = events
        .iter()
        .find(|event| matches!(event, TriggerEvent::PrMerged | TriggerEvent::PrClosed));
    match gone {
        Some(event) if crate::workflow::runs::needs_pull_request(workflow) => Err(IpcError::new(
            "trigger_pull_request_gone",
            format!(
                "`{}` needs the workspace's open pull request, with `{{{{ pr.* }}}}` or \
                 `wait_pr_activity`, and after `{}` there is none. Use `{{{{ event.url }}}}`, \
                 `{{{{ event.number }}}}` and `{{{{ event.title }}}}` instead.",
                workflow.id,
                event.as_str()
            ),
        )),
        _ => Ok(()),
    }
}

fn fields<'a>(draft: &'a Draft<'_>, checked: &'a Checked) -> TriggerFields<'a> {
    TriggerFields {
        name: &checked.name,
        preset: draft.preset,
        project_id: &checked.workspace.project_id,
        workspace_id: &checked.workspace.id,
        workspace_name: &checked.workspace.name,
        session_id: draft.session_id,
        events: &checked.events,
        filters: &checked.filters,
        target_kind: draft.target.as_str(),
        target_id: draft.target_id,
        inputs: &checked.inputs,
        quiet_ms: draft.quiet_ms,
        cap: draft.cap,
        notify: draft.notify.as_str(),
    }
}

/// Add a trigger, switched on. Only what happens from `now` counts. One on a `pr.*` event has
/// its workspace's pull request watched from what the forge says now (see [`watch::seed`]).
pub fn add(
    store: &Store,
    data_dir: &Path,
    look: &dyn Look,
    draft: &Draft<'_>,
    created_by: &str,
    now: i64,
) -> IpcResult<TriggerRow> {
    let checked = check_draft(store, data_dir, look, draft)?;
    let row = store.add_trigger(&fields(draft, &checked), created_by, now)?;
    watch_if_asked(store, look, draft, &checked, now)?;
    Ok(row)
}

/// Change a trigger. Whether it is on, and what it has seen, stay.
pub fn change(
    store: &Store,
    data_dir: &Path,
    look: &dyn Look,
    id: &str,
    draft: &Draft<'_>,
    now: i64,
) -> IpcResult<TriggerRow> {
    let checked = check_draft(store, data_dir, look, draft)?;
    if !store.update_trigger(id, &fields(draft, &checked))? {
        return Err(not_found());
    }
    watch_if_asked(store, look, draft, &checked, now)?;
    store.trigger(id)?.ok_or_else(not_found)
}

fn watch_if_asked(
    store: &Store,
    look: &dyn Look,
    draft: &Draft<'_>,
    checked: &Checked,
    now: i64,
) -> IpcResult<()> {
    if draft.events.iter().any(|event| event.of_a_pull_request()) {
        watch::seed(store, look, &checked.workspace, now)?;
    }
    Ok(())
}

/// Switch a trigger on or off. Switched on, only what happens from `now` counts: what happened
/// while it was off is not made up for.
pub fn set_enabled(store: &Store, id: &str, enabled: bool, now: i64) -> IpcResult<TriggerRow> {
    let trigger = store.trigger(id)?.ok_or_else(not_found)?;
    if enabled && trigger.workspace_id.is_none() {
        return Err(IpcError::new(
            "workspace_not_found",
            format!(
                "`{}` was deleted, so this has nothing to listen to.",
                trigger.workspace_name
            ),
        ));
    }
    store.set_trigger_enabled(id, enabled, now)?;
    store.trigger(id)?.ok_or_else(not_found)
}

/// Whether an enabled trigger in the workspace listens for `event`: what decides whether an
/// agent is told about `ys workspace done`.
pub fn listens_for(store: &Store, workspace_id: &str, event: TriggerEvent) -> IpcResult<bool> {
    Ok(store
        .workspace_triggers(workspace_id)?
        .iter()
        .any(|trigger| events_of(trigger).contains(&event)))
}

/// The custom events enabled triggers in the workspace listen for, by name: what tells
/// `ys event emit` whether anything heard it, and what a mistyped name may have meant.
pub fn custom_names_heard(store: &Store, workspace_id: &str) -> IpcResult<BTreeSet<String>> {
    Ok(store
        .workspace_triggers(workspace_id)?
        .iter()
        .filter(|trigger| events_of(trigger).contains(&TriggerEvent::Custom))
        .flat_map(|trigger| Filters::parse(&trigger.filters).names)
        .collect())
}

/// What the first message of an agent in a workspace with a trigger on `workspace.done` is
/// told. A command to run, never anything its output is read for.
pub const DONE_SECTION: &str = "## When you are done\n\n\
     This workspace starts something when its work is done — a review, say. When your work is \
     committed and ready to be looked at, run `ys workspace done`, with `--note \"…\"` for \
     anything the next one should know. Run it once the work is done, not to pause or to ask a \
     question.";

/// Where a trigger stands, for the list: what a person needs to know at a glance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TriggerStanding {
    /// On, listening.
    Listening,
    /// A run it started is going.
    Running,
    /// A run it started waits for the user to approve a test file.
    AwaitingApproval,
    /// It has gathered events and waits for a run in its way to end.
    Waiting,
    /// A subject has had its runs. Nothing more fires for it until Reset.
    Capped,
    /// Its last fire could not start a run.
    Refused,
    /// Its workspace was deleted.
    NoWorkspace,
    /// Switched off by the user.
    Off,
}

impl TriggerStanding {
    /// Whether the user has something to do before this trigger can do its job.
    pub fn needs_attention(self) -> bool {
        matches!(
            self,
            Self::AwaitingApproval | Self::Capped | Self::Refused | Self::NoWorkspace
        )
    }
}

/// Where `trigger` stands, given its fires, newest first: enough of them to reach its newest
/// decided one.
pub fn standing(trigger: &TriggerRow, fires: &[TriggerFireRow]) -> TriggerStanding {
    if trigger.workspace_id.is_none() {
        return TriggerStanding::NoWorkspace;
    }
    let going = fires
        .iter()
        .find(|fire| fire.outcome == "started" && fire.result.is_none());
    if let Some(fire) = going {
        return if fire.told_waiting {
            TriggerStanding::AwaitingApproval
        } else {
            TriggerStanding::Running
        };
    }
    if !trigger.enabled {
        return TriggerStanding::Off;
    }
    if fires.iter().any(|fire| fire.outcome == "waiting") {
        return TriggerStanding::Waiting;
    }
    match fires
        .iter()
        .find(|fire| fire.outcome != "gathering")
        .map(|fire| fire.outcome.as_str())
    {
        Some("capped") => TriggerStanding::Capped,
        Some("refused") => TriggerStanding::Refused,
        _ => TriggerStanding::Listening,
    }
}

/// Whether `trigger`'s newest decided line is a cap still in force: its subject's runs still
/// reach the cap. After **Reset**, or a cap raised by an edit, the capped line stays the newest
/// until the next fire, but nothing is held by it. A run started before the cap may still be
/// going.
pub fn cap_in_force(
    store: &Store,
    trigger: &TriggerRow,
    fires: &[TriggerFireRow],
) -> IpcResult<bool> {
    match fires.iter().find(|fire| fire.outcome != "gathering") {
        Some(line) if line.outcome == "capped" => {
            Ok(store.counted_trigger_runs(&trigger.id, &line.subject)? >= trigger.cap)
        }
        _ => Ok(false),
    }
}

/// [`standing`], with a cap that was lifted read as lifted.
pub fn standing_now(
    store: &Store,
    trigger: &TriggerRow,
    fires: &[TriggerFireRow],
) -> IpcResult<TriggerStanding> {
    Ok(match standing(trigger, fires) {
        TriggerStanding::Capped if !cap_in_force(store, trigger, fires)? => {
            TriggerStanding::Listening
        }
        standing => standing,
    })
}

/// A trigger in words: "When an agent says its work is done, run the workflow `review`".
pub fn describe(trigger: &TriggerRow) -> String {
    describe_on(trigger, None)
}

/// [`describe`], naming the pull request it watches when it has one: "When a review comes in
/// on #142, …".
pub fn describe_on(trigger: &TriggerRow, pull_request: Option<u32>) -> String {
    let filters = Filters::parse(&trigger.filters);
    let events: Vec<String> = events_of(trigger)
        .iter()
        .map(|event| match pull_request {
            _ if *event == TriggerEvent::Custom => {
                let names: Vec<String> = event_names(&[*event], &filters)
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect();
                match names.as_slice() {
                    [] => event.words().to_owned(),
                    [one] => format!("{one} is raised"),
                    [rest @ .., last] => format!("{} or {last} is raised", rest.join(", ")),
                }
            }
            Some(number) => event
                .words()
                .replace("the workspace's pull request", &format!("#{number}"))
                .replace("the pull request", &format!("#{number}")),
            None => event.words().to_owned(),
        })
        .collect();
    let what = ScheduleTarget::parse(&trigger.target_kind).map_or("run", ScheduleTarget::words);
    let when = match events.as_slice() {
        [] => "When something this Yardsort does not know happens".to_owned(),
        [one] => format!("When {one}"),
        [rest @ .., last] => format!("When {} or {last}", rest.join(", ")),
    };
    format!("{when}, run the {what} `{}`", trigger.target_id)
}

/// Every `{{ event.* }}` field a workflow uses, for tests and the dialog.
pub fn event_fields(workflow: &crate::workflow::Workflow) -> BTreeSet<String> {
    crate::workflow::runs::variables(workflow)
        .into_iter()
        .filter_map(|used| used.strip_prefix("event.").map(str::to_owned))
        .collect()
}

pub(crate) fn not_found() -> IpcError {
    IpcError::new("trigger_not_found", "That trigger is not there.")
}

#[cfg(test)]
mod tests;
