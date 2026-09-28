use std::collections::HashMap;

use super::*;
use crate::workflow::parse;

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
}

impl World for Fake {
    fn now(&self) -> i64 {
        self.now
    }
    fn session(&self, pty_id: &str) -> SessionFacts {
        self.sessions.get(pty_id).copied().unwrap_or_default()
    }
    fn var(&self, path: &[String]) -> Option<String> {
        (path.join(".") == "workspace.branch").then(|| "fix-login".to_owned())
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
            })
            .collect();
        Self {
            workflow,
            inputs: [("who".to_owned(), "claude".to_owned())].into(),
            rows,
            world: Fake {
                now: 1_000_000,
                sessions: HashMap::new(),
            },
            finished: None,
        }
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
