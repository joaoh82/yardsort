use std::collections::{BTreeMap, HashMap};

use super::*;
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
        let row = self
            .rows
            .iter_mut()
            .find(|r| r.step_id == change.step)
            .unwrap();
        let current = StepStatus::parse(&row.status).unwrap();
        if !change.from.contains(&current) {
            return false;
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
