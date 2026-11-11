//! Moving a run on: given its steps as the store has them and what is true in the world now,
//! what should change and what should be done.
//!
//! [`advance`] is a function, not a thread, and returns at once. It never waits: a step waiting
//! for an agent to settle stays `waiting` in the store until a later call finds it settled. It
//! never acts either. It returns [`Move`]s for the store and [`Effect`]s for whoever drives it —
//! the app — to carry out and report back. That keeps the whole state machine testable with a
//! fake world and no terminal at all.

use std::collections::BTreeMap;

use super::artifact;
use super::clarify::{self, ClarificationState};
use super::template;
use super::{
    ancestors, steps_from, Action, OutcomeName, PrActivity, SessionRef, Until, Workflow,
    MAX_ATTEMPTS,
};
use crate::store::{StepStatus, WorkflowStepRow};

/// What a step concluded, apart from whether its row succeeded or failed.
///
/// A step that judges something produces one: a smoke test's verdict, or an approval.
/// A branch reads this, not free text. With no routes, [`react`] is the one place the
/// run takes that verdict. It is stored on the step row, not in `outputs`. `outputs` stay the strings
/// a later step reads as `{{ steps.<id>.<field> }}`, and this is not `wait_session`'s
/// `outcome` field.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct StepOutcome {
    pub verdict: StepVerdict,
    /// Why, when the verdict is failed. Null when it passed. A row saved before this
    /// field omits it, and that reads the same as null.
    #[serde(default)]
    pub reason: Option<String>,
    /// Set when a person decided an approval. Absent on a smoke-test verdict, and on a row
    /// saved before approvals existed.
    #[serde(default)]
    pub decision: Option<ApprovalDecision>,
    /// What they wrote with the decision. Null when they wrote nothing.
    #[serde(default)]
    pub feedback: Option<String>,
    /// Who decided: `app` from the window, or the name of the account `ys` ran as.
    #[serde(default)]
    pub actor: Option<String>,
    /// When they decided, in milliseconds since the epoch.
    #[serde(default)]
    // A JavaScript number holds a timestamp. `i64` is not exported.
    #[specta(type = Option<f64>)]
    pub decided_at: Option<i64>,
}

impl StepOutcome {
    /// A verdict that is not an approval. The approval fields stay empty.
    fn judged(verdict: StepVerdict, reason: Option<String>) -> Self {
        Self {
            verdict,
            reason,
            decision: None,
            feedback: None,
            actor: None,
            decided_at: None,
        }
    }
}

/// What a person decided on an approval step.
///
/// `changes_requested` is its own decision, not a rejection. With no `on` routes, both a
/// rejection and a request for changes stop the run. A route can send either somewhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Rejected,
    ChangesRequested,
}

impl ApprovalDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::ChangesRequested => "changes_requested",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "approved" => Some(Self::Approved),
            "rejected" => Some(Self::Rejected),
            "changes_requested" => Some(Self::ChangesRequested),
            _ => None,
        }
    }

    /// The note a step keeps, before any feedback.
    fn word(self) -> &'static str {
        match self {
            Self::Approved => "Approved",
            Self::Rejected => "Rejected",
            Self::ChangesRequested => "Changes requested",
        }
    }
}

/// A decision the store already accepted, as the engine reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedApproval {
    pub decision: ApprovalDecision,
    pub feedback: Option<String>,
    pub actor: String,
    pub decided_at: i64,
}

/// Passed or failed. The words match a smoke test's verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum StepVerdict {
    Passed,
    Failed,
}

/// What a session is doing, as the driver sees it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionFacts {
    /// The process is still there.
    pub alive: bool,
    /// It is producing output right now.
    pub busy: bool,
    /// The last time it was judged settled: gone quiet after working, or finished a turn it
    /// reported. `None` when it never has.
    pub settled_at: Option<i64>,
}

/// The world as a run sees it. The app answers from its terminals and its database; tests
/// answer from a table.
pub trait World {
    /// Milliseconds since the epoch.
    fn now(&self) -> i64;
    /// A session by its terminal id. Unknown sessions are not alive.
    fn session(&self, pty_id: &str) -> SessionFacts;
    /// The value of a variable outside `inputs` and `steps`: `workspace.branch`, `memory`, ….
    fn var(&self, path: &[String]) -> Option<String>;
    /// The terminal id of the workspace's own agent, found when the run started.
    fn origin(&self) -> Option<String>;
    /// Reviews or comments of `kind` on the workspace's pull request since the run started, as
    /// the forge last said. `None` while nothing is known yet.
    fn pr_activity(&self, kind: PrActivity) -> Option<PrSeen>;
    /// A smoke test run by id: its status and the summary `ys test wait` prints. `None` when the
    /// row is gone.
    fn test_run(&self, run_id: &str) -> Option<TestRun>;
    /// A decision a person has recorded for this step, if they have. The default is none, so a
    /// world that has no approvals does not have to say so.
    fn approval(&self, _step_id: &str) -> Option<RecordedApproval> {
        None
    }
    /// Rounds a clarifying step has asked, and whether a summary was recorded. The default
    /// is none. A clarifying step does not finish because the session went quiet.
    fn clarification(&self, _step_id: &str) -> ClarificationState {
        ClarificationState::default()
    }
}

/// A smoke test run as a `run_test` step sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRun {
    /// `queued`, `awaiting_approval`, `running`, `passed`, `failed`, `cancelled`, `errored`, ….
    pub status: String,
    /// The verdict and the failing steps, the text `ys test wait` prints.
    pub summary: String,
}

impl TestRun {
    /// Still going: queued, waiting for the user to approve it, or running.
    fn active(&self) -> bool {
        matches!(
            self.status.as_str(),
            "queued" | "awaiting_approval" | "running"
        )
    }
}

/// Activity found on a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrSeen {
    /// How many reviews and comments, of the kind asked for.
    pub count: u32,
    /// Where to read the newest.
    pub latest_url: Option<String>,
}

/// A change to one step, to be applied only if the step is still in one of `from`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub step: String,
    pub from: Vec<StepStatus>,
    pub to: StepStatus,
    /// Replaces the step's outputs, as a JSON object of strings.
    pub outputs: Option<String>,
    pub note: Option<String>,
    /// Replaces the step's structured outcome, as JSON. Absent leaves whatever is stored,
    /// unless `clear_outcome`, which sets it back to nothing.
    pub outcome: Option<String>,
    /// Set the stored outcome back to nothing. A step that starts again does this, so a
    /// later revision can re-run it and record a new verdict.
    pub clear_outcome: bool,
    /// This step is a gate whose verdict failed. The run skips every step that has not
    /// finished, in this same plan, and ends failed.
    pub halt: bool,
    /// An approval timeout. The move applies only when nobody has recorded a decision.
    pub keep_decision: bool,
    /// A passed outcome routed to `stop`. Skip steps that have not finished and end the
    /// run succeeded. A failed outcome routed to `stop` uses `halt` instead.
    pub stop: bool,
    /// Set these steps back to pending. Applied once, and only while this step's attempt
    /// count is still `seen`.
    pub rewind: Option<Rewind>,
    /// A note this step leaves for a later step, on a route that does not loop. A loop
    /// carries its note on [`Rewind::feedback`] instead, in the same change as the rewind.
    pub feedback: Option<FeedbackCopy>,
    /// An answer to type into the session that asked, then leave the step waiting.
    /// The driver sets `applied_at` and types it. The step does not finish.
    pub apply_answer: Option<AnswerDelivery>,
}

/// An answer the driver types into the same session, once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerDelivery {
    pub round: u32,
    /// The terminal the session is running in.
    pub pty_id: String,
    /// The questions and the person's reply.
    pub text: String,
}

/// A loop back to an earlier step. The driver applies it in one change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewind {
    /// The target and every step that needs it, including the step that branched.
    pub steps: Vec<String>,
    /// The branching step's attempt count when this was planned. A second apply, or one
    /// after a restart already counted a new entry, does not match and does nothing.
    pub seen: u32,
    /// A named artifact for each step this rewind runs again. Two overlapping rewinds
    /// in one tick share one change, and both notes are here.
    pub feedback: Vec<FeedbackCopy>,
}

/// One attempt's feedback, stored under a reserved artifact name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackCopy {
    pub name: String,
    pub body: String,
    /// `decision` for an approval, `verdict` for a smoke test.
    pub source: String,
}

/// Something only the driver can do. Each belongs to a step the plan has just moved to
/// `running`; the driver reports how it went with [`done`] or [`failed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Start a harness in the run's workspace.
    Start {
        step: String,
        harness: String,
        prompt: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        skip_memory: bool,
    },
    /// Type `text` into a live, quiet session and submit it.
    Paste {
        step: String,
        pty_id: String,
        text: String,
    },
    /// Queue a smoke test run in the run's workspace. The driver reports the queued run's id,
    /// and the step then waits for that run to end.
    RunTest {
        step: String,
        test: String,
        inputs: Inputs,
    },
    /// Tell the person who started the run.
    Notify {
        step: String,
        title: String,
        body: Option<String>,
    },
    /// The step succeeded in the engine, and it leaves named artifacts. The driver reads them
    /// and then marks the step succeeded with [`recorded`]. Until then the step stays running,
    /// so a later step does not start on an empty handoff.
    Capture {
        step: String,
        outputs: Outputs,
        note: Option<String>,
        outcome: Option<String>,
    },
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finish {
    Succeeded,
    /// With what to say about it: which step failed and why.
    Failed(String),
}

/// What one call to [`advance`] decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub moves: Vec<Move>,
    pub effects: Vec<Effect>,
    /// Set when every step has ended.
    pub finish: Option<Finish>,
}

/// The inputs a run was given, by input id.
pub type Inputs = BTreeMap<String, String>;

/// What a step left for later steps, by field.
pub type Outputs = BTreeMap<String, String>;

/// Decide what happens next in a running run.
pub fn advance(
    workflow: &Workflow,
    inputs: &Inputs,
    rows: &[WorkflowStepRow],
    world: &dyn World,
) -> Plan {
    let now = world.now();
    let mut plan = Plan::default();
    // Each step's status as this plan leaves it, so a step can start in the same call as the
    // one it needs finishes.
    let mut status: BTreeMap<&str, StepStatus> = rows
        .iter()
        .map(|row| {
            let parsed = StepStatus::parse(&row.status).unwrap_or(StepStatus::Failed);
            (row.step_id.as_str(), parsed)
        })
        .collect();
    // Outputs as this plan leaves them too: a step can use what one before it left in the same
    // call.
    let mut outputs: BTreeMap<&str, Outputs> = rows
        .iter()
        .map(|row| (row.step_id.as_str(), parse_outputs(&row.outputs)))
        .collect();
    let started: BTreeMap<&str, i64> = rows
        .iter()
        .filter_map(|row| row.started_at.map(|at| (row.step_id.as_str(), at)))
        .collect();
    let mut outcomes: BTreeMap<String, StepOutcome> = rows
        .iter()
        .filter_map(|row| {
            let parsed = serde_json::from_str(row.outcome.as_deref()?).ok()?;
            Some((row.step_id.clone(), parsed))
        })
        .collect();
    // Notes a forward route leaves during this plan. A later step in the same plan reads
    // them. The driver stores the same note when it applies the move.
    let mut handed: BTreeMap<String, String> = BTreeMap::new();

    // Steps are visited in file order until a pass changes nothing, which is at most one pass
    // per step: a step can only be unblocked by one before it in the graph. A rewind does not
    // start later steps in this pass. Judged steps later in the file still conclude, so a halt
    // or a stop in the same tick wins over the rewind whichever step comes first.
    let mut rewound = false;
    loop {
        let before = plan.moves.len();
        for step in &workflow.steps {
            let id = step.id.as_str();
            let Some(&current) = status.get(id) else {
                continue;
            };
            if rewound && current == StepStatus::Pending {
                continue;
            }
            // An answer is typed back while the step stays waiting. Planning it again on the
            // next pass of this same tick would never stop.
            if plan
                .moves
                .iter()
                .any(|planned| planned.step == id && planned.apply_answer.is_some())
            {
                continue;
            }
            let lookup = |path: &[String]| {
                if let [namespace, name] = path {
                    if namespace == "artifacts" {
                        if let Some(body) = handed.get(name) {
                            return Some(body.clone());
                        }
                    }
                }
                value(path, inputs, &outputs, world)
            };
            let session_of = |reference: &SessionRef| match reference {
                SessionRef::Step(from) => outputs
                    .get(from.as_str())
                    .and_then(|o| o.get("session"))
                    .cloned(),
                SessionRef::Origin => world.origin(),
            };
            let moved = match current {
                StepStatus::Pending => {
                    let needs: Vec<StepStatus> = step
                        .needs
                        .iter()
                        .map(|need| {
                            status
                                .get(need.as_str())
                                .copied()
                                .unwrap_or(StepStatus::Failed)
                        })
                        .collect();
                    if let Some(blocked) = step
                        .needs
                        .iter()
                        .zip(&needs)
                        .find(|(_, s)| s.is_final() && **s != StepStatus::Succeeded)
                    {
                        Some(skip(
                            id,
                            StepStatus::Pending,
                            &format!("`{}` did not succeed", blocked.0),
                        ))
                    } else if needs.iter().all(|s| *s == StepStatus::Succeeded) {
                        if let Some(why) = branch_blocked(step, workflow, &outcomes) {
                            Some(skip(id, StepStatus::Pending, &why))
                        } else {
                            Some(begin(
                                step,
                                &lookup,
                                &session_of,
                                world,
                                &mut plan.effects,
                                clarifies(workflow, id),
                            ))
                        }
                    } else {
                        None
                    }
                }
                StepStatus::Waiting => {
                    let stored = outputs.get(id).cloned().unwrap_or_default();
                    if stored.contains_key(ARTIFACT_PENDING) {
                        let row = rows.iter().find(|row| row.step_id == step.id);
                        Some(resume_capture(
                            step,
                            &stored,
                            row.and_then(|row| row.note.clone()),
                            row.and_then(|row| row.outcome.clone()),
                        ))
                    } else {
                        let since = started.get(id).copied().unwrap_or(now);
                        wait(
                            step,
                            workflow,
                            rows,
                            since,
                            now,
                            &lookup,
                            &session_of,
                            world,
                            &mut plan.effects,
                        )
                    }
                }
                _ => None,
            };
            if let Some(change) = moved {
                let change = hold_for_artifacts(step, change, &mut plan.effects);
                if change.rewind.is_none() {
                    if let Some(copy) = &change.feedback {
                        handed.insert(copy.name.clone(), copy.body.clone());
                    }
                }
                if let Some(rewind) = change.rewind.clone() {
                    // Two looping steps in one tick can name overlapping sets. A second
                    // rewind would find its step already pending and drop its note. Fold
                    // it into the rewind already planned, and keep both notes.
                    if let Some(earlier) = plan.moves.iter_mut().find(|planned| {
                        planned.rewind.as_ref().is_some_and(|first| {
                            first.steps.iter().any(|id| rewind.steps.contains(id))
                        })
                    }) {
                        if let Some(first) = &mut earlier.rewind {
                            for id in rewind.steps {
                                if !first.steps.contains(&id) {
                                    first.steps.push(id);
                                }
                            }
                            first.feedback.extend(rewind.feedback);
                        }
                        rewound = true;
                        continue;
                    }
                    plan.moves.push(change);
                    rewound = true;
                    continue;
                }
                if let Some(text) = change.outcome.as_deref() {
                    if let Ok(parsed) = serde_json::from_str::<StepOutcome>(text) {
                        outcomes.insert(step.id.clone(), parsed);
                    }
                } else if change.clear_outcome {
                    outcomes.remove(&step.id);
                }
                status.insert(id, change.to);
                if let Some(left) = &change.outputs {
                    outputs.insert(id, parse_outputs(left));
                }
                plan.moves.push(change);
            }
        }
        if rewound || plan.moves.len() == before {
            break;
        }
    }

    // A failed gate stops the whole run, not only the steps that name it in `needs`.
    // Done before the finish check, so the steps it skips count as finished this tick.
    // A step that already succeeded onto `stop` (after its artifacts were stored, including
    // on a later tick) stops the run the same way.
    halt_the_run(&mut plan, workflow, &mut status, &outcomes);

    if status.values().all(|s| s.is_final()) {
        // Name the gate that halted the run. Another step may already be failed, earlier
        // in the file, and that one is not why the run stopped.
        let failed = plan
            .moves
            .iter()
            .find(|change| change.halt)
            .and_then(|change| workflow.steps.iter().find(|step| step.id == change.step))
            .or_else(|| {
                workflow
                    .steps
                    .iter()
                    .find(|step| status.get(step.id.as_str()) == Some(&StepStatus::Failed))
            });
        plan.finish = Some(match failed {
            None => Finish::Succeeded,
            Some(step) => {
                let failing = plan.moves.iter().rev().find(|m| m.step == step.id);
                let halt = failing.is_some_and(|m| m.halt);
                let note = failing.and_then(|m| m.note.clone()).or_else(|| {
                    rows.iter()
                        .find(|r| r.step_id == step.id)
                        .and_then(|r| r.note.clone())
                });
                Finish::Failed(match note {
                    Some(note) if halt => {
                        format!("Step `{}` failed: {}", step.id, gate_failure_line(&note))
                    }
                    Some(note) => format!("Step `{}` failed: {note}", step.id),
                    None => format!("Step `{}` failed.", step.id),
                })
            }
        });
    }
    plan
}

/// A step that has already succeeded, whose stored outcome routes to `stop`.
///
/// Saving artifacts finishes the step on a later tick, after the move that chose `stop`
/// was turned into "still running". The stored outcome is what says the branch ends.
fn finished_route_stop(
    workflow: &Workflow,
    status: &BTreeMap<&str, StepStatus>,
    outcomes: &BTreeMap<String, StepOutcome>,
) -> Option<String> {
    workflow.steps.iter().find_map(|step| {
        if status.get(step.id.as_str()) != Some(&StepStatus::Succeeded) {
            return None;
        }
        let outcome = outcomes.get(&step.id)?;
        if outcome.verdict == StepVerdict::Failed {
            return None;
        }
        let when = judged_name(step, outcome)?;
        let route = step.routes.iter().find(|route| route.when == when)?;
        route.stops().then(|| step.id.clone())
    })
}

/// Skip every step that has not finished, because a gate failed.
///
/// An effect this plan was about to carry out is dropped, so it never starts. A step that
/// is already `running` from an earlier tick is skipped too: the run does not wait for it.
/// The driver then cancels a smoke test a skipped `run_test` step was waiting on. Anything
/// else that effect already started is left running: stopping it here could throw away
/// work, and the run ending is what the gate asked for.
fn halt_the_run(
    plan: &mut Plan,
    workflow: &Workflow,
    status: &mut BTreeMap<&str, StepStatus>,
    outcomes: &BTreeMap<String, StepOutcome>,
) {
    let gate = plan
        .moves
        .iter()
        .find(|change| change.halt)
        .map(|change| change.step.clone());
    let ended = plan
        .moves
        .iter()
        .find(|change| change.stop)
        .map(|change| change.step.clone())
        .or_else(|| finished_route_stop(workflow, status, outcomes));
    let Some(cause) = gate.clone().or(ended) else {
        return;
    };
    let failed_gate = gate.is_some();
    if failed_gate {
        plan.effects.clear();
    }
    let why = if failed_gate {
        format!("the gate `{cause}` failed")
    } else {
        format!("`{cause}` ended the branch")
    };
    for step in &workflow.steps {
        let id = step.id.as_str();
        if id == cause {
            continue;
        }
        let Some(&current) = status.get(id) else {
            continue;
        };
        if current.is_final() {
            continue;
        }
        // The key is already in the map, from the row. Assign through it so this does not
        // have to store a borrow of the workflow.
        if let Some(existing) = plan.moves.iter_mut().rfind(|change| change.step == id) {
            // The row is still `existing.from`. Skip from there, instead of adding a second
            // move the store would refuse because the first has not been applied.
            existing.to = StepStatus::Skipped;
            existing.note = Some(format!("Skipped: {why}."));
            existing.outputs = None;
            existing.outcome = None;
            existing.clear_outcome = true;
            existing.halt = false;
            // A rewind or a `stop` on this same move must not still be applied. The run is
            // ending, and sending steps back to pending would leave them unfinished.
            existing.rewind = None;
            existing.stop = false;
            existing.feedback = None;
            existing.apply_answer = None;
        } else {
            plan.moves.push(skip(id, current, &why));
        }
        if let Some(slot) = status.get_mut(id) {
            *slot = StepStatus::Skipped;
        }
    }
    if !failed_gate {
        // A passed `stop` keeps the effect of the step that ended the branch. Effects for
        // steps this pass is skipping are dropped, so they never start.
        plan.effects
            .retain(|effect| status.get(effect_step(effect)) != Some(&StepStatus::Skipped));
    }
}

fn effect_step(effect: &Effect) -> &str {
    match effect {
        Effect::Start { step, .. }
        | Effect::Paste { step, .. }
        | Effect::RunTest { step, .. }
        | Effect::Notify { step, .. }
        | Effect::Capture { step, .. } => step,
    }
}

/// The short clause a failed gate leaves on the run. The note starts with "The test failed:",
/// and the useful line is the first one after that. A trailing colon is dropped, and the
/// first character is lowercased only when the next one is already lowercase, so "HTTP 500"
/// stays as it is. The line ends with a period. The step's note keeps the full report.
fn gate_failure_line(note: &str) -> String {
    let lines: Vec<&str> = note
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let chosen = match lines.as_slice() {
        [head, detail, ..]
            if head
                .trim_end_matches(':')
                .trim()
                .eq_ignore_ascii_case("The test failed") =>
        {
            *detail
        }
        [head, ..] => head.trim_end_matches(':').trim(),
        [] => "failed",
    };
    let chosen = if chosen.is_empty() { "failed" } else { chosen };
    let mut chars = chosen.chars();
    let Some(first) = chars.next() else {
        return "failed.".to_owned();
    };
    let rest: String = chars.collect();
    let mut out = String::new();
    if rest.chars().next().is_some_and(char::is_lowercase) {
        out.extend(first.to_lowercase());
    } else {
        out.push(first);
    }
    out.push_str(&rest);
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

/// On a running step whose work already finished and is only waiting for its artifacts to be
/// stored. A restart puts that step back to waiting, so the capture runs again.
const ARTIFACT_PENDING: &str = "artifact_pending";

/// Steps the driver was in the middle of when it stopped. Whether the session started or the
/// text was typed cannot be known now, so they fail, saying why, rather than happen twice.
/// A step that already finished and was only waiting for its artifacts to be stored goes back
/// to waiting: that capture is safe to do again, and failing it would skip the steps after it.
pub fn recover(rows: &[WorkflowStepRow]) -> Vec<Move> {
    rows.iter()
        .filter(|row| row.status == StepStatus::Running.as_str())
        .map(|row| {
            let outputs = parse_outputs(&row.outputs);
            if outputs.contains_key(ARTIFACT_PENDING) {
                // Keep the marker. The next advance resumes the capture from it, including when
                // the step's own action would not succeed a second time (a notification already
                // sent, a transcript read that had not finished).
                return Move {
                    step: row.step_id.clone(),
                    from: vec![StepStatus::Running],
                    to: StepStatus::Waiting,
                    outputs: None,
                    note: row.note.clone(),
                    outcome: row.outcome.clone(),
                    clear_outcome: false,
                    halt: false,
                    keep_decision: false,
                    stop: false,
                    rewind: None,
                    feedback: None,
                    apply_answer: None,
                };
            }
            Move {
                step: row.step_id.clone(),
                from: vec![StepStatus::Running],
                to: StepStatus::Failed,
                outputs: None,
                note: Some("Yardsort stopped while this step was running.".to_owned()),
                outcome: None,
                clear_outcome: false,
                halt: false,
                keep_decision: false,
                stop: false,
                rewind: None,
                feedback: None,
                apply_answer: None,
            }
        })
        .collect()
}

/// How the run takes a step's [`StepOutcome`].
///
/// A gate whose verdict is [`StepVerdict::Failed`] fails the step and sets [`Move::halt`],
/// so the run skips every step still going and ends failed. The step's note keeps `reason`.
/// The run's own error is a short line, so the report is not shown twice. Any other verdict
/// succeeds the step, so a later step can still read what happened. `success_note` is what
/// that succeeded step says. Actions that judge something call this instead of deciding a
/// stop of their own.
pub fn react(
    step_id: &str,
    from: StepStatus,
    gate: bool,
    outcome: &StepOutcome,
    outputs: &Outputs,
    success_note: String,
) -> Move {
    let stop = gate && outcome.verdict == StepVerdict::Failed;
    let note = if stop {
        outcome
            .reason
            .clone()
            .filter(|reason| !reason.trim().is_empty())
            .unwrap_or_else(|| "Failed.".to_owned())
    } else {
        success_note
    };
    Move {
        step: step_id.to_owned(),
        from: vec![from],
        to: if stop {
            StepStatus::Failed
        } else {
            StepStatus::Succeeded
        },
        outputs: Some(serde_json::to_string(outputs).unwrap_or_else(|_| "{}".to_owned())),
        note: Some(note),
        outcome: Some(serde_json::to_string(outcome).unwrap_or_else(|_| "{}".to_owned())),
        clear_outcome: false,
        halt: stop,
        keep_decision: false,
        stop: false,
        rewind: None,
        feedback: None,
        apply_answer: None,
    }
}

/// The words a failed test verdict leaves as its reason: the report `ys test wait` prints,
/// under a line that says the test failed.
pub fn test_failure_reason(summary: &str) -> String {
    let summary = summary.trim();
    if summary.is_empty() {
        "The test failed.".to_owned()
    } else {
        format!("The test failed:\n{summary}")
    }
}

/// The driver stored the artifacts a step leaves, so the step can finish.
pub fn recorded(
    step: &str,
    outputs: &Outputs,
    note: Option<String>,
    outcome: Option<String>,
) -> Move {
    Move {
        step: step.to_owned(),
        from: vec![StepStatus::Running],
        to: StepStatus::Succeeded,
        outputs: Some(serde_json::to_string(outputs).unwrap_or_else(|_| "{}".to_owned())),
        note,
        outcome,
        clear_outcome: false,
        halt: false,
        keep_decision: false,
        stop: false,
        rewind: None,
        feedback: None,
        apply_answer: None,
    }
}

/// The driver carried out a step's effect.
pub fn done(step: &str, outputs: &Outputs) -> Move {
    Move {
        step: step.to_owned(),
        from: vec![StepStatus::Running],
        to: StepStatus::Succeeded,
        outputs: Some(serde_json::to_string(outputs).unwrap_or_else(|_| "{}".to_owned())),
        note: None,
        outcome: None,
        clear_outcome: false,
        halt: false,
        keep_decision: false,
        stop: false,
        rewind: None,
        feedback: None,
        apply_answer: None,
    }
}

/// The driver queued a test run for a `run_test` step: it keeps its run id and waits for it to
/// end, rather than finishing here.
pub fn queued(step: &str, outputs: &Outputs) -> Move {
    Move {
        step: step.to_owned(),
        from: vec![StepStatus::Running],
        to: StepStatus::Waiting,
        outputs: Some(serde_json::to_string(outputs).unwrap_or_else(|_| "{}".to_owned())),
        note: None,
        outcome: None,
        clear_outcome: false,
        halt: false,
        keep_decision: false,
        stop: false,
        rewind: None,
        feedback: None,
        apply_answer: None,
    }
}

/// The driver found the step's effect no longer made sense, and why.
pub fn skipped(step: &str, why: &str) -> Move {
    Move {
        step: step.to_owned(),
        from: vec![StepStatus::Running],
        to: StepStatus::Skipped,
        outputs: None,
        note: Some(format!("Skipped: {why}.")),
        outcome: None,
        clear_outcome: false,
        halt: false,
        keep_decision: false,
        stop: false,
        rewind: None,
        feedback: None,
        apply_answer: None,
    }
}

/// The artifact read has not finished. The step waits, with the marker set, so the next tick
/// resumes the same capture instead of running the action again.
pub fn still_reading(
    step: &str,
    outputs: &Outputs,
    note: Option<String>,
    outcome: Option<String>,
) -> Move {
    let mut marked = outputs.clone();
    marked.insert(ARTIFACT_PENDING.to_owned(), "1".to_owned());
    Move {
        step: step.to_owned(),
        from: vec![StepStatus::Running],
        to: StepStatus::Waiting,
        outputs: Some(serde_json::to_string(&marked).unwrap_or_else(|_| "{}".to_owned())),
        note,
        outcome,
        clear_outcome: false,
        halt: false,
        keep_decision: false,
        stop: false,
        rewind: None,
        feedback: None,
        apply_answer: None,
    }
}

/// The driver found the moment wrong — a session busy again — and put the step back to wait.
/// It keeps the time it first waited from, so its timeout still counts from then.
pub fn not_yet(step: &str) -> Move {
    Move {
        step: step.to_owned(),
        from: vec![StepStatus::Running],
        to: StepStatus::Waiting,
        outputs: None,
        note: None,
        outcome: None,
        clear_outcome: false,
        halt: false,
        keep_decision: false,
        stop: false,
        rewind: None,
        feedback: None,
        apply_answer: None,
    }
}

/// The driver could not carry out a step's effect.
pub fn failed(step: &str, why: &str) -> Move {
    Move {
        step: step.to_owned(),
        from: vec![StepStatus::Running],
        to: StepStatus::Failed,
        outputs: None,
        note: Some(why.to_owned()),
        outcome: None,
        clear_outcome: false,
        halt: false,
        keep_decision: false,
        stop: false,
        rewind: None,
        feedback: None,
        apply_answer: None,
    }
}

pub fn parse_outputs(json: &str) -> Outputs {
    serde_json::from_str(json).unwrap_or_default()
}

fn value(
    path: &[String],
    inputs: &Inputs,
    outputs: &BTreeMap<&str, Outputs>,
    world: &dyn World,
) -> Option<String> {
    match path {
        [namespace, id] if namespace == "inputs" => inputs.get(id).cloned(),
        [namespace, id, field] if namespace == "steps" => {
            outputs.get(id.as_str()).and_then(|o| o.get(field)).cloned()
        }
        _ => world.var(path),
    }
}

/// A pending step whose needs have all succeeded.
/// A later step asks questions in the session this step starts.
fn clarifies(workflow: &Workflow, id: &str) -> bool {
    workflow.steps.iter().any(|other| {
        matches!(
            &other.action,
            Action::WaitSession {
                session: SessionRef::Step(source),
                clarify: true,
                ..
            } if source == id
        )
    })
}

fn begin(
    step: &super::Step,
    lookup: &dyn Fn(&[String]) -> Option<String>,
    session_of: &dyn Fn(&SessionRef) -> Option<String>,
    world: &dyn World,
    effects: &mut Vec<Effect>,
    clarify: bool,
) -> Move {
    let id = step.id.as_str();
    let render = |text: &str| template::render(text, lookup);
    let mut change = match &step.action {
        Action::StartSession {
            harness,
            prompt,
            model,
            effort,
            skip_memory,
        } => {
            // The launcher gives an agent the project's memory after its first message. A
            // prompt that places `{{ memory }}` itself has done that already.
            let places_memory = prompt.as_deref().is_some_and(|text| {
                template::placeholders(text)
                    .iter()
                    .any(|var| var.path == ["memory"])
            });
            let prompt = prompt
                .as_deref()
                .map(|text| handoff_prompt(text, &step.consumes, lookup))
                .or_else(|| {
                    let only = handoff_prompt("", &step.consumes, lookup);
                    (!only.trim().is_empty()).then_some(only)
                })
                .filter(|p| !p.trim().is_empty());
            let prompt = if clarify {
                clarify::with_note(prompt)
            } else {
                prompt
            };
            effects.push(Effect::Start {
                step: id.to_owned(),
                harness: render(harness),
                prompt,
                model: model
                    .as_deref()
                    .map(render)
                    .filter(|m| !m.trim().is_empty()),
                effort: effort
                    .as_deref()
                    .map(render)
                    .filter(|e| !e.trim().is_empty()),
                skip_memory: *skip_memory || places_memory,
            });
            to(id, StepStatus::Pending, StepStatus::Running)
        }
        Action::Notify { title, body } => {
            effects.push(Effect::Notify {
                step: id.to_owned(),
                title: render(title),
                body: body.as_deref().map(render).filter(|b| !b.trim().is_empty()),
            });
            to(id, StepStatus::Pending, StepStatus::Running)
        }
        Action::WaitSession { session, .. } => match session_of(session) {
            Some(_) => to(id, StepStatus::Pending, StepStatus::Waiting),
            None => fail(id, StepStatus::Pending, "There is no session to wait for."),
        },
        Action::SendToSession {
            session, prompt, ..
        } => match session_of(session) {
            None if *session == SessionRef::Origin => skip(
                id,
                StepStatus::Pending,
                "no agent was running in the workspace when the run started",
            ),
            None => skip(id, StepStatus::Pending, "there is no session to send to"),
            Some(pty) => {
                let facts = world.session(&pty);
                if !facts.alive {
                    skip(id, StepStatus::Pending, "the session had already ended")
                } else if facts.busy {
                    // Never interrupt a working agent: wait for it to be quiet.
                    to(id, StepStatus::Pending, StepStatus::Waiting)
                } else {
                    effects.push(Effect::Paste {
                        step: id.to_owned(),
                        pty_id: pty,
                        text: handoff_prompt(prompt, &step.consumes, lookup),
                    });
                    to(id, StepStatus::Pending, StepStatus::Running)
                }
            }
        },
        Action::WaitPrActivity { .. } => to(id, StepStatus::Pending, StepStatus::Waiting),
        Action::RunTest { test, inputs, .. } => {
            effects.push(Effect::RunTest {
                step: id.to_owned(),
                test: render(test),
                inputs: inputs
                    .iter()
                    .map(|(name, value)| (name.clone(), render(value)))
                    .collect(),
            });
            to(id, StepStatus::Pending, StepStatus::Running)
        }
        Action::Approve { prompt, .. } => {
            let rendered = prompt
                .as_deref()
                .map(render)
                .filter(|text| !text.trim().is_empty());
            let mut change = to(id, StepStatus::Pending, StepStatus::Waiting);
            change.note = Some(match &rendered {
                Some(text) => format!("Waiting for your approval.\n{text}"),
                None => "Waiting for your approval.".to_owned(),
            });
            if let Some(text) = rendered {
                let mut left = Outputs::new();
                left.insert("prompt".to_owned(), text);
                change.outputs = Some(serde_json::to_string(&left).unwrap_or_else(|_| "{}".into()));
            }
            change
        }
    };
    // Starting again is a new attempt. Drop the previous verdict so a later revision can
    // re-run the step and record a new one. A skip or a failure from pending is not a new
    // attempt, and keeps whatever was stored.
    if !change.to.is_final() {
        change.clear_outcome = true;
    }
    change
}

/// A clarifying wait. It finishes on a recorded summary, or when `until` is exited and the
/// session has ended. A quiet session keeps waiting until the timeout, then the step fails.
/// A claim that was not confirmed typed fails the step and is not typed again.
#[allow(clippy::too_many_arguments)]
fn clarify_wait(
    id: &str,
    session: &super::SessionRef,
    until: super::Until,
    timeout_secs: &Option<u32>,
    max_rounds: u32,
    since: i64,
    now: i64,
    session_of: &dyn Fn(&super::SessionRef) -> Option<String>,
    world: &dyn World,
) -> Option<Move> {
    let state = world.clarification(id);
    let limit = max_rounds.clamp(1, clarify::MAX_ROUNDS);
    let timed_out = timeout_secs.is_some_and(|secs| now - since >= i64::from(secs) * 1000);
    if state.rounds > limit {
        return Some(fail(
            id,
            StepStatus::Waiting,
            &format!(
                "Clarification was asked {} times, which is past the limit ({limit}).",
                state.rounds
            ),
        ));
    }
    if state.undelivered {
        return Some(fail(
            id,
            StepStatus::Waiting,
            "The answer may not have reached the session.",
        ));
    }
    if let Some(pending) = &state.pending {
        let pty = session_of(session);
        let facts = pty
            .as_deref()
            .map(|pty| world.session(pty))
            .unwrap_or_default();
        if !facts.alive {
            return Some(fail(
                id,
                StepStatus::Waiting,
                "The session ended before the answer could be sent back.",
            ));
        }
        if facts.busy {
            return None;
        }
        let Some(pty_id) = pty else {
            return Some(fail(
                id,
                StepStatus::Waiting,
                "There is no session to send the answer to.",
            ));
        };
        return Some(Move {
            apply_answer: Some(AnswerDelivery {
                round: pending.round,
                pty_id,
                text: pending.text.clone(),
            }),
            ..to(id, StepStatus::Waiting, StepStatus::Waiting)
        });
    }
    if state.waiting {
        return if timed_out {
            Some(fail(
                id,
                StepStatus::Waiting,
                &format!(
                    "Timed out waiting for an answer after {}.",
                    duration(timeout_secs.unwrap_or(0))
                ),
            ))
        } else {
            None
        };
    }
    if state.summary {
        return Some(Move {
            outputs: Some("{\"outcome\":\"summary\"}".to_owned()),
            ..to(id, StepStatus::Waiting, StepStatus::Succeeded)
        });
    }
    let facts = session_of(session)
        .map(|pty| world.session(&pty))
        .unwrap_or_default();
    if until == super::Until::Exited && !facts.alive {
        return Some(Move {
            outputs: Some("{\"outcome\":\"exited\"}".to_owned()),
            ..to(id, StepStatus::Waiting, StepStatus::Succeeded)
        });
    }
    if timed_out {
        return Some(fail(
            id,
            StepStatus::Waiting,
            &format!(
                "Timed out waiting for a summary after {}.",
                duration(timeout_secs.unwrap_or(0))
            ),
        ));
    }
    None
}

/// A waiting step: has what it waits for happened, or has it waited too long?
#[allow(clippy::too_many_arguments)]
fn wait(
    step: &super::Step,
    workflow: &Workflow,
    rows: &[WorkflowStepRow],
    since: i64,
    now: i64,
    lookup: &dyn Fn(&[String]) -> Option<String>,
    session_of: &dyn Fn(&SessionRef) -> Option<String>,
    world: &dyn World,
    effects: &mut Vec<Effect>,
) -> Option<Move> {
    let id = step.id.as_str();
    let timed_out =
        |limit: &Option<u32>| limit.is_some_and(|secs| now - since >= i64::from(secs) * 1000);
    match &step.action {
        Action::WaitSession {
            session,
            until,
            timeout_secs,
            clarify,
            max_rounds,
        } => {
            if *clarify {
                return clarify_wait(
                    id,
                    session,
                    *until,
                    timeout_secs,
                    *max_rounds,
                    since,
                    now,
                    session_of,
                    world,
                );
            }
            let facts = session_of(session)
                .map(|pty| world.session(&pty))
                .unwrap_or_default();
            let outcome = if !facts.alive {
                Some("exited")
            } else if *until == Until::Settled && facts.settled_at.is_some_and(|at| at >= since) {
                Some("settled")
            } else {
                None
            };
            match outcome {
                Some(outcome) => Some(Move {
                    outputs: Some(format!("{{\"outcome\":\"{outcome}\"}}")),
                    ..to(id, StepStatus::Waiting, StepStatus::Succeeded)
                }),
                None if timed_out(timeout_secs) => Some(fail(
                    id,
                    StepStatus::Waiting,
                    &format!("Timed out after {}.", duration(timeout_secs.unwrap_or(0))),
                )),
                None => None,
            }
        }
        Action::WaitPrActivity { kind, timeout_secs } => match world.pr_activity(*kind) {
            Some(seen) if seen.count > 0 => {
                let mut left = Outputs::new();
                left.insert("count".into(), seen.count.to_string());
                if let Some(url) = seen.latest_url {
                    left.insert("latest_url".into(), url);
                }
                Some(Move {
                    outputs: Some(serde_json::to_string(&left).unwrap_or_else(|_| "{}".into())),
                    ..to(id, StepStatus::Waiting, StepStatus::Succeeded)
                })
            }
            _ if timed_out(timeout_secs) => Some(fail(
                id,
                StepStatus::Waiting,
                &format!(
                    "No {} appeared on the pull request within {}.",
                    match kind {
                        PrActivity::Review => "review",
                        PrActivity::Comment => "comment",
                        PrActivity::Any => "review or comment",
                    },
                    duration(timeout_secs.unwrap_or(0))
                ),
            )),
            _ => None,
        },
        Action::RunTest { timeout_secs, .. } => {
            // The run's id was left when the driver queued it; wait for that run to end.
            let run = lookup(&["steps".to_owned(), id.to_owned(), "run".to_owned()])?;
            match world.test_run(&run) {
                None => Some(fail(id, StepStatus::Waiting, "The test run is gone.")),
                Some(test) if test.active() => {
                    if timed_out(timeout_secs) {
                        Some(fail(
                            id,
                            StepStatus::Waiting,
                            &format!(
                                "The test run did not end within {}.",
                                duration(timeout_secs.unwrap_or(0))
                            ),
                        ))
                    } else {
                        None
                    }
                }
                Some(test) => {
                    let mut left = Outputs::new();
                    left.insert("run".into(), run);
                    left.insert("verdict".into(), test.status.clone());
                    left.insert("summary".into(), test.summary.clone());
                    // A verdict, passed or failed, is a step outcome. A gate whose verdict
                    // failed stops the whole run; any other verdict succeeds the step, so a
                    // later step can read `verdict` and act on either. That is how "Smoke
                    // test, then tell the agent" reports a failure for the agent to repair.
                    // A run that reached no verdict (cancelled, errored) fails the step and
                    // leaves no outcome, because there was nothing to react to. It does not
                    // halt the other branches: only a failed verdict on a gate does.
                    match test.status.as_str() {
                        "passed" | "failed" => {
                            let outcome = match test.status.as_str() {
                                "passed" => StepOutcome::judged(StepVerdict::Passed, None),
                                _ => StepOutcome::judged(
                                    StepVerdict::Failed,
                                    Some(test_failure_reason(&test.summary)),
                                ),
                            };
                            Some(route_judged(
                                step,
                                workflow,
                                rows,
                                StepStatus::Waiting,
                                &outcome,
                                &left,
                                format!("The test {}.", test.status),
                            ))
                        }
                        other => Some(Move {
                            outputs: Some(
                                serde_json::to_string(&left).unwrap_or_else(|_| "{}".into()),
                            ),
                            note: Some(format!("The test run {other}.")),
                            ..to(id, StepStatus::Waiting, StepStatus::Failed)
                        }),
                    }
                }
            }
        }
        Action::SendToSession {
            session,
            prompt,
            timeout_secs,
        } => {
            let pty = session_of(session)?;
            let facts = world.session(&pty);
            if !facts.alive {
                Some(skip(
                    id,
                    StepStatus::Waiting,
                    "the session ended before it was quiet",
                ))
            } else if !facts.busy {
                effects.push(Effect::Paste {
                    step: id.to_owned(),
                    pty_id: pty,
                    text: handoff_prompt(prompt, &step.consumes, lookup),
                });
                Some(to(id, StepStatus::Waiting, StepStatus::Running))
            } else if timed_out(timeout_secs) {
                Some(fail(
                    id,
                    StepStatus::Waiting,
                    &format!(
                        "The session was still busy after {}.",
                        duration(timeout_secs.unwrap_or(0))
                    ),
                ))
            } else {
                None
            }
        }
        Action::Approve { timeout_secs, .. } => {
            if let Some(recorded) = world.approval(id) {
                Some(approval_move(step, workflow, rows, recorded))
            } else if timed_out(timeout_secs) {
                // A decision recorded after this plan was built, but before the move is
                // applied, must win. The store refuses this move when a decision exists.
                let mut change = fail(
                    id,
                    StepStatus::Waiting,
                    &format!("Timed out after {}.", duration(timeout_secs.unwrap_or(0))),
                );
                change.keep_decision = true;
                Some(change)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Apply a recorded approval. With no routes, approve succeeds the step and the other
/// decisions fail it and halt the run. Routes send the outcome on. A timeout is not a
/// decision and does not come through here.
fn approval_move(
    step: &super::Step,
    workflow: &Workflow,
    rows: &[WorkflowStepRow],
    recorded: RecordedApproval,
) -> Move {
    let feedback = recorded.feedback.filter(|text| !text.trim().is_empty());
    let note = match &feedback {
        Some(text) => format!("{}: {text}", recorded.decision.word()),
        None => format!("{}.", recorded.decision.word()),
    };
    let verdict = if recorded.decision == ApprovalDecision::Approved {
        StepVerdict::Passed
    } else {
        StepVerdict::Failed
    };
    let mut left = Outputs::new();
    left.insert("decision".to_owned(), recorded.decision.as_str().to_owned());
    left.insert("feedback".to_owned(), feedback.clone().unwrap_or_default());
    let outcome = StepOutcome {
        verdict,
        reason: (verdict == StepVerdict::Failed).then(|| note.clone()),
        decision: Some(recorded.decision),
        feedback,
        actor: Some(recorded.actor),
        decided_at: Some(recorded.decided_at),
    };
    route_judged(
        step,
        workflow,
        rows,
        StepStatus::Waiting,
        &outcome,
        &left,
        note,
    )
}

/// The work already finished. Store the artifacts again from what the step left, rather than
/// judging the action a second time.
fn resume_capture(
    step: &super::Step,
    stored: &Outputs,
    note: Option<String>,
    outcome: Option<String>,
) -> Move {
    let mut plain = stored.clone();
    plain.remove(ARTIFACT_PENDING);
    Move {
        step: step.id.clone(),
        from: vec![StepStatus::Waiting],
        to: StepStatus::Succeeded,
        outputs: Some(serde_json::to_string(&plain).unwrap_or_else(|_| "{}".into())),
        note,
        outcome,
        clear_outcome: false,
        halt: false,
        keep_decision: false,
        stop: false,
        rewind: None,
        feedback: None,
        apply_answer: None,
    }
}

/// A producing step that just succeeded waits until its artifacts are stored. The success
/// itself is kept on the effect, and the step stays running so nothing that needs it starts
/// in this same plan.
fn hold_for_artifacts(step: &super::Step, change: Move, effects: &mut Vec<Effect>) -> Move {
    if change.to != StepStatus::Succeeded || step.produces.is_empty() {
        return change;
    }
    let outputs = change
        .outputs
        .as_deref()
        .map(parse_outputs)
        .unwrap_or_default();
    effects.push(Effect::Capture {
        step: step.id.clone(),
        outputs: outputs.clone(),
        note: change.note.clone(),
        outcome: change.outcome.clone(),
    });
    // The marker is how a restart tells this hold apart from a step whose effect was still
    // going. The capture above keeps the outputs without it, and those are what get stored.
    let mut marked = outputs;
    marked.insert(ARTIFACT_PENDING.to_owned(), "1".to_owned());
    let clear_outcome = change.outcome.is_none();
    Move {
        to: StepStatus::Running,
        outputs: Some(serde_json::to_string(&marked).unwrap_or_else(|_| "{}".to_owned())),
        clear_outcome,
        halt: false,
        keep_decision: false,
        stop: false,
        rewind: None,
        ..change
    }
}

/// Render `template`, then append each consumed artifact the template did not place itself.
/// A start or a send uses this, so the next agent does not start cold. A notification does
/// not: its title would otherwise grow by the whole document.
fn handoff_prompt(
    template: &str,
    consumes: &[String],
    lookup: &dyn Fn(&[String]) -> Option<String>,
) -> String {
    let mut out = if template.is_empty() {
        String::new()
    } else {
        template::render(template, lookup)
    };
    for name in consumes {
        let placed = template::placeholders(template)
            .iter()
            .any(|var| var.path.len() == 2 && var.path[0] == "artifacts" && var.path[1] == *name);
        if placed {
            continue;
        }
        let Some(body) = lookup(&["artifacts".to_owned(), name.clone()]) else {
            continue;
        };
        if !out.is_empty() {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
        out.push_str("## Artifact: ");
        out.push_str(name);
        out.push_str("\n\n");
        out.push_str(&body);
    }
    out
}

/// Where a judged outcome goes. No routes keeps today's rule: a failed gate halts the run.
fn route_judged(
    step: &super::Step,
    workflow: &Workflow,
    rows: &[WorkflowStepRow],
    from: StepStatus,
    outcome: &StepOutcome,
    outputs: &Outputs,
    note: String,
) -> Move {
    if step.routes.is_empty() {
        return react(&step.id, from, step.is_gate(), outcome, outputs, note);
    }
    let Some(when) = judged_name(step, outcome) else {
        return react(&step.id, from, true, outcome, outputs, note);
    };
    let Some(route) = step.routes.iter().find(|route| route.when == when) else {
        return react(
            &step.id,
            from,
            true,
            outcome,
            outputs,
            format!("`{}` has no route.", when.as_str()),
        );
    };
    if route.stops() {
        if outcome.verdict == StepVerdict::Failed {
            return react(&step.id, from, true, outcome, outputs, note);
        }
        let mut change = react(&step.id, from, false, outcome, outputs, note);
        change.stop = true;
        return change;
    }
    let back = route.to == step.id
        || ancestors(&workflow.steps)
            .get(&step.id)
            .is_some_and(|set| set.contains(&route.to));
    if !back {
        // A later step was chosen. Succeed this one even when the verdict failed, so the
        // chosen step's needs are met. The outcome stays, and that is what the branch reads.
        // A failed verdict or a request for changes leaves its note for that step.
        let mut change = react(&step.id, from, false, outcome, outputs, note);
        change.feedback = feedback_copy(&step.id, when, outcome);
        return change;
    }
    let seen = rows
        .iter()
        .find(|row| row.step_id == step.id)
        .map(|row| row.attempts)
        .unwrap_or(0);
    let limit = step.max_attempts.unwrap_or(MAX_ATTEMPTS).min(MAX_ATTEMPTS);
    if seen >= limit {
        let reason = format!(
            "{} {seen} times, which is the limit ({limit}). Start a new run to try again.",
            capitalize(when.limit_phrase())
        );
        let mut failed = outcome.clone();
        failed.verdict = StepVerdict::Failed;
        failed.reason = Some(reason.clone());
        return react(&step.id, from, true, &failed, outputs, reason);
    }
    Move {
        step: step.id.clone(),
        from: vec![from],
        to: StepStatus::Pending,
        outputs: None,
        note: Some(note),
        outcome: None,
        clear_outcome: true,
        halt: false,
        keep_decision: false,
        stop: false,
        rewind: Some(Rewind {
            steps: steps_from(&workflow.steps, &route.to),
            seen,
            feedback: feedback_copy(&step.id, when, outcome).into_iter().collect(),
        }),
        feedback: None,
        apply_answer: None,
    }
}

fn judged_name(step: &super::Step, outcome: &StepOutcome) -> Option<OutcomeName> {
    match &step.action {
        Action::Approve { .. } => match outcome.decision? {
            ApprovalDecision::Approved => Some(OutcomeName::Approved),
            ApprovalDecision::Rejected => Some(OutcomeName::Rejected),
            ApprovalDecision::ChangesRequested => Some(OutcomeName::ChangesRequested),
        },
        Action::RunTest { .. } => match outcome.verdict {
            StepVerdict::Passed => Some(OutcomeName::Passed),
            StepVerdict::Failed => Some(OutcomeName::Failed),
        },
        _ => None,
    }
}

fn feedback_copy(step_id: &str, when: OutcomeName, outcome: &StepOutcome) -> Option<FeedbackCopy> {
    match when {
        OutcomeName::ChangesRequested => Some(FeedbackCopy {
            name: artifact::changes_artifact_name(step_id),
            body: outcome
                .feedback
                .clone()
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| "Changes were requested.".to_owned()),
            source: "decision".to_owned(),
        }),
        OutcomeName::Failed => Some(FeedbackCopy {
            name: artifact::failure_artifact_name(step_id),
            body: outcome
                .reason
                .clone()
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| "The test failed.".to_owned()),
            source: "verdict".to_owned(),
        }),
        _ => None,
    }
}

/// A pending step whose need took a branch that does not include this step.
fn branch_blocked(
    step: &super::Step,
    workflow: &Workflow,
    outcomes: &BTreeMap<String, StepOutcome>,
) -> Option<String> {
    let before = ancestors(&workflow.steps);
    for need_id in &step.needs {
        let Some(need) = workflow
            .steps
            .iter()
            .find(|candidate| candidate.id == *need_id)
        else {
            continue;
        };
        if need.routes.is_empty() {
            continue;
        }
        let Some(outcome) = outcomes.get(&need.id) else {
            continue;
        };
        let Some(when) = judged_name(need, outcome) else {
            continue;
        };
        let Some(route) = need.routes.iter().find(|route| route.when == when) else {
            continue;
        };
        if route.stops() {
            return Some(format!("`{}` ended the branch", need.id));
        }
        let chosen = &route.to;
        let allowed =
            chosen == &step.id || before.get(&step.id).is_some_and(|set| set.contains(chosen));
        if !allowed {
            return Some(format!("`{}` went to `{chosen}`", need.id));
        }
    }
    None
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn to(step: &str, from: StepStatus, to: StepStatus) -> Move {
    Move {
        step: step.to_owned(),
        from: vec![from],
        to,
        outputs: None,
        note: None,
        outcome: None,
        clear_outcome: false,
        halt: false,
        keep_decision: false,
        stop: false,
        rewind: None,
        feedback: None,
        apply_answer: None,
    }
}

fn fail(step: &str, from: StepStatus, why: &str) -> Move {
    Move {
        note: Some(why.to_owned()),
        ..to(step, from, StepStatus::Failed)
    }
}

fn skip(step: &str, from: StepStatus, why: &str) -> Move {
    Move {
        note: Some(format!("Skipped: {why}.")),
        ..to(step, from, StepStatus::Skipped)
    }
}

/// `2h`, `45m`, `90s`: the way a file writes it.
fn duration(secs: u32) -> String {
    if secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests;
