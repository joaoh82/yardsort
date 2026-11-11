use std::collections::{BTreeMap, HashMap};

use super::*;
use crate::workflow::clarify::{ClarificationState, PendingAnswer};
use crate::workflow::{parse, PrActivity};

const WORKFLOW: &str = r#"id: t
name: T
version: 1
trigger:
  kind: manual
inputs:
  - id: who
    kind: harness
steps:
  - id: start
    action: start_session
    harness: "{{ inputs.who }}"
    prompt: "Work on {{ workspace.branch }}."
  - id: settle
    action: wait_session
    needs: [start]
    session: "{{ steps.start.session }}"
    timeout: 10m
  - id: tell
    action: notify
    needs: [settle]
    title: "{{ inputs.who }} is done ({{ steps.settle.outcome }})"
  - id: nudge
    action: send_to_session
    needs: [settle]
    session: "{{ steps.start.session }}"
    prompt: Now write the tests.
    timeout: 5m
"#;

struct Fake {
    now: i64,
    sessions: HashMap<String, SessionFacts>,
    origin: Option<String>,
    /// Reviews and comments on the pull request since the run started.
    posts: Vec<PrActivity>,
    /// Smoke test runs by id, as `run_test` would see them.
    test_runs: HashMap<String, TestRun>,
    /// Named artifacts already stored for this run. A consumer reads these at render time,
    /// so replacing a value is what a re-run looks like.
    artifacts: BTreeMap<String, String>,
    /// Decisions a person has recorded, by step id.
    approvals: HashMap<String, RecordedApproval>,
    /// Clarification rounds, by step id. Empty means the session finishing is not a question.
    clarifications: HashMap<String, ClarificationState>,
}

impl World for Fake {
    fn now(&self) -> i64 {
        self.now
    }
    fn session(&self, pty_id: &str) -> SessionFacts {
        self.sessions.get(pty_id).copied().unwrap_or_default()
    }
    fn var(&self, path: &[String]) -> Option<String> {
        if let [namespace, name] = path {
            if namespace == "artifacts" {
                return self.artifacts.get(name).cloned();
            }
        }
        match path.join(".").as_str() {
            "workspace.branch" => Some("fix-login".to_owned()),
            "pr.number" => Some("42".to_owned()),
            _ => None,
        }
    }
    fn origin(&self) -> Option<String> {
        self.origin.clone()
    }
    fn pr_activity(&self, kind: PrActivity) -> Option<PrSeen> {
        let count = self
            .posts
            .iter()
            .filter(|posted| kind == PrActivity::Any || **posted == kind)
            .count() as u32;
        Some(PrSeen {
            count,
            latest_url: (count > 0).then(|| "https://example.test/pull/42".to_owned()),
        })
    }
    fn test_run(&self, run_id: &str) -> Option<TestRun> {
        self.test_runs.get(run_id).cloned()
    }
    fn clarification(&self, step_id: &str) -> ClarificationState {
        self.clarifications
            .get(step_id)
            .cloned()
            .unwrap_or_default()
    }
    fn approval(&self, step_id: &str) -> Option<RecordedApproval> {
        self.approvals.get(step_id).cloned()
    }
}

/// A run and its world, with the store's rules for applying a plan.
struct Run {
    workflow: Workflow,
    inputs: Inputs,
    rows: Vec<WorkflowStepRow>,
    world: Fake,
    finished: Option<Finish>,
}

impl Run {
    fn new() -> Self {
        let workflow = parse(WORKFLOW).unwrap();
        let rows = workflow
            .steps
            .iter()
            .enumerate()
            .map(|(i, step)| WorkflowStepRow {
                run_id: "run".into(),
                step_id: step.id.clone(),
                position: i as i64,
                action: step.action.name().into(),
                status: "pending".into(),
                started_at: None,
                ended_at: None,
                outputs: "{}".into(),
                note: None,
                outcome: None,
                attempts: 0,
            })
            .collect();
        Self {
            workflow,
            inputs: [("who".to_owned(), "claude".to_owned())].into(),
            rows,
            world: Fake {
                now: 1_000_000,
                sessions: HashMap::new(),
                origin: None,
                posts: Vec::new(),
                test_runs: HashMap::new(),
                artifacts: BTreeMap::new(),
                approvals: HashMap::new(),
                clarifications: HashMap::new(),
            },
            finished: None,
        }
    }

    /// Build a run from a given workflow source, every step pending.
    fn from(source: &str) -> Self {
        let mut run = Self::new();
        run.workflow = parse(source).unwrap();
        run.rows = run
            .workflow
            .steps
            .iter()
            .enumerate()
            .map(|(i, step)| WorkflowStepRow {
                run_id: "run".into(),
                step_id: step.id.clone(),
                position: i as i64,
                action: step.action.name().into(),
                status: "pending".into(),
                started_at: None,
                ended_at: None,
                outputs: "{}".into(),
                note: None,
                outcome: None,
                attempts: 0,
            })
            .collect();
        run
    }

    /// One call to the engine, its moves applied. Returns the effects to carry out.
    fn advance(&mut self) -> Vec<Effect> {
        let plan = advance(&self.workflow, &self.inputs, &self.rows, &self.world);
        for change in &plan.moves {
            self.apply(change);
        }
        if plan.finish.is_some() {
            self.finished = plan.finish;
        }
        plan.effects
    }

    /// As `Store::move_workflow_step` does it: only from an expected state.
    fn apply(&mut self, change: &Move) -> bool {
        let now = self.world.now;
        let current = self
            .rows
            .iter()
            .find(|r| r.step_id == change.step)
            .and_then(|row| StepStatus::parse(&row.status))
            .unwrap();
        if !change.from.contains(&current) {
            return false;
        }
        if let Some(rewind) = &change.rewind {
            let seen = self
                .rows
                .iter()
                .find(|r| r.step_id == change.step)
                .map(|row| row.attempts)
                .unwrap_or(0);
            if seen != rewind.seen {
                return false;
            }
            let steps = rewind.steps.clone();
            let feedback = rewind.feedback.clone();
            for id in &steps {
                if let Some(target) = self
                    .rows
                    .iter_mut()
                    .find(|candidate| candidate.step_id == *id)
                {
                    target.status = "pending".into();
                    target.started_at = None;
                    target.ended_at = None;
                    target.outputs = "{}".into();
                    target.note = None;
                    target.outcome = None;
                }
                self.world.approvals.remove(id);
            }
            let reset: std::collections::BTreeSet<&str> =
                steps.iter().map(String::as_str).collect();
            // A note from an earlier pass must not be handed out again. The version a
            // real run keeps is the store's; this map is only the body a consumer reads.
            self.world.artifacts.retain(|name, _| {
                crate::workflow::artifact::feedback_step(name).is_none_or(|id| !reset.contains(id))
            });
            for copy in feedback {
                self.world.artifacts.insert(copy.name, copy.body);
            }
            return true;
        }
        let row = self
            .rows
            .iter_mut()
            .find(|r| r.step_id == change.step)
            .unwrap();
        if current == StepStatus::Pending
            && matches!(
                change.to,
                StepStatus::Running | StepStatus::Waiting | StepStatus::Succeeded
            )
        {
            row.attempts += 1;
        }
        row.status = change.to.as_str().into();
        if change.to.is_final() {
            row.ended_at = Some(now);
        } else {
            row.started_at.get_or_insert(now);
        }
        if let Some(outputs) = &change.outputs {
            row.outputs = outputs.clone();
        }
        if let Some(note) = &change.note {
            row.note = Some(note.clone());
        }
        if change.clear_outcome {
            row.outcome = None;
        } else if let Some(outcome) = &change.outcome {
            row.outcome = Some(outcome.clone());
        }
        if let Some(copy) = &change.feedback {
            self.world
                .artifacts
                .insert(copy.name.clone(), copy.body.clone());
        }
        true
    }

    fn status(&self, step: &str) -> &str {
        &self.rows.iter().find(|r| r.step_id == step).unwrap().status
    }

    fn note(&self, step: &str) -> Option<&str> {
        self.rows
            .iter()
            .find(|r| r.step_id == step)
            .unwrap()
            .note
            .as_deref()
    }

    fn session(&mut self, pty: &str, facts: SessionFacts) {
        self.world.sessions.insert(pty.to_owned(), facts);
    }

    fn later(&mut self, ms: i64) {
        self.world.now += ms;
    }

    /// Start the agent and let it work, so `settle` is waiting on a busy session.
    fn started_and_working(&mut self) {
        assert_eq!(self.advance().len(), 1);
        self.working();
    }

    /// The agent `start` asked for is up and busy, so `settle` is waiting on it.
    fn working(&mut self) {
        self.session(
            "pty-1",
            SessionFacts {
                alive: true,
                busy: true,
                settled_at: None,
            },
        );
        let outputs: Outputs = [
            ("session".to_owned(), "pty-1".to_owned()),
            ("run".to_owned(), "run-1".to_owned()),
        ]
        .into();
        assert!(self.apply(&done("start", &outputs)));
        assert!(self.advance().is_empty());
        assert_eq!(self.status("settle"), "waiting");
    }
}

#[test]
fn a_run_goes_from_start_to_finish_with_two_steps_side_by_side() {
    let mut run = Run::new();
    let effects = run.advance();
    assert_eq!(
        effects,
        vec![Effect::Start {
            step: "start".into(),
            harness: "claude".into(),
            prompt: Some("Work on fix-login.".into()),
            model: None,
            effort: None,
            skip_memory: false,
        }]
    );
    assert_eq!(run.status("start"), "running");
    assert_eq!(
        run.status("settle"),
        "pending",
        "it waits for start to succeed"
    );

    run.working();
    run.later(60_000);
    let settled_at = run.world.now;
    run.session(
        "pty-1",
        SessionFacts {
            alive: true,
            busy: false,
            settled_at: Some(settled_at),
        },
    );
    let effects = run.advance();
    assert_eq!(run.status("settle"), "succeeded");
    assert_eq!(
        effects,
        vec![
            Effect::Notify {
                step: "tell".into(),
                title: "claude is done (settled)".into(),
                body: None,
            },
            Effect::Paste {
                step: "nudge".into(),
                pty_id: "pty-1".into(),
                text: "Now write the tests.".into(),
            },
        ],
        "both steps that need `settle` start together"
    );
    assert_eq!(run.finished, None);
    run.apply(&done("tell", &Outputs::new()));
    run.apply(&done("nudge", &Outputs::new()));
    assert!(run.advance().is_empty());
    assert_eq!(run.finished, Some(Finish::Succeeded));
}

#[test]
fn an_effect_is_asked_for_once_however_often_the_engine_is_called() {
    let mut run = Run::new();
    assert_eq!(run.advance().len(), 1);
    assert!(
        run.advance().is_empty(),
        "start is running; nothing new to do"
    );
    assert!(run.advance().is_empty());
}

#[test]
fn a_failed_step_skips_everything_after_it_and_fails_the_run() {
    let mut run = Run::new();
    run.advance();
    run.apply(&failed("start", "`claude` was not found on PATH."));
    assert!(run.advance().is_empty());
    for step in ["settle", "tell", "nudge"] {
        assert_eq!(run.status(step), "skipped", "{step}");
    }
    assert_eq!(
        run.note("settle"),
        Some("Skipped: `start` did not succeed.")
    );
    assert_eq!(run.note("tell"), Some("Skipped: `settle` did not succeed."));
    assert_eq!(
        run.finished,
        Some(Finish::Failed(
            "Step `start` failed: `claude` was not found on PATH.".into()
        ))
    );
}

#[test]
fn quiet_before_the_step_began_is_not_settled() {
    let mut run = Run::new();
    run.advance();
    // The agent went quiet once while starting up, before anyone was waiting for it.
    run.session(
        "pty-1",
        SessionFacts {
            alive: true,
            busy: false,
            settled_at: Some(run.world.now - 1),
        },
    );
    let outputs: Outputs = [("session".to_owned(), "pty-1".to_owned())].into();
    run.apply(&done("start", &outputs));
    run.later(1_000);
    run.advance();
    assert_eq!(run.status("settle"), "waiting");
}

#[test]
fn an_agent_that_exits_ends_the_wait_and_the_message_to_it_is_skipped() {
    let mut run = Run::new();
    run.started_and_working();
    run.session("pty-1", SessionFacts::default());
    assert_eq!(
        run.advance(),
        vec![Effect::Notify {
            step: "tell".into(),
            title: "claude is done (exited)".into(),
            body: None,
        }]
    );
    assert_eq!(run.status("nudge"), "skipped");
    assert_eq!(
        run.note("nudge"),
        Some("Skipped: the session had already ended.")
    );
    run.apply(&done("tell", &Outputs::new()));
    run.advance();
    assert_eq!(
        run.finished,
        Some(Finish::Succeeded),
        "a skipped message is not a failure"
    );
}

#[test]
fn a_busy_agent_is_never_written_to_and_waiting_for_it_can_time_out() {
    let mut run = Run::new();
    run.started_and_working();
    // Settled, but it picked up work again before the message went out.
    run.session(
        "pty-1",
        SessionFacts {
            alive: true,
            busy: true,
            settled_at: Some(run.world.now),
        },
    );
    let effects = run.advance();
    assert!(
        effects.iter().all(|e| !matches!(e, Effect::Paste { .. })),
        "{effects:?}"
    );
    assert_eq!(run.status("nudge"), "waiting");

    run.later(4 * 60_000);
    assert!(run.advance().is_empty(), "still busy, not yet timed out");
    run.later(60_000);
    run.advance();
    assert_eq!(run.status("nudge"), "failed");
    assert_eq!(
        run.note("nudge"),
        Some("The session was still busy after 5m.")
    );
}

#[test]
fn a_waiting_message_goes_out_the_moment_the_agent_is_quiet() {
    let mut run = Run::new();
    run.started_and_working();
    run.session(
        "pty-1",
        SessionFacts {
            alive: true,
            busy: true,
            settled_at: Some(run.world.now),
        },
    );
    run.advance();
    run.later(30_000);
    run.session(
        "pty-1",
        SessionFacts {
            alive: true,
            busy: false,
            settled_at: Some(run.world.now),
        },
    );
    let effects = run.advance();
    assert_eq!(
        effects,
        vec![Effect::Paste {
            step: "nudge".into(),
            pty_id: "pty-1".into(),
            text: "Now write the tests.".into(),
        }]
    );
    assert_eq!(run.status("nudge"), "running");
}

#[test]
fn waiting_for_a_session_times_out_once() {
    let mut run = Run::new();
    run.started_and_working();
    run.later(10 * 60_000);
    run.advance();
    assert_eq!(run.status("settle"), "failed");
    assert_eq!(run.note("settle"), Some("Timed out after 10m."));
    assert!(run.advance().is_empty());
    assert!(matches!(run.finished, Some(Finish::Failed(ref why)) if why.contains("Timed out")));
}

#[test]
fn steps_interrupted_by_a_restart_fail_rather_than_run_twice() {
    let mut run = Run::new();
    run.advance();
    let recovered = recover(&run.rows);
    assert_eq!(recovered.len(), 1);
    assert!(run.apply(&recovered[0]));
    assert_eq!(run.status("start"), "failed");
    assert_eq!(
        run.note("start"),
        Some("Yardsort stopped while this step was running.")
    );
    run.advance();
    assert!(matches!(run.finished, Some(Finish::Failed(_))));
}

#[test]
fn a_prompt_that_places_the_memory_itself_does_not_get_it_twice() {
    let mut run = Run::new();
    run.workflow = parse(&WORKFLOW.replace(
        "prompt: \"Work on {{ workspace.branch }}.\"",
        "prompt: \"Work on it.\\n\\n{{ memory }}\"",
    ))
    .unwrap();
    match &run.advance()[0] {
        Effect::Start {
            skip_memory,
            prompt,
            ..
        } => {
            assert!(skip_memory, "the launcher must not append it again");
            assert_eq!(
                prompt.as_deref(),
                Some("Work on it.\n\n"),
                "no memory in this fake"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(
            Run::new().advance()[0],
            Effect::Start {
                skip_memory: false,
                ..
            }
        ),
        "without it, the launcher appends the memory as it always does"
    );
}

/// The shape of the built-in review: an agent reviews, the forge is watched for what it posted,
/// then the author's own agent is told.
const REVIEW: &str = r##"id: r
name: R
version: 1
trigger:
  kind: manual
steps:
  - id: review
    action: start_session
    harness: claude
    prompt: "Review #{{ pr.number }}"
  - id: posted
    action: wait_pr_activity
    needs: [review]
    kind: review
    timeout: 15m
  - id: tell_author
    action: send_to_session
    needs: [posted]
    session: origin
    prompt: "#{{ pr.number }} has a review: {{ steps.posted.latest_url }}"
"##;

fn review_run(origin: Option<&str>) -> Run {
    let mut run = Run::new();
    run.workflow = parse(REVIEW).unwrap();
    run.rows = run
        .workflow
        .steps
        .iter()
        .enumerate()
        .map(|(i, step)| WorkflowStepRow {
            run_id: "run".into(),
            step_id: step.id.clone(),
            position: i as i64,
            action: step.action.name().into(),
            status: "pending".into(),
            started_at: None,
            ended_at: None,
            outputs: "{}".into(),
            note: None,
            outcome: None,
            attempts: 0,
        })
        .collect();
    run.world.origin = origin.map(str::to_owned);
    match &run.advance()[0] {
        Effect::Start { prompt, .. } => assert_eq!(prompt.as_deref(), Some("Review #42")),
        other => panic!("{other:?}"),
    }
    run.apply(&done(
        "review",
        &[("session".to_owned(), "pty-r".to_owned())].into(),
    ));
    run
}

#[test]
fn the_forge_is_watched_until_a_post_of_the_kind_asked_for_appears() {
    let mut run = review_run(Some("pty-author"));
    run.session(
        "pty-author",
        SessionFacts {
            alive: true,
            busy: false,
            settled_at: None,
        },
    );
    assert!(run.advance().is_empty());
    assert_eq!(run.status("posted"), "waiting");
    run.world.posts.push(PrActivity::Comment);
    assert!(
        run.advance().is_empty(),
        "a comment is not the review asked for"
    );
    run.world.posts.push(PrActivity::Review);
    let effects = run.advance();
    assert_eq!(run.status("posted"), "succeeded");
    assert_eq!(
        effects,
        vec![Effect::Paste {
            step: "tell_author".into(),
            pty_id: "pty-author".into(),
            text: "#42 has a review: https://example.test/pull/42".into(),
        }],
        "the author's own agent is told, and where the review is"
    );
}

#[test]
fn a_review_that_never_comes_fails_the_wait_saying_so() {
    let mut run = review_run(Some("pty-author"));
    run.advance();
    run.later(15 * 60_000);
    run.advance();
    assert_eq!(run.status("posted"), "failed");
    assert_eq!(
        run.note("posted"),
        Some("No review appeared on the pull request within 15m.")
    );
    assert_eq!(run.status("tell_author"), "skipped");
}

#[test]
fn with_no_agent_of_its_own_in_the_workspace_the_author_step_is_skipped_saying_why() {
    let mut run = review_run(None);
    run.world.posts.push(PrActivity::Review);
    run.advance();
    assert_eq!(run.status("tell_author"), "skipped");
    assert_eq!(
        run.note("tell_author"),
        Some("Skipped: no agent was running in the workspace when the run started.")
    );
    assert_eq!(run.finished, Some(Finish::Succeeded));
}

const RUN_TEST_WF: &str = r#"id: t
name: T
version: 1
trigger:
  kind: manual
steps:
  - id: smoke
    action: run_test
    test: login
    timeout: 30m
"#;

#[test]
fn a_run_test_step_queues_the_run_waits_and_passes_with_it() {
    let mut run = Run::from(RUN_TEST_WF);
    // It begins by asking the driver to queue the run, and goes running.
    let effects = run.advance();
    assert_eq!(
        effects,
        vec![Effect::RunTest {
            step: "smoke".into(),
            test: "login".into(),
            inputs: Inputs::new(),
        }]
    );
    assert_eq!(run.status("smoke"), "running");
    // The driver queued it, leaving the run id; the step waits for the run to end.
    let queued_out: Outputs = [("run".to_owned(), "tr-1".to_owned())].into();
    assert!(run.apply(&queued("smoke", &queued_out)));
    assert_eq!(run.status("smoke"), "waiting");
    // While the run is still going, the step keeps waiting.
    run.world.test_runs.insert(
        "tr-1".into(),
        TestRun {
            status: "running".into(),
            summary: "going".into(),
        },
    );
    assert!(run.advance().is_empty());
    assert_eq!(run.status("smoke"), "waiting");
    // When it passes, the step succeeds and carries the run, verdict and summary.
    run.world.test_runs.insert(
        "tr-1".into(),
        TestRun {
            status: "passed".into(),
            summary: "passed  login  (6 of 6 steps passed)".into(),
        },
    );
    run.advance();
    assert_eq!(run.status("smoke"), "succeeded");
    let row = run.rows.iter().find(|r| r.step_id == "smoke").unwrap();
    let out = parse_outputs(&row.outputs);
    assert_eq!(out.get("run").map(String::as_str), Some("tr-1"));
    assert_eq!(out.get("verdict").map(String::as_str), Some("passed"));
    assert!(out.get("summary").unwrap().contains("passed  login"));
    assert_eq!(run.finished, Some(Finish::Succeeded));
}

#[test]
fn a_failed_test_succeeds_the_step_with_a_failed_verdict() {
    // A test that ran and failed is still a verdict: the step succeeds and carries `failed`, so
    // a later step can read it and act on the failure.
    let mut run = Run::from(RUN_TEST_WF);
    run.advance();
    let queued_out: Outputs = [("run".to_owned(), "tr-9".to_owned())].into();
    assert!(run.apply(&queued("smoke", &queued_out)));
    run.world.test_runs.insert(
        "tr-9".into(),
        TestRun {
            status: "failed".into(),
            summary: "failed  login  (2 of 6 steps passed)".into(),
        },
    );
    run.advance();
    assert_eq!(run.status("smoke"), "succeeded");
    let out = parse_outputs(&run.rows[0].outputs);
    assert_eq!(out.get("verdict").map(String::as_str), Some("failed"));
    assert!(out.get("summary").unwrap().contains("failed  login"));
    assert_eq!(run.note("smoke"), Some("The test failed."));
    let outcome = step_outcome(&run, "smoke");
    assert_eq!(outcome.verdict, StepVerdict::Failed);
    assert!(outcome.reason.unwrap().contains("failed  login"));
    assert_eq!(run.finished, Some(Finish::Succeeded));
}

const GATED: &str = r#"id: t
name: T
version: 1
trigger:
  kind: manual
steps:
  - id: smoke
    action: run_test
    test: login
    gate: true
    timeout: 30m
  - id: tell
    action: notify
    needs: [smoke]
    title: Tests passed
"#;

fn step_outcome(run: &Run, step: &str) -> StepOutcome {
    let row = run.rows.iter().find(|r| r.step_id == step).unwrap();
    serde_json::from_str(row.outcome.as_deref().expect("an outcome")).unwrap()
}

fn finish_test(run: &mut Run, id: &str, status: &str, summary: &str) {
    run.advance();
    let queued_out: Outputs = [("run".to_owned(), id.to_owned())].into();
    assert!(run.apply(&queued("smoke", &queued_out)));
    run.world.test_runs.insert(
        id.to_owned(),
        TestRun {
            status: status.to_owned(),
            summary: summary.to_owned(),
        },
    );
    run.advance();
}

#[test]
fn a_failed_verdict_on_a_gate_stops_the_run_with_the_reason() {
    let mut run = Run::from(GATED);
    finish_test(
        &mut run,
        "tr-gate",
        "failed",
        "failed  login  (2 of 6 steps passed)",
    );
    assert_eq!(run.status("smoke"), "failed");
    assert_eq!(run.status("tell"), "skipped");
    let note = run.note("smoke").unwrap();
    assert!(note.contains("The test failed:"), "{note}");
    assert!(note.contains("failed  login"), "{note}");
    assert!(note.contains("2 of 6 steps passed"), "{note}");
    let outcome = step_outcome(&run, "smoke");
    assert_eq!(outcome.verdict, StepVerdict::Failed);
    assert_eq!(outcome.reason.as_deref(), Some(note));
    match &run.finished {
        Some(Finish::Failed(why)) => {
            assert_eq!(
                why,
                "Step `smoke` failed: failed  login  (2 of 6 steps passed)."
            );
            assert!(
                !why.contains("The test failed"),
                "the header stays on the step, not the run: {why}"
            );
        }
        other => panic!("expected the run to fail, got {other:?}"),
    }
}

#[test]
fn a_failed_gate_halts_a_parallel_wait_on_the_same_tick() {
    // `watch` does not need the gate, and its timeout is two hours. A failed gate still
    // ends the run now, and skips the wait, instead of leaving it running.
    let mut run = Run::from(
        r#"id: t
name: T
version: 1
trigger:
  kind: manual
steps:
  - id: smoke
    action: run_test
    test: login
    gate: true
    timeout: 30m
  - id: watch
    action: wait_session
    session: origin
    timeout: 2h
  - id: seen
    action: wait_pr_activity
    timeout: 2h
  - id: late
    action: notify
    needs: [seen]
    title: Still going
"#,
    );
    run.world.origin = Some("pty-o".into());
    // Busy, so `watch` stays waiting rather than settling.
    run.session(
        "pty-o",
        SessionFacts {
            alive: true,
            busy: true,
            settled_at: None,
        },
    );
    let started = run.advance();
    assert!(
        started
            .iter()
            .any(|effect| matches!(effect, Effect::RunTest { .. })),
        "the test is queued"
    );
    assert_eq!(run.status("watch"), "waiting");
    assert_eq!(run.status("seen"), "waiting");
    assert_eq!(run.status("late"), "pending");
    let queued_out: Outputs = [("run".to_owned(), "tr-halt".to_owned())].into();
    assert!(run.apply(&queued("smoke", &queued_out)));
    run.world.test_runs.insert(
        "tr-halt".into(),
        TestRun {
            status: "failed".into(),
            summary: "failed  login".into(),
        },
    );
    // A review arrives on the same tick the gate fails, which would have started `late`.
    run.world.posts.push(PrActivity::Review);
    let effects = run.advance();
    assert!(
        effects.is_empty(),
        "the notify that was about to start is dropped: {effects:?}"
    );
    assert_eq!(run.status("smoke"), "failed");
    assert!(run.note("smoke").unwrap().contains("failed  login"));
    assert_eq!(run.note("watch"), Some("Skipped: the gate `smoke` failed."));
    assert_eq!(run.status("watch"), "skipped");
    assert_eq!(run.status("seen"), "succeeded");
    assert_eq!(run.status("late"), "skipped");
    assert_eq!(run.note("late"), Some("Skipped: the gate `smoke` failed."));
    assert_eq!(
        run.finished,
        Some(Finish::Failed("Step `smoke` failed: failed  login.".into()))
    );
}

#[test]
fn a_step_left_running_is_skipped_when_a_gate_fails() {
    // `ping` started on an earlier tick and has no move of its own this plan. The halt
    // still skips it, with the gate's note, and the run finishes.
    let mut run = Run::from(
        r#"id: t
name: T
version: 1
trigger:
  kind: manual
steps:
  - id: smoke
    action: run_test
    test: login
    gate: true
    timeout: 30m
  - id: ping
    action: notify
    title: Still going
"#,
    );
    let started = run.advance();
    assert!(
        started
            .iter()
            .any(|effect| matches!(effect, Effect::Notify { .. })),
        "the notify starts"
    );
    assert_eq!(run.status("ping"), "running");
    assert_eq!(run.status("smoke"), "running");
    let queued_out: Outputs = [("run".to_owned(), "tr-left".to_owned())].into();
    assert!(run.apply(&queued("smoke", &queued_out)));
    assert_eq!(
        run.status("ping"),
        "running",
        "the earlier step is still going"
    );
    run.world.test_runs.insert(
        "tr-left".into(),
        TestRun {
            status: "failed".into(),
            summary: "failed  login  (2 of 6 steps passed)".into(),
        },
    );
    let effects = run.advance();
    assert!(effects.is_empty(), "nothing else starts: {effects:?}");
    assert_eq!(run.status("smoke"), "failed");
    assert_eq!(run.status("ping"), "skipped");
    assert_eq!(run.note("ping"), Some("Skipped: the gate `smoke` failed."));
    assert_eq!(
        run.finished,
        Some(Finish::Failed(
            "Step `smoke` failed: failed  login  (2 of 6 steps passed).".into()
        ))
    );
}

#[test]
fn a_halted_run_names_the_gate_not_an_earlier_failure() {
    let mut run = Run::from(
        r#"id: t
name: T
version: 1
trigger:
  kind: manual
steps:
  - id: early
    action: notify
    title: Hi
  - id: smoke
    action: run_test
    test: login
    gate: true
    timeout: 30m
"#,
    );
    let early = run
        .rows
        .iter_mut()
        .find(|row| row.step_id == "early")
        .unwrap();
    early.status = "failed".into();
    early.note = Some("boom".into());
    let smoke = run
        .rows
        .iter_mut()
        .find(|row| row.step_id == "smoke")
        .unwrap();
    smoke.status = "waiting".into();
    smoke.outputs = r#"{"run":"tr-gate"}"#.into();
    run.world.test_runs.insert(
        "tr-gate".into(),
        TestRun {
            status: "failed".into(),
            summary: "failed  login".into(),
        },
    );
    run.advance();
    assert_eq!(run.status("early"), "failed");
    assert_eq!(run.note("early"), Some("boom"));
    assert_eq!(
        run.finished,
        Some(Finish::Failed("Step `smoke` failed: failed  login.".into()))
    );
}

#[test]
fn a_gate_failure_line_keeps_a_capital_that_starts_an_acronym() {
    assert_eq!(
        gate_failure_line("The test failed:\nHTTP 500 from login"),
        "HTTP 500 from login."
    );
    assert_eq!(gate_failure_line("Failed."), "failed.");
    assert_eq!(gate_failure_line("The test failed."), "the test failed.");
    assert_eq!(
        gate_failure_line("The test failed:\nfailed  login  (2 of 6 steps passed)"),
        "failed  login  (2 of 6 steps passed)."
    );
}

#[test]
fn starting_a_step_again_clears_the_outcome_it_had() {
    let mut run = Run::from(GATED);
    run.rows[0].outcome = Some(r#"{"verdict":"failed","reason":"old"}"#.into());
    run.advance();
    assert_eq!(run.status("smoke"), "running");
    assert!(
        run.rows[0].outcome.is_none(),
        "a new attempt does not keep the previous verdict"
    );
}

fn assert_gate_failed_without_a_verdict(run: &Run, note: &str) {
    assert_eq!(run.status("smoke"), "failed");
    assert_eq!(run.note("smoke"), Some(note));
    assert!(
        run.rows
            .iter()
            .find(|row| row.step_id == "smoke")
            .unwrap()
            .outcome
            .is_none(),
        "no verdict, so no outcome"
    );
    assert_eq!(run.status("tell"), "skipped");
    assert_eq!(
        run.finished.as_ref(),
        Some(&Finish::Failed(format!("Step `smoke` failed: {note}")))
    );
}

#[test]
fn a_gate_that_times_out_fails_the_run_with_no_outcome() {
    let mut run = Run::from(GATED);
    run.advance();
    let queued_out: Outputs = [("run".to_owned(), "tr-slow".to_owned())].into();
    assert!(run.apply(&queued("smoke", &queued_out)));
    run.world.test_runs.insert(
        "tr-slow".into(),
        TestRun {
            status: "running".into(),
            summary: "going".into(),
        },
    );
    assert!(run.advance().is_empty());
    run.later(31 * 60 * 1000);
    run.advance();
    assert_gate_failed_without_a_verdict(&run, "The test run did not end within 30m.");
}

#[test]
fn a_gate_whose_test_run_is_gone_fails_with_no_outcome() {
    let mut run = Run::from(GATED);
    run.advance();
    let queued_out: Outputs = [("run".to_owned(), "tr-gone".to_owned())].into();
    assert!(run.apply(&queued("smoke", &queued_out)));
    run.advance();
    assert_gate_failed_without_a_verdict(&run, "The test run is gone.");
}

#[test]
fn a_cancelled_test_on_a_gate_fails_with_no_outcome() {
    let mut run = Run::from(GATED);
    finish_test(&mut run, "tr-c", "cancelled", "cancelled");
    assert_gate_failed_without_a_verdict(&run, "The test run cancelled.");
}

#[test]
fn an_errored_test_on_a_gate_fails_with_no_outcome() {
    let mut run = Run::from(GATED);
    finish_test(&mut run, "tr-e", "errored", "the runner died");
    assert_gate_failed_without_a_verdict(&run, "The test run errored.");
}

#[test]
fn a_passing_verdict_on_a_gate_continues() {
    let mut run = Run::from(GATED);
    finish_test(
        &mut run,
        "tr-ok",
        "passed",
        "passed  login  (6 of 6 steps passed)",
    );
    assert_eq!(run.status("smoke"), "succeeded");
    assert_eq!(run.note("smoke"), Some("The test passed."));
    assert_eq!(step_outcome(&run, "smoke").verdict, StepVerdict::Passed);
    let raw = run
        .rows
        .iter()
        .find(|row| row.step_id == "smoke")
        .unwrap()
        .outcome
        .as_deref()
        .unwrap();
    assert!(
        raw.contains("\"reason\":null"),
        "a pass still records the field, as null: {raw}"
    );
    assert_eq!(run.status("tell"), "running");
    assert!(
        run.finished.is_none(),
        "the notify step is still to be done"
    );
    assert!(run.apply(&done("tell", &Outputs::new())));
    run.advance();
    assert_eq!(run.status("tell"), "succeeded");
    assert_eq!(run.finished, Some(Finish::Succeeded));
}

#[test]
fn an_explicit_false_gate_keeps_a_failed_verdict_from_stopping_the_run() {
    let mut run = Run::from(&RUN_TEST_WF.replace("timeout: 30m", "gate: false\n    timeout: 30m"));
    finish_test(&mut run, "tr-no", "failed", "failed  login");
    assert_eq!(run.status("smoke"), "succeeded");
    assert_eq!(run.note("smoke"), Some("The test failed."));
    assert_eq!(step_outcome(&run, "smoke").verdict, StepVerdict::Failed);
    assert_eq!(run.finished, Some(Finish::Succeeded));
}

#[test]
fn a_run_test_step_fails_when_the_run_reaches_no_verdict() {
    // Cancelled or errored is not a verdict: the step fails, so `needs` can gate on it.
    let mut run = Run::from(RUN_TEST_WF);
    run.advance();
    let queued_out: Outputs = [("run".to_owned(), "tr-c".to_owned())].into();
    assert!(run.apply(&queued("smoke", &queued_out)));
    run.world.test_runs.insert(
        "tr-c".into(),
        TestRun {
            status: "cancelled".into(),
            summary: "cancelled".into(),
        },
    );
    run.advance();
    assert_eq!(run.status("smoke"), "failed");
    assert_eq!(run.note("smoke"), Some("The test run cancelled."));
    assert!(run.rows[0].outcome.is_none(), "no verdict, so no outcome");
    assert!(matches!(run.finished, Some(Finish::Failed(_))));
}

const SMOKE_THEN_TELL_WF: &str = r#"
id: stt
name: T
version: 1
trigger:
  kind: manual
steps:
  - id: test
    action: run_test
    test: login
    timeout: 30m
  - id: tell
    action: send_to_session
    needs: [test]
    session: origin
    timeout: 1h
    prompt: "how it went: {{ steps.test.summary }}"
"#;

#[test]
fn a_failing_test_still_tells_the_agent() {
    // The shape of the "Smoke test, then tell the agent" built-in: `tell` needs `test`, and a
    // failing test must still reach it, summary and all, so the agent can repair.
    let mut run = Run::from(SMOKE_THEN_TELL_WF);
    run.world.origin = Some("pty-o".into());
    // A quiet origin session is waiting, so `tell` can fire the moment `test` ends.
    run.session(
        "pty-o",
        SessionFacts {
            alive: true,
            busy: false,
            settled_at: Some(0),
        },
    );
    run.advance();
    let queued_out: Outputs = [("run".to_owned(), "tr".to_owned())].into();
    assert!(run.apply(&queued("test", &queued_out)));
    run.world.test_runs.insert(
        "tr".into(),
        TestRun {
            status: "failed".into(),
            summary: "failed  login  (2 of 6 steps passed)".into(),
        },
    );
    // One pass: `test` succeeds with the failed verdict, which unblocks `tell`, which pastes.
    let effects = run.advance();
    assert_eq!(run.status("test"), "succeeded");
    assert_eq!(run.status("tell"), "running");
    match effects.iter().find(|e| matches!(e, Effect::Paste { .. })) {
        Some(Effect::Paste { text, .. }) => assert!(text.contains("failed  login")),
        other => panic!("expected a paste, got {other:?}"),
    }
}

#[test]
fn a_run_test_step_times_out_if_the_run_never_ends() {
    let mut run = Run::from(RUN_TEST_WF);
    run.advance();
    let queued_out: Outputs = [("run".to_owned(), "tr-x".to_owned())].into();
    assert!(run.apply(&queued("smoke", &queued_out)));
    run.world.test_runs.insert(
        "tr-x".into(),
        TestRun {
            status: "running".into(),
            summary: "going".into(),
        },
    );
    assert!(run.advance().is_empty());
    assert_eq!(run.status("smoke"), "waiting");
    // Past the 30m timeout, still going: the step fails.
    run.later(31 * 60 * 1000);
    run.advance();
    assert_eq!(run.status("smoke"), "failed");
    assert!(run.note("smoke").unwrap().contains("did not end within"));
}

const HANDOFF: &str = r#"id: hand
name: Hand
version: 1
trigger:
  kind: manual
steps:
  - id: draft
    action: wait_session
    session: origin
    produces:
      - name: spec
        path: SPEC.md
  - id: plan
    action: notify
    needs: [draft]
    consumes: [spec]
    title: "Plan {{ artifacts.spec }}"
  - id: send
    action: send_to_session
    needs: [draft]
    session: origin
    consumes: [spec]
    prompt: "Use the spec."
"#;

#[test]
fn a_producing_step_waits_to_finish_and_the_next_step_reads_the_latest_body() {
    let mut run = Run::from(HANDOFF);
    run.world.origin = Some("pty-1".into());
    run.session(
        "pty-1",
        SessionFacts {
            alive: true,
            busy: false,
            settled_at: None,
        },
    );
    assert!(run.advance().is_empty());
    assert_eq!(run.status("draft"), "waiting");

    run.session(
        "pty-1",
        SessionFacts {
            alive: true,
            busy: false,
            settled_at: Some(run.world.now),
        },
    );
    let effects = run.advance();
    assert!(
        effects
            .iter()
            .all(|effect| matches!(effect, Effect::Capture { .. })),
        "the consumer must not start before the artifact is stored: {effects:?}"
    );
    assert_eq!(run.status("draft"), "running");
    assert_eq!(run.status("plan"), "pending");
    let Effect::Capture {
        outputs,
        note,
        outcome,
        ..
    } = effects.last().unwrap()
    else {
        panic!("expected a capture");
    };
    assert!(run.apply(&recorded("draft", outputs, note.clone(), outcome.clone())));
    run.world.artifacts.insert("spec".into(), "Be kind.".into());

    let effects = run.advance();
    let title = effects.iter().find_map(|effect| match effect {
        Effect::Notify { title, .. } => Some(title.as_str()),
        _ => None,
    });
    assert_eq!(title, Some("Plan Be kind."));
    let text = effects.iter().find_map(|effect| match effect {
        Effect::Paste { text, .. } => Some(text.as_str()),
        _ => None,
    });
    assert_eq!(
        text,
        Some("Use the spec.\n\n## Artifact: spec\n\nBe kind."),
        "a prompt that does not place the artifact is given it once"
    );

    run.world
        .artifacts
        .insert("spec".into(), "Be stricter.".into());
    let plan = run
        .rows
        .iter_mut()
        .find(|row| row.step_id == "plan")
        .unwrap();
    plan.status = "pending".into();
    plan.ended_at = None;
    let effects = run.advance();
    let title = effects.iter().find_map(|effect| match effect {
        Effect::Notify { title, .. } => Some(title.clone()),
        _ => None,
    });
    assert_eq!(title.as_deref(), Some("Plan Be stricter."));
}

const GATE_AND_ARTIFACT: &str = r#"id: both
name: Both
version: 1
trigger:
  kind: manual
steps:
  - id: gate
    action: run_test
    test: demo
    gate: true
    timeout: 30m
  - id: draft
    action: wait_session
    session: origin
    produces:
      - name: spec
        text: hello
"#;

#[test]
fn a_gate_halt_drops_an_artifact_capture_in_the_same_plan() {
    let mut run = Run::from(GATE_AND_ARTIFACT);
    run.world.origin = Some("pty-1".into());
    run.session(
        "pty-1",
        SessionFacts {
            alive: true,
            busy: false,
            settled_at: None,
        },
    );
    let effects = run.advance();
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::RunTest { .. })),
        "{effects:?}"
    );
    assert_eq!(run.status("draft"), "waiting");
    let queued_out: Outputs = [("run".to_owned(), "tr-gate".to_owned())].into();
    assert!(run.apply(&queued("gate", &queued_out)));
    run.world.test_runs.insert(
        "tr-gate".into(),
        TestRun {
            status: "failed".into(),
            summary: "wanted 200, got 500".into(),
        },
    );
    run.session(
        "pty-1",
        SessionFacts {
            alive: true,
            busy: false,
            settled_at: Some(run.world.now),
        },
    );
    let effects = run.advance();
    assert!(
        effects.is_empty(),
        "a failed gate must not leave the artifact capture to run: {effects:?}"
    );
    assert_eq!(run.status("gate"), "failed");
    assert_eq!(run.status("draft"), "skipped");
    assert!(matches!(run.finished, Some(Finish::Failed(_))));
}

#[test]
fn a_step_awaiting_artifacts_is_captured_again_after_a_restart() {
    let mut run = Run::from(HANDOFF);
    run.world.origin = Some("pty-1".into());
    run.session(
        "pty-1",
        SessionFacts {
            alive: true,
            busy: false,
            settled_at: Some(run.world.now),
        },
    );
    let draft = run
        .rows
        .iter_mut()
        .find(|row| row.step_id == "draft")
        .unwrap();
    draft.status = "running".into();
    draft.started_at = Some(1);
    draft.outputs = r#"{"artifact_pending":"1","outcome":"settled"}"#.into();
    draft.note = Some("The session settled.".into());

    let recovered = recover(&run.rows);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].to, crate::store::StepStatus::Waiting);
    assert!(run.apply(&recovered[0]));
    assert_eq!(run.status("draft"), "waiting");
    let outputs = &run
        .rows
        .iter()
        .find(|row| row.step_id == "draft")
        .unwrap()
        .outputs;
    assert!(
        outputs.contains("artifact_pending"),
        "the marker stays, so the next tick resumes the capture: {outputs}"
    );
    assert_eq!(
        run.note("draft"),
        Some("The session settled."),
        "the finished step keeps what it had already concluded"
    );

    let effects = run.advance();
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Capture { step, .. } if step == "draft")),
        "the capture runs again: {effects:?}"
    );
    assert_eq!(run.status("draft"), "running");
    assert_eq!(run.status("plan"), "pending");
}

const APPROVAL: &str = r#"id: sign
name: Sign
version: 1
trigger:
  kind: manual
steps:
  - id: sign-off
    action: approve
    prompt: Look at the plan.
  - id: next
    needs: [sign-off]
    action: notify
    title: Go
"#;

fn decided(decision: ApprovalDecision, feedback: Option<&str>) -> RecordedApproval {
    RecordedApproval {
        decision,
        feedback: feedback.map(str::to_owned),
        actor: "ada".into(),
        decided_at: 1_700_000_000_000,
    }
}

#[test]
fn an_outcome_saved_before_approvals_still_reads() {
    let outcome: StepOutcome =
        serde_json::from_str(r#"{"verdict":"failed","reason":"old"}"#).unwrap();
    assert_eq!(outcome.verdict, StepVerdict::Failed);
    assert_eq!(outcome.reason.as_deref(), Some("old"));
    assert!(outcome.decision.is_none());
    assert!(outcome.feedback.is_none());
    assert!(outcome.actor.is_none());
    assert!(outcome.decided_at.is_none());
}

#[test]
fn an_approval_blocks_the_next_step_until_someone_decides() {
    let mut run = Run::from(APPROVAL);
    let effects = run.advance();
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(run.status("sign-off"), "waiting");
    assert_eq!(run.status("next"), "pending");
    assert_eq!(
        run.note("sign-off"),
        Some("Waiting for your approval.\nLook at the plan.")
    );
    assert!(run.finished.is_none());
}

#[test]
fn approving_lets_the_next_step_start() {
    let mut run = Run::from(APPROVAL);
    run.advance();
    run.world.approvals.insert(
        "sign-off".into(),
        decided(ApprovalDecision::Approved, Some("ship it")),
    );
    let effects = run.advance();
    assert_eq!(run.status("sign-off"), "succeeded");
    assert_eq!(run.note("sign-off"), Some("Approved: ship it"));
    let outcome = step_outcome(&run, "sign-off");
    assert_eq!(outcome.verdict, StepVerdict::Passed);
    assert_eq!(outcome.decision, Some(ApprovalDecision::Approved));
    assert_eq!(outcome.feedback.as_deref(), Some("ship it"));
    assert_eq!(outcome.actor.as_deref(), Some("ada"));
    assert_eq!(outcome.decided_at, Some(1_700_000_000_000));
    let left = parse_outputs(
        &run.rows
            .iter()
            .find(|row| row.step_id == "sign-off")
            .unwrap()
            .outputs,
    );
    assert_eq!(left.get("decision").map(String::as_str), Some("approved"));
    assert_eq!(left.get("feedback").map(String::as_str), Some("ship it"));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Notify { step, .. } if step == "next")),
        "the next step starts: {effects:?}"
    );
    assert_eq!(run.status("next"), "running");
    assert!(run.finished.is_none());
}

#[test]
fn rejecting_halts_the_run_and_is_not_an_approval() {
    let mut run = Run::from(APPROVAL);
    run.advance();
    run.world
        .approvals
        .insert("sign-off".into(), decided(ApprovalDecision::Rejected, None));
    run.advance();
    assert_eq!(run.status("sign-off"), "failed");
    assert_eq!(run.note("sign-off"), Some("Rejected."));
    assert_eq!(run.status("next"), "skipped");
    let outcome = step_outcome(&run, "sign-off");
    assert_eq!(outcome.verdict, StepVerdict::Failed);
    assert_eq!(outcome.decision, Some(ApprovalDecision::Rejected));
    assert!(outcome.feedback.is_none());
    match run.finished {
        Some(Finish::Failed(why)) => {
            assert!(why.contains("sign-off"), "{why}");
            assert!(why.to_lowercase().contains("rejected"), "{why}");
        }
        other => panic!("the run should fail, got {other:?}"),
    }
}

#[test]
fn requesting_changes_is_recorded_apart_from_a_rejection_and_stops_the_run() {
    let mut run = Run::from(APPROVAL);
    run.advance();
    run.world.approvals.insert(
        "sign-off".into(),
        decided(
            ApprovalDecision::ChangesRequested,
            Some("split the migration"),
        ),
    );
    run.advance();
    assert_eq!(run.status("sign-off"), "failed");
    assert_eq!(run.status("next"), "skipped");
    assert_eq!(
        run.note("sign-off"),
        Some("Changes requested: split the migration")
    );
    let outcome = step_outcome(&run, "sign-off");
    assert_eq!(outcome.decision, Some(ApprovalDecision::ChangesRequested));
    assert_eq!(outcome.feedback.as_deref(), Some("split the migration"));
    assert_ne!(outcome.decision, Some(ApprovalDecision::Rejected));
    let why = match run.finished {
        Some(Finish::Failed(why)) => why,
        other => panic!("the run should stop, got {other:?}"),
    };
    assert!(why.to_lowercase().contains("changes requested"), "{why}");
    assert!(!why.to_lowercase().contains("rejected"), "{why}");
}

#[test]
fn an_approval_timeout_fails_that_step_and_does_not_halt_another() {
    let mut run = Run::from(
        r#"id: sign
name: Sign
version: 1
trigger:
  kind: manual
steps:
  - id: sign-off
    action: approve
    timeout: 1s
  - id: other
    action: notify
    title: Hi
"#,
    );
    run.advance();
    assert_eq!(run.status("sign-off"), "waiting");
    assert_eq!(run.status("other"), "running");
    run.later(2_000);
    run.advance();
    assert_eq!(run.status("sign-off"), "failed");
    assert_eq!(run.note("sign-off"), Some("Timed out after 1s."));
    assert!(
        run.rows
            .iter()
            .find(|row| row.step_id == "sign-off")
            .unwrap()
            .outcome
            .is_none(),
        "a timeout is not a decision"
    );
    assert_eq!(run.status("other"), "running");
    assert!(run.finished.is_none());
}

#[test]
fn a_waiting_approval_stays_waiting_when_the_run_is_recovered() {
    let mut run = Run::from(APPROVAL);
    run.advance();
    assert!(recover(&run.rows).is_empty());
    assert_eq!(run.status("sign-off"), "waiting");
    run.advance();
    assert_eq!(run.status("sign-off"), "waiting");
    assert_eq!(run.status("next"), "pending");
    assert!(run.finished.is_none());
}

#[test]
fn a_notification_waiting_on_its_artifacts_is_captured_again() {
    let mut run = Run::from(
        r#"id: hand
name: Hand
version: 1
trigger:
  kind: manual
steps:
  - id: draft
    action: notify
    title: Drafted
    produces:
      - name: spec
        text: Be kind.
"#,
    );
    let draft = run
        .rows
        .iter_mut()
        .find(|row| row.step_id == "draft")
        .unwrap();
    draft.status = "waiting".into();
    draft.started_at = Some(1);
    draft.outputs = r#"{"artifact_pending":"1"}"#.into();
    draft.note = Some("Told.".into());
    let effects = run.advance();
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Capture { step, .. } if step == "draft")),
        "the capture resumes without sending the notification again: {effects:?}"
    );
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Notify { .. })),
        "the notification already happened: {effects:?}"
    );
    assert_eq!(run.status("draft"), "running");
}

const BRANCHED: &str = r#"id: demo
name: Demo
version: 1
trigger:
  kind: manual
steps:
  - id: plan
    action: notify
    title: Plan
  - id: sign-off
    needs: [plan]
    action: approve
    prompt: Look at the plan.
    on:
      approved: implement
      rejected: abandon
      changes_requested: plan
    max_attempts: 2
  - id: implement
    needs: [sign-off]
    action: notify
    title: Build
  - id: abandon
    needs: [sign-off]
    action: notify
    title: Stop
  - id: verify
    needs: [implement]
    action: run_test
    test: login
    gate: true
    on:
      passed: stop
      failed: implement
    max_attempts: 3
"#;

fn finish_notify(run: &mut Run, id: &str) {
    assert!(run.apply(&done(id, &Outputs::new())));
}

fn attempts(run: &Run, id: &str) -> u32 {
    run.rows
        .iter()
        .find(|row| row.step_id == id)
        .unwrap()
        .attempts
}

#[test]
fn an_approved_plan_runs_implementation_and_skips_the_other_path() {
    let mut run = Run::from(BRANCHED);
    run.advance();
    finish_notify(&mut run, "plan");
    run.advance();
    assert_eq!(attempts(&run, "sign-off"), 1);
    run.world
        .approvals
        .insert("sign-off".into(), decided(ApprovalDecision::Approved, None));
    run.advance();
    assert_eq!(run.status("sign-off"), "succeeded");
    assert_eq!(run.status("implement"), "running");
    assert_eq!(run.status("abandon"), "skipped");
    assert!(run.note("abandon").unwrap().contains("went to `implement`"));
    assert!(run.finished.is_none());
}

#[test]
fn a_rejection_follows_its_path_and_stop_ends_the_run() {
    let mut run = Run::from(BRANCHED);
    run.advance();
    finish_notify(&mut run, "plan");
    run.advance();
    run.world
        .approvals
        .insert("sign-off".into(), decided(ApprovalDecision::Rejected, None));
    run.advance();
    assert_eq!(run.status("sign-off"), "succeeded");
    assert_eq!(run.status("abandon"), "running");
    assert_eq!(run.status("implement"), "skipped");

    let mut stopped = Run::from(&BRANCHED.replace("rejected: abandon", "rejected: stop"));
    stopped.advance();
    finish_notify(&mut stopped, "plan");
    stopped.advance();
    stopped
        .world
        .approvals
        .insert("sign-off".into(), decided(ApprovalDecision::Rejected, None));
    stopped.advance();
    assert_eq!(stopped.status("sign-off"), "failed");
    assert_eq!(stopped.status("implement"), "skipped");
    assert_eq!(stopped.status("abandon"), "skipped");
    assert!(matches!(stopped.finished, Some(Finish::Failed(_))));
}

#[test]
fn changes_requested_returns_the_feedback_and_asks_again() {
    let mut run = Run::from(BRANCHED);
    run.advance();
    finish_notify(&mut run, "plan");
    run.advance();
    run.world.approvals.insert(
        "sign-off".into(),
        decided(
            ApprovalDecision::ChangesRequested,
            Some("name the edge cases"),
        ),
    );
    run.advance();
    assert_eq!(run.status("plan"), "pending");
    assert_eq!(run.status("sign-off"), "pending");
    assert_eq!(attempts(&run, "sign-off"), 1);
    assert_eq!(attempts(&run, "plan"), 1);
    assert!(run.world.approvals.is_empty());
    assert_eq!(
        run.world
            .artifacts
            .get("changes-sign-off")
            .map(String::as_str),
        Some("name the edge cases")
    );
    assert!(run.finished.is_none());

    // The old decision is gone, so the next pass waits. It does not approve itself.
    run.advance();
    finish_notify(&mut run, "plan");
    run.advance();
    assert_eq!(run.status("sign-off"), "waiting");
    assert_eq!(attempts(&run, "sign-off"), 2);
    assert!(run.world.approvals.is_empty());
}

#[test]
fn a_loop_stops_at_its_limit_and_says_what_to_do() {
    let mut run = Run::from(BRANCHED);
    run.advance();
    finish_notify(&mut run, "plan");
    run.advance();
    run.world.approvals.insert(
        "sign-off".into(),
        decided(ApprovalDecision::ChangesRequested, Some("once")),
    );
    run.advance();
    run.advance();
    finish_notify(&mut run, "plan");
    run.advance();
    assert_eq!(attempts(&run, "sign-off"), 2);
    run.world.approvals.insert(
        "sign-off".into(),
        decided(ApprovalDecision::ChangesRequested, Some("twice")),
    );
    run.advance();
    assert_eq!(run.status("sign-off"), "failed");
    assert_eq!(attempts(&run, "sign-off"), 2);
    assert_eq!(run.status("plan"), "succeeded");
    let Finish::Failed(error) = run.finished.clone().unwrap() else {
        panic!("the run should stop: {:?}", run.finished);
    };
    assert!(
        error.contains("changes were requested 2 times, which is the limit (2)"),
        "{error}"
    );
    assert!(error.contains("Start a new run to try again."), "{error}");
}

#[test]
fn a_failed_verification_returns_to_implementation_and_keeps_the_approval() {
    let mut run = Run::from(BRANCHED);
    run.advance();
    finish_notify(&mut run, "plan");
    run.advance();
    run.world
        .approvals
        .insert("sign-off".into(), decided(ApprovalDecision::Approved, None));
    run.advance();
    finish_notify(&mut run, "implement");
    let effects = run.advance();
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::RunTest { step, .. } if step == "verify")),
        "{effects:?}"
    );
    assert!(run.apply(&queued(
        "verify",
        &Outputs::from([("run".to_owned(), "tr-1".to_owned())])
    )));
    run.world.test_runs.insert(
        "tr-1".into(),
        TestRun {
            status: "failed".into(),
            summary: "login: the button is missing".into(),
        },
    );
    let approval = run.world.approvals.get("sign-off").cloned();
    run.advance();
    assert_eq!(run.status("implement"), "pending");
    assert_eq!(run.status("verify"), "pending");
    assert_eq!(run.status("sign-off"), "succeeded");
    assert_eq!(attempts(&run, "verify"), 1);
    assert_eq!(run.world.approvals.get("sign-off"), approval.as_ref());
    assert!(run
        .world
        .artifacts
        .get("failure-verify")
        .is_some_and(|body| body.contains("the button is missing")));
    assert!(run.finished.is_none());
}

#[test]
fn a_forward_failure_hands_the_report_to_the_next_step() {
    let source = r#"id: demo
name: Demo
version: 1
trigger:
  kind: manual
steps:
  - id: verify
    action: run_test
    test: login
    on:
      passed: stop
      failed: report
  - id: report
    needs: [verify]
    action: start_session
    harness: claude
    consumes: [failure-verify]
"#;
    let mut run = Run::from(source);
    run.advance();
    assert!(run.apply(&queued(
        "verify",
        &Outputs::from([("run".to_owned(), "tr-1".to_owned())])
    )));
    run.world.test_runs.insert(
        "tr-1".into(),
        TestRun {
            status: "failed".into(),
            summary: "login: the button is missing".into(),
        },
    );
    let effects = run.advance();
    assert_eq!(run.status("verify"), "succeeded");
    assert_eq!(run.status("report"), "running");
    let prompt = effects.iter().find_map(|effect| match effect {
        Effect::Start { step, prompt, .. } if step == "report" => prompt.clone(),
        _ => None,
    });
    assert!(
        prompt.as_deref().unwrap().contains("the button is missing"),
        "the next step was started without the report: {prompt:?}"
    );
    assert!(run
        .world
        .artifacts
        .get("failure-verify")
        .is_some_and(|body| body.contains("the button is missing")));
}

#[test]
fn an_earlier_failure_is_not_handed_out_after_a_later_rewind() {
    let source = r#"id: demo
name: Demo
version: 1
trigger:
  kind: manual
steps:
  - id: plan
    action: notify
    title: Plan
  - id: implement
    needs: [plan]
    action: start_session
    harness: claude
    consumes: [failure-verify]
  - id: verify
    needs: [implement]
    action: run_test
    test: login
    on:
      passed: sign-off
      failed: implement
    max_attempts: 3
  - id: sign-off
    needs: [verify]
    action: approve
    prompt: Look
    on:
      approved: stop
      rejected: stop
      changes_requested: plan
    max_attempts: 3
"#;
    let mut run = Run::from(source);
    run.advance();
    finish_notify(&mut run, "plan");
    run.advance();
    finish_notify(&mut run, "implement");
    run.advance();
    assert!(run.apply(&queued(
        "verify",
        &Outputs::from([("run".to_owned(), "tr-1".to_owned())])
    )));
    run.world.test_runs.insert(
        "tr-1".into(),
        TestRun {
            status: "failed".into(),
            summary: "login: the button is missing".into(),
        },
    );
    run.advance();
    assert!(run
        .world
        .artifacts
        .get("failure-verify")
        .is_some_and(|body| body.contains("the button is missing")));
    let again = run.advance();
    let prompt = again.iter().find_map(|effect| match effect {
        Effect::Start { step, prompt, .. } if step == "implement" => prompt.clone(),
        _ => None,
    });
    assert!(
        prompt.as_deref().unwrap().contains("the button is missing"),
        "the step being run again should still see this failure: {prompt:?}"
    );
    finish_notify(&mut run, "implement");
    run.advance();
    assert!(run.apply(&queued(
        "verify",
        &Outputs::from([("run".to_owned(), "tr-2".to_owned())])
    )));
    run.world.test_runs.insert(
        "tr-2".into(),
        TestRun {
            status: "passed".into(),
            summary: "ok".into(),
        },
    );
    run.advance();
    assert_eq!(run.status("sign-off"), "waiting");
    run.world.approvals.insert(
        "sign-off".into(),
        decided(ApprovalDecision::ChangesRequested, Some("rename it")),
    );
    run.advance();
    assert!(
        !run.world.artifacts.contains_key("failure-verify"),
        "the old report was handed out again: {:?}",
        run.world.artifacts
    );
    assert_eq!(
        run.world
            .artifacts
            .get("changes-sign-off")
            .map(String::as_str),
        Some("rename it")
    );
    run.advance();
    finish_notify(&mut run, "plan");
    let restarted = run.advance();
    let prompt = restarted.iter().find_map(|effect| match effect {
        Effect::Start { step, prompt, .. } if step == "implement" => prompt.clone(),
        _ => None,
    });
    assert!(
        prompt
            .as_deref()
            .is_none_or(|text| !text.contains("the button is missing")),
        "implement was given the old report: {prompt:?}"
    );
}

#[test]
fn two_parallel_verdicts_do_not_rewind_when_the_run_halts() {
    let source = r#"id: demo
name: Demo
version: 1
trigger:
  kind: manual
steps:
  - id: prep
    action: notify
    title: Prep
  - id: left
    needs: [prep]
    action: run_test
    test: left
    on:
      passed: stop
      failed: stop
  - id: right
    needs: [prep]
    action: run_test
    test: right
    on:
      passed: stop
      failed: prep
    max_attempts: 3
"#;
    let mut halted = Run::from(source);
    halted.advance();
    finish_notify(&mut halted, "prep");
    halted.advance();
    assert!(halted.apply(&queued(
        "left",
        &Outputs::from([("run".to_owned(), "left-run".to_owned())])
    )));
    assert!(halted.apply(&queued(
        "right",
        &Outputs::from([("run".to_owned(), "right-run".to_owned())])
    )));
    halted.world.test_runs.insert(
        "left-run".into(),
        TestRun {
            status: "failed".into(),
            summary: "left broke".into(),
        },
    );
    halted.world.test_runs.insert(
        "right-run".into(),
        TestRun {
            status: "failed".into(),
            summary: "right broke".into(),
        },
    );
    halted.advance();
    assert_eq!(halted.status("left"), "failed");
    assert_eq!(halted.status("right"), "skipped");
    assert_eq!(halted.status("prep"), "succeeded");
    assert_eq!(attempts(&halted, "prep"), 1);
    assert!(matches!(halted.finished, Some(Finish::Failed(_))));
    assert!(halted.rows.iter().all(|row| row.status != "pending"));

    let mut stopped = Run::from(source);
    stopped.advance();
    finish_notify(&mut stopped, "prep");
    stopped.advance();
    assert!(stopped.apply(&queued(
        "left",
        &Outputs::from([("run".to_owned(), "left-run".to_owned())])
    )));
    assert!(stopped.apply(&queued(
        "right",
        &Outputs::from([("run".to_owned(), "right-run".to_owned())])
    )));
    stopped.world.test_runs.insert(
        "left-run".into(),
        TestRun {
            status: "passed".into(),
            summary: "ok".into(),
        },
    );
    stopped.world.test_runs.insert(
        "right-run".into(),
        TestRun {
            status: "failed".into(),
            summary: "right broke".into(),
        },
    );
    stopped.advance();
    assert_eq!(stopped.status("left"), "succeeded");
    assert_eq!(stopped.status("right"), "skipped");
    assert_eq!(stopped.status("prep"), "succeeded");
    assert_eq!(attempts(&stopped, "prep"), 1);
    assert!(matches!(stopped.finished, Some(Finish::Succeeded)));
    assert!(stopped.rows.iter().all(|row| row.status != "pending"));
}

#[test]
fn a_pass_that_stops_still_stops_after_the_step_saves_what_it_produces() {
    let source = r#"id: demo
name: Demo
version: 1
trigger:
  kind: manual
steps:
  - id: verify
    action: run_test
    test: login
    produces:
      - name: report
        text: The report.
    on:
      passed: stop
      failed: stop
  - id: side
    action: notify
    title: Side
"#;
    let mut run = Run::from(source);
    let started = run.advance();
    assert!(
        started
            .iter()
            .any(|effect| matches!(effect, Effect::RunTest { step, .. } if step == "verify")),
        "{started:?}"
    );
    assert!(
        started
            .iter()
            .any(|effect| matches!(effect, Effect::Notify { step, .. } if step == "side")),
        "{started:?}"
    );
    assert!(run.apply(&queued(
        "verify",
        &Outputs::from([("run".to_owned(), "tr-1".to_owned())])
    )));
    assert_eq!(run.status("side"), "running");
    run.world.test_runs.insert(
        "tr-1".into(),
        TestRun {
            status: "passed".into(),
            summary: "ok".into(),
        },
    );
    let saving = run.advance();
    assert_eq!(run.status("verify"), "running");
    assert_eq!(run.status("side"), "running");
    assert!(run.finished.is_none());
    let Effect::Capture {
        outputs,
        note,
        outcome,
        ..
    } = saving
        .iter()
        .find(|effect| matches!(effect, Effect::Capture { step, .. } if step == "verify"))
        .unwrap()
    else {
        panic!("the pass should wait to save the report: {saving:?}");
    };
    assert!(run.apply(&recorded("verify", outputs, note.clone(), outcome.clone())));
    assert_eq!(run.status("verify"), "succeeded");
    run.advance();
    assert_eq!(run.status("side"), "skipped");
    assert_eq!(run.status("verify"), "succeeded");
    assert!(matches!(run.finished, Some(Finish::Succeeded)));
    assert!(run.rows.iter().all(|row| row.status != "pending"));
}

#[test]
fn a_halt_wins_over_a_rewind_when_the_looping_step_comes_first() {
    let source = r#"id: demo
name: Demo
version: 1
trigger:
  kind: manual
steps:
  - id: prep
    action: notify
    title: Prep
  - id: loop
    needs: [prep]
    action: run_test
    test: loop
    on:
      passed: stop
      failed: prep
    max_attempts: 3
  - id: gate
    needs: [prep]
    action: run_test
    test: gate
    on:
      passed: stop
      failed: stop
"#;
    let mut halted = Run::from(source);
    halted.advance();
    finish_notify(&mut halted, "prep");
    halted.advance();
    assert!(halted.apply(&queued(
        "loop",
        &Outputs::from([("run".to_owned(), "loop-run".to_owned())])
    )));
    assert!(halted.apply(&queued(
        "gate",
        &Outputs::from([("run".to_owned(), "gate-run".to_owned())])
    )));
    halted.world.test_runs.insert(
        "loop-run".into(),
        TestRun {
            status: "failed".into(),
            summary: "loop broke".into(),
        },
    );
    halted.world.test_runs.insert(
        "gate-run".into(),
        TestRun {
            status: "failed".into(),
            summary: "gate broke".into(),
        },
    );
    halted.advance();
    assert_eq!(halted.status("gate"), "failed");
    assert_eq!(halted.status("loop"), "skipped");
    assert_eq!(halted.status("prep"), "succeeded");
    assert_eq!(attempts(&halted, "prep"), 1);
    assert!(matches!(halted.finished, Some(Finish::Failed(_))));
    assert!(halted.rows.iter().all(|row| row.status != "pending"));

    let mut stopped = Run::from(source);
    stopped.advance();
    finish_notify(&mut stopped, "prep");
    stopped.advance();
    assert!(stopped.apply(&queued(
        "loop",
        &Outputs::from([("run".to_owned(), "loop-run".to_owned())])
    )));
    assert!(stopped.apply(&queued(
        "gate",
        &Outputs::from([("run".to_owned(), "gate-run".to_owned())])
    )));
    stopped.world.test_runs.insert(
        "loop-run".into(),
        TestRun {
            status: "failed".into(),
            summary: "loop broke".into(),
        },
    );
    stopped.world.test_runs.insert(
        "gate-run".into(),
        TestRun {
            status: "passed".into(),
            summary: "ok".into(),
        },
    );
    stopped.advance();
    assert_eq!(stopped.status("gate"), "succeeded");
    assert_eq!(stopped.status("loop"), "skipped");
    assert_eq!(stopped.status("prep"), "succeeded");
    assert_eq!(attempts(&stopped, "prep"), 1);
    assert!(matches!(stopped.finished, Some(Finish::Succeeded)));
    assert!(stopped.rows.iter().all(|row| row.status != "pending"));
}

const CLARIFY: &str = r#"id: clarify
name: Clarify
version: 1
trigger:
  kind: manual
steps:
  - id: start
    action: start_session
    harness: claude
    prompt: Clarify the problem.
  - id: discover
    needs: [start]
    action: wait_session
    session: "{{ steps.start.session }}"
    until: settled
    clarify: true
    max_rounds: 3
"#;

fn session_up(run: &mut Run) {
    run.advance();
    assert!(run.apply(&done(
        "start",
        &Outputs::from([("session".to_owned(), "pty-9".to_owned())])
    )));
    // Alive before the wait starts. A missing session is one that has already ended,
    // and the step would finish as exited instead of waiting.
    run.world.sessions.insert(
        "pty-9".into(),
        SessionFacts {
            alive: true,
            busy: false,
            settled_at: None,
        },
    );
    run.advance();
    assert_eq!(run.status("discover"), "waiting");
    run.world.sessions.insert(
        "pty-9".into(),
        SessionFacts {
            alive: true,
            busy: false,
            settled_at: Some(run.world.now),
        },
    );
}

#[test]
fn a_settled_session_with_no_question_finishes_and_a_printed_question_is_not_one() {
    let mut run = Run::from(CLARIFY);
    let effects = run.advance();
    let prompt = effects.iter().find_map(|effect| match effect {
        Effect::Start { prompt, .. } => prompt.clone(),
        _ => None,
    });
    assert!(
        prompt.unwrap().contains("ys workflow ask"),
        "the session was not told how to ask"
    );
    session_up(&mut run);
    run.advance();
    assert_eq!(
        run.status("discover"),
        "waiting",
        "a quiet session does not finish a clarifying step"
    );
    run.world.clarifications.insert(
        "discover".into(),
        ClarificationState {
            summary: true,
            ..ClarificationState::default()
        },
    );
    run.advance();
    assert_eq!(run.status("discover"), "succeeded");
    assert_eq!(
        run.rows
            .iter()
            .find(|row| row.step_id == "discover")
            .and_then(|row| parse_outputs(&row.outputs).get("outcome").cloned())
            .as_deref(),
        Some("summary")
    );
}

#[test]
fn a_clarifying_step_finishes_when_until_exited_and_the_session_ends() {
    let source = CLARIFY.replace("until: settled", "until: exited");
    let mut run = Run::from(&source);
    session_up(&mut run);
    run.advance();
    assert_eq!(run.status("discover"), "waiting");
    run.world.sessions.insert(
        "pty-9".into(),
        SessionFacts {
            alive: false,
            busy: false,
            settled_at: None,
        },
    );
    run.advance();
    assert_eq!(run.status("discover"), "succeeded");
    assert_eq!(
        run.rows
            .iter()
            .find(|row| row.step_id == "discover")
            .and_then(|row| parse_outputs(&row.outputs).get("outcome").cloned())
            .as_deref(),
        Some("exited")
    );
}

#[test]
fn a_claimed_answer_that_was_not_typed_fails_the_step() {
    let mut run = Run::from(CLARIFY);
    session_up(&mut run);
    run.world.clarifications.insert(
        "discover".into(),
        ClarificationState {
            rounds: 1,
            undelivered: true,
            ..ClarificationState::default()
        },
    );
    run.advance();
    assert_eq!(run.status("discover"), "failed");
    assert!(run
        .note("discover")
        .unwrap()
        .contains("may not have reached the session"));
}

#[test]
fn a_clarifying_step_without_a_summary_fails_at_its_timeout() {
    let source = CLARIFY.replace("until: settled", "until: settled\n    timeout: 1m");
    let mut run = Run::from(&source);
    session_up(&mut run);
    let started = run
        .rows
        .iter()
        .find(|row| row.step_id == "discover")
        .and_then(|row| row.started_at)
        .unwrap();
    run.world.now = started + 61_000;
    run.advance();
    assert_eq!(run.status("discover"), "failed");
    assert!(run
        .note("discover")
        .unwrap()
        .contains("waiting for a summary"));
}

#[test]
fn a_dead_session_with_no_summary_fails_without_a_timeout() {
    let mut run = Run::from(CLARIFY);
    session_up(&mut run);
    run.advance();
    assert_eq!(run.status("discover"), "waiting");
    run.world.sessions.insert(
        "pty-9".into(),
        SessionFacts {
            alive: false,
            busy: false,
            settled_at: None,
        },
    );
    run.advance();
    assert_eq!(run.status("discover"), "failed");
    assert!(run
        .note("discover")
        .unwrap()
        .contains("The session ended without a summary."));
}

#[test]
fn an_open_question_fails_when_the_session_ends() {
    let mut run = Run::from(CLARIFY);
    session_up(&mut run);
    run.world.clarifications.insert(
        "discover".into(),
        ClarificationState {
            rounds: 1,
            waiting: true,
            ..ClarificationState::default()
        },
    );
    run.world.sessions.insert(
        "pty-9".into(),
        SessionFacts {
            alive: false,
            busy: false,
            settled_at: None,
        },
    );
    run.advance();
    assert_eq!(run.status("discover"), "failed");
    assert!(run
        .note("discover")
        .unwrap()
        .contains("The session ended without a summary."));
}

#[test]
fn a_pending_answer_fails_when_the_session_ends() {
    let mut run = Run::from(CLARIFY);
    session_up(&mut run);
    run.world.clarifications.insert(
        "discover".into(),
        ClarificationState {
            rounds: 1,
            pending: Some(PendingAnswer {
                round: 1,
                text: "The person answered.\n\nAnswer:\n30 seconds\n".into(),
            }),
            ..ClarificationState::default()
        },
    );
    run.world.sessions.insert(
        "pty-9".into(),
        SessionFacts {
            alive: false,
            busy: false,
            settled_at: None,
        },
    );
    run.advance();
    assert_eq!(run.status("discover"), "failed");
    assert!(run
        .note("discover")
        .unwrap()
        .contains("before the answer could be sent back"));
}

#[test]
fn an_open_question_does_not_finish_a_settled_session() {
    let mut run = Run::from(CLARIFY);
    session_up(&mut run);
    run.world.clarifications.insert(
        "discover".into(),
        ClarificationState {
            rounds: 1,
            pending: None,
            waiting: true,
            summary: false,
            undelivered: false,
        },
    );
    run.advance();
    assert_eq!(run.status("discover"), "waiting");
    assert!(run.finished.is_none());
}

#[test]
fn a_pending_answer_is_sent_back_to_the_same_session() {
    let mut run = Run::from(CLARIFY);
    session_up(&mut run);
    run.world.clarifications.insert(
        "discover".into(),
        ClarificationState {
            rounds: 1,
            waiting: false,
            summary: false,
            undelivered: false,
            pending: Some(PendingAnswer {
                round: 1,
                text: "The person answered.\n\nAnswer:\n30 seconds\n".into(),
            }),
        },
    );
    let plan = advance(&run.workflow, &run.inputs, &run.rows, &run.world);
    let change = plan
        .moves
        .iter()
        .find(|change| change.step == "discover")
        .expect("a delivery");
    let delivery = change
        .apply_answer
        .as_ref()
        .expect("the answer is typed back");
    assert_eq!(delivery.pty_id, "pty-9");
    assert_eq!(delivery.round, 1);
    assert!(delivery.text.contains("30 seconds"));
    assert_eq!(change.to, StepStatus::Waiting);
    assert!(plan.effects.is_empty());
}

#[test]
fn too_many_clarification_rounds_fail_the_step() {
    let mut run = Run::from(CLARIFY);
    session_up(&mut run);
    run.world.clarifications.insert(
        "discover".into(),
        ClarificationState {
            rounds: 4,
            pending: None,
            waiting: true,
            summary: false,
            undelivered: false,
        },
    );
    run.advance();
    assert_eq!(run.status("discover"), "failed");
    assert!(run.note("discover").unwrap().contains("past the limit (3)"));
}

#[test]
fn two_parallel_loops_keep_both_failure_notes() {
    let source = r#"id: demo
name: Demo
version: 1
trigger:
  kind: manual
steps:
  - id: plan
    action: notify
    title: Plan
  - id: left
    needs: [plan]
    action: run_test
    test: left
    on:
      passed: stop
      failed: plan
    max_attempts: 3
  - id: right
    needs: [plan]
    action: run_test
    test: right
    on:
      passed: stop
      failed: plan
    max_attempts: 3
"#;
    let mut run = Run::from(source);
    run.advance();
    finish_notify(&mut run, "plan");
    run.advance();
    assert!(run.apply(&queued(
        "left",
        &Outputs::from([("run".to_owned(), "left-run".to_owned())])
    )));
    assert!(run.apply(&queued(
        "right",
        &Outputs::from([("run".to_owned(), "right-run".to_owned())])
    )));
    run.world.test_runs.insert(
        "left-run".into(),
        TestRun {
            status: "failed".into(),
            summary: "left broke".into(),
        },
    );
    run.world.test_runs.insert(
        "right-run".into(),
        TestRun {
            status: "failed".into(),
            summary: "right broke".into(),
        },
    );
    let plan = advance(&run.workflow, &run.inputs, &run.rows, &run.world);
    let rewinds: Vec<_> = plan
        .moves
        .iter()
        .filter(|change| change.rewind.is_some())
        .collect();
    assert_eq!(rewinds.len(), 1, "overlapping rewinds are one change");
    let notes: Vec<_> = rewinds[0]
        .rewind
        .as_ref()
        .unwrap()
        .feedback
        .iter()
        .map(|copy| copy.name.as_str())
        .collect();
    assert!(notes.contains(&"failure-left"), "{notes:?}");
    assert!(notes.contains(&"failure-right"), "{notes:?}");
    run.advance();
    assert_eq!(run.status("plan"), "pending");
    assert_eq!(run.status("left"), "pending");
    assert_eq!(run.status("right"), "pending");
    assert!(run
        .world
        .artifacts
        .get("failure-left")
        .is_some_and(|body| body.contains("left broke")));
    assert!(run
        .world
        .artifacts
        .get("failure-right")
        .is_some_and(|body| body.contains("right broke")));
}
