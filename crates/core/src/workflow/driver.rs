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
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use pty_host::{HostError, HostEvent, SessionId, SessionInfo, SessionState, TerminalHost};

use super::engine::{
    self, Effect, Finish, Inputs, Move, Outputs, Plan, PrSeen, SessionFacts, TestRun, World,
};
use super::runs::{self, Facts, Place};
use super::{artifact, ArtifactSource, PrActivity, SessionRef};
use crate::error::IpcResult;
use crate::forge::{PrPost, PrPostKind};
use crate::git::Git;
use crate::launch::{HarnessRequest, HARNESS_LABEL, RECORD_LABEL, RUN_LABEL, WORKSPACE_LABEL};
use crate::store::{
    now_ms, NewWorkflowArtifact, StepStatus, Store, WorkflowRunRow, WorkflowStepRow,
};

/// A quiet after at least this much output counts as an agent having settled. The same bar the
/// window uses to tell finished work from the echo of a keystroke, or of a paste.
pub const SETTLE_BUSY_MS: u32 = 8_000;

/// How often a run waiting on its pull request asks the forge. A review takes minutes to write;
/// asking every second would only spend the user's API allowance.
pub const FORGE_EVERY: Duration = Duration::from_secs(30);

/// What the driver cannot do by itself.
pub trait Hands {
    /// Start a harness in a workspace, as the app would from its composer.
    fn start(&self, workspace_id: &str, request: HarnessRequest) -> Result<SessionInfo, String>;
    /// Tell the person who started the run.
    fn notify(&self, title: &str, body: Option<&str>) -> Result<(), String>;
    /// The workspace's handoff packet, as `ys workspace handoff` prints it.
    fn handoff(&self, workspace_id: &str) -> Option<String>;
    /// A job that asks the forge for the reviews and comments on the run's pull request `pr`:
    /// the workspace's own by its number, or one it tracks elsewhere by its link
    /// ([`super::runs::PrFacts::selector`]). Called on the driver's thread, so it must return at
    /// once: the job it returns is what does the asking, on a thread of its own, and must give
    /// up by itself — the app's stops `gh` after [`FORGE_LIMIT`].
    fn pr_posts(&self, workspace_id: &str, pr: &super::runs::PrFacts) -> PrAsk;

    /// One session's conversation, for a step that asked for a transcript. `raw` asks for each
    /// turn's own text. Without it, the budgeted digest. `Ok(None)` when that session has no
    /// conversation to copy. The default is none, so a driver test that never asks is unchanged.
    fn conversation(
        &self,
        _workspace_id: &str,
        _pty_id: &str,
        _raw: bool,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }
}

/// Asking the forge, as a job for a thread of its own.
pub type PrAsk = Box<dyn FnOnce() -> Result<Vec<PrPost>, String> + Send + 'static>;

/// How long one question to the forge may take before it is given up on.
pub const FORGE_LIMIT: Duration = Duration::from_secs(60);

/// What the forge last said about one run's pull request.
#[derive(Debug, Default)]
struct Asked {
    /// When it was last asked.
    at: i64,
    /// A question is out and not answered yet.
    waiting: bool,
    /// The last answer; `None` until there is one.
    posts: Option<Vec<PrPost>>,
}

/// What a driver keeps between ticks. The app holds one for as long as it runs.
#[derive(Debug)]
pub struct Driver {
    settled: Settled,
    /// Runs whose interrupted steps were dealt with since this driver started.
    recovered: Mutex<HashSet<String>>,
    /// What the forge last said about each run's pull request. Shared with the threads that
    /// ask it, which put their answers here.
    forge: std::sync::Arc<Mutex<HashMap<String, Asked>>>,
    forge_every_ms: i64,
    /// Something about a run changed during the current tick.
    touched: std::sync::atomic::AtomicBool,
    /// The runs that were active at the last tick. One that is gone since was ended from
    /// outside — `ys workflow cancel` — which is a change too.
    active: Mutex<HashSet<String>>,
}

impl Default for Driver {
    fn default() -> Self {
        Self::new(SETTLE_BUSY_MS, FORGE_EVERY)
    }
}

impl Driver {
    /// A driver that counts a quiet after `settle_busy_ms` of output as settled, and asks the
    /// forge at most every `forge_every`. Tests shorten both.
    pub fn new(settle_busy_ms: u32, forge_every: Duration) -> Self {
        Self {
            settled: Settled::new(settle_busy_ms),
            recovered: Mutex::new(HashSet::new()),
            forge: std::sync::Arc::new(Mutex::new(HashMap::new())),
            forge_every_ms: i64::try_from(forge_every.as_millis()).unwrap_or(i64::MAX),
            touched: std::sync::atomic::AtomicBool::new(false),
            active: Mutex::new(HashSet::new()),
        }
    }

    /// Take in a host event. True when it could move a run on — a quiet or an exit — so the
    /// driver should look now rather than at its next tick.
    pub fn observe(&self, event: &HostEvent) -> bool {
        self.settled.observe(event)
    }

    /// Move every active run on as far as it goes now. True when anything about a run changed,
    /// so whoever shows runs knows to look again.
    pub fn tick(
        &self,
        store: &Store,
        host: &dyn TerminalHost,
        hands: &dyn Hands,
        git: &Git,
    ) -> IpcResult<bool> {
        self.touched
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let runs = store.active_workflow_runs()?;
        // A run that was active after the last tick and is not now ended without this driver:
        // cancelled from `ys`, say. A new one is a change as well, whether or not it can start.
        let ids: HashSet<String> = runs.iter().map(|run| run.id.clone()).collect();
        if *self.active.lock().unwrap_or_else(PoisonError::into_inner) != ids {
            self.touch();
        }
        for run in &runs {
            if let Err(error) = move_on(self, store, host, hands, git, run) {
                eprintln!("workflows: run {}: {}", run.id, error.message);
            }
        }
        // What the forge said about runs that have ended is no longer wanted.
        self.forge
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|id, _| runs.iter().any(|run| &run.id == id));
        // What is active now, after this tick's own moves, is what the next tick compares to:
        // a run this tick finished is not news then.
        *self.active.lock().unwrap_or_else(PoisonError::into_inner) = store
            .active_workflow_runs()?
            .into_iter()
            .map(|run| run.id)
            .collect();
        Ok(self.touched.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn touch(&self) {
        self.touched
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// What is known of the posts on a run's pull request, without waiting for the forge.
    ///
    /// When the last answer is old and no question is out, `ask` makes one, and it goes to a
    /// thread of its own: a slow or stalled forge must not hold up other runs, their messages
    /// and notifications, or the timeouts that are checked on this thread. Its answer is here
    /// for a later tick. At most one question per run is ever out.
    fn pr_posts(&self, run_id: &str, ask: impl FnOnce() -> PrAsk) -> Option<Vec<PrPost>> {
        let now = now_ms();
        let mut forge = self.forge.lock().unwrap_or_else(PoisonError::into_inner);
        let asked = forge.entry(run_id.to_owned()).or_default();
        let due = asked.posts.is_none() || now - asked.at >= self.forge_every_ms;
        if due && !asked.waiting {
            asked.at = now;
            asked.waiting = true;
            let job = ask();
            let answers = std::sync::Arc::clone(&self.forge);
            let run = run_id.to_owned();
            let spawned = std::thread::Builder::new()
                .name("workflow-forge".into())
                .spawn(move || {
                    let answer = job();
                    let mut forge = answers.lock().unwrap_or_else(PoisonError::into_inner);
                    let Some(asked) = forge.get_mut(&run) else {
                        return; // The run ended meanwhile.
                    };
                    asked.waiting = false;
                    match answer {
                        Ok(posts) => asked.posts = Some(posts),
                        // Asked again at the next interval; the step's timeout gives up.
                        Err(why) => {
                            eprintln!("workflows: run {run}: cannot read the pull request: {why}");
                        }
                    }
                });
            if spawned.is_err() {
                asked.waiting = false;
            }
        }
        asked.posts.clone()
    }
}

/// When each session last went quiet after real work, from the host's events. Kept in memory:
/// after a restart, a session that settled while nobody watched is known by the turn it
/// reported, when its harness reports turns.
#[derive(Debug)]
struct Settled {
    threshold_ms: u32,
    at: Mutex<HashMap<String, i64>>,
}

impl Settled {
    fn new(threshold_ms: u32) -> Self {
        Self {
            threshold_ms,
            at: Mutex::new(HashMap::new()),
        }
    }

    fn observe(&self, event: &HostEvent) -> bool {
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

fn move_on(
    driver: &Driver,
    store: &Store,
    host: &dyn TerminalHost,
    hands: &dyn Hands,
    git: &Git,
    run: &WorkflowRunRow,
) -> IpcResult<()> {
    let Ok(workflow) = super::parse(&run.definition) else {
        driver.touch();
        store.finish_workflow_run(
            &run.id,
            "failed",
            Some("The workflow file no longer checks out."),
        )?;
        return Ok(());
    };
    let place = match run.workspace_id.as_deref() {
        Some(id) => Place::load(store, id, git)?,
        None => None,
    };
    let Some(place) = place else {
        driver.touch();
        store.finish_workflow_run(&run.id, "failed", Some("The workspace is gone."))?;
        return Ok(());
    };
    let mut facts = Facts::parse(&run.context);
    let mut started_at = run.started_at;
    if run.status == "queued" {
        // The workspace's own agent is whichever was newest when the run began: found now,
        // before the run has started any of its own for it to be mistaken for.
        if runs::needs_origin(&workflow) {
            let opened_by = facts
                .pr
                .as_ref()
                .and_then(|pr| pr.elsewhere.as_ref())
                .and_then(|elsewhere| elsewhere.opened_by.clone());
            facts.origin = match (facts.origin_record.as_deref(), opened_by.as_deref()) {
                // Asked for on behalf of one agent: that one, or none when it is not running.
                (Some(record), _) => live_terminal(host, &place.workspace.id, record),
                // About a pull request elsewhere: the agent that opened it while it runs, else
                // the workspace's, which may well be the one working on it now.
                (None, Some(record)) => live_terminal(host, &place.workspace.id, record)
                    .or_else(|| origin_session(store, host, &place.workspace.id)),
                (None, None) => origin_session(store, host, &place.workspace.id),
            };
        }
        if !store.start_workflow_run(&run.id, Some(&facts.to_json()))? {
            return Ok(()); // Cancelled between the read and now.
        }
        driver.touch();
        started_at = Some(now_ms());
    }
    if driver
        .recovered
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(run.id.clone())
    {
        for change in engine::recover(&store.workflow_steps(&run.id)?) {
            apply(driver, store, &run.id, &change)?;
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
            driver,
            store,
            host,
            hands,
            run_id: &run.id,
            started_at: started_at.unwrap_or(run.created_at),
            facts: &facts,
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
                apply(driver, store, &run.id, change)?;
            }
        }
        if plan.moves.iter().any(|change| change.halt) {
            // Best effort. A failure here must not skip finishing the run on this tick.
            cancel_tests_a_halt_skipped(store, &workflow, &steps, &plan);
        }
        let mut delivered = false;
        let finishing = Finishing {
            store,
            hands,
            place: &place,
            facts: &facts,
            inputs: &inputs,
            steps: &steps,
            workflow: &workflow,
            run_id: &run.id,
        };
        for effect in plan.effects {
            let step = step_of(&effect).to_owned();
            let Some(claim) = claims.remove(&step) else {
                continue;
            };
            if !apply(driver, store, &run.id, &claim)? {
                if !still_running(store, &run.id)? {
                    return Ok(()); // Cancelled: nothing more of this run happens.
                }
                continue; // The step moved on without us.
            }
            if let Effect::Capture {
                outputs,
                note,
                outcome,
                ..
            } = &effect
            {
                let change = finishing.finish(&step, outputs, note.clone(), outcome.clone());
                delivered |= change.to != StepStatus::Waiting;
                apply(driver, store, &run.id, &change)?;
                continue;
            }
            let outcome = carry_out(
                store,
                host,
                hands,
                &place,
                effect,
                facts.unattended,
                &mut typed_to,
            );
            let change = match outcome {
                Outcome::Done(outputs) => finishing.finish(&step, &outputs, None, None),
                Outcome::Queued(outputs) => engine::queued(&step, &outputs),
                Outcome::Failed(why) => engine::failed(&step, &why),
                Outcome::Skipped(why) => engine::skipped(&step, &why),
                Outcome::NotYet => engine::not_yet(&step),
            };
            delivered |= change.to != StepStatus::Waiting;
            apply(driver, store, &run.id, &change)?;
        }
        if let Some(finish) = plan.finish {
            let (status, error) = match finish {
                Finish::Succeeded => ("succeeded", None),
                Finish::Failed(why) => ("failed", Some(why)),
            };
            driver.touch();
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

/// A halted plan skipped these steps after they had already queued a smoke test. Cancel
/// that test, so it does not keep running or keep asking for approval. A test that has
/// already ended is left as it is.
fn cancel_tests_a_halt_skipped(
    store: &Store,
    workflow: &super::Workflow,
    steps: &[WorkflowStepRow],
    plan: &Plan,
) {
    for change in &plan.moves {
        if change.to != StepStatus::Skipped {
            continue;
        }
        let Some(step) = workflow.steps.iter().find(|step| step.id == change.step) else {
            continue;
        };
        if step.action.name() != "run_test" {
            continue;
        }
        let Some(row) = steps.iter().find(|row| row.step_id == change.step) else {
            continue;
        };
        let Some(test_run) = engine::parse_outputs(&row.outputs).get("run").cloned() else {
            continue;
        };
        if let Err(error) =
            store.cancel_test_run(&test_run, "The workflow stopped because a gate failed.")
        {
            eprintln!(
                "workflows: could not cancel smoke test {test_run} after a gate failed: {error}"
            );
        }
    }
}

fn still_running(store: &Store, run_id: &str) -> IpcResult<bool> {
    Ok(store
        .workflow_run(run_id)?
        .is_some_and(|run| run.status == "running"))
}

/// Type `text` into a session and submit it, as the first-prompt delivery does. Nobody here
/// typed it, so it takes no control; while someone controls the session from elsewhere it
/// waits rather than type over them.
fn type_into(host: &dyn TerminalHost, id: &SessionId, text: &str) -> Outcome {
    let failed =
        |error: HostError| Outcome::Failed(format!("Could not type into the session: {error}"));
    match host.type_unattended(id, text, true) {
        Ok(()) => {}
        Err(HostError::ControlHeld { .. }) => return Outcome::NotYet,
        Err(error) => return failed(error),
    }
    std::thread::sleep(pty_host::SUBMIT_DELAY);
    // Typed already: a lease taken since leaves it unsent, and it is not typed a second time.
    match host.type_unattended(id, "\r", false) {
        Ok(()) => Outcome::Done(Outputs::new()),
        Err(error) => failed(error),
    }
}

/// How carrying out one effect went.
enum Outcome {
    Done(Outputs),
    /// A test run was queued: the step keeps its run id and waits for the run to end.
    Queued(Outputs),
    Failed(String),
    /// It no longer made sense: the session it was for had ended.
    Skipped(String),
    /// Not now: the session was busy, or controlled from elsewhere, when the moment came. The
    /// step waits again.
    NotYet,
}

fn apply(driver: &Driver, store: &Store, run_id: &str, change: &Move) -> IpcResult<bool> {
    let moved = store.move_workflow_step(
        run_id,
        &change.step,
        &change.from,
        change.to,
        change.outputs.as_deref(),
        change.note.as_deref(),
        change.outcome.as_deref(),
        change.clear_outcome,
    )?;
    if moved {
        driver.touch();
    }
    Ok(moved)
}

fn step_of(effect: &Effect) -> &str {
    match effect {
        Effect::Start { step, .. }
        | Effect::Paste { step, .. }
        | Effect::Notify { step, .. }
        | Effect::RunTest { step, .. }
        | Effect::Capture { step, .. } => step,
    }
}

/// Do what a step asks, and say what it left or why it could not.
#[allow(clippy::too_many_arguments)]
fn carry_out(
    store: &Store,
    host: &dyn TerminalHost,
    hands: &dyn Hands,
    place: &Place,
    effect: Effect,
    unattended: bool,
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
                // Started by a workflow, not by hand: the notes, without asking for proposals.
                quiet_memory: true,
                skip_context: false,
                fork_packet: None,
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
            type_into(host, &id, &text)
        }
        Effect::Notify { title, body, .. } => done(
            hands
                .notify(&title, body.as_deref())
                .map(|()| Outputs::new()),
        ),
        Effect::RunTest { test, inputs, .. } => {
            // Queue the run in this workspace; the smoke runner, ticking on this same thread,
            // carries it out. The step then waits for the run's row to end. A run that needs the
            // user to approve the file is still queued — it waits in the app, and the step waits
            // with it until its timeout. Not when a schedule started this run: nobody is there
            // to approve it or to answer a step, a step with no timeout would wait for ever,
            // and every later time of the schedule would find this run still going. The door
            // refuses such a test, and the step fails saying why.
            let request = crate::smoke::runs::Request {
                test_id: &test,
                workspace_id: &place.workspace.id,
                inputs: &inputs,
                requested_by: "workflow",
                session_record_id: None,
                unattended,
            };
            match crate::smoke::runs::queue(store, &request) {
                Ok(queued) => {
                    let mut outputs = Outputs::new();
                    outputs.insert("run".into(), queued.id);
                    Outcome::Queued(outputs)
                }
                Err(error) => Outcome::Failed(error.message),
            }
        }
        // Recorded before `carry_out`, from the effect loop.
        Effect::Capture { .. } => {
            Outcome::Failed("An artifact was recorded in the wrong place.".into())
        }
    }
}

/// What a step needs in order to store the artifacts it leaves and then finish.
struct Finishing<'a> {
    store: &'a Store,
    hands: &'a dyn Hands,
    place: &'a Place,
    facts: &'a Facts,
    inputs: &'a Inputs,
    steps: &'a [WorkflowStepRow],
    workflow: &'a super::Workflow,
    run_id: &'a str,
}

impl Finishing<'_> {
    /// Mark a step succeeded once any artifact it leaves has been stored. A step that leaves
    /// none just finishes. A capture that fails fails the step and leaves the previous bodies
    /// in place, so a retry that does not succeed does not wipe the last good handoff.
    fn finish(
        &self,
        step_id: &str,
        outputs: &Outputs,
        note: Option<String>,
        outcome: Option<String>,
    ) -> Move {
        let Some(step) = self.workflow.steps.iter().find(|step| step.id == step_id) else {
            return engine::done(step_id, outputs);
        };
        if step.produces.is_empty() {
            return if note.is_none() && outcome.is_none() {
                engine::done(step_id, outputs)
            } else {
                engine::recorded(step_id, outputs, note, outcome)
            };
        }
        let capture = Capture {
            store: self.store,
            hands: self.hands,
            place: self.place,
            facts: self.facts,
            inputs: self.inputs,
            steps: self.steps,
            run_id: self.run_id,
            step,
            outputs,
        };
        match capture.record() {
            Ok(()) => engine::recorded(step_id, outputs, note, outcome),
            Err(why) => {
                let mut change = engine::failed(step_id, &why);
                change.clear_outcome = true;
                change
            }
        }
    }
}

/// What reading one step's artifacts needs. The engine stays free of files and transcripts.
struct Capture<'a> {
    store: &'a Store,
    hands: &'a dyn Hands,
    place: &'a Place,
    facts: &'a Facts,
    inputs: &'a Inputs,
    steps: &'a [WorkflowStepRow],
    run_id: &'a str,
    step: &'a super::Step,
    outputs: &'a Outputs,
}

impl Capture<'_> {
    /// Read every artifact first, then write them all. One failure writes none.
    fn record(&self) -> Result<(), String> {
        let memory = OnceCell::new();
        let handoff = OnceCell::new();
        let mut saved = Vec::with_capacity(self.step.produces.len());
        for decl in &self.step.produces {
            let (source, detail, text) = self.read(&decl.source, &memory, &handoff)?;
            let (body, truncated) = artifact::require_text(&decl.name, text)?;
            saved.push(NewWorkflowArtifact {
                name: decl.name.clone(),
                source,
                detail,
                body,
                truncated,
            });
        }
        self.store
            .replace_step_artifacts(self.run_id, &self.step.id, &saved)
            .map_err(|error| error.to_string())
    }

    fn read(
        &self,
        source: &ArtifactSource,
        memory: &OnceCell<Option<String>>,
        handoff: &OnceCell<Option<String>>,
    ) -> Result<(String, Option<String>, String), String> {
        let lookup = |path: &[String]| self.lookup(path, memory, handoff);
        match source {
            ArtifactSource::File { path } => {
                let path = super::template::render(path, lookup);
                let text = artifact::read_file(Path::new(&self.place.workspace.path), &path)?;
                Ok(("file".into(), Some(path), text))
            }
            ArtifactSource::Text { text } => {
                Ok(("text".into(), None, super::template::render(text, lookup)))
            }
            ArtifactSource::Output { field } => {
                let text = self
                    .outputs
                    .get(field)
                    .cloned()
                    .ok_or_else(|| format!("The step left no `{field}` to hand on."))?;
                Ok(("output".into(), Some(field.clone()), text))
            }
            ArtifactSource::Transcript { session, raw } => {
                let pty = self
                    .session_pty(session)
                    .ok_or_else(|| "There is no session to copy a conversation from.".to_owned())?;
                let text = self
                    .hands
                    .conversation(&self.place.workspace.id, &pty, *raw)?
                    .ok_or_else(|| {
                        "There is no conversation to copy for that session.".to_owned()
                    })?;
                Ok((
                    "transcript".into(),
                    Some(if *raw { "raw" } else { "digest" }.into()),
                    text,
                ))
            }
        }
    }

    fn session_pty(&self, reference: &SessionRef) -> Option<String> {
        match reference {
            SessionRef::Origin => self.facts.origin.clone(),
            SessionRef::Step(id) if id == &self.step.id => self.outputs.get("session").cloned(),
            SessionRef::Step(id) => self
                .steps
                .iter()
                .find(|row| &row.step_id == id)
                .and_then(|row| engine::parse_outputs(&row.outputs).get("session").cloned()),
        }
    }

    fn lookup(
        &self,
        path: &[String],
        memory: &OnceCell<Option<String>>,
        handoff: &OnceCell<Option<String>>,
    ) -> Option<String> {
        match path {
            [namespace, id] if namespace == "inputs" => self.inputs.get(id).cloned(),
            [namespace, id, field] if namespace == "steps" => {
                if id == &self.step.id {
                    self.outputs.get(field).cloned()
                } else {
                    self.steps
                        .iter()
                        .find(|row| &row.step_id == id)
                        .and_then(|row| engine::parse_outputs(&row.outputs).get(field).cloned())
                }
            }
            [namespace, name] if namespace == "artifacts" => self
                .store
                .workflow_artifacts(self.run_id)
                .ok()
                .and_then(|rows| {
                    rows.into_iter()
                        .find(|row| row.name == *name)
                        .map(|row| row.body)
                }),
            [whole] if whole == "memory" => memory
                .get_or_init(|| {
                    crate::memory::notes_section(self.store, &self.place.project.id)
                        .ok()
                        .flatten()
                })
                .clone(),
            [whole] if whole == "handoff" => handoff
                .get_or_init(|| self.hands.handoff(&self.place.workspace.id))
                .clone(),
            other => self.place.var(other).or_else(|| self.facts.var(other)),
        }
    }
}

/// The world as the engine sees it from a driver.
struct Seen<'a> {
    driver: &'a Driver,
    store: &'a Store,
    host: &'a dyn TerminalHost,
    hands: &'a dyn Hands,
    run_id: &'a str,
    /// When the run started: pull request activity counts from here.
    started_at: i64,
    facts: &'a Facts,
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
            settled_at: self
                .driver
                .settled
                .get(pty_id)
                .max(self.turn_completed_at(pty_id)),
        }
    }

    fn var(&self, path: &[String]) -> Option<String> {
        match path {
            [whole] if whole == "memory" => self
                .memory
                .get_or_init(|| {
                    crate::memory::notes_section(self.store, &self.place.project.id)
                        .ok()
                        .flatten()
                })
                .clone(),
            [whole] if whole == "handoff" => self
                .handoff
                .get_or_init(|| self.hands.handoff(&self.place.workspace.id))
                .clone(),
            [namespace, _] if namespace == "pr" || namespace == "event" => self.facts.var(path),
            [namespace, name] if namespace == "artifacts" => self
                .store
                .workflow_artifacts(self.run_id)
                .ok()
                .and_then(|rows| {
                    rows.into_iter()
                        .find(|row| row.name == *name)
                        .map(|row| row.body)
                }),
            _ => self.place.var(path),
        }
    }

    fn origin(&self) -> Option<String> {
        self.facts.origin.clone()
    }

    fn pr_activity(&self, kind: PrActivity) -> Option<PrSeen> {
        let pr = self.facts.pr.as_ref()?;
        let posts = self.driver.pr_posts(self.run_id, || {
            self.hands.pr_posts(&self.place.workspace.id, pr)
        })?;
        // The forge keeps whole seconds; a post in the second the run started counts.
        let since = self.started_at - self.started_at.rem_euclid(1000);
        let wanted: Vec<&PrPost> = posts
            .iter()
            .filter(|post| post.at_ms >= since)
            .filter(|post| match kind {
                PrActivity::Any => true,
                PrActivity::Review => post.kind == PrPostKind::Review,
                PrActivity::Comment => post.kind == PrPostKind::Comment,
            })
            .collect();
        Some(PrSeen {
            count: u32::try_from(wanted.len()).unwrap_or(u32::MAX),
            latest_url: wanted.last().and_then(|post| post.url.clone()),
        })
    }

    fn test_run(&self, run_id: &str) -> Option<TestRun> {
        let row = self.store.test_run(run_id).ok().flatten()?;
        let steps = self.store.test_steps(run_id).unwrap_or_default();
        let summary = crate::smoke::report::summary(&row, &steps);
        Some(TestRun {
            status: row.status,
            summary,
        })
    }
}

/// The workspace's newest live harness session: the conversation records newest first, matched
/// to the sessions the host is running. Shells and programs are not agents.
fn origin_session(store: &Store, host: &dyn TerminalHost, workspace_id: &str) -> Option<String> {
    let records = store.sessions(workspace_id).ok()?;
    let live: Vec<SessionInfo> = host
        .list()
        .into_iter()
        .filter(|session| {
            matches!(session.state, SessionState::Running)
                && session.labels.get(WORKSPACE_LABEL).map(String::as_str) == Some(workspace_id)
                && session.labels.contains_key(HARNESS_LABEL)
        })
        .collect();
    records.iter().find_map(|record| {
        live.iter()
            .find(|session| session.labels.get(RECORD_LABEL) == Some(&record.id))
            .map(|session| session.id.0.clone())
    })
}

/// The live terminal of one conversation record in the workspace, if it is running.
fn live_terminal(host: &dyn TerminalHost, workspace_id: &str, record: &str) -> Option<String> {
    host.list()
        .into_iter()
        .find(|session| {
            matches!(session.state, SessionState::Running)
                && session.labels.get(WORKSPACE_LABEL).map(String::as_str) == Some(workspace_id)
                && session.labels.get(RECORD_LABEL).map(String::as_str) == Some(record)
        })
        .map(|session| session.id.0)
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
mod portable_tests;
#[cfg(all(test, unix))]
mod tests;
