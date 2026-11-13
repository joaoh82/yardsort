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

use std::cell::{OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use pty_host::{HostError, HostEvent, SessionId, SessionInfo, SessionState, TerminalHost};

use super::engine::{
    self, ApprovalDecision, Effect, Finish, Inputs, Move, Outputs, Plan, PrSeen, RecordedApproval,
    SessionFacts, StepOutcome, TestRun, World,
};
use super::runs::{self, Facts, Place, PrFacts};
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

    /// A job that asks whether the workspace has an open pull request. Called on the driver's
    /// thread, so it returns at once: the job it returns does the asking, on a thread of its
    /// own. `Ok(None)` is an answer: the branch has no open pull request. The default is that
    /// answer, so a test that queued the run with one already known does not have to ask.
    fn open_pull_request(&self, _workspace_id: &str) -> OpenPr {
        Box::new(|| Ok(None))
    }

    /// A job that asks the forge for the workspace branch's pull requests in any state, with
    /// when each was opened. A report that lists the run's pull requests asks this first, since
    /// the app's own look at the forge comes about once a minute. Same threading rules as
    /// [`Self::open_pull_request`]. The default is none.
    fn branch_pull_requests(&self, _workspace_id: &str) -> BranchPr {
        Box::new(|| Ok(Vec::new()))
    }

    /// One session's conversation, for a step that asked for a transcript. `raw` asks for each
    /// turn's own text. Without it, the budgeted digest. [`Conversation::Ready`]`(None)` when
    /// that session has no conversation to copy. [`Conversation::Later`] when the read has not
    /// finished: the driver asks again on the next tick and does not fail the step. The default
    /// is none, so a driver test that never asks is unchanged. `run_id` and `step_id` keep one
    /// read from being reused by a later step.
    fn conversation(
        &self,
        _workspace_id: &str,
        _pty_id: &str,
        _raw: bool,
        _run_id: &str,
        _step_id: &str,
    ) -> Result<Conversation, String> {
        Ok(Conversation::Ready(None))
    }

    /// Remember the one-time code so the window can show it. The default forgets it. Do not
    /// write the code to a log, a note, or a step's outputs.
    fn approval_code(&self, _run_id: &str, _step_id: &str, _code: &str) {}

    /// Whether the waiting notification may contain the one-time code. The default follows
    /// the platform: included on macOS and Windows, left out on Linux, where other programs
    /// can read a notification on D-Bus. The app reads the setting and overrides this.
    fn include_approval_code(&self) -> bool {
        crate::settings::default_show_approval_code()
    }
}

/// Where a test stops a tick, as if the process died after that commit. Production leaves
/// this at [`StopAfter::Never`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopAfter {
    Never,
    /// The decision is on the step, the wait is applied, and the decision's artifacts are
    /// written. Those are one change. The next step has not started.
    AfterDecisionApplied,
    /// The next step has been claimed (`running`) and its effect has not been carried out.
    AfterNextClaim,
    /// The engine has planned a rewind and the store has not applied it.
    BeforeRewindApplied,
    /// An answer is about to be marked sent. It has not been, and it has not been typed.
    BeforeAnswerApplied,
    /// The answer is claimed, and it has not been typed. A restart fails the step and does
    /// not type it: the keystrokes may already have landed.
    AfterAnswerApplied,
}

/// How far a transcript read got. A slow read must not fail the step, and must not be started
/// again from scratch on the next tick.
#[derive(Debug)]
pub enum Conversation {
    /// The read finished. `None` means that session has no conversation to copy.
    Ready(Option<String>),
    /// Still reading. Ask again later. The same read should be reused.
    Later,
}

/// Asking the forge, as a job for a thread of its own.
pub type PrAsk = Box<dyn FnOnce() -> Result<Vec<PrPost>, String> + Send + 'static>;

/// Asking whether the workspace has an open pull request, as a job for a thread of its own.
pub type OpenPr = Box<dyn FnOnce() -> Result<Option<PrFacts>, String> + Send + 'static>;

/// Asking for the branch's pull requests in any state, as a job for a thread of its own.
pub type BranchPr =
    Box<dyn FnOnce() -> Result<Vec<crate::forge::PullRequest>, String> + Send + 'static>;

/// What the forge said about the branch for each report step, by run and step: `None` while
/// the question is out.
type ReportLookups = HashMap<(String, String), Option<Vec<crate::forge::PullRequest>>>;

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
    /// A pull request looked up after the run was queued, because the branch had none then.
    pulls: std::sync::Arc<Mutex<HashMap<String, PullLookup>>>,
    /// The forge asked about the branch for a report step, keyed by run and step. `None`
    /// while the question is out; `Some` once answered, whatever the answer.
    reporting: std::sync::Arc<Mutex<ReportLookups>>,
    /// Runs whose `factory/` directory was already kept out of commits.
    factory_excluded: Mutex<HashSet<String>>,
    forge_every_ms: i64,
    /// Something about a run changed during the current tick.
    touched: std::sync::atomic::AtomicBool,
    /// The runs that were active at the last tick. One that is gone since was ended from
    /// outside — `ys workflow cancel` — which is a change too.
    active: Mutex<HashSet<String>>,
    /// One-time codes this process has issued. A restart has none, which is how a lost code
    /// is replaced once. Listing a run does not write here.
    codes: Mutex<HashMap<(String, String), String>>,
    /// A test asks the tick to die after one commit. Production stays at [`StopAfter::Never`].
    stop: Mutex<StopAfter>,
    halt: std::sync::atomic::AtomicBool,
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
            pulls: std::sync::Arc::new(Mutex::new(HashMap::new())),
            reporting: std::sync::Arc::new(Mutex::new(HashMap::new())),
            factory_excluded: Mutex::new(HashSet::new()),
            forge_every_ms: i64::try_from(forge_every.as_millis()).unwrap_or(i64::MAX),
            touched: std::sync::atomic::AtomicBool::new(false),
            active: Mutex::new(HashSet::new()),
            codes: Mutex::new(HashMap::new()),
            stop: Mutex::new(StopAfter::Never),
            halt: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Remember a code the window just showed, so this process does not replace it on the
    /// next look. The plain code stays in memory only.
    pub fn note_approval_code(&self, run_id: &str, step_id: &str, code: &str) {
        self.codes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert((run_id.to_owned(), step_id.to_owned()), code.to_owned());
    }

    /// Issue a code and remember it. `force` replaces a code this process already holds.
    /// Nothing when the step is already decided, or when `force` is false and this process
    /// already holds the code. The bool is whether a code row existed before this issue.
    ///
    /// The slow hash runs before the code lock is taken again, so the lock is not held
    /// while the hash runs. `force` is Show a new code, which resets the wrong-try count. A
    /// restart reissue (`force` false) keeps that count, so a lockout survives the restart.
    pub fn issue_approval_code(
        &self,
        store: &Store,
        run_id: &str,
        step_id: &str,
        force: bool,
    ) -> crate::store::StoreResult<Option<(String, bool)>> {
        let key = (run_id.to_owned(), step_id.to_owned());
        {
            let codes = self.codes.lock().unwrap_or_else(PoisonError::into_inner);
            if !force && codes.contains_key(&key) {
                return Ok(None);
            }
        }
        let (code, salt, hash) = crate::workflow::code::issue();
        let mut codes = self.codes.lock().unwrap_or_else(PoisonError::into_inner);
        if !force && codes.contains_key(&key) {
            return Ok(None);
        }
        let had = store.approval_code_row(run_id, step_id)?;
        let Some(code) = store.save_approval_code(run_id, step_id, &code, &salt, &hash, force)?
        else {
            return Ok(None);
        };
        codes.insert(key, code.clone());
        Ok(Some((code, had)))
    }

    /// Issue the one-time code for an open question. `force` is Show a new code, which
    /// resets the wrong-try count. A restart (`force` false) keeps that count. A round that
    /// cleared the hash is issued again even when this process still holds the previous code.
    pub fn issue_answer_code(
        &self,
        store: &Store,
        run_id: &str,
        step_id: &str,
        force: bool,
    ) -> crate::store::StoreResult<Option<(String, bool)>> {
        let key = (run_id.to_owned(), step_id.to_owned());
        let present = store.answer_code_present(run_id, step_id)?;
        {
            let codes = self.codes.lock().unwrap_or_else(PoisonError::into_inner);
            if !force && present && codes.contains_key(&key) {
                return Ok(None);
            }
        }
        let (code, salt, hash) = crate::workflow::code::issue();
        let mut codes = self.codes.lock().unwrap_or_else(PoisonError::into_inner);
        if !force && present && codes.contains_key(&key) {
            return Ok(None);
        }
        let Some(code) = store.save_answer_code(run_id, step_id, &code, &salt, &hash, force)?
        else {
            return Ok(None);
        };
        codes.insert(key, code.clone());
        Ok(Some((code, present)))
    }

    /// The code a signed-in phone may submit. The one this process already holds, when it
    /// still matches the stored hash, so the desktop and the phone share it. Otherwise a
    /// restart reissue, which does not reset the wrong-try count. Nothing when the step is
    /// not waiting, or the code is locked: the computer has to show a new one.
    ///
    /// The plain code stays in this process. It is not written, and a caller must not log it.
    pub fn code_for_phone(
        &self,
        store: &Store,
        run_id: &str,
        step_id: &str,
    ) -> crate::store::StoreResult<Option<String>> {
        if store.approval_code_locked(run_id, step_id)?
            || store.answer_code_locked(run_id, step_id)?
        {
            return Ok(None);
        }
        let key = (run_id.to_owned(), step_id.to_owned());
        let held = {
            let codes = self.codes.lock().unwrap_or_else(PoisonError::into_inner);
            codes.get(&key).cloned()
        };
        if let Some(code) = held {
            if store.presented_code_matches(run_id, step_id, &code)? {
                return Ok(Some(code));
            }
            self.codes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&key);
        }
        if store.approval_is_open(run_id, step_id)? {
            return Ok(self
                .issue_approval_code(store, run_id, step_id, false)?
                .map(|(code, _)| code));
        }
        if store.question_is_open(run_id, step_id)? {
            return Ok(self
                .issue_answer_code(store, run_id, step_id, false)?
                .map(|(code, _)| code));
        }
        Ok(None)
    }

    fn holds_code(&self, run_id: &str, step_id: &str) -> bool {
        self.codes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&(run_id.to_owned(), step_id.to_owned()))
    }

    fn forget_code(&self, run_id: &str, step_id: &str) {
        self.codes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&(run_id.to_owned(), step_id.to_owned()));
    }

    /// Die after `when`. Tests use this to leave the database as a crash would.
    #[cfg(test)]
    fn testing_stop_after(&self, when: StopAfter) {
        *self.stop.lock().unwrap_or_else(PoisonError::into_inner) = when;
        self.halt.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    fn halt_now(&self) -> bool {
        self.halt.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn arm_halt(&self, when: StopAfter) -> bool {
        let armed = *self.stop.lock().unwrap_or_else(PoisonError::into_inner) == when;
        if armed {
            self.halt.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        armed
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
        self.pulls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|id, _| runs.iter().any(|run| &run.id == id));
        self.reporting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(id, _), _| runs.iter().any(|run| &run.id == id));
        self.factory_excluded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|id| runs.iter().any(|run| &run.id == id));
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

    /// The open pull request looked up for a run that was queued without one.
    ///
    /// The same throttle as [`Self::pr_posts`]: a slow forge must not hold the tick, and a
    /// branch with no pull request is not asked again until the interval has passed.
    fn open_pr(&self, run_id: &str, ask: impl FnOnce() -> OpenPr) -> Open {
        let now = now_ms();
        let mut pulls = self.pulls.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = pulls.entry(run_id.to_owned()).or_default();
        // An error leaves `found` empty. The next ask waits out the same interval as an answer
        // that there is no pull request: asking again on the next tick would hammer `gh`.
        let due = slot.at == 0 || now.saturating_sub(slot.at) >= self.forge_every_ms;
        if due && !slot.waiting {
            slot.at = now;
            slot.waiting = true;
            let job = ask();
            let answers = std::sync::Arc::clone(&self.pulls);
            let run = run_id.to_owned();
            let spawned = std::thread::Builder::new()
                .name("workflow-pr".into())
                .spawn(move || {
                    let answer = job();
                    let mut pulls = answers.lock().unwrap_or_else(PoisonError::into_inner);
                    let Some(slot) = pulls.get_mut(&run) else {
                        return;
                    };
                    slot.waiting = false;
                    match answer {
                        Ok(pr) => slot.found = Some(pr),
                        Err(why) => {
                            eprintln!("workflows: run {run}: cannot find the pull request: {why}");
                        }
                    }
                });
            if spawned.is_err() {
                slot.waiting = false;
            }
        }
        match &slot.found {
            None => Open::Looking,
            Some(None) => Open::Missing,
            Some(Some(pr)) => Open::Found(pr.clone()),
        }
    }
}

impl Driver {
    /// Ask the forge once about the branch for one report step. `None` while the question is
    /// out. Once answered, what it found; an error counts as an answer of nothing, so
    /// a forge that cannot be reached does not hold the report forever.
    fn branch_pr_for_report(
        &self,
        run_id: &str,
        step_id: &str,
        ask: impl FnOnce() -> BranchPr,
    ) -> Option<Vec<crate::forge::PullRequest>> {
        let key = (run_id.to_owned(), step_id.to_owned());
        let mut asked = self
            .reporting
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(answer) = asked.get(&key) {
            return answer.clone();
        }
        asked.insert(key.clone(), None);
        let job = ask();
        let answers = std::sync::Arc::clone(&self.reporting);
        let spawned = std::thread::Builder::new()
            .name("workflow-report-pr".into())
            .spawn(move || {
                let answer = job().unwrap_or_else(|why| {
                    eprintln!(
                        "workflows: run {}: cannot ask about the pull request for `{}`: {why}",
                        key.0, key.1
                    );
                    Vec::new()
                });
                answers
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(key, Some(answer));
            });
        if spawned.is_err() {
            return Some(Vec::new());
        }
        None
    }
}

/// What a later look found for a run that started with no pull request.
#[derive(Debug, Default)]
struct PullLookup {
    at: i64,
    waiting: bool,
    /// `None` until the forge has answered. `Some(None)` means it answered: no open one.
    found: Option<Option<PrFacts>>,
}

/// A pull request looked up after the run was queued.
enum Open {
    /// The question is out, or has not been answered.
    Looking,
    /// Asked, and the branch has no open pull request.
    Missing,
    Found(PrFacts),
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
        note_run_finished(store, &run.id, "failed");
        return Ok(());
    };
    let place = match run.workspace_id.as_deref() {
        Some(id) => Place::load(store, id, git)?,
        None => None,
    };
    let Some(place) = place else {
        driver.touch();
        store.finish_workflow_run(&run.id, "failed", Some("The workspace is gone."))?;
        note_run_finished(store, &run.id, "failed");
        return Ok(());
    };
    let facts = RefCell::new(Facts::parse(&run.context));
    let place = place.in_run(&facts.borrow());
    let mut started_at = run.started_at;
    if run.status == "queued" {
        // The workspace's own agent is whichever was newest when the run began: found now,
        // before the run has started any of its own for it to be mistaken for.
        if runs::needs_origin(&workflow) {
            let opened_by = facts
                .borrow()
                .pr
                .as_ref()
                .and_then(|pr| pr.elsewhere.as_ref())
                .and_then(|elsewhere| elsewhere.opened_by.clone());
            let origin_record = facts.borrow().origin_record.clone();
            facts.borrow_mut().origin = match (origin_record.as_deref(), opened_by.as_deref()) {
                // Asked for on behalf of one agent: that one, or none when it is not running.
                (Some(record), _) => live_terminal(host, &place.workspace.id, record),
                // About a pull request elsewhere: the agent that opened it while it runs, else
                // the workspace's, which may well be the one working on it now.
                (None, Some(record)) => live_terminal(host, &place.workspace.id, record)
                    .or_else(|| origin_session(store, host, &place.workspace.id)),
                (None, None) => origin_session(store, host, &place.workspace.id),
            };
        }
        if !store.start_workflow_run(&run.id, Some(&facts.borrow().to_json()))? {
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
            apply(driver, store, host, &run.id, &change)?;
            if driver.halt_now() {
                return Ok(());
            }
        }
        // A code this process does not hold is replaced once, here, and the person is told.
        // A decision already recorded is left for the engine to apply, and is not replaced.
        resume_human_waits(driver, store, hands, &workflow, run)?;
    }
    tell_open_questions(driver, store, hands, run)?;
    if writes_factory(&workflow)
        && driver
            .factory_excluded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(run.id.clone())
    {
        exclude_factory_dir(Path::new(&place.workspace.path));
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
                // Drop a previous decision before the step is waiting again, while it is still
                // pending, so a decision recorded for this attempt cannot be deleted by the clear.
                if change.clear_outcome && change.from.contains(&StepStatus::Pending) {
                    store.clear_workflow_approval(&run.id, &change.step)?;
                    store.clear_workflow_wait(&run.id, &change.step)?;
                    driver.forget_code(&run.id, &change.step);
                }
                if change.rewind.is_some() && driver.arm_halt(StopAfter::BeforeRewindApplied) {
                    return Ok(());
                }
                if apply(driver, store, host, &run.id, change)? {
                    if driver.halt_now() {
                        return Ok(());
                    }
                    follow_approval(driver, store, hands, &workflow, run, change)?;
                }
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
            if !apply(driver, store, host, &run.id, &claim)? {
                if !still_running(store, &run.id)? {
                    return Ok(()); // Cancelled: nothing more of this run happens.
                }
                continue; // The step moved on without us.
            }
            // The claim is committed and the effect has not run. A crash here leaves the step
            // running, and the next start fails it rather than doing the effect twice.
            if driver.arm_halt(StopAfter::AfterNextClaim) {
                return Ok(());
            }
            if let Effect::Capture {
                outputs,
                note,
                outcome,
                ..
            } = &effect
            {
                delivered |= deliver(
                    driver,
                    store,
                    host,
                    &run.id,
                    finishing.finish(&step, outputs, note.clone(), outcome.clone()),
                )?;
                continue;
            }
            if let Effect::Start { step, .. } = &effect {
                clear_rewritten_files(&workflow, step, Path::new(&place.workspace.path), git);
            }
            let outcome = carry_out(
                store,
                host,
                hands,
                &place,
                effect,
                facts.borrow().unattended,
                &mut typed_to,
            );
            let change = match outcome {
                Outcome::Done(outputs) => {
                    delivered |= deliver(
                        driver,
                        store,
                        host,
                        &run.id,
                        finishing.finish(&step, &outputs, None, None),
                    )?;
                    continue;
                }
                Outcome::Queued(outputs) => engine::queued(&step, &outputs),
                Outcome::Failed(why) => engine::failed(&step, &why),
                Outcome::Skipped(why) => engine::skipped(&step, &why),
                Outcome::NotYet => engine::not_yet(&step),
            };
            delivered |= change.to != StepStatus::Waiting;
            apply(driver, store, host, &run.id, &change)?;
        }
        if let Some(finish) = plan.finish {
            let (status, error) = match finish {
                Finish::Succeeded => ("succeeded", None),
                Finish::Failed(why) => ("failed", Some(why)),
            };
            driver.touch();
            store.finish_workflow_run(&run.id, status, error.as_deref())?;
            note_run_finished(store, &run.id, status);
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

/// Apply `change`, unless the artifacts and the success were already written together.
/// True when the step moved on, so a later step in this run may be able to start.
fn deliver(
    driver: &Driver,
    store: &Store,
    host: &dyn TerminalHost,
    run_id: &str,
    (change, committed): (Move, bool),
) -> IpcResult<bool> {
    let moved = change.to != StepStatus::Waiting;
    if committed {
        driver.touch();
        return Ok(moved);
    }
    apply(driver, store, host, run_id, &change)?;
    Ok(moved)
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
///
/// The claim (`applied_at`) is taken before the keystrokes. `typed_at` is set only when
/// typing returns done. A session that is held rolls the claim back, and the next tick
/// tries again. A crash after the claim and before `typed_at` is not retried: the engine
/// fails the step, because the keystrokes may already have landed.
fn deliver_answer(
    driver: &Driver,
    store: &Store,
    host: &dyn TerminalHost,
    run_id: &str,
    step_id: &str,
    delivery: &engine::AnswerDelivery,
) -> IpcResult<bool> {
    if driver.arm_halt(StopAfter::BeforeAnswerApplied) {
        return Ok(true);
    }
    let id = SessionId(delivery.pty_id.clone());
    let info = match host.info(&id) {
        Ok(info) => info,
        Err(_) => return Ok(false),
    };
    if !matches!(info.state, SessionState::Running) || info.busy {
        return Ok(false);
    }
    let (note, _) = crate::workflow::artifact::cap(&format!("Answered.\n{}", delivery.text));
    let claimed = store.claim_clarification_answer(run_id, step_id, delivery.round, &note)?;
    if !claimed {
        return Ok(false);
    }
    driver.touch();
    if driver.arm_halt(StopAfter::AfterAnswerApplied) {
        return Ok(true);
    }
    match type_into(host, &id, &delivery.text) {
        Outcome::Done(_) => {
            store.mark_clarification_typed(run_id, step_id, delivery.round)?;
            Ok(true)
        }
        Outcome::Failed(why) => {
            store.move_workflow_step(
                run_id,
                step_id,
                &[StepStatus::Waiting],
                StepStatus::Failed,
                None,
                Some(&format!(
                    "The answer was recorded and could not be typed into the session: {why}"
                )),
                None,
                false,
            )?;
            Ok(true)
        }
        Outcome::NotYet => {
            // Nothing was typed. The next tick claims it again.
            store.release_clarification_claim(run_id, step_id, delivery.round)?;
            Ok(false)
        }
        Outcome::Skipped(_) | Outcome::Queued(_) => {
            store.release_clarification_claim(run_id, step_id, delivery.round)?;
            Ok(false)
        }
    }
}

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

fn apply(
    driver: &Driver,
    store: &Store,
    host: &dyn TerminalHost,
    run_id: &str,
    change: &Move,
) -> IpcResult<bool> {
    if let Some(delivery) = &change.apply_answer {
        return deliver_answer(driver, store, host, run_id, &change.step, delivery);
    }
    // A loop is one change: the steps go back to pending, the old decision is removed, and
    // the feedback is kept. It is not the path that records a decision. Checked first, so a
    // request for changes that routes back is not also applied as a finished decision.
    if let Some(rewind) = &change.rewind {
        let feedback: Vec<NewWorkflowArtifact> = rewind
            .feedback
            .iter()
            .map(|copy| NewWorkflowArtifact {
                name: copy.name.clone(),
                source: copy.source.clone(),
                detail: None,
                body: copy.body.clone(),
                truncated: false,
            })
            .collect();
        // The rewind cancels a smoke test it sends back, in the same transaction, so a
        // crash after the commit still leaves that test cancelled.
        let moved = store.rewind_workflow(
            run_id,
            &change.step,
            &change.from,
            rewind.seen,
            &rewind.steps,
            &feedback,
        )?;
        if !moved {
            return Ok(false);
        }
        for step in &rewind.steps {
            driver.forget_code(run_id, step);
        }
        driver.touch();
        return Ok(true);
    }
    // A person's decision, the wait's applied mark, and the decision's artifacts are one
    // change. The step is no longer waiting, so the decision is not applied a second time,
    // and the artifact is not written again on the next start.
    if is_applied_decision(change) {
        let artifacts = changes_artifacts(store, run_id, change)?.unwrap_or_default();
        let moved = store.move_workflow_step_applying_wait(
            run_id,
            &change.step,
            &change.from,
            change.to,
            change.outputs.as_deref(),
            change.note.as_deref(),
            change.outcome.as_deref(),
            change.clear_outcome,
            &artifacts,
        )?;
        if !moved {
            return Ok(false);
        }
        driver.touch();
        if driver.arm_halt(StopAfter::AfterDecisionApplied) {
            return Ok(true);
        }
        return Ok(true);
    }
    let moved = if change.keep_decision {
        store.move_workflow_step_unless_decided(
            run_id,
            &change.step,
            &change.from,
            change.to,
            change.outputs.as_deref(),
            change.note.as_deref(),
            change.outcome.as_deref(),
            change.clear_outcome,
        )?
    } else if let Some(copy) = &change.feedback {
        let artifact = NewWorkflowArtifact {
            name: copy.name.clone(),
            source: copy.source.clone(),
            detail: None,
            body: copy.body.clone(),
            truncated: false,
        };
        store.move_workflow_step_leaving(
            run_id,
            &change.step,
            &change.from,
            change.to,
            change.outputs.as_deref(),
            change.note.as_deref(),
            change.outcome.as_deref(),
            change.clear_outcome,
            &[artifact],
        )?
    } else {
        store.move_workflow_step(
            run_id,
            &change.step,
            &change.from,
            change.to,
            change.outputs.as_deref(),
            change.note.as_deref(),
            change.outcome.as_deref(),
            change.clear_outcome,
        )?
    };
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
        | Effect::Report { step, .. }
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
        Effect::Report { title, body, .. } => {
            let mut outputs = Outputs::new();
            if let Some(title) = title {
                outputs.insert("title".to_owned(), title);
            }
            outputs.insert("body".to_owned(), super::artifact::cap(&body).0);
            done(Ok(outputs))
        }
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
    facts: &'a RefCell<Facts>,
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
    ) -> (Move, bool) {
        let Some(step) = self.workflow.steps.iter().find(|step| step.id == step_id) else {
            return (engine::done(step_id, outputs), false);
        };
        if step.produces.is_empty() {
            let change = if note.is_none() && outcome.is_none() {
                engine::done(step_id, outputs)
            } else {
                engine::recorded(step_id, outputs, note, outcome)
            };
            return (change, false);
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
            Ok(saved) => {
                let change = engine::recorded(step_id, outputs, note, outcome);
                match self.store.finish_step_with_artifacts(
                    self.run_id,
                    step_id,
                    &[StepStatus::Running],
                    change.to,
                    &saved,
                    change.outputs.as_deref(),
                    change.note.as_deref(),
                    change.outcome.as_deref(),
                    change.clear_outcome,
                ) {
                    Ok(true) => (change, true),
                    // The run stopped, or the step moved. The artifacts were not kept. Putting
                    // the step back to wait, when it is still running, lets the next tick try
                    // the capture again. A success applied on its own would finish with no body.
                    Ok(false) => (engine::not_yet(step_id), false),
                    Err(error) => (capture_failed(step_id, outputs, &error.to_string()), false),
                }
            }
            Err(CaptureMiss::Later) => (
                engine::still_reading(step_id, outputs, note, outcome),
                false,
            ),
            Err(CaptureMiss::Failed(why)) => (capture_failed(step_id, outputs, &why), false),
        }
    }
}

/// A capture that failed must not leave the pending marker in the step's outputs. The outputs
/// argument is the unmarked set the capture was given.
fn capture_failed(step_id: &str, outputs: &Outputs, why: &str) -> Move {
    let mut change = engine::failed(step_id, why);
    change.clear_outcome = true;
    change.outputs = Some(serde_json::to_string(outputs).unwrap_or_else(|_| "{}".into()));
    change
}

/// Why a capture did not store anything.
enum CaptureMiss {
    Failed(String),
    /// The transcript is still being read. The step waits and the next tick asks again.
    Later,
}

/// What reading one step's artifacts needs. The engine stays free of files and transcripts.
struct Capture<'a> {
    store: &'a Store,
    hands: &'a dyn Hands,
    place: &'a Place,
    facts: &'a RefCell<Facts>,
    inputs: &'a Inputs,
    steps: &'a [WorkflowStepRow],
    run_id: &'a str,
    step: &'a super::Step,
    outputs: &'a Outputs,
}

impl Capture<'_> {
    /// Read every artifact first. One failure returns none, so nothing is written.
    fn record(&self) -> Result<Vec<NewWorkflowArtifact>, CaptureMiss> {
        let memory = OnceCell::new();
        let handoff = OnceCell::new();
        let mut saved = Vec::with_capacity(self.step.produces.len());
        for decl in &self.step.produces {
            // An approval may leave `feedback` empty. That must not fail the step after the
            // person has already approved. Other actions still refuse an empty field.
            if let ArtifactSource::Output { field } = &decl.source {
                if matches!(self.step.action, super::Action::Approve { .. }) {
                    let text = self.outputs.get(field).map(String::as_str).unwrap_or("");
                    if text.trim().is_empty() {
                        continue;
                    }
                }
            }
            let (source, detail, text) = self.read(&decl.source, &memory, &handoff)?;
            let (body, truncated) =
                artifact::require_text(&decl.name, text).map_err(CaptureMiss::Failed)?;
            saved.push(NewWorkflowArtifact {
                name: decl.name.clone(),
                source,
                detail,
                body,
                truncated,
            });
        }
        Ok(saved)
    }

    fn read(
        &self,
        source: &ArtifactSource,
        memory: &OnceCell<Option<String>>,
        handoff: &OnceCell<Option<String>>,
    ) -> Result<(String, Option<String>, String), CaptureMiss> {
        let lookup = |path: &[String]| self.lookup(path, memory, handoff);
        let failed = CaptureMiss::Failed;
        match source {
            ArtifactSource::File { path } => {
                let path = super::template::render(path, lookup);
                let text = artifact::read_file(Path::new(&self.place.workspace.path), &path)
                    .map_err(failed)?;
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
                    .ok_or_else(|| failed(format!("The step left no `{field}` to hand on.")))?;
                Ok(("output".into(), Some(field.clone()), text))
            }
            ArtifactSource::Brief => {
                let rounds = self
                    .store
                    .clarification_rounds(self.run_id, &self.step.id)
                    .map_err(|error| failed(error.to_string()))?;
                let summary = self
                    .store
                    .clarification_summary(self.run_id, &self.step.id)
                    .map_err(|error| failed(error.to_string()))?;
                let text = super::clarify::render_brief(&rounds, summary.as_deref());
                Ok(("brief".into(), None, text))
            }
            ArtifactSource::Transcript { session, raw } => {
                let pty = self.session_pty(session).ok_or_else(|| {
                    failed("There is no session to copy a conversation from.".to_owned())
                })?;
                let text = match self
                    .hands
                    .conversation(
                        &self.place.workspace.id,
                        &pty,
                        *raw,
                        self.run_id,
                        &self.step.id,
                    )
                    .map_err(failed)?
                {
                    Conversation::Later => return Err(CaptureMiss::Later),
                    Conversation::Ready(Some(text)) => text,
                    Conversation::Ready(None) => {
                        return Err(failed(
                            "There is no conversation to copy for that session.".to_owned(),
                        ));
                    }
                };
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
            SessionRef::Origin => self.facts.borrow().origin.clone(),
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
            [namespace, field] if namespace == "run" => run_var(self.store, self.run_id, field),
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
            other => self
                .place
                .var(other)
                .or_else(|| self.facts.borrow().var(other)),
        }
    }
}

/// `{{ run.<field> }}`: the run's id, or the pull requests it has opened so far.
fn run_var(store: &Store, run_id: &str, field: &str) -> Option<String> {
    match field {
        "id" => Some(run_id.to_owned()),
        "pull_requests" => {
            let run = store.workflow_run(run_id).ok().flatten()?;
            let made = super::made::pull_requests_made(store, &run).ok()?;
            Some(super::made::pull_requests_text(&made))
        }
        _ => None,
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
    facts: &'a RefCell<Facts>,
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
            [namespace, _] if namespace == "pr" || namespace == "event" => {
                if namespace == "pr" {
                    self.pull_request();
                }
                self.facts.borrow().var(path)
            }
            [namespace, field] if namespace == "run" => run_var(self.store, self.run_id, field),
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
        self.facts.borrow().origin.clone()
    }

    fn pull_requests_current(&self, step_id: &str) -> bool {
        let Some(found) = self.driver.branch_pr_for_report(self.run_id, step_id, || {
            self.hands.branch_pull_requests(&self.place.workspace.id)
        }) else {
            return false;
        };
        // The same matching as the app's look: on the branch, opened during the run.
        if let Err(error) =
            super::made::observe_pull_requests(self.store, &self.place.project.id, &found, self.now)
        {
            eprintln!(
                "workflows: run {}: could not record its pull requests: {error}",
                self.run_id
            );
        }
        true
    }

    fn pr_known(&self) -> Option<bool> {
        match self.ensure_pr() {
            Open::Found(_) => Some(true),
            Open::Missing => Some(false),
            Open::Looking => None,
        }
    }

    fn pr_activity(&self, kind: PrActivity, since: i64) -> Option<PrSeen> {
        // No pull request yet: keep waiting. A lookup still out is the same answer.
        let Open::Found(pr) = self.ensure_pr() else {
            return None;
        };
        let posts = self.driver.pr_posts(self.run_id, || {
            self.hands.pr_posts(&self.place.workspace.id, &pr)
        })?;
        // The forge keeps whole seconds. A post in the second this wait began counts, and
        // not one from before it — a review that finished the previous pass does not finish
        // the next. The run's own start is the earliest a post can count.
        let floor = floor_sec(self.started_at.max(since));
        let wanted: Vec<&PrPost> = posts
            .iter()
            .filter(|post| post.at_ms >= floor)
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

    fn clarification(&self, step_id: &str) -> super::clarify::ClarificationState {
        let rounds = self
            .store
            .clarification_rounds(self.run_id, step_id)
            .unwrap_or_default();
        let mut state = super::clarify::state_from(&rounds);
        state.summary = self
            .store
            .clarification_summary(self.run_id, step_id)
            .ok()
            .flatten()
            .is_some_and(|text| !text.trim().is_empty());
        state
    }

    fn approval(&self, step_id: &str) -> Option<RecordedApproval> {
        let row = self
            .store
            .workflow_approval(self.run_id, step_id)
            .ok()
            .flatten()?;
        let decision = ApprovalDecision::parse(&row.decision)?;
        Some(RecordedApproval {
            decision,
            feedback: row.feedback,
            actor: row.actor,
            decided_at: row.decided_at,
        })
    }
}

impl Seen<'_> {
    /// The pull request this run knows. A run queued before the branch had one looks again,
    /// and stores it once it is there. [`Open::Looking`] while that question is out.
    fn ensure_pr(&self) -> Open {
        if let Some(pr) = self.facts.borrow().pr.clone() {
            return Open::Found(pr);
        }
        let found = self.driver.open_pr(self.run_id, || {
            self.hands.open_pull_request(&self.place.workspace.id)
        });
        if let Open::Found(pr) = &found {
            self.remember(pr.clone());
        }
        found
    }

    fn pull_request(&self) {
        let _ = self.ensure_pr();
    }

    fn remember(&self, pr: PrFacts) {
        if self.facts.borrow().pr.is_some() {
            return;
        }
        self.facts.borrow_mut().pr = Some(pr);
        let json = self.facts.borrow().to_json();
        if self
            .store
            .set_workflow_run_context(self.run_id, &json)
            .is_err()
        {
            eprintln!(
                "workflows: run {}: could not store the pull request",
                self.run_id
            );
        }
    }
}

/// Whole seconds, as the forge records a post. A post in that second counts.
fn floor_sec(ms: i64) -> i64 {
    ms - ms.rem_euclid(1000)
}

/// Whether a workflow writes its notes under `factory/`. Those stay out of commits.
fn writes_factory(workflow: &super::Workflow) -> bool {
    workflow.steps.iter().any(|step| {
        step.produces.iter().any(|decl| {
            matches!(&decl.source, super::ArtifactSource::File { path } if path == "factory" || path.starts_with("factory/"))
        })
    })
}

/// Drop files a session is about to write again, so a previous pass cannot be captured as
/// this one. Only an untracked file under `factory/`: a tracked file is the project's, and a
/// file elsewhere is not one of these notes.
fn clear_rewritten_files(workflow: &super::Workflow, step: &str, root: &Path, git: &Git) {
    for later in &workflow.steps {
        let super::Action::WaitSession {
            session: super::SessionRef::Step(source),
            ..
        } = &later.action
        else {
            continue;
        };
        if source != step {
            continue;
        }
        for decl in &later.produces {
            let super::ArtifactSource::File { path } = &decl.source else {
                continue;
            };
            if !under_factory(path) || tracked(git, root, path) {
                continue;
            }
            if let Err(why) = super::artifact::remove_file(root, path) {
                eprintln!("workflows: could not clear `{path}` before `{step}`: {why}");
            }
        }
    }
}

fn under_factory(path: &str) -> bool {
    let path = path.trim().trim_start_matches("./");
    path == "factory" || path.starts_with("factory/")
}

/// `git ls-files` prints a tracked path and nothing for an untracked or ignored one. When git
/// cannot say, the file stays: clearing it might delete the project's own work.
fn tracked(git: &Git, root: &Path, path: &str) -> bool {
    match git.run(root, &["ls-files", "--", path.trim()]) {
        Ok(listed) => !listed.trim().is_empty(),
        Err(_) => true,
    }
}

/// Keep `factory/` out of commits in this worktree only.
///
/// `info/exclude` belongs to the repository and is shared with every linked worktree, so a line
/// there would ignore `factory/` in the main checkout too. `factory/.gitignore` lives in this
/// worktree. `*` ignores the notes and the gitignore itself, and git still applies it here.
/// A failure leaves the prompt's request not to commit the notes; it does not fail the run.
fn exclude_factory_dir(root: &Path) {
    let path = root.join("factory/.gitignore");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing.lines().any(|line| line.trim() == "*") {
        return;
    }
    if std::fs::create_dir_all(root.join("factory")).is_err() {
        return;
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str("*\n");
    let _ = std::fs::write(&path, next);
}

fn is_applied_decision(change: &Move) -> bool {
    change.outcome.as_deref().is_some_and(|raw| {
        serde_json::from_str::<StepOutcome>(raw)
            .ok()
            .is_some_and(|outcome| outcome.decision.is_some())
    })
}

/// Open the wait and issue the first code when an approval starts waiting. A notification
/// that cannot be shown does not fail the step. The code is not written into the step.
/// A question opened since the last tick, or reopened for another round. The one-time code
/// is issued the same way as an approval, including when `notify` is `never`: a terminal
/// still has to present it. The notification is what stays quiet.
fn tell_open_questions(
    driver: &Driver,
    store: &Store,
    hands: &dyn Hands,
    run: &WorkflowRunRow,
) -> IpcResult<()> {
    // A lookup error is told anyway: missing a notice is worse than an extra one.
    // The code is issued either way, so `ys workflow answer` is not left with no code.
    let tell = store.tell_workflow_approval(&run.id).unwrap_or(true);
    for wait in store.workflow_waits_unapplied()? {
        if wait.run_id != run.id || wait.kind != super::waits::KIND_QUESTION {
            continue;
        }
        if wait.answer.is_some() || wait.applied_at.is_some() {
            continue;
        }
        let present = store
            .answer_code_present(&run.id, &wait.step_id)
            .unwrap_or(false);
        if wait.notified_at.is_some() && present && driver.holds_code(&run.id, &wait.step_id) {
            continue;
        }
        let Some((code, had)) = driver.issue_answer_code(store, &run.id, &wait.step_id, false)?
        else {
            continue;
        };
        hands.approval_code(&run.id, &wait.step_id, &code);
        if !tell {
            continue;
        }
        let replaced = had && wait.notified_at.is_some();
        let title = if replaced {
            super::waits::TITLE_ANSWER_REPLACED
        } else {
            super::waits::TITLE_ANSWER
        };
        let _ = hands.notify(
            title,
            Some(&approval_body(
                run,
                &wait.step_id,
                wait.prompt.as_deref(),
                shown_code(hands, Some(&code)),
                replaced,
            )),
        );
        // The phone is told that a question is open. The one-time code stays on the computer.
        note_mobile(
            store,
            crate::notify::workflow_question(&run.id, &wait.step_id, wait.created_at),
        );
        if wait.notified_at.is_none() {
            store.mark_workflow_wait_notified(&run.id, &wait.step_id)?;
        }
    }
    Ok(())
}

fn follow_approval(
    driver: &Driver,
    store: &Store,
    hands: &dyn Hands,
    workflow: &super::Workflow,
    run: &WorkflowRunRow,
    change: &Move,
) -> IpcResult<()> {
    let Some(step) = workflow.steps.iter().find(|step| step.id == change.step) else {
        return Ok(());
    };
    let approve = matches!(step.action, super::Action::Approve { .. });
    if !(approve && change.to == StepStatus::Waiting && change.from.contains(&StepStatus::Pending))
    {
        return Ok(());
    }
    let prompt = change
        .outputs
        .as_deref()
        .and_then(|raw| engine::parse_outputs(raw).get("prompt").cloned());
    open_approval_wait(store, run, step, prompt.as_deref())?;
    let Some((code, _)) = driver.issue_approval_code(store, &run.id, &step.id, false)? else {
        return Ok(());
    };
    hands.approval_code(&run.id, &step.id, &code);
    let told = store
        .workflow_wait(&run.id, &step.id)?
        .and_then(|wait| wait.notified_at)
        .is_some();
    // A lookup error is told anyway: missing a notice is worse than an extra one.
    let tell = store.tell_workflow_approval(&run.id).unwrap_or(true);
    if tell && !told {
        let _ = hands.notify(
            super::waits::TITLE_WAITING,
            Some(&approval_body(
                run,
                &step.id,
                prompt.as_deref(),
                shown_code(hands, Some(&code)),
                false,
            )),
        );
        if let Some(wait) = store.workflow_wait(&run.id, &step.id)? {
            note_mobile(
                store,
                crate::notify::workflow_approval(&run.id, &step.id, wait.created_at),
            );
        }
        store.mark_workflow_wait_notified(&run.id, &step.id)?;
    }
    Ok(())
}

/// The first look after this process started. A waiting approval whose code is not in memory
/// is given a new one, because the old plaintext died with the last process, and the person
/// is told. A decision already recorded is not given a code and is not told again: the engine
/// applies that decision once.
fn resume_human_waits(
    driver: &Driver,
    store: &Store,
    hands: &dyn Hands,
    workflow: &super::Workflow,
    run: &WorkflowRunRow,
) -> IpcResult<()> {
    let steps = store.workflow_steps(&run.id)?;
    for step in &workflow.steps {
        if !matches!(step.action, super::Action::Approve { .. }) {
            continue;
        }
        let Some(row) = steps.iter().find(|row| row.step_id == step.id) else {
            continue;
        };
        if row.status != StepStatus::Waiting.as_str() {
            continue;
        }
        let prompt = engine::parse_outputs(&row.outputs).get("prompt").cloned();
        open_approval_wait(store, run, step, prompt.as_deref())?;
        if store.workflow_approval(&run.id, &step.id)?.is_some() {
            continue;
        }
        let Some((code, had)) = driver.issue_approval_code(store, &run.id, &step.id, false)? else {
            continue;
        };
        hands.approval_code(&run.id, &step.id, &code);
        let tell = store.tell_workflow_approval(&run.id).unwrap_or(true);
        if !tell {
            continue;
        }
        let told = store
            .workflow_wait(&run.id, &step.id)?
            .and_then(|wait| wait.notified_at)
            .is_some();
        if had {
            let _ = hands.notify(
                super::waits::TITLE_REPLACED,
                Some(&approval_body(
                    run,
                    &step.id,
                    prompt.as_deref(),
                    shown_code(hands, Some(&code)),
                    true,
                )),
            );
            if let Some(wait) = store.workflow_wait(&run.id, &step.id)? {
                note_mobile(
                    store,
                    crate::notify::workflow_approval(&run.id, &step.id, wait.created_at),
                );
            }
        } else if !told {
            let _ = hands.notify(
                super::waits::TITLE_WAITING,
                Some(&approval_body(
                    run,
                    &step.id,
                    prompt.as_deref(),
                    shown_code(hands, Some(&code)),
                    false,
                )),
            );
            if let Some(wait) = store.workflow_wait(&run.id, &step.id)? {
                note_mobile(
                    store,
                    crate::notify::workflow_approval(&run.id, &step.id, wait.created_at),
                );
            }
            store.mark_workflow_wait_notified(&run.id, &step.id)?;
        }
    }
    Ok(())
}

fn open_approval_wait(
    store: &Store,
    run: &WorkflowRunRow,
    step: &super::Step,
    prompt: Option<&str>,
) -> IpcResult<()> {
    let phase = super::waits::Phase {
        workflow_id: run.workflow_id.clone(),
        workflow_name: run.workflow_name.clone(),
        workspace_name: run.workspace_name.clone(),
        step_id: step.id.clone(),
        action: "approve".to_owned(),
    };
    let refs = serde_json::to_string(&step.consumes).unwrap_or_else(|_| "[]".to_owned());
    store.open_workflow_wait(
        &run.id,
        &step.id,
        super::waits::KIND_APPROVAL,
        super::waits::BLOCKED_APPROVAL,
        super::waits::ACTION_APPROVAL,
        prompt,
        &refs,
        &phase.to_json(),
    )?;
    Ok(())
}

/// A failed run, for the phone. The error text stays on the computer.
/// Tell the phone a run failed. The notice is keyed by the run and by when it ended, read back
/// from the row: a run that is continued and fails again is told about again.
fn note_run_finished(store: &Store, run_id: &str, status: &str) {
    if status != "failed" {
        return;
    }
    let Ok(Some(run)) = store.workflow_run(run_id) else {
        return;
    };
    let ended_at = run.ended_at.unwrap_or(run.created_at);
    note_mobile(
        store,
        crate::notify::workflow_failure(run_id, &run.status, ended_at),
    );
}

/// Queue a phone notice. The desktop notification is separate and may still carry a code.
fn note_mobile(store: &Store, draft: Option<crate::notify::NoticeDraft>) {
    if let Some(draft) = draft {
        let _ = crate::notify::enqueue(store, &draft, now_ms());
    }
}

/// The code to put in a notification, or none when the setting leaves it out.
fn shown_code<'a>(hands: &'a dyn Hands, code: Option<&'a str>) -> Option<&'a str> {
    code.filter(|_| hands.include_approval_code())
}

fn approval_body(
    run: &WorkflowRunRow,
    step: &str,
    prompt: Option<&str>,
    code: Option<&str>,
    replaced: bool,
) -> String {
    super::waits::notice_body(
        &run.workflow_name,
        &run.workspace_name,
        step,
        prompt,
        code,
        replaced,
    )
}

/// The feedback on a request for changes, when this move is that decision.
fn changes_feedback(outcome: Option<&str>) -> Option<String> {
    let outcome: StepOutcome = serde_json::from_str(outcome?).ok()?;
    if outcome.decision == Some(ApprovalDecision::ChangesRequested) {
        Some(outcome.feedback.unwrap_or_default())
    } else {
        None
    }
}

/// The artifacts a request for changes stores, written in the same change as the step move.
/// `None` when this move is not that decision. Empty feedback becomes a sentence, so the
/// artifact is never blank.
fn changes_artifacts(
    store: &Store,
    run_id: &str,
    change: &Move,
) -> IpcResult<Option<Vec<NewWorkflowArtifact>>> {
    let Some(feedback) = changes_feedback(change.outcome.as_deref()) else {
        return Ok(None);
    };
    let definition = store
        .workflow_run(run_id)?
        .map(|run| run.definition)
        .unwrap_or_default();
    let step = super::parse(&definition).ok().and_then(|workflow| {
        workflow
            .steps
            .into_iter()
            .find(|step| step.id == change.step)
    });
    let outputs = change
        .outputs
        .as_deref()
        .map(engine::parse_outputs)
        .unwrap_or_default();
    let changes_name = artifact::changes_artifact_name(&change.step);
    let mut saved = Vec::new();
    if let Some(step) = step {
        for decl in &step.produces {
            let ArtifactSource::Output { field } = &decl.source else {
                continue;
            };
            if decl.name == changes_name {
                continue;
            }
            let Some(text) = outputs.get(field).filter(|text| !text.trim().is_empty()) else {
                continue;
            };
            let Ok((body, truncated)) = artifact::require_text(&decl.name, text.clone()) else {
                continue;
            };
            saved.push(NewWorkflowArtifact {
                name: decl.name.clone(),
                source: "output".into(),
                detail: Some(field.clone()),
                body,
                truncated,
            });
        }
    }
    let body = if feedback.trim().is_empty() {
        "Changes requested.".to_owned()
    } else {
        feedback
    };
    let (body, truncated) = artifact::require_text(&changes_name, body)
        .unwrap_or(("Changes requested.".to_owned(), false));
    saved.push(NewWorkflowArtifact {
        name: changes_name,
        source: "decision".into(),
        detail: Some("changes_requested".into()),
        body,
        truncated,
    });
    Ok(Some(saved))
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
