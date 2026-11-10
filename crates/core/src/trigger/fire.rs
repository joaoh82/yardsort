//! What becomes of something that happened.
//!
//! Three steps, each a function over rows that is handed the time:
//!
//! - [`observe`] takes one [`Observation`] and adds it to every enabled trigger it matches: to
//!   that trigger's open fire for the same subject, or to a new one, due when the trigger's
//!   quiet time after it is up. [`went_busy`] lets go of what was gathering on an agent's
//!   settling when the agent goes back to work.
//! - [`tick`] decides every fire that is due: whether it may start a run, and if so asks for it
//!   at the door a person would use. A run of the same thing in the way makes it wait; it is
//!   decided again at every tick until that run ends.
//! - [`settle`] copies the endings of the runs fires started onto their lines, and says what to
//!   tell the user.
//!
//! Where observations come from is [`super::sources`]. Neither reads the clock, starts a thread
//! or shows anything. The app calls them from its trigger thread and does the showing.

use std::collections::BTreeMap;
use std::path::Path;

use super::{
    events_of, not_found, test_of, workflow_waits_for_ever, ExitFilter, Filters, FromFilter,
    OutcomeFilter, ReviewStateFilter, Subject, TriggerEvent, VerdictFilter, MOST_DEPTH,
    MOST_GATHER_MS, RATE_PER_HOUR,
};
use crate::error::IpcResult;
use crate::forge::ReviewState;
use crate::git::Git;
use crate::schedule::{ScheduleNotify, ScheduleTarget};
use crate::store::{FireDecision, NewTriggerFire, Store, TriggerFireRow, TriggerRow};
use crate::workflow::runs::Look;

/// Something that happened, as a trigger hears of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub event: TriggerEvent,
    pub workspace_id: String,
    /// The conversation record, for an agent's events and a done signal said in one.
    pub session_id: Option<String>,
    /// The agent's terminal and its agent run, for an agent's events: what tells an agent a
    /// workflow started from one the user did.
    pub terminal: Option<String>,
    pub agent_run: Option<String>,
    pub at: i64,
    /// What it leaves for `{{ event.* }}`, besides `kind`.
    pub facts: BTreeMap<String, String>,
    /// The depth a run it starts would have.
    pub depth: i64,
    /// For a run that ended: the workflow's or the test's id, and how it ended. For an exit:
    /// `exited` or `failed`. For a pull request opening: `open`, `draft`, or `ready` when a
    /// draft was marked ready.
    pub target_id: Option<String>,
    pub status: Option<String>,
    /// For a review or a comment: who posted it, and what a review concluded.
    pub post: Option<Post>,
    /// For a pull request's event: the link of the one the workspace tracks in another
    /// repository it is about. `None` for the workspace's own pull request, and every other
    /// event.
    pub tracked: Option<String>,
}

/// A review or a comment on a pull request, as the filters see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    /// By login. `None` for an account that no longer exists.
    pub author: Option<String>,
    /// Posted as the account `gh` uses: the user's, and so every agent's.
    pub mine: bool,
    pub state: Option<ReviewState>,
}

impl Observation {
    /// What it is as a person writes it, and as `{{ event.kind }}` says: `pr.review`, or
    /// `custom.<name>`.
    pub fn kind(&self) -> String {
        match (self.event, self.facts.get("name")) {
            (TriggerEvent::Custom, Some(name)) => format!("{}{name}", super::CUSTOM_PREFIX),
            (event, _) => event.as_str().to_owned(),
        }
    }
}

impl Post {
    fn by_a_bot(&self) -> bool {
        self.author
            .as_deref()
            .is_some_and(|login| login.ends_with("[bot]"))
    }
}

/// What a trigger counts its runs for: one agent session, one pull request, or the workspace.
/// One tracked elsewhere is counted by its link, which no number of the workspace's own can
/// be.
fn subject_of(seen: &Observation) -> Option<String> {
    match seen.event.subject() {
        Subject::Session => seen.session_id.as_ref().map(|id| format!("session:{id}")),
        Subject::PullRequest => match &seen.tracked {
            Some(url) => Some(format!("pr:{url}")),
            None => seen.facts.get("number").map(|n| format!("pr:{n}")),
        },
        Subject::Workspace => Some(format!("workspace:{}", seen.workspace_id)),
    }
}

/// The link of the pull request tracked elsewhere a fire's subject is, if it is one.
pub fn tracked_subject(subject: &str) -> Option<&str> {
    subject
        .strip_prefix("pr:")
        .filter(|rest| rest.starts_with("https://") || rest.starts_with("http://"))
}

/// Whether a review or a comment is one the filters let through. Posts from the user's account
/// under `from: anyone` are checked again in [`observe`], which knows what runs are going.
fn post_holds(filters: &Filters, event: TriggerEvent, post: Option<&Post>) -> bool {
    let Some(post) = post else {
        return false;
    };
    if filters.from.unwrap_or_default() == FromFilter::Others && post.mine {
        return false;
    }
    if filters.no_bots && post.by_a_bot() {
        return false;
    }
    if event == TriggerEvent::PrReview && !filters.states.is_empty() {
        let state = match post.state {
            Some(ReviewState::Approved) => Some(ReviewStateFilter::Approved),
            Some(ReviewState::ChangesRequested) => Some(ReviewStateFilter::ChangesRequested),
            Some(ReviewState::Commented) => Some(ReviewStateFilter::Commented),
            Some(ReviewState::Dismissed) | None => None,
        };
        return state.is_some_and(|state| filters.states.contains(&state));
    }
    true
}

/// Whether the filters a trigger has hold for what was seen. Those that need git — new commits,
/// a clean worktree — are for when the fire is decided, after its quiet time.
fn holds(filters: &Filters, seen: &Observation) -> bool {
    // A trigger about pull requests elsewhere hears those, and one about the workspace's own
    // hears that one.
    if seen.event.of_a_pull_request() && filters.elsewhere != seen.tracked.is_some() {
        return false;
    }
    let status = seen.status.as_deref();
    match seen.event {
        TriggerEvent::AgentExited => match filters.exit {
            Some(ExitFilter::Failed) => status == Some("failed"),
            _ => true,
        },
        TriggerEvent::TestFinished => {
            filters
                .test
                .as_deref()
                .is_none_or(|test| seen.target_id.as_deref() == Some(test))
                && match filters.verdict {
                    Some(VerdictFilter::Passed) => status == Some("passed"),
                    Some(VerdictFilter::Failed) => matches!(status, Some("failed" | "errored")),
                    _ => true,
                }
        }
        TriggerEvent::WorkflowFinished => {
            filters
                .workflow
                .as_deref()
                .is_none_or(|workflow| seen.target_id.as_deref() == Some(workflow))
                && match filters.outcome {
                    Some(OutcomeFilter::Succeeded) => status == Some("succeeded"),
                    Some(OutcomeFilter::Failed) => matches!(status, Some("failed" | "cancelled")),
                    _ => true,
                }
        }
        TriggerEvent::PrOpened => match status {
            Some("draft") => filters.drafts,
            // A trigger that took drafts was told when it opened.
            Some("ready") => !filters.drafts,
            _ => true,
        },
        TriggerEvent::PrReview | TriggerEvent::PrComment => {
            post_holds(filters, seen.event, seen.post.as_ref())
        }
        TriggerEvent::Custom => seen
            .facts
            .get("name")
            .is_some_and(|name| filters.names.contains(name)),
        TriggerEvent::WorkspaceDone
        | TriggerEvent::AgentSettled
        | TriggerEvent::PrChecksFailed
        | TriggerEvent::PrChecksPassed
        | TriggerEvent::PrConflicted
        | TriggerEvent::PrMerged
        | TriggerEvent::PrClosed => true,
    }
}

/// Whether a run `trigger` started for `subject` has not ended yet.
fn own_run_going(store: &Store, trigger_id: &str, subject: &str) -> IpcResult<bool> {
    Ok(store
        .unsettled_trigger_fires()?
        .iter()
        .any(|line| line.trigger_id == trigger_id && line.subject == subject))
}

/// Add what was seen to every enabled trigger in its workspace that listens for it. Returns how
/// many took it.
pub fn observe(store: &Store, seen: &Observation) -> IpcResult<usize> {
    let Some(subject) = subject_of(seen) else {
        return Ok(0);
    };
    let mut by_workflow: Option<bool> = None;
    let mut taken = 0;
    for trigger in store.workspace_triggers(&seen.workspace_id)? {
        if seen.at < trigger.since || !events_of(&trigger).contains(&seen.event) {
            continue;
        }
        if seen.event.of_a_session() {
            // A session trigger hears its own agent only.
            if trigger
                .session_id
                .as_deref()
                .is_some_and(|own| seen.session_id.as_deref() != Some(own))
            {
                continue;
            }
            // An agent a workflow run started — a reviewer — is that run's, not the user's.
            let started = match by_workflow {
                Some(started) => started,
                None => {
                    let started = store
                        .started_by_workflow(seen.terminal.as_deref(), seen.agent_run.as_deref())?;
                    by_workflow = Some(started);
                    started
                }
            };
            if started {
                continue;
            }
        }
        if !holds(&Filters::parse(&trigger.filters), seen) {
            continue;
        }
        // Under `from: anyone`, what the user's account posts while this trigger's own run is
        // going is the agent answering.
        if seen.post.as_ref().is_some_and(|post| post.mine)
            && own_run_going(store, &trigger.id, &subject)?
        {
            continue;
        }
        let kind = seen.kind();
        let mut facts = seen.facts.clone();
        facts.insert("kind".into(), kind.clone());
        // Each custom name is its own milestone, with its own note: two names in one look
        // start two runs, not one that knows only the last.
        let apart = (seen.event == TriggerEvent::Custom).then_some(kind.as_str());
        match store.open_trigger_fire(&trigger.id, &subject, apart)? {
            Some(open) => {
                if seen.post.is_some() {
                    let gathered: BTreeMap<String, String> =
                        serde_json::from_str(&open.facts).unwrap_or_default();
                    facts = super::watch::gather_posts(&gathered, &facts);
                }
                let facts = serde_json::to_string(&facts).unwrap_or_else(|_| "{}".to_owned());
                // Moved later by every event, but never past the most a fire gathers.
                let due = (seen.at + trigger.quiet_ms).min(open.first_at + MOST_GATHER_MS);
                store.gather_into_trigger_fire(
                    &open.id,
                    &kind,
                    due.max(open.first_at),
                    &facts,
                    seen.depth,
                )?;
            }
            None => {
                let facts = serde_json::to_string(&facts).unwrap_or_else(|_| "{}".to_owned());
                store.open_new_trigger_fire(&NewTriggerFire {
                    trigger_id: &trigger.id,
                    subject: &subject,
                    event: &kind,
                    at: seen.at,
                    due_at: seen.at + trigger.quiet_ms,
                    facts: &facts,
                    depth: seen.depth,
                })?;
            }
        }
        taken += 1;
    }
    Ok(taken)
}

/// An agent went back to work: a fire still gathering on its settling did not stay quiet for
/// its trigger's quiet time, and goes without a line. Returns how many went.
pub fn went_busy(store: &Store, session_id: &str) -> IpcResult<usize> {
    let mut dropped = 0;
    let subject = format!("session:{session_id}");
    for fire in store.gathering_trigger_fires(&subject, TriggerEvent::AgentSettled.as_str())? {
        if store.drop_trigger_fire(&fire.id)? {
            dropped += 1;
        }
    }
    Ok(dropped)
}

/// A fire that was decided: the trigger as it was, and the line written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fired {
    pub trigger: TriggerRow,
    pub fire: TriggerFireRow,
    /// What to tell the user about it now: that it could not start its run, that there was no
    /// one for its run to tell, or that it has started all the runs its cap allows. Said once
    /// while it lasts.
    pub notice: Option<Notice>,
}

/// Something to tell the user about a trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub trigger_id: String,
    pub title: String,
    pub body: String,
}

/// What became of a fire that was due.
enum Decision {
    /// It turned out not to count: no line.
    Drop,
    /// The subject is capped and its last line says so already: that line takes the events.
    Fold(String),
    Decided {
        outcome: &'static str,
        run_id: Option<String>,
        note: Option<String>,
    },
}

fn refused(note: impl Into<String>) -> Decision {
    Decision::Decided {
        outcome: "refused",
        run_id: None,
        note: Some(note.into()),
    }
}

/// Decide every fire that is due at `now`. A fire is decided once, by whoever gets there first;
/// a second caller finds nothing left to do.
pub fn tick(
    store: &Store,
    data_dir: &Path,
    look: &dyn Look,
    git: &Git,
    now: i64,
) -> IpcResult<Vec<Fired>> {
    let mut fired = Vec::new();
    for fire in store.due_trigger_fires(now)? {
        let Some(trigger) = store.trigger(&fire.trigger_id)? else {
            continue;
        };
        match decide(store, data_dir, look, git, &trigger, &fire, now)? {
            Decision::Drop => {
                if fire.outcome == "gathering" {
                    store.drop_trigger_fire(&fire.id)?;
                } else {
                    store.decide_trigger_fire(
                        &fire.id,
                        &fire.outcome,
                        &FireDecision {
                            outcome: "skipped",
                            run_id: None,
                            note: Some("The trigger was switched off."),
                            counted: false,
                            at: now,
                        },
                    )?;
                }
            }
            Decision::Fold(line) => {
                if store.drop_trigger_fire(&fire.id)? {
                    store.add_to_trigger_fire(&line, fire.events_count)?;
                }
            }
            Decision::Decided {
                outcome,
                run_id,
                note,
            } => {
                if outcome == "waiting" && fire.outcome == "waiting" {
                    continue;
                }
                let previous = store
                    .trigger_fires(&trigger.id, 5)?
                    .into_iter()
                    .find(|line| line.id != fire.id && line.decided_at.is_some());
                let decided = store.decide_trigger_fire(
                    &fire.id,
                    &fire.outcome,
                    &FireDecision {
                        outcome,
                        run_id: run_id.as_deref(),
                        note: note.as_deref(),
                        counted: outcome == "started",
                        at: now,
                    },
                )?;
                if !decided {
                    continue;
                }
                let notice = notice_for(&trigger, outcome, note.as_deref(), previous.as_ref());
                let fire = store.trigger_fire(&fire.id)?.ok_or_else(not_found)?;
                fired.push(Fired {
                    trigger: trigger.clone(),
                    fire,
                    notice,
                });
            }
        }
    }
    Ok(fired)
}

/// What to tell the user about a fire that could not start its run: once, not at every fire
/// while the cause is still there.
fn notice_for(
    trigger: &TriggerRow,
    outcome: &str,
    note: Option<&str>,
    previous: Option<&TriggerFireRow>,
) -> Option<Notice> {
    if ScheduleNotify::parse(&trigger.notify) == Some(ScheduleNotify::Never) {
        return None;
    }
    if !matches!(outcome, "refused" | "capped" | "skipped") {
        return None;
    }
    if previous.is_some_and(|line| line.outcome == outcome) {
        return None;
    }
    let title = match outcome {
        "capped" => format!("{} has stopped", trigger.name),
        "skipped" => format!("{} had no one to tell", trigger.name),
        _ => format!("{} could not start", trigger.name),
    };
    Some(Notice {
        trigger_id: trigger.id.clone(),
        title,
        body: note.unwrap_or_default().to_owned(),
    })
}

fn decide(
    store: &Store,
    data_dir: &Path,
    look: &dyn Look,
    git: &Git,
    trigger: &TriggerRow,
    fire: &TriggerFireRow,
    now: i64,
) -> IpcResult<Decision> {
    if !trigger.enabled {
        return Ok(Decision::Drop);
    }
    let Some(workspace_id) = trigger.workspace_id.as_deref() else {
        // No later fire can do better with no workspace, so it stops listening.
        store.set_trigger_enabled(&trigger.id, false, now)?;
        return Ok(refused(format!(
            "`{}` was deleted, so this has nowhere to run.",
            trigger.workspace_name
        )));
    };
    let Some(workspace) = store.workspace(workspace_id)? else {
        return Ok(refused("The workspace is not there."));
    };
    if workspace.archived {
        return Ok(refused(format!(
            "`{}` is archived. Restore it for this trigger to start anything.",
            workspace.name
        )));
    }
    if let Some(session_id) = trigger.session_id.as_deref() {
        let running = store.session(session_id)?.is_some_and(|s| s.running);
        if !running {
            return Ok(Decision::Decided {
                outcome: "skipped",
                run_id: None,
                note: Some(
                    "Its agent session was not running, so there was no one to tell.".into(),
                ),
            });
        }
    }
    let mut facts: BTreeMap<String, String> = serde_json::from_str(&fire.facts).unwrap_or_default();

    // The filters that need git, now that the quiet time is up.
    let filters = Filters::parse(&trigger.filters);
    if fire.event == TriggerEvent::AgentSettled.as_str() && (filters.new_commits || filters.clean) {
        let root = Path::new(&workspace.path);
        let head = git.commit_id(root, "HEAD").ok().flatten();
        if filters.clean && !is_clean(git, root) {
            return Ok(Decision::Drop);
        }
        if filters.new_commits {
            // The commit it was on the last time this trigger started a run for this agent.
            let last_head = store
                .trigger_fires(&trigger.id, crate::store::TRIGGER_FIRES_KEPT)?
                .into_iter()
                .find(|line| line.subject == fire.subject && line.outcome == "started")
                .and_then(|line| {
                    serde_json::from_str::<BTreeMap<String, String>>(&line.facts)
                        .ok()?
                        .remove("head")
                });
            let moved = match (&last_head, &head) {
                (Some(before), Some(now)) => before != now,
                // Never started a run for this session: commits its base does not have.
                (None, Some(_)) => ahead_of_base(git, root, workspace.base_branch.as_deref()),
                (_, None) => false,
            };
            if !moved {
                return Ok(Decision::Drop);
            }
        }
        if let Some(head) = head {
            facts.insert("head".into(), head);
            store.set_trigger_fire_facts(
                &fire.id,
                &serde_json::to_string(&facts).unwrap_or_else(|_| "{}".to_owned()),
            )?;
        }
    }

    let counted = store.counted_trigger_runs(&trigger.id, &fire.subject)?;
    if counted >= trigger.cap {
        if let Some(line) = store
            .last_trigger_fire(&trigger.id, &fire.subject)?
            .filter(|line| line.outcome == "capped")
        {
            return Ok(Decision::Fold(line.id));
        }
        return Ok(Decision::Decided {
            outcome: "capped",
            run_id: None,
            note: Some(format!(
                "It has started {counted} run{} for this, as many as it may. Reset it to let it \
                 go on.",
                if counted == 1 { "" } else { "s" }
            )),
        });
    }
    if store.trigger_runs_since(&trigger.id, now - 60 * 60_000)? >= RATE_PER_HOUR {
        return Ok(refused(format!(
            "It has started {RATE_PER_HOUR} runs in the last hour, the most it may."
        )));
    }
    if fire.depth > MOST_DEPTH {
        return Ok(refused(format!(
            "It was set off by a run that triggers started, {} deep. More than {MOST_DEPTH} \
             is taken for a loop.",
            fire.depth - 1
        )));
    }

    let inputs: BTreeMap<String, String> =
        serde_json::from_str(&trigger.inputs).unwrap_or_default();
    match ScheduleTarget::parse(&trigger.target_kind) {
        Some(ScheduleTarget::Workflow) => {
            // A smoke test it runs may have gained a step a person does since it was saved.
            if let Some(entry) = crate::workflow::find(data_dir, &trigger.target_id) {
                if let Some(workflow) = entry.workflow.as_ref() {
                    if let Some(why) = workflow_waits_for_ever(&workspace, workflow) {
                        return Ok(refused(why));
                    }
                    // A run that talks to the workspace's agent with none running would skip
                    // every step and call that a success. The event would be lost unseen.
                    let anyone = store.sessions(&workspace.id)?.iter().any(|s| s.running);
                    if crate::workflow::runs::needs_origin(workflow) && !anyone {
                        return Ok(Decision::Decided {
                            outcome: "skipped",
                            run_id: None,
                            note: Some(format!(
                                "No agent was running in `{}`, so there was no one to tell.",
                                workspace.name
                            )),
                        });
                    }
                }
            }
            // One about a pull request tracked elsewhere is about the one that set it off, read
            // now by its link: it must still be tracked, and open.
            let tracked = match tracked_subject(&fire.subject) {
                Some(url) => {
                    let Some(row) = store
                        .tracked_pull_requests(&workspace.id)?
                        .into_iter()
                        .find(|row| row.url == url)
                    else {
                        return Ok(refused(format!(
                            "`{}` no longer tracks {url}.",
                            workspace.name
                        )));
                    };
                    if store.workflow_active_in(&trigger.target_id, &workspace.id)? {
                        return Ok(waiting());
                    }
                    match look.tracked_pull_request(&workspace, &row) {
                        Ok(pr) => Some(pr),
                        Err(why) => return Ok(refused(why)),
                    }
                }
                None => None,
            };
            if store.workflow_active_in(&trigger.target_id, &workspace.id)? {
                return Ok(waiting());
            }
            facts.insert("count".into(), fire.events_count.to_string());
            let asked = crate::workflow::runs::queue(
                store,
                data_dir,
                &crate::workflow::runs::Request {
                    workflow_id: &trigger.target_id,
                    workspace_id: &workspace.id,
                    inputs: &inputs,
                    // The app is what queues it. Which trigger asked is in the trigger's
                    // history.
                    requested_by: "app",
                    // A smoke test it runs may wait for the user's approval; nothing else.
                    unattended: false,
                    origin_record: trigger.session_id.as_deref(),
                    depth: fire.depth,
                    event: Some(&facts),
                    tracked: tracked.as_ref(),
                },
                look,
            );
            Ok(queued(asked))
        }
        Some(ScheduleTarget::Test) => {
            let test = match test_of(&workspace, &trigger.target_id) {
                Ok(test) => test,
                Err(error) => return Ok(refused(error.message)),
            };
            if let Some(why) = crate::schedule::needs_a_person(&test) {
                return Ok(refused(why));
            }
            if store.test_active_in(&trigger.target_id, &workspace.id)? {
                return Ok(waiting());
            }
            let asked = crate::smoke::runs::queue(
                store,
                &crate::smoke::runs::Request {
                    test_id: &trigger.target_id,
                    workspace_id: &workspace.id,
                    inputs: &inputs,
                    requested_by: "app",
                    session_record_id: None,
                    // A file nobody approved waits in the open, with the trigger saying so and
                    // the user told (see `settle`).
                    unattended: false,
                },
            )
            .map(|queued| queued.id);
            Ok(queued(asked))
        }
        None => Ok(refused(format!(
            "This Yardsort does not know how to start a `{}`.",
            trigger.target_kind
        ))),
    }
}

fn waiting() -> Decision {
    Decision::Decided {
        outcome: "waiting",
        run_id: None,
        note: None,
    }
}

/// What the door said, as a decision. A run of the same thing already going is one to wait
/// for: an event, unlike a schedule's time, does not come again.
fn queued(asked: IpcResult<String>) -> Decision {
    match asked {
        Ok(run_id) => Decision::Decided {
            outcome: "started",
            run_id: Some(run_id),
            note: None,
        },
        Err(error) if error.code.ends_with("_already_running") => waiting(),
        Err(error) => refused(error.message),
    }
}

/// Nothing in the worktree is uncommitted. A worktree git cannot answer for is not clean.
fn is_clean(git: &Git, root: &Path) -> bool {
    git.run(root, &["status", "--porcelain"])
        .is_ok_and(|out| out.trim().is_empty())
}

/// The branch has commits its base does not. Without a base to compare with, any commit counts.
fn ahead_of_base(git: &Git, root: &Path, base: Option<&str>) -> bool {
    let base = base
        .map(str::to_owned)
        .or_else(|| git.default_branch(root).ok().flatten());
    match base {
        Some(base) => git
            .ahead_behind(root, &base)
            .ok()
            .flatten()
            .is_none_or(|(ahead, _)| ahead > 0),
        None => true,
    }
}

/// Lift a trigger's cap. How many runs stopped counting.
pub fn reset(store: &Store, id: &str) -> IpcResult<usize> {
    store.trigger(id)?.ok_or_else(not_found)?;
    Ok(store.reset_trigger(id)?)
}

/// What [`settle`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settled {
    /// Whether any line of any history changed, so lists should be read again.
    pub changed: bool,
    pub notices: Vec<Notice>,
}

/// Look at the runs fires started and have not seen end. One that ended has its ending copied
/// onto its line, and a notice when the trigger asks for one. One that waits for the user to
/// approve its file gets a notice, once, whatever the trigger says: nobody else will tell them.
pub fn settle(store: &Store) -> IpcResult<Settled> {
    let mut settled = Settled::default();
    for line in store.unsettled_trigger_fires()? {
        let Some(trigger) = store.trigger(&line.trigger_id)? else {
            continue;
        };
        let notify = ScheduleNotify::parse(&trigger.notify).unwrap_or_default();
        let Some(run_id) = line.run_id.as_deref() else {
            settled.changed |= store.settle_trigger_fire(&line.id, "gone", false)?;
            continue;
        };
        let (status, reason, waiting) = match ScheduleTarget::parse(&trigger.target_kind) {
            Some(ScheduleTarget::Workflow) => match store.workflow_run(run_id)? {
                Some(run) if run.active() => continue,
                Some(run) => {
                    let reason = run.error.clone().or_else(|| {
                        store
                            .workflow_steps(run_id)
                            .unwrap_or_default()
                            .iter()
                            .find(|step| step.status == "failed")
                            .map(|step| failed_step(&step.step_id, step.note.as_deref()))
                    });
                    (run.status, reason, false)
                }
                None => ("gone".to_owned(), None, false),
            },
            Some(ScheduleTarget::Test) => match store.test_run(run_id)? {
                Some(run) if run.status == "awaiting_approval" => (run.status, None, true),
                Some(run) if run.active() => continue,
                Some(run) => {
                    let reason = run.error.clone().or_else(|| {
                        store
                            .test_steps(run_id)
                            .unwrap_or_default()
                            .iter()
                            .find(|step| step.status == "failed")
                            .map(|step| failed_step(&step.step_id, step.note.as_deref()))
                    });
                    (run.status, reason, false)
                }
                None => ("gone".to_owned(), None, false),
            },
            None => ("gone".to_owned(), None, false),
        };
        if waiting {
            if !line.told_waiting && store.mark_trigger_fire_told_waiting(&line.id)? {
                settled.changed = true;
                if notify != ScheduleNotify::Never {
                    settled.notices.push(Notice {
                        trigger_id: trigger.id.clone(),
                        title: format!("{} is waiting for your approval", trigger.name),
                        body: format!(
                            "The smoke test `{}` is new, or has changed since you approved it. \
                             Open Yardsort to look at it and run it.",
                            trigger.target_id
                        ),
                    });
                }
            }
            continue;
        }
        let failed = matches!(status.as_str(), "failed" | "errored");
        let tell = status != "gone"
            && match notify {
                ScheduleNotify::Always => true,
                ScheduleNotify::Failure => failed,
                ScheduleNotify::Never => false,
            };
        if !store.settle_trigger_fire(&line.id, &status, tell)? {
            continue;
        }
        settled.changed = true;
        if tell {
            let what =
                ScheduleTarget::parse(&trigger.target_kind).map_or("run", ScheduleTarget::words);
            let place = format!(
                "The {what} `{}` in `{}`.",
                trigger.target_id, trigger.workspace_name
            );
            settled.notices.push(Notice {
                trigger_id: trigger.id.clone(),
                title: format!(
                    "{} {}",
                    trigger.name,
                    crate::schedule::fire::ending(&status)
                ),
                body: reason.filter(|_| failed).unwrap_or(place),
            });
        }
    }
    Ok(settled)
}

fn failed_step(step_id: &str, note: Option<&str>) -> String {
    match note {
        Some(note) => format!("`{step_id}`: {note}"),
        None => format!("The step `{step_id}` failed."),
    }
}

#[cfg(test)]
mod tests;
