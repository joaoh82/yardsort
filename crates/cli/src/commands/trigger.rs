//! `ys trigger …` — standing instructions to start a workflow or a smoke test when something
//! happens in a workspace: an agent says its work is done, finishes a turn or exits, a test or
//! a workflow run ends, or something happens on the workspace's pull request.
//!
//! A trigger is a row; `ys` writes it and the app, when it is open, listens and acts within a
//! second. See `yardsort_core::trigger` and docs/guide/triggers.md.

use std::collections::BTreeMap;

use serde::Serialize;
use yardsort_core::memory;
use yardsort_core::presence;
use yardsort_core::schedule::{clock, ScheduleNotify, ScheduleTarget};
use yardsort_core::store::{TriggerFireRow, TriggerRow};
use yardsort_core::trigger::{
    self, fire, presets, Draft, ExitFilter, Filters, FromFilter, OutcomeFilter, ReviewStateFilter,
    TriggerEvent, TriggerStanding, VerdictFilter,
};

use super::workflow::Looking;
use crate::{table, Failure, Output, Yardsort};

/// How many of a trigger's fires are read to know where it stands.
const STANDING_FIRES: usize = 20;

#[derive(clap::Subcommand)]
pub enum Command {
    /// Every trigger: what it listens for, where, what it runs, and how its last fire went.
    List,
    /// The ready-made triggers `add --preset` takes, and the question each asks.
    Presets,
    /// One trigger: what it listens for and its filters, its inputs, and where it stands.
    Show {
        /// The trigger: its name, its id, or the start of its id.
        trigger: String,
    },
    /// Add a trigger: a preset (`--preset`), or what to listen for (`--on`, one or more) and
    /// what to run (`--workflow` or `--test`). Triggers fire while Yardsort is open.
    Add(Box<Add>),
    /// Switch a trigger off. It keeps its history, and fires nothing.
    Pause { trigger: String },
    /// Switch a trigger back on. Only what happens from now counts.
    Resume { trigger: String },
    /// Lift a trigger's cap, so it starts runs again where it had started all it may.
    Reset { trigger: String },
    /// A trigger's history: every fire and what became of it, newest first.
    Fires {
        trigger: String,
        /// How many to list.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Remove a trigger and its history. The runs it started stay where they are.
    Remove { trigger: String },
}

/// What `add` takes.
#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("what").args(["workflow", "test"])))]
pub struct Add {
    /// A ready-made trigger, from `ys trigger presets`. Give its one answer with `--input`.
    #[arg(long, value_name = "PRESET", conflicts_with_all = ["events", "workflow", "test"])]
    pub preset: Option<String>,
    /// What to listen for. Repeat for more than one: `workspace.done`, `agent.settled`,
    /// `agent.exited`, `test.finished`, `workflow.finished`, `pr.opened`, `pr.review`,
    /// `pr.comment`, `pr.checks_failed`, `pr.checks_passed`, `pr.conflicted`, `pr.merged`,
    /// `pr.closed`.
    #[arg(long = "on", value_name = "EVENT", required_unless_present = "preset")]
    pub events: Vec<String>,
    /// Run this workflow, from `ys workflow list`.
    #[arg(long, value_name = "ID")]
    pub workflow: Option<String>,
    /// Run this smoke test, from `ys test list`.
    #[arg(long, value_name = "ID")]
    pub test: Option<String>,
    /// Which workspace, by name or id. Default: the one this is run in.
    #[arg(long, value_name = "WORKSPACE")]
    pub workspace: Option<String>,
    /// Only this agent session's events, by its conversation id or the start of it: a trigger
    /// that belongs to one agent, and whose runs talk to it.
    #[arg(long, value_name = "SESSION")]
    pub session: Option<String>,
    /// An answer to one of its inputs. Repeat for each.
    #[arg(long = "input", value_name = "ID=VALUE")]
    pub inputs: Vec<String>,
    /// What to call it. Default: the workflow's or the test's own name.
    #[arg(long)]
    pub name: Option<String>,
    /// How long to wait after the last event before firing, so a burst is one run: 30s, 2m.
    /// Default: 2m for reviews and comments, none for the rest.
    #[arg(long, value_name = "INTERVAL")]
    pub quiet: Option<String>,
    /// How many runs it may start for one agent session, one pull request, or the workspace,
    /// before it stops and asks.
    #[arg(long, default_value_t = 3)]
    pub cap: i64,
    /// When to be told how a run ended.
    #[arg(long, value_enum, default_value_t = NotifyArg::Failure)]
    pub notify: NotifyArg,
    /// `agent.settled`: only when the agent's branch has new commits.
    #[arg(long)]
    pub new_commits: bool,
    /// `agent.settled`: only when nothing in the worktree is uncommitted.
    #[arg(long)]
    pub clean: bool,
    /// `agent.exited`: only when its program failed.
    #[arg(long)]
    pub failed_exit: bool,
    /// `test.finished`: only this smoke test.
    #[arg(long, value_name = "ID")]
    pub of_test: Option<String>,
    /// `test.finished`: only this verdict.
    #[arg(long, value_enum)]
    pub verdict: Option<VerdictArg>,
    /// `workflow.finished`: only this workflow.
    #[arg(long, value_name = "ID")]
    pub of_workflow: Option<String>,
    /// `workflow.finished`: only this outcome.
    #[arg(long, value_enum)]
    pub outcome: Option<OutcomeArg>,
    /// `pr.opened`: a draft counts as soon as it opens. Without it, once it is marked ready.
    #[arg(long)]
    pub drafts: bool,
    /// `pr.review`, `pr.comment`: whose posts count. Default: accounts other than yours.
    #[arg(long, value_enum)]
    pub from: Option<FromArg>,
    /// `pr.review`: only reviews that concluded this. Repeat for more than one.
    #[arg(long = "state", value_enum, value_name = "STATE")]
    pub states: Vec<StateArg>,
    /// `pr.review`, `pr.comment`: leave out accounts whose login ends in `[bot]`.
    #[arg(long)]
    pub no_bots: bool,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum FromArg {
    /// Accounts other than the one `gh` posts as: yours, and every agent's.
    Others,
    /// Yours too, except while a run this trigger started is going.
    Anyone,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum StateArg {
    Approved,
    ChangesRequested,
    Commented,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum NotifyArg {
    Failure,
    Always,
    Never,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum VerdictArg {
    Passed,
    /// Failed, or could not finish.
    Failed,
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum OutcomeArg {
    Succeeded,
    /// Failed, or cancelled.
    Failed,
}

pub fn run(ys: &Yardsort, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::List => list(ys, out),
        Command::Presets => list_presets(out),
        Command::Show { trigger } => show(ys, &trigger, out),
        Command::Add(asked) => add(ys, *asked, out),
        Command::Pause { trigger } => switch(ys, &trigger, false, out),
        Command::Resume { trigger } => switch(ys, &trigger, true, out),
        Command::Reset { trigger } => reset(ys, &trigger, out),
        Command::Fires { trigger, limit } => fires(ys, &trigger, limit, out),
        Command::Remove { trigger } => remove(ys, &trigger, out),
    }
}

/// `30s`, `2m`, `1h`, or a bare number of seconds, as milliseconds.
fn quiet_ms(text: &str) -> Result<i64, Failure> {
    let text = text.trim().to_ascii_lowercase();
    let (number, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((at, _)) => text.split_at(at),
        None => (text.as_str(), "s"),
    };
    let each = match unit.trim() {
        "s" | "sec" | "secs" | "second" | "seconds" => Some(1_000),
        "m" | "min" | "mins" | "minute" | "minutes" => Some(60_000),
        "h" | "hr" | "hour" | "hours" => Some(3_600_000),
        _ => None,
    };
    number
        .parse::<i64>()
        .ok()
        .zip(each)
        .and_then(|(number, each)| number.checked_mul(each))
        .ok_or_else(|| {
            Failure::new(format!(
                "`{text}` is not a length of time. Write it as 30s, 2m or 1h."
            ))
        })
}

/// A trigger by its id, the start of its id, or its name.
fn find(ys: &Yardsort, wanted: &str) -> Result<TriggerRow, Failure> {
    let wanted = wanted.trim();
    let all = ys.store.triggers()?;
    if let Some(exact) = all.iter().find(|t| t.id == wanted) {
        return Ok(exact.clone());
    }
    let matches: Vec<&TriggerRow> = all
        .iter()
        .filter(|t| {
            !wanted.is_empty() && (t.name.eq_ignore_ascii_case(wanted) || t.id.starts_with(wanted))
        })
        .collect();
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] if all.is_empty() => Err(Failure::new(
            "There are no triggers. `ys trigger add` makes one.",
        )),
        [] => Err(Failure::new(format!(
            "No trigger called {wanted:?}. There are: {}.",
            all.iter()
                .map(|t| format!("{} ({})", t.name, memory::short_id(&t.id)))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
        many => Err(Failure::new(format!(
            "{} triggers match {wanted:?}. Use the id instead: {}.",
            many.len(),
            many.iter()
                .map(|t| format!("{} ({})", memory::short_id(&t.id), t.workspace_name))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// A trigger as `list` and `show` print it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
    id: String,
    name: String,
    events: Vec<String>,
    filters: Filters,
    target: String,
    target_id: String,
    workspace: String,
    workspace_id: Option<String>,
    session_id: Option<String>,
    inputs: BTreeMap<String, String>,
    quiet_ms: i64,
    cap: i64,
    notify: String,
    enabled: bool,
    when: String,
    standing: TriggerStanding,
    last: Option<Line>,
}

/// One fire in a trigger's history.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Line {
    at: i64,
    event: String,
    events: i64,
    outcome: String,
    run: Option<String>,
    result: Option<String>,
    note: Option<String>,
}

impl From<&TriggerFireRow> for Line {
    fn from(fire: &TriggerFireRow) -> Self {
        Self {
            at: fire.decided_at.unwrap_or(fire.first_at),
            event: fire.event.clone(),
            events: fire.events_count,
            outcome: fire.outcome.clone(),
            run: fire.run_id.clone(),
            result: fire.result.clone(),
            note: fire.note.clone(),
        }
    }
}

fn listed(row: &TriggerRow, fires: &[TriggerFireRow]) -> Listed {
    Listed {
        id: row.id.clone(),
        name: row.name.clone(),
        events: trigger::events_of(row)
            .iter()
            .map(|event| event.as_str().to_owned())
            .collect(),
        filters: Filters::parse(&row.filters),
        target: row.target_kind.clone(),
        target_id: row.target_id.clone(),
        workspace: row.workspace_name.clone(),
        workspace_id: row.workspace_id.clone(),
        session_id: row.session_id.clone(),
        inputs: serde_json::from_str(&row.inputs).unwrap_or_default(),
        quiet_ms: row.quiet_ms,
        cap: row.cap,
        notify: row.notify.clone(),
        enabled: row.enabled,
        when: trigger::describe(row),
        standing: trigger::standing(row, fires),
        last: fires
            .iter()
            .find(|fire| fire.outcome != "gathering")
            .map(Line::from),
    }
}

fn listed_one(ys: &Yardsort, row: &TriggerRow) -> Result<Listed, Failure> {
    let fires = ys.store.trigger_fires(&row.id, STANDING_FIRES)?;
    Ok(listed(row, &fires))
}

/// Where a trigger stands, in a word or two.
fn standing_words(standing: TriggerStanding) -> &'static str {
    match standing {
        TriggerStanding::Listening => "on",
        TriggerStanding::Running => "running",
        TriggerStanding::AwaitingApproval => "waiting for approval",
        TriggerStanding::Waiting => "waiting",
        TriggerStanding::Capped => "capped",
        TriggerStanding::Refused => "refused",
        TriggerStanding::NoWorkspace => "no workspace",
        TriggerStanding::Off => "off",
    }
}

/// What became of a fire, in a word or two: the run's ending when it has one.
fn outcome_words(fire: &TriggerFireRow) -> String {
    match (fire.outcome.as_str(), fire.result.as_deref()) {
        ("started", Some(result)) => result.to_owned(),
        ("started", None) if fire.told_waiting => "waiting for approval".to_owned(),
        ("started", None) => "running".to_owned(),
        (outcome, _) => outcome.to_owned(),
    }
}

/// Said under anything about what a trigger will do: nothing, with no app.
fn not_running_note(ys: &Yardsort) {
    match presence::holder(&ys.data_dir) {
        Some(presence::Holder::App) => {}
        Some(presence::Holder::Host) => {
            sayln!();
            sayln!(
                "This host does not fire triggers yet: yardsort-host is running it, not the app."
            );
        }
        None => {
            sayln!();
            sayln!("Yardsort is not running. Triggers fire while it is open.");
        }
    }
}

fn list(ys: &Yardsort, out: &Output) -> Result<(), Failure> {
    let rows = ys.store.triggers()?;
    let all = rows
        .iter()
        .map(|row| listed_one(ys, row))
        .collect::<Result<Vec<_>, _>>()?;
    out.emit(&all, || {
        if rows.is_empty() {
            sayln!("No triggers. `ys trigger add` makes one.");
            return;
        }
        let mut lines = vec![vec![
            "ID".to_owned(),
            "NAME".to_owned(),
            "ON".to_owned(),
            "RUNS".to_owned(),
            "IN".to_owned(),
            "LAST".to_owned(),
            "".to_owned(),
        ]];
        lines.extend(rows.iter().zip(&all).map(|(row, item)| {
            vec![
                memory::short_id(&row.id).to_owned(),
                row.name.clone(),
                item.events.join(", "),
                format!("{} {}", row.target_kind, row.target_id),
                match &row.session_id {
                    Some(session) => format!(
                        "{} (session {})",
                        row.workspace_name,
                        memory::short_id(session)
                    ),
                    None => row.workspace_name.clone(),
                },
                item.last
                    .as_ref()
                    .map_or_else(|| "-".to_owned(), |line| line.outcome.clone()),
                standing_words(item.standing).to_owned(),
            ]
        }));
        table(&lines);
        not_running_note(ys);
    })
}

fn show(ys: &Yardsort, wanted: &str, out: &Output) -> Result<(), Failure> {
    let row = find(ys, wanted)?;
    let fires = ys.store.trigger_fires(&row.id, STANDING_FIRES)?;
    let item = listed(&row, &fires);
    out.emit(&item, || {
        sayln!("{} ({})", row.name, memory::short_id(&row.id));
        sayln!("{}, in {}.", item.when, row.workspace_name);
        if let Some(session) = &row.session_id {
            sayln!(
                "Only the agent session {}'s own events; its runs talk to it.",
                memory::short_id(session)
            );
        }
        let filters = &item.filters;
        let mut only = Vec::new();
        if filters.new_commits {
            only.push("the branch has new commits".to_owned());
        }
        if filters.clean {
            only.push("nothing is uncommitted".to_owned());
        }
        if filters.exit == Some(ExitFilter::Failed) {
            only.push("the program failed".to_owned());
        }
        if let Some(test) = &filters.test {
            only.push(format!("the test is `{test}`"));
        }
        if let Some(verdict) = filters.verdict.filter(|v| *v != VerdictFilter::Any) {
            only.push(format!(
                "it {}",
                if verdict == VerdictFilter::Passed {
                    "passed"
                } else {
                    "failed"
                }
            ));
        }
        if let Some(workflow) = &filters.workflow {
            only.push(format!("the workflow is `{workflow}`"));
        }
        if let Some(outcome) = filters.outcome.filter(|o| *o != OutcomeFilter::Any) {
            only.push(format!(
                "it {}",
                if outcome == OutcomeFilter::Succeeded {
                    "succeeded"
                } else {
                    "failed"
                }
            ));
        }
        if filters.from == Some(FromFilter::Anyone) {
            only.push("anyone posts it, you too".to_owned());
        } else if trigger::events_of(&row)
            .iter()
            .any(|event| matches!(event, TriggerEvent::PrReview | TriggerEvent::PrComment))
        {
            only.push("someone other than you posts it".to_owned());
        }
        if !filters.states.is_empty() {
            let states: Vec<&str> = filters.states.iter().map(|state| state.as_str()).collect();
            only.push(format!("the review is {}", states.join(" or ")));
        }
        if filters.no_bots {
            only.push("not a bot".to_owned());
        }
        if !only.is_empty() {
            sayln!("Only when {}.", only.join(", and "));
        }
        if filters.drafts {
            sayln!("A draft counts as soon as it opens.");
        }
        for (id, value) in &item.inputs {
            sayln!("  {id} = {value}");
        }
        if row.quiet_ms > 0 {
            sayln!(
                "Waits {}s after the last event, so a burst is one run.",
                row.quiet_ms / 1_000
            );
        }
        sayln!(
            "Starts at most {} run{} for each before it stops and asks. Tells you: {}.",
            row.cap,
            if row.cap == 1 { "" } else { "s" },
            match ScheduleNotify::parse(&row.notify).unwrap_or_default() {
                ScheduleNotify::Failure => "when a run fails",
                ScheduleNotify::Always => "how every run ends",
                ScheduleNotify::Never => "never",
            }
        );
        sayln!("It is {}.", standing_words(item.standing));
        if let Some(last) = fires.iter().find(|fire| fire.outcome != "gathering") {
            sayln!(
                "Last: {} — {}{}",
                clock::show(last.decided_at.unwrap_or(last.first_at)),
                outcome_words(last),
                last.note
                    .as_deref()
                    .map(|note| format!(". {note}"))
                    .unwrap_or_default()
            );
        }
        not_running_note(ys);
    })
}

/// A preset as `presets` prints it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PresetListed {
    id: &'static str,
    name: &'static str,
    does: &'static str,
    events: Vec<&'static str>,
    workflow: &'static str,
    asks: Option<&'static str>,
}

fn list_presets(out: &Output) -> Result<(), Failure> {
    let all: Vec<PresetListed> = presets::PRESETS
        .iter()
        .map(|preset| PresetListed {
            id: preset.id,
            name: preset.name,
            does: preset.does,
            events: preset.events.iter().map(|event| event.as_str()).collect(),
            workflow: preset.workflow,
            asks: preset.asks.map(|(input, _)| input),
        })
        .collect();
    out.emit(&all, || {
        for preset in presets::PRESETS {
            sayln!("{} — {}", preset.id, preset.name);
            sayln!("  {}", preset.does);
            match preset.asks {
                Some((input, question)) => sayln!(
                    "  Runs `{}`. {question}? --input {input}=…",
                    preset.workflow
                ),
                None => sayln!("  Runs `{}`.", preset.workflow),
            }
        }
    })
}

fn add(ys: &Yardsort, asked: Add, out: &Output) -> Result<(), Failure> {
    let preset = match asked.preset.as_deref() {
        Some(id) => Some(presets::find(id).ok_or_else(|| {
            Failure::new(format!(
                "There is no preset `{id}`. There are: {}.",
                presets::PRESETS
                    .iter()
                    .map(|preset| preset.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?),
        None => None,
    };
    let mut events = preset
        .map(|preset| preset.events.to_vec())
        .unwrap_or_default();
    for name in asked.events.iter().flat_map(|name| name.split(',')) {
        let name = name.trim();
        let event = TriggerEvent::parse(name).ok_or_else(|| {
            Failure::new(format!(
                "There is no event `{name}`. There are: {}.",
                TriggerEvent::ALL
                    .iter()
                    .map(|event| event.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        events.push(event);
    }
    let (target, target_id) = match (preset, &asked.workflow, &asked.test) {
        (Some(preset), _, _) => (ScheduleTarget::Workflow, preset.workflow.to_owned()),
        (None, Some(id), _) => (ScheduleTarget::Workflow, id.clone()),
        (None, None, Some(id)) => (ScheduleTarget::Test, id.clone()),
        (None, None, None) => return Err(Failure::new("Say what to run: --workflow or --test.")),
    };
    // A session names its workspace; otherwise the one named, or the one this is run in.
    let session = match asked.session.as_deref() {
        Some(wanted) => Some(find_session(ys, wanted, asked.workspace.as_deref())?),
        None => None,
    };
    let workspace = match (&session, asked.workspace.as_deref()) {
        (Some(session), _) => super::workspace::find_workspace(ys, &session.workspace_id)?,
        (None, Some(wanted)) => super::workspace::find_workspace(ys, wanted)?,
        (None, None) => super::workflow::here(ys)?,
    };
    let mut inputs = BTreeMap::new();
    for pair in &asked.inputs {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(Failure::new(format!(
                "`--input {pair}` needs an `=`: `--input id=value`."
            )));
        };
        inputs.insert(key.trim().to_owned(), value.to_owned());
    }
    if let Some((input, question)) = preset.and_then(|preset| preset.asks) {
        if !inputs.contains_key(input) {
            return Err(Failure::new(format!(
                "{question}? Say with `--input {input}=…`."
            )));
        }
    }
    let filters = Filters {
        new_commits: asked.new_commits,
        clean: asked.clean,
        exit: asked.failed_exit.then_some(ExitFilter::Failed),
        test: asked.of_test.clone(),
        verdict: asked.verdict.map(|verdict| match verdict {
            VerdictArg::Passed => VerdictFilter::Passed,
            VerdictArg::Failed => VerdictFilter::Failed,
        }),
        workflow: asked.of_workflow.clone(),
        outcome: asked.outcome.map(|outcome| match outcome {
            OutcomeArg::Succeeded => OutcomeFilter::Succeeded,
            OutcomeArg::Failed => OutcomeFilter::Failed,
        }),
        drafts: asked.drafts,
        from: asked.from.map(|from| match from {
            FromArg::Others => FromFilter::Others,
            FromArg::Anyone => FromFilter::Anyone,
        }),
        states: asked
            .states
            .iter()
            .map(|state| match state {
                StateArg::Approved => ReviewStateFilter::Approved,
                StateArg::ChangesRequested => ReviewStateFilter::ChangesRequested,
                StateArg::Commented => ReviewStateFilter::Commented,
            })
            .collect(),
        no_bots: asked.no_bots,
    };
    let quiet = match asked.quiet.as_deref() {
        Some(text) => quiet_ms(text)?,
        None => events
            .iter()
            .map(|event| event.default_quiet_ms())
            .max()
            .unwrap_or(0),
    };
    let row = trigger::add(
        &ys.store,
        &ys.data_dir,
        &Looking(ys),
        &Draft {
            name: asked.name.as_deref().or(preset.map(|preset| preset.name)),
            preset: preset.map(|preset| preset.id),
            workspace_id: &workspace.id,
            session_id: session.as_ref().map(|s| s.id.as_str()),
            events: &events,
            filters: &filters,
            target,
            target_id: &target_id,
            inputs: &inputs,
            quiet_ms: quiet,
            cap: asked.cap,
            notify: match asked.notify {
                NotifyArg::Failure => ScheduleNotify::Failure,
                NotifyArg::Always => ScheduleNotify::Always,
                NotifyArg::Never => ScheduleNotify::Never,
            },
        },
        "cli",
        clock::now().timestamp_millis(),
    )?;
    // An agent already running there was told nothing about `ys workspace done` when it began.
    let untold = events.contains(&TriggerEvent::WorkspaceDone)
        && ys
            .store
            .sessions(&workspace.id)?
            .iter()
            .any(|session| session.running);
    let watch = if events.iter().any(|event| event.of_a_pull_request()) {
        ys.store.pr_watch(&workspace.id)?
    } else {
        None
    };
    let item = listed(&row, &[]);
    out.emit(&item, || {
        sayln!(
            "Added `{}` ({}): {}.",
            row.name,
            memory::short_id(&row.id),
            item.when
        );
        if let Some(watch) = &watch {
            match (watch.number, &watch.problem) {
                (_, Some(problem)) => sayln!(
                    "Could not look at its pull request yet ({problem}). Yardsort looks again \
                     when it is open."
                ),
                (Some(number), None) => sayln!(
                    "Watching #{number}: what is there now is not news, only what comes next."
                ),
                (None, None) => sayln!(
                    "{} has no pull request yet. One opening is `pr.opened`.",
                    workspace.name
                ),
            }
        }
        if untold {
            sayln!(
                "Note: agents already running in {} were not told about `ys workspace done`. \
                 Agents started from now are.",
                workspace.name
            );
        }
        not_running_note(ys);
    })
}

/// An agent session by its conversation id or the start of it, in `workspace` when one is
/// named.
fn find_session(
    ys: &Yardsort,
    wanted: &str,
    workspace: Option<&str>,
) -> Result<yardsort_core::store::SessionRow, Failure> {
    let workspaces = match workspace {
        Some(wanted) => vec![super::workspace::find_workspace(ys, wanted)?],
        None => ys.store.workspaces()?,
    };
    let mut found = Vec::new();
    for workspace in &workspaces {
        for session in ys.store.sessions(&workspace.id)? {
            if session.id == wanted || session.id.starts_with(wanted.trim()) {
                found.push(session);
            }
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(Failure::new(format!(
            "No agent session {wanted:?}. `ys session list` shows them."
        ))),
        n => Err(Failure::new(format!(
            "{n} sessions start with {wanted:?}. Give more of the id."
        ))),
    }
}

fn switch(ys: &Yardsort, wanted: &str, enabled: bool, out: &Output) -> Result<(), Failure> {
    let found = find(ys, wanted)?;
    let row = trigger::set_enabled(
        &ys.store,
        &found.id,
        enabled,
        clock::now().timestamp_millis(),
    )?;
    out.emit(&listed_one(ys, &row)?, || {
        if enabled {
            sayln!("`{}` is on, from now.", row.name);
            not_running_note(ys);
        } else {
            sayln!(
                "`{}` is off. `ys trigger resume` switches it back on.",
                row.name
            );
        }
    })
}

fn reset(ys: &Yardsort, wanted: &str, out: &Output) -> Result<(), Failure> {
    let found = find(ys, wanted)?;
    let lifted = fire::reset(&ys.store, &found.id)?;
    out.emit(&listed_one(ys, &found)?, || {
        sayln!(
            "`{}` starts runs again: the {lifted} it started no longer count against its cap.",
            found.name
        );
    })
}

fn fires(ys: &Yardsort, wanted: &str, limit: usize, out: &Output) -> Result<(), Failure> {
    let found = find(ys, wanted)?;
    let history = ys.store.trigger_fires(&found.id, limit)?;
    let lines: Vec<Line> = history.iter().map(Line::from).collect();
    out.emit(&lines, || {
        if history.is_empty() {
            sayln!("`{}` has not fired yet.", found.name);
            return;
        }
        let mut rows = vec![vec![
            "WHEN".to_owned(),
            "ON".to_owned(),
            "WHAT".to_owned(),
            "RUN".to_owned(),
            "NOTE".to_owned(),
        ]];
        rows.extend(history.iter().map(|fire| {
            vec![
                clock::show(fire.decided_at.unwrap_or(fire.first_at)),
                if fire.events_count > 1 {
                    format!("{} ×{}", fire.event, fire.events_count)
                } else {
                    fire.event.clone()
                },
                outcome_words(fire),
                fire.run_id
                    .as_deref()
                    .map_or_else(|| "-".to_owned(), |id| memory::short_id(id).to_owned()),
                fire.note.clone().unwrap_or_default(),
            ]
        }));
        table(&rows);
    })
}

fn remove(ys: &Yardsort, wanted: &str, out: &Output) -> Result<(), Failure> {
    let found = find(ys, wanted)?;
    ys.store.remove_trigger(&found.id)?;
    #[derive(Serialize)]
    struct Removed<'a> {
        trigger: &'a str,
    }
    out.emit(&Removed { trigger: &found.id }, || {
        sayln!(
            "Removed `{}` and its history. The runs it started are still there.",
            found.name
        );
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_times_are_read_in_seconds_minutes_and_hours() {
        let ms = |text: &str| quiet_ms(text).ok();
        assert_eq!(ms("30s"), Some(30_000));
        assert_eq!(ms("2m"), Some(120_000));
        assert_eq!(ms("1h"), Some(3_600_000));
        assert_eq!(ms("45"), Some(45_000));
        for bad in ["", "m", "2d", "-5s", "1.5m"] {
            assert_eq!(ms(bad), None, "{bad}");
        }
    }
}
