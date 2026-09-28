//! Carrying runs out: the engine's plans applied to the store, its effects to real sessions.
//!
//! [`tick`] moves every queued and running run on as far as it can go now, then returns. The
//! app calls it from a background thread of its own — every second, and at once when a session
//! goes quiet or ends — never from the UI's. Nothing here waits for an agent: a step that waits
//! is a row marked `waiting` until a later tick finds what it waits for.
//!
//! What only the app can do comes in through [`Hands`]: starting an agent exactly as the window
//! does, showing a notification, and writing the handoff packet, which needs git and the login
//! environment. Typing into a session needs only the terminal host, so it is done here.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use pty_host::{HostEvent, SessionId, SessionInfo, SessionState, TerminalHost};

use super::engine::{self, Effect, Finish, Inputs, Move, Outputs, SessionFacts, World};
use super::runs::Place;
use crate::error::IpcResult;
use crate::launch::{HarnessRequest, RUN_LABEL};
use crate::store::{now_ms, StepStatus, Store, WorkflowRunRow};

/// A quiet after at least this much output counts as an agent having settled. The same bar the
/// window uses to tell finished work from the echo of a keystroke, or of a paste.
pub const SETTLE_BUSY_MS: u32 = 8_000;

/// How long to let a pasted message land before pressing Enter, as the first-prompt delivery
/// does.
const SUBMIT_DELAY: Duration = Duration::from_millis(150);

/// What the driver cannot do by itself.
pub trait Hands {
    /// Start a harness in a workspace, as the app would from its composer.
    fn start(&self, workspace_id: &str, request: HarnessRequest) -> Result<SessionInfo, String>;
    /// Tell the person who started the run.
    fn notify(&self, title: &str, body: Option<&str>) -> Result<(), String>;
    /// The workspace's handoff packet, as `ys workspace handoff` prints it.
    fn handoff(&self, workspace_id: &str) -> Option<String>;
}

/// When each session last went quiet after real work, from the host's events. Kept in memory:
/// after a restart, a session that settled while nobody watched is known by the turn it
/// reported, when its harness reports turns.
#[derive(Debug)]
pub struct Settled {
    threshold_ms: u32,
    at: Mutex<HashMap<String, i64>>,
}

impl Default for Settled {
    fn default() -> Self {
        Self::new(SETTLE_BUSY_MS)
    }
}

impl Settled {
    pub fn new(threshold_ms: u32) -> Self {
        Self {
            threshold_ms,
            at: Mutex::new(HashMap::new()),
        }
    }

    /// Take in a host event. True when it could move a run on — a quiet or an exit — so the
    /// driver should look now rather than at its next tick.
    pub fn observe(&self, event: &HostEvent) -> bool {
        match event {
            HostEvent::Quiet { id, busy_ms } => {
                if *busy_ms >= self.threshold_ms {
                    self.at
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(id.0.clone(), now_ms());
                }
                true
            }
            HostEvent::Exited { .. } => true,
            HostEvent::Busy { .. } => false,
        }
    }

    fn get(&self, pty_id: &str) -> Option<i64> {
        self.at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(pty_id)
            .copied()
    }
}

/// Move every active run on as far as it goes now. `recovered` remembers the runs whose
/// interrupted steps were dealt with since the driver started; keep it between calls.
pub fn tick(
    store: &Store,
    host: &dyn TerminalHost,
    hands: &dyn Hands,
    settled: &Settled,
    recovered: &mut HashSet<String>,
) -> IpcResult<()> {
    for run in store.active_workflow_runs()? {
        if let Err(error) = move_on(store, host, hands, settled, &run, recovered) {
            eprintln!("workflows: run {}: {}", run.id, error.message);
        }
    }
    Ok(())
}

fn move_on(
    store: &Store,
    host: &dyn TerminalHost,
    hands: &dyn Hands,
    settled: &Settled,
    run: &WorkflowRunRow,
    recovered: &mut HashSet<String>,
) -> IpcResult<()> {
    if run.status == "queued" && !store.start_workflow_run(&run.id)? {
        return Ok(()); // Cancelled between the read and now.
    }
    let Ok(workflow) = super::parse(&run.definition) else {
        store.finish_workflow_run(
            &run.id,
            "failed",
            Some("The workflow file no longer checks out."),
        )?;
        return Ok(());
    };
    let place = match run.workspace_id.as_deref() {
        Some(id) => Place::load(store, id)?,
        None => None,
    };
    let Some(place) = place else {
        store.finish_workflow_run(&run.id, "failed", Some("The workspace is gone."))?;
        return Ok(());
    };
    if recovered.insert(run.id.clone()) {
        for change in engine::recover(&store.workflow_steps(&run.id)?) {
            apply(store, &run.id, &change)?;
        }
    }
    let inputs: Inputs = serde_json::from_str(&run.inputs).unwrap_or_default();

    // Sessions typed to during this tick. The host marks a session busy only once output comes
    // back, so a second message moments after the first would find it looking quiet still.
    let mut typed_to: HashSet<String> = HashSet::new();

    // Each pass may finish effects that let later steps start. A step moves at most a few times,
    // so this bound is never what stops a healthy run.
    for _ in 0..=workflow.steps.len() * 3 {
        let steps = store.workflow_steps(&run.id)?;
        let world = Seen {
            store,
            host,
            hands,
            settled,
            place: &place,
            now: now_ms(),
            memory: OnceCell::new(),
            handoff: OnceCell::new(),
        };
        let plan = engine::advance(&workflow, &inputs, &steps, &world);
        // A step is claimed for its effect only just before the effect is carried out, not with
        // the rest of the plan: the store takes the claim only while the run is still running,
        // so a cancel that lands while an earlier effect is under way stops the ones after it.
        let acting: HashSet<&str> = plan.effects.iter().map(step_of).collect();
        let mut claims: HashMap<String, Move> = HashMap::new();
        for change in &plan.moves {
            if change.to == StepStatus::Running && acting.contains(change.step.as_str()) {
                claims.insert(change.step.clone(), change.clone());
            } else {
                apply(store, &run.id, change)?;
            }
        }
        let mut delivered = false;
        for effect in plan.effects {
            let step = step_of(&effect).to_owned();
            let Some(claim) = claims.remove(&step) else {
                continue;
            };
            if !apply(store, &run.id, &claim)? {
                if !still_running(store, &run.id)? {
                    return Ok(()); // Cancelled: nothing more of this run happens.
                }
                continue; // The step moved on without us.
            }
            let change = match carry_out(host, hands, &place, effect, &mut typed_to) {
                Outcome::Done(outputs) => engine::done(&step, &outputs),
                Outcome::Failed(why) => engine::failed(&step, &why),
                Outcome::Skipped(why) => engine::skipped(&step, &why),
                Outcome::NotYet => engine::not_yet(&step),
            };
            delivered |= change.to != StepStatus::Waiting;
            apply(store, &run.id, &change)?;
        }
        if let Some(finish) = plan.finish {
            let (status, error) = match finish {
                Finish::Succeeded => ("succeeded", None),
                Finish::Failed(why) => ("failed", Some(why)),
            };
            store.finish_workflow_run(&run.id, status, error.as_deref())?;
            return Ok(());
        }
        // Only an effect that happened can let another step start. One put back to wait would
        // come straight back from the engine, and be put back again.
        if !delivered {
            return Ok(());
        }
    }
    Ok(())
}

fn still_running(store: &Store, run_id: &str) -> IpcResult<bool> {
    Ok(store
        .workflow_run(run_id)?
        .is_some_and(|run| run.status == "running"))
}

/// How carrying out one effect went.
enum Outcome {
    Done(Outputs),
    Failed(String),
    /// It no longer made sense: the session it was for had ended.
    Skipped(String),
    /// Not now: the session was busy when the moment came. The step waits again.
    NotYet,
}

fn apply(store: &Store, run_id: &str, change: &Move) -> IpcResult<bool> {
    Ok(store.move_workflow_step(
        run_id,
        &change.step,
        &change.from,
        change.to,
        change.outputs.as_deref(),
        change.note.as_deref(),
    )?)
}

fn step_of(effect: &Effect) -> &str {
    match effect {
        Effect::Start { step, .. } | Effect::Paste { step, .. } | Effect::Notify { step, .. } => {
            step
        }
    }
}

/// Do what a step asks, and say what it left or why it could not.
fn carry_out(
    host: &dyn TerminalHost,
    hands: &dyn Hands,
    place: &Place,
    effect: Effect,
    typed_to: &mut HashSet<String>,
) -> Outcome {
    let done = |result: Result<Outputs, String>| match result {
        Ok(outputs) => Outcome::Done(outputs),
        Err(why) => Outcome::Failed(why),
    };
    match effect {
        Effect::Start {
            harness,
            prompt,
            model,
            effort,
            skip_memory,
            ..
        } => {
            let request = HarnessRequest {
                id: harness,
                model,
                effort,
                prompt,
                handoff: false,
                skip_memory,
            };
            done(hands.start(&place.workspace.id, request).map(|session| {
                let mut outputs = Outputs::new();
                outputs.insert("session".into(), session.id.0.clone());
                if let Some(run) = session.labels.get(RUN_LABEL) {
                    outputs.insert("run".into(), run.clone());
                }
                outputs
            }))
        }
        Effect::Paste { pty_id, text, .. } => {
            // The engine saw the session quiet when it planned this, but the plan is carried
            // out step by step: an earlier message, or the user typing, may have set the agent
            // working since. Look again now, and never type into a busy agent.
            let id = SessionId(pty_id.clone());
            match host.info(&id) {
                Ok(info) if !matches!(info.state, SessionState::Running) => {
                    return Outcome::Skipped("the session ended before it was quiet".into());
                }
                Err(_) => return Outcome::Skipped("the session ended before it was quiet".into()),
                Ok(info) if info.busy || typed_to.contains(&pty_id) => return Outcome::NotYet,
                Ok(_) => {}
            }
            typed_to.insert(pty_id);
            done(type_into(host, &id, &text))
        }
        Effect::Notify { title, body, .. } => done(
            hands
                .notify(&title, body.as_deref())
                .map(|()| Outputs::new()),
        ),
    }
}

/// Type `text` into a session and submit it, as the first-prompt delivery does.
fn type_into(host: &dyn TerminalHost, id: &SessionId, text: &str) -> Result<Outputs, String> {
    host.paste(id, text)
        .map_err(|error| format!("Could not type into the session: {error}"))?;
    std::thread::sleep(SUBMIT_DELAY);
    host.write(id, b"\r")
        .map_err(|error| format!("Could not submit the message: {error}"))?;
    Ok(Outputs::new())
}

/// The world as the engine sees it from a driver.
struct Seen<'a> {
    store: &'a Store,
    host: &'a dyn TerminalHost,
    hands: &'a dyn Hands,
    settled: &'a Settled,
    place: &'a Place,
    now: i64,
    memory: OnceCell<Option<String>>,
    handoff: OnceCell<Option<String>>,
}

impl World for Seen<'_> {
    fn now(&self) -> i64 {
        self.now
    }

    fn session(&self, pty_id: &str) -> SessionFacts {
        let Ok(info) = self.host.info(&SessionId(pty_id.to_owned())) else {
            return SessionFacts::default();
        };
        SessionFacts {
            alive: matches!(info.state, SessionState::Running),
            busy: info.busy,
            settled_at: self.settled.get(pty_id).max(self.turn_completed_at(pty_id)),
        }
    }

    fn var(&self, path: &[String]) -> Option<String> {
        match path {
            [whole] if whole == "memory" => self
                .memory
                .get_or_init(|| {
                    crate::memory::prompt_section(self.store, &self.place.project.id)
                        .ok()
                        .flatten()
                })
                .clone(),
            [whole] if whole == "handoff" => self
                .handoff
                .get_or_init(|| self.hands.handoff(&self.place.workspace.id))
                .clone(),
            _ => self.place.var(path),
        }
    }
}

impl Seen<'_> {
    /// When the agent in this session last reported a finished turn, for harnesses that report
    /// turns: surer than any quiet.
    fn turn_completed_at(&self, pty_id: &str) -> Option<i64> {
        let run = self.store.run_by_pty(pty_id).ok().flatten()?;
        self.store
            .events_of_kinds(&run.workspace_id, &["turn.completed"])
            .ok()?
            .into_iter()
            .filter(|event| event.run_id.as_deref() == Some(run.id.as_str()))
            .map(|event| event.received_at)
            .max()
    }
}

#[cfg(test)]
mod cancel_tests;
#[cfg(all(test, unix))]
mod tests;
