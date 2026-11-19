use super::*;
use crate::notifications::{
    list, seen, settled_terminal, Notification, NotificationKind, NotificationPart,
};
use crate::store::{
    now_ms, FireDecision, NewScheduleRun, NewSession, NewTestRun, NewTriggerFire, NewWorkflowRun,
    ScheduleFields, StepStatus, TriggerFields,
};

/// A project with one workspace, in a store of its own, and a list that has had its first look.
struct Fixture {
    store: Store,
    project: String,
    workspace: String,
}

impl Fixture {
    fn new() -> Self {
        let store = Store::in_memory();
        let project = store
            .add_project("weather-cli", "/code/weather-cli")
            .unwrap();
        let workspace = store
            .add_worktree(
                &project.id,
                "fix-timezone-offsets",
                "/code/weather-cli-fix",
                Some("ys/fix-timezone-offsets"),
                Some("main"),
            )
            .unwrap();
        // A first look a moment ago: what ends from now on is news.
        assert!(!read(&store, now_ms() - 1_000).unwrap());
        Self {
            store,
            project: project.id,
            workspace: workspace.id,
        }
    }

    /// A run of the workflow `review`, going, with these steps as `(id, action)`.
    fn run(&self, id: &str, steps: &[(&str, &str)]) {
        self.store
            .queue_workflow_run(&NewWorkflowRun {
                id,
                workflow_id: "review",
                workflow_name: "Review",
                definition: "id: review\n",
                project_id: &self.project,
                workspace_id: &self.workspace,
                workspace_name: "fix-timezone-offsets",
                inputs: "{}",
                context: "{}",
                requested_by: "app",
                steps,
            })
            .unwrap();
        assert!(self.store.start_workflow_run(id, None).unwrap());
    }

    /// `step` of `run` starts waiting for a person: `kind` is an approval or a question.
    fn waits(&self, run: &str, step: &str, kind: &str, prompt: Option<&str>) {
        assert!(self
            .store
            .move_workflow_step(
                run,
                step,
                &[StepStatus::Pending],
                StepStatus::Waiting,
                None,
                None,
                None,
                false,
            )
            .unwrap());
        assert!(self
            .store
            .open_workflow_wait(run, step, kind, "you", "Do it.", prompt, "[]", "{}")
            .unwrap());
    }

    fn read(&self) -> bool {
        read(&self.store, now_ms() + 1).unwrap()
    }

    fn list(&self) -> Vec<Notification> {
        list(&self.store, now_ms() + 2).unwrap()
    }

    fn parts(&self) -> Vec<(NotificationKind, NotificationPart)> {
        self.list().iter().map(|n| (n.kind, n.part)).collect()
    }

    fn schedule(&self, name: &str, workspace: Option<&str>) -> String {
        self.store
            .add_schedule(
                &ScheduleFields {
                    name,
                    target_kind: "workflow",
                    target_id: "review",
                    project_id: &self.project,
                    workspace_id: workspace,
                    workspace_name: "fix-timezone-offsets",
                    new_workspace: false,
                    base_branch: None,
                    keep: 3,
                    inputs: "{}",
                    spec: "{}",
                    missed: "skip",
                    notify: "failure",
                    next_at: Some(now_ms() + 3_600_000),
                },
                "app",
            )
            .unwrap()
            .id
    }

    fn trigger(&self, name: &str) -> String {
        self.store
            .add_trigger(
                &TriggerFields {
                    name,
                    preset: None,
                    project_id: &self.project,
                    workspace_id: &self.workspace,
                    workspace_name: "fix-timezone-offsets",
                    session_id: None,
                    events: "[\"agent_done\"]",
                    filters: "{}",
                    target_kind: "workflow",
                    target_id: "review",
                    inputs: "{}",
                    quiet_ms: 0,
                    cap: 1,
                    notify: "failure",
                },
                "app",
                0,
            )
            .unwrap()
            .id
    }

    /// A fire of `trigger`, decided as `outcome`.
    fn fire(&self, trigger: &str, outcome: &str, note: Option<&str>, counted: bool) {
        let at = now_ms();
        let fire = self
            .store
            .open_new_trigger_fire(&NewTriggerFire {
                trigger_id: trigger,
                subject: &format!("workspace:{}", self.workspace),
                event: "agent_done",
                at,
                due_at: at,
                facts: "{}",
                depth: 0,
            })
            .unwrap();
        assert!(self
            .store
            .decide_trigger_fire(
                &fire.id,
                "gathering",
                &FireDecision {
                    outcome,
                    run_id: None,
                    note,
                    counted,
                    at,
                },
            )
            .unwrap());
    }
}

#[test]
fn a_workflow_waiting_for_an_approval_is_one_row_until_it_is_decided() {
    let f = Fixture::new();
    f.run("run-1", &[("sign-off", "approve")]);
    assert!(!f.read(), "a run that only goes is nothing to tell");

    f.waits("run-1", "sign-off", waits::KIND_APPROVAL, None);
    assert!(f.read());
    let rows = f.list();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.kind, NotificationKind::WorkflowApproval);
    assert_eq!(row.part, NotificationPart::Needs);
    assert_eq!(row.workspace_id.as_deref(), Some(f.workspace.as_str()));
    assert_eq!(row.project_name, "weather-cli");
    assert_eq!(row.title, "Review");
    assert_eq!(row.workflow_id.as_deref(), Some("review"));
    assert_eq!(row.subject_id.as_deref(), Some("run-1"));
    assert_eq!(row.step_id.as_deref(), Some("sign-off"));

    // Looking again is not a second row, and nothing to tell.
    assert!(!f.read());
    assert_eq!(f.list().len(), 1);
    // A row that waits cannot be marked seen: it leaves when it is answered.
    assert_eq!(
        seen(&f.store, std::slice::from_ref(&row.id), now_ms()).unwrap(),
        0,
        "a waiting row is not seen away"
    );

    // Decided, and not applied by the engine yet: the person has done their part.
    f.store
        .record_workflow_approval("run-1", "sign-off", "approved", None, "josh", None)
        .unwrap();
    assert!(f.read());
    assert_eq!(
        f.parts(),
        [(
            NotificationKind::WorkflowApproval,
            NotificationPart::Earlier
        )]
    );
    assert!(!f.read());
}

#[test]
fn a_workflows_question_is_shown_and_leaves_when_it_is_answered() {
    let f = Fixture::new();
    f.run("run-1", &[("clarify", "wait_session")]);
    f.waits(
        "run-1",
        "clarify",
        waits::KIND_QUESTION,
        Some("\nWhich zones should the fix cover?\nAll of them, or only half-hour ones?"),
    );
    assert!(f.read());
    let rows = f.list();
    assert_eq!(rows[0].kind, NotificationKind::WorkflowQuestion);
    assert_eq!(rows[0].part, NotificationPart::Needs);
    assert_eq!(
        rows[0].said.as_deref(),
        Some("Which zones should the fix cover?")
    );

    f.store
        .answer_workflow_wait("run-1", "clarify", "All of them.", "josh", None)
        .unwrap();
    assert!(f.read());
    assert_eq!(
        f.parts(),
        [(
            NotificationKind::WorkflowQuestion,
            NotificationPart::Earlier
        )]
    );
}

#[test]
fn a_run_that_fails_while_it_waits_takes_its_question_with_it() {
    let f = Fixture::new();
    f.run("run-1", &[("sign-off", "approve")]);
    f.waits("run-1", "sign-off", waits::KIND_APPROVAL, None);
    assert!(f.read());
    assert!(f
        .store
        .finish_workflow_run("run-1", "failed", Some("The agent left."))
        .unwrap());
    assert!(f.read());
    // Newest first among what still waits, then what left.
    assert_eq!(
        f.parts(),
        [
            (NotificationKind::WorkflowFailed, NotificationPart::Finished),
            (
                NotificationKind::WorkflowApproval,
                NotificationPart::Earlier
            ),
        ]
    );
}

#[test]
fn a_run_that_ends_is_a_finished_row_and_one_that_was_cancelled_is_not() {
    let f = Fixture::new();
    f.run("run-1", &[("work", "start_session")]);
    assert!(f
        .store
        .finish_workflow_run("run-1", "succeeded", None)
        .unwrap());
    assert!(f.read());
    let rows = f.list();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, NotificationKind::WorkflowSucceeded);
    assert_eq!(rows[0].part, NotificationPart::Finished);
    assert_eq!(rows[0].title, "Review");
    assert_eq!(rows[0].subject_id.as_deref(), Some("run-1"));
    // Read again, the same ending is the same row.
    assert!(!f.read());
    assert_eq!(f.list().len(), 1);

    // Looked at: it is under Earlier, and stays one row.
    assert_eq!(
        seen(&f.store, std::slice::from_ref(&rows[0].id), now_ms()).unwrap(),
        1
    );
    assert!(!f.read());
    assert_eq!(
        f.parts(),
        [(
            NotificationKind::WorkflowSucceeded,
            NotificationPart::Earlier
        )]
    );

    // A person cancelled this one: they know.
    f.run("run-2", &[("work", "start_session")]);
    assert!(f.store.cancel_workflow_run("run-2", "Not now.").unwrap());
    assert!(!f.read());
    assert_eq!(f.list().len(), 1);
}

#[test]
fn a_run_its_person_rejected_is_not_news_to_them() {
    let f = Fixture::new();
    for (run, decision, feedback) in [
        ("run-1", "rejected", None),
        (
            "run-2",
            "changes_requested",
            Some("Cover the half-hour zones too."),
        ),
    ] {
        f.run(run, &[("sign-off", "approve")]);
        f.waits(run, "sign-off", waits::KIND_APPROVAL, None);
        assert!(f.read());
        f.store
            .record_workflow_approval(run, "sign-off", decision, feedback, "josh", None)
            .unwrap();
        // The engine applies it: the step fails, and the run with it.
        assert!(f
            .store
            .move_workflow_step(
                run,
                "sign-off",
                &[StepStatus::Waiting],
                StepStatus::Failed,
                None,
                Some("Rejected."),
                None,
                false,
            )
            .unwrap());
        assert!(f
            .store
            .finish_workflow_run(run, "failed", Some("Step `sign-off` failed: rejected."))
            .unwrap());
        assert!(f.read(), "its question leaves");
        assert!(
            f.list()
                .iter()
                .all(|row| row.kind == NotificationKind::WorkflowApproval
                    && row.part == NotificationPart::Earlier),
            "{decision}: the run's end is not listed"
        );
    }
}

#[test]
fn a_failed_run_says_why_and_leaves_when_it_is_taken_up_again() {
    let f = Fixture::new();
    f.run("run-1", &[("work", "start_session")]);
    assert!(f
        .store
        .finish_workflow_run(
            "run-1",
            "failed",
            Some("The agent's terminal closed.\nNothing more."),
        )
        .unwrap());
    assert!(f.read());
    let rows = f.list();
    assert_eq!(rows[0].kind, NotificationKind::WorkflowFailed);
    assert_eq!(rows[0].part, NotificationPart::Finished);
    assert_eq!(
        rows[0].note.as_deref(),
        Some("The agent's terminal closed.")
    );

    // Continued: the run is going again, and what was said of its failing is over.
    f.store.continue_workflow_run("run-1", None).unwrap();
    assert!(f.read());
    assert_eq!(
        f.parts(),
        [(NotificationKind::WorkflowFailed, NotificationPart::Earlier)]
    );

    // It ends a second time: a second row, for the second ending.
    std::thread::sleep(std::time::Duration::from_millis(3));
    assert!(f
        .store
        .finish_workflow_run("run-1", "succeeded", None)
        .unwrap());
    assert!(f.read());
    assert_eq!(
        f.parts(),
        [
            (
                NotificationKind::WorkflowSucceeded,
                NotificationPart::Finished
            ),
            (NotificationKind::WorkflowFailed, NotificationPart::Earlier),
        ]
    );
}

#[test]
fn a_run_that_ended_before_the_first_look_is_not_news() {
    let store = Store::in_memory();
    let project = store
        .add_project("weather-cli", "/code/weather-cli")
        .unwrap();
    let workspace = store
        .add_worktree(&project.id, "fix", "/code/weather-cli-fix", None, None)
        .unwrap();
    let f = Fixture {
        store,
        project: project.id,
        workspace: workspace.id,
    };
    f.run("run-1", &[("work", "start_session")]);
    assert!(f
        .store
        .finish_workflow_run("run-1", "failed", None)
        .unwrap());
    assert!(!f.read());
    assert!(f.list().is_empty());
    // A wait that is open at the first look is listed all the same: it needs its person now.
    f.run("run-2", &[("sign-off", "approve")]);
    f.waits("run-2", "sign-off", waits::KIND_APPROVAL, None);
    assert!(f.read());
    assert_eq!(
        f.parts(),
        [(NotificationKind::WorkflowApproval, NotificationPart::Needs)]
    );
}

#[test]
fn a_smoke_test_waiting_for_its_file_to_be_approved_is_a_row_until_it_is_let_run() {
    let f = Fixture::new();
    let queue = |id: &str, test: &str| {
        f.store
            .queue_test_run(&NewTestRun {
                id,
                project_id: &f.project,
                workspace_id: &f.workspace,
                workspace_name: "fix-timezone-offsets",
                test_id: test,
                test_name: "Half-hour zones",
                definition: "name: Half-hour zones\n",
                content_hash: "abc",
                inputs: "{}",
                status: "awaiting_approval",
                requested_by: "cli",
                session_record_id: None,
                steps: &[],
            })
            .unwrap();
    };
    queue("test-1", "zones");
    queue("test-2", "offsets");
    assert!(f.read());
    let rows = f.list();
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|row| row.kind == NotificationKind::TestWaits
            && row.part == NotificationPart::Needs
            && row.title == "Half-hour zones"));

    assert!(f.store.approve_test_run("test-1").unwrap());
    assert!(f.store.refuse_test_run("test-2").unwrap());
    assert!(f.read());
    assert!(f
        .list()
        .iter()
        .all(|row| row.part == NotificationPart::Earlier));
}

#[test]
fn a_schedule_that_was_refused_needs_you_until_its_next_run_starts() {
    let f = Fixture::new();
    let schedule = f.schedule("Nightly review", Some(&f.workspace));
    assert!(!f.read(), "a schedule with a time to come needs nobody");

    let at = now_ms();
    f.store
        .record_schedule_run(&NewScheduleRun {
            schedule_id: &schedule,
            due_at: at,
            fired_at: at,
            outcome: "refused",
            manual: false,
            run_id: None,
            note: Some("The workflow `review` is not there any more."),
        })
        .unwrap();
    assert!(f.read());
    let rows = f.list();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, NotificationKind::ScheduleRefused);
    assert_eq!(rows[0].part, NotificationPart::Needs);
    assert_eq!(rows[0].title, "Nightly review");
    assert_eq!(rows[0].workspace_name, "fix-timezone-offsets");
    assert_eq!(rows[0].at, at);
    assert_eq!(
        rows[0].note.as_deref(),
        Some("The workflow `review` is not there any more.")
    );
    assert!(!f.read());

    // Its next time starts a run.
    f.store
        .record_schedule_run(&NewScheduleRun {
            schedule_id: &schedule,
            due_at: at + 10,
            fired_at: at + 10,
            outcome: "started",
            manual: false,
            run_id: Some("run-9"),
            note: None,
        })
        .unwrap();
    assert!(f.read());
    let rows = f.list();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, NotificationKind::ScheduleRefused);
    assert_eq!(rows[0].part, NotificationPart::Earlier);
    // Why it was refused is not said of a schedule that has run since.
    assert_eq!(rows[0].note, None);
}

#[test]
fn a_schedule_whose_run_waits_for_a_test_is_not_a_row_of_its_own() {
    let f = Fixture::new();
    let schedule = f.schedule("Nightly tests", Some(&f.workspace));
    let at = now_ms();
    let line = f
        .store
        .record_schedule_run(&NewScheduleRun {
            schedule_id: &schedule,
            due_at: at,
            fired_at: at,
            outcome: "started",
            manual: false,
            run_id: Some("test-1"),
            note: None,
        })
        .unwrap();
    assert!(f.store.mark_schedule_run_told_waiting(&line.id).unwrap());
    // The test's run is the row, when there is one; the schedule adds none.
    assert!(!f.read());
    assert!(f.list().is_empty());
}

#[test]
fn a_schedule_and_a_trigger_whose_workspace_is_gone_are_rows_under_their_project() {
    let f = Fixture::new();
    f.schedule("Nightly review", Some(&f.workspace));
    f.trigger("Review when done");
    f.store
        .conn()
        .execute("DELETE FROM workspaces WHERE id = ?", [&f.workspace])
        .unwrap();
    assert!(f.read());
    let mut rows = f.list();
    rows.sort_by_key(|row| row.title.clone());
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].kind, NotificationKind::ScheduleNoWorkspace);
    assert_eq!(rows[1].kind, NotificationKind::TriggerNoWorkspace);
    for row in &rows {
        assert_eq!(row.part, NotificationPart::Needs);
        assert_eq!(row.workspace_id, None);
        // What the workspace was called is kept by the schedule and the trigger.
        assert_eq!(row.workspace_name, "fix-timezone-offsets");
        assert_eq!(row.project_name, "weather-cli");
    }
    assert!(!f.read());

    // Removed, they wait on nobody.
    let schedule = rows[0].subject_id.clone().unwrap();
    assert!(f.store.remove_schedule(&schedule).unwrap());
    assert!(f.read());
    let rows = f.list();
    assert_eq!(rows.len(), 1, "a row whose schedule is gone is not shown");
    assert_eq!(rows[0].kind, NotificationKind::TriggerNoWorkspace);
}

#[test]
fn a_trigger_that_was_refused_or_reached_its_limit_needs_you() {
    let f = Fixture::new();
    let trigger = f.trigger("Review when done");
    assert!(!f.read(), "a trigger that listens needs nobody");

    f.fire(
        &trigger,
        "refused",
        Some("The workspace has no agent."),
        false,
    );
    assert!(f.read());
    let rows = f.list();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, NotificationKind::TriggerRefused);
    assert_eq!(rows[0].part, NotificationPart::Needs);
    assert_eq!(rows[0].title, "Review when done");
    assert_eq!(rows[0].note.as_deref(), Some("The workspace has no agent."));

    // Its next fire starts a run, which counts against its limit of one.
    std::thread::sleep(std::time::Duration::from_millis(2));
    f.fire(&trigger, "started", None, true);
    // That run ended, as its own row would say.
    let started = f.store.unsettled_trigger_fires().unwrap();
    assert!(f
        .store
        .settle_trigger_fire(&started[0].id, "succeeded", false)
        .unwrap());
    assert!(f.read());
    assert_eq!(
        f.parts(),
        [(NotificationKind::TriggerRefused, NotificationPart::Earlier)]
    );

    // One more event, and the limit holds it.
    std::thread::sleep(std::time::Duration::from_millis(2));
    f.fire(&trigger, "capped", Some("It has had its 1 run."), false);
    assert!(f.read());
    let rows = f.list();
    assert_eq!(rows[0].kind, NotificationKind::TriggerCapped);
    assert_eq!(rows[0].part, NotificationPart::Needs);
    assert_eq!(rows[0].note.as_deref(), Some("It has had its 1 run."));

    // Reset lifts the limit: nothing is held by it any more.
    assert_eq!(f.store.reset_trigger(&trigger).unwrap(), 1);
    assert!(f.read());
    assert!(f
        .list()
        .iter()
        .all(|row| row.part == NotificationPart::Earlier));
}

#[test]
fn an_agent_a_running_workflow_started_finishing_is_not_a_row() {
    let f = Fixture::new();
    let agent = |record: &str, terminal: &str| {
        f.store
            .add_session(&NewSession {
                id: record,
                workspace_id: &f.workspace,
                harness_id: "claude",
                model: None,
                effort: None,
                harness_session_id: None,
                title: "Review the change",
                forked_from: None,
                pty_session_id: terminal,
                prompt: None,
            })
            .unwrap();
    };
    agent("s1", "pty-1");
    agent("s2", "pty-2");
    f.run("run-1", &[("work", "start_session")]);
    assert!(f
        .store
        .move_workflow_step(
            "run-1",
            "work",
            &[StepStatus::Pending],
            StepStatus::Running,
            Some("{\"session\":\"pty-1\"}"),
            None,
            None,
            false,
        )
        .unwrap());

    // The run reads its own agent's turn and goes on: nobody has to look.
    assert!(!settled_terminal(&f.store, "pty-1", 30_000, now_ms()).unwrap());
    // Another agent in the same workspace is the person's own.
    assert!(settled_terminal(&f.store, "pty-2", 30_000, now_ms()).unwrap());
    assert_eq!(
        f.parts(),
        [(NotificationKind::AgentFinished, NotificationPart::Finished)]
    );

    // Once the run is over, its agent is an agent like any other.
    assert!(f
        .store
        .finish_workflow_run("run-1", "succeeded", None)
        .unwrap());
    assert!(settled_terminal(&f.store, "pty-1", 30_000, now_ms()).unwrap());
}
