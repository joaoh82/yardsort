//! Moving a run on: given its steps as the store has them and what is true in the world now,
//! what should change and what should be done.
//!
//! [`advance`] is a function, not a thread, and returns at once. It never waits: a step waiting
//! for an agent to settle stays `waiting` in the store until a later call finds it settled. It
//! never acts either. It returns [`Move`]s for the store and [`Effect`]s for whoever drives it —
//! the app — to carry out and report back. That keeps the whole state machine testable with a
//! fake world and no terminal at all.

use std::collections::BTreeMap;

use super::template;
use super::{Action, SessionRef, Until, Workflow};
use crate::store::{StepStatus, WorkflowStepRow};

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
    /// Tell the person who started the run.
    Notify {
        step: String,
        title: String,
        body: Option<String>,
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

    // Steps are visited in file order until a pass changes nothing, which is at most one pass
    // per step: a step can only be unblocked by one before it in the graph.
    loop {
        let before = plan.moves.len();
        for step in &workflow.steps {
            let id = step.id.as_str();
            let Some(&current) = status.get(id) else {
                continue;
            };
            let lookup = |path: &[String]| value(path, inputs, &outputs, world);
            let session_of = |reference: &SessionRef| match reference {
                SessionRef::Step(from) => outputs
                    .get(from.as_str())
                    .and_then(|o| o.get("session"))
                    .cloned(),
                // The workspace's own agent is found by the driver before a run starts, in a
                // later version; until then a run that needs it is refused at the door.
                SessionRef::Origin => None,
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
                        Some(begin(step, &lookup, &session_of, world, &mut plan.effects))
                    } else {
                        None
                    }
                }
                StepStatus::Waiting => {
                    let since = started.get(id).copied().unwrap_or(now);
                    wait(
                        step,
                        since,
                        now,
                        &lookup,
                        &session_of,
                        world,
                        &mut plan.effects,
                    )
                }
                _ => None,
            };
            if let Some(change) = moved {
                status.insert(id, change.to);
                if let Some(left) = &change.outputs {
                    outputs.insert(id, parse_outputs(left));
                }
                plan.moves.push(change);
            }
        }
        if plan.moves.len() == before {
            break;
        }
    }

    if status.values().all(|s| s.is_final()) {
        let failed = workflow
            .steps
            .iter()
            .find(|step| status.get(step.id.as_str()) == Some(&StepStatus::Failed));
        plan.finish = Some(match failed {
            None => Finish::Succeeded,
            Some(step) => {
                let note = plan
                    .moves
                    .iter()
                    .rev()
                    .find(|m| m.step == step.id)
                    .and_then(|m| m.note.clone())
                    .or_else(|| {
                        rows.iter()
                            .find(|r| r.step_id == step.id)
                            .and_then(|r| r.note.clone())
                    });
                Finish::Failed(match note {
                    Some(note) => format!("Step `{}` failed: {note}", step.id),
                    None => format!("Step `{}` failed.", step.id),
                })
            }
        });
    }
    plan
}

/// Steps the driver was in the middle of when it stopped. Whether the session started or the
/// text was typed cannot be known now, so they fail, saying why, rather than happen twice.
pub fn recover(rows: &[WorkflowStepRow]) -> Vec<Move> {
    rows.iter()
        .filter(|row| row.status == StepStatus::Running.as_str())
        .map(|row| Move {
            step: row.step_id.clone(),
            from: vec![StepStatus::Running],
            to: StepStatus::Failed,
            outputs: None,
            note: Some("Yardsort stopped while this step was running.".to_owned()),
        })
        .collect()
}

/// The driver carried out a step's effect.
pub fn done(step: &str, outputs: &Outputs) -> Move {
    Move {
        step: step.to_owned(),
        from: vec![StepStatus::Running],
        to: StepStatus::Succeeded,
        outputs: Some(serde_json::to_string(outputs).unwrap_or_else(|_| "{}".to_owned())),
        note: None,
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
fn begin(
    step: &super::Step,
    lookup: &dyn Fn(&[String]) -> Option<String>,
    session_of: &dyn Fn(&SessionRef) -> Option<String>,
    world: &dyn World,
    effects: &mut Vec<Effect>,
) -> Move {
    let id = step.id.as_str();
    let render = |text: &str| template::render(text, lookup);
    match &step.action {
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
            effects.push(Effect::Start {
                step: id.to_owned(),
                harness: render(harness),
                prompt: prompt
                    .as_deref()
                    .map(render)
                    .filter(|p| !p.trim().is_empty()),
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
                        text: render(prompt),
                    });
                    to(id, StepStatus::Pending, StepStatus::Running)
                }
            }
        },
        Action::WaitPrActivity { .. } => fail(
            id,
            StepStatus::Pending,
            "Waiting for pull request activity is not in this version of Yardsort.",
        ),
    }
}

/// A waiting step: has what it waits for happened, or has it waited too long?
fn wait(
    step: &super::Step,
    since: i64,
    now: i64,
    lookup: &dyn Fn(&[String]) -> Option<String>,
    session_of: &dyn Fn(&SessionRef) -> Option<String>,
    world: &dyn World,
    effects: &mut Vec<Effect>,
) -> Option<Move> {
    let id = step.id.as_str();
    let timed_out = |limit: &Option<u64>| {
        limit.is_some_and(|secs| now - since >= i64::try_from(secs).unwrap_or(i64::MAX) * 1000)
    };
    match &step.action {
        Action::WaitSession {
            session,
            until,
            timeout_secs,
        } => {
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
                    text: template::render(prompt, lookup),
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
        _ => None,
    }
}

fn to(step: &str, from: StepStatus, to: StepStatus) -> Move {
    Move {
        step: step.to_owned(),
        from: vec![from],
        to,
        outputs: None,
        note: None,
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
fn duration(secs: u64) -> String {
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
