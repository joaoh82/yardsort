//! Seeing, gathering, deciding and settling, against a real store with a made-up clock.

use std::collections::BTreeMap;

use super::*;
use crate::schedule::ScheduleNotify;
use crate::trigger::tests::{Tx, NOW, RUNS_THE_TEST_IN_TIME};
use crate::trigger::{standing, Draft, Filters, TriggerStanding};
use crate::workflow::runs::{Accepting, Facts};

fn done(tx: &Tx, at: i64, note: &str) -> Observation {
    Observation {
        event: TriggerEvent::WorkspaceDone,
        workspace_id: tx.workspace_id().to_owned(),
        session_id: None,
        terminal: None,
        agent_run: None,
        at,
        facts: BTreeMap::from([("note".to_owned(), note.to_owned())]),
        depth: 0,
        target_id: None,
        status: None,
    }
}

fn settled(tx: &Tx, record: &str, terminal: &str, at: i64) -> Observation {
    Observation {
        event: TriggerEvent::AgentSettled,
        workspace_id: tx.workspace_id().to_owned(),
        session_id: Some(record.to_owned()),
        terminal: Some(terminal.to_owned()),
        agent_run: Some(format!("run-{record}")),
        at,
        facts: BTreeMap::new(),
        depth: 0,
        target_id: None,
        status: None,
    }
}

fn tick_at(tx: &Tx, now: i64) -> Vec<Fired> {
    let git = crate::git::testing::git();
    tick(
        tx.store(),
        &tx.fx.data_dir,
        &Accepting::default(),
        &git,
        now,
    )
    .unwrap()
}

fn fires(tx: &Tx, trigger: &str) -> Vec<TriggerFireRow> {
    tx.store().trigger_fires(trigger, 50).unwrap()
}

/// End the run a fire started, as its driver would.
fn finish(tx: &Tx, run_id: &str, status: &str) {
    tx.store().start_workflow_run(run_id, None).unwrap();
    assert!(tx
        .store()
        .finish_workflow_run(run_id, status, None)
        .unwrap());
}

#[test]
fn a_done_signal_starts_the_workflow_with_what_the_agent_said() {
    let tx = Tx::new();
    let trigger = tx.on_done(0, 3);
    assert_eq!(
        observe(tx.store(), &done(&tx, NOW + 5, "tests added")).unwrap(),
        1
    );

    let fired = tick_at(&tx, NOW + 5);
    assert_eq!(fired.len(), 1);
    let fire = &fired[0].fire;
    assert_eq!(fire.outcome, "started");
    assert!(fire.counted);
    assert!(fired[0].notice.is_none());

    let run = tx
        .store()
        .workflow_run(fire.run_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(run.workflow_id, "tell");
    assert_eq!(run.requested_by, "app");
    let facts = Facts::parse(&run.context);
    assert_eq!(
        facts.event.get("note").map(String::as_str),
        Some("tests added")
    );
    assert_eq!(
        facts.event.get("kind").map(String::as_str),
        Some("workspace.done")
    );
    assert_eq!(facts.event.get("count").map(String::as_str), Some("1"));
    assert!(!facts.unattended);
    assert_eq!(facts.depth, 0);

    let row = tx.store().trigger(&trigger.id).unwrap().unwrap();
    assert_eq!(
        standing(&row, &fires(&tx, &trigger.id)),
        TriggerStanding::Running
    );
}

#[test]
fn what_happened_before_the_trigger_was_saved_never_fires() {
    let tx = Tx::new();
    tx.on_done(0, 3);
    assert_eq!(observe(tx.store(), &done(&tx, NOW - 1, "old")).unwrap(), 0);
    assert!(tick_at(&tx, NOW + 60_000).is_empty());
}

#[test]
fn a_burst_is_one_fire_after_its_quiet_time() {
    let tx = Tx::new();
    let trigger = tx.on_done(120_000, 3);
    observe(tx.store(), &done(&tx, NOW + 1_000, "one")).unwrap();
    observe(tx.store(), &done(&tx, NOW + 61_000, "two")).unwrap();
    // Two minutes after the first is not two minutes after the last.
    assert!(tick_at(&tx, NOW + 121_000).is_empty());

    let fired = tick_at(&tx, NOW + 181_000);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].fire.events_count, 2);
    let run = fired[0].fire.run_id.clone().unwrap();
    let facts = Facts::parse(&tx.store().workflow_run(&run).unwrap().unwrap().context);
    assert_eq!(facts.event.get("note").map(String::as_str), Some("two"));
    assert_eq!(facts.event.get("count").map(String::as_str), Some("2"));
    assert_eq!(fires(&tx, &trigger.id).len(), 1);
}

#[test]
fn however_many_events_keep_coming_a_fire_is_decided_ten_minutes_after_its_first() {
    let tx = Tx::new();
    tx.on_done(120_000, 3);
    // One a minute, each moving the fire two minutes on, until the tenth minute holds it.
    for minute in 0..11 {
        observe(tx.store(), &done(&tx, NOW + minute * 60_000, "again")).unwrap();
    }
    assert!(tick_at(&tx, NOW + MOST_GATHER_MS - 1).is_empty());
    let fired = tick_at(&tx, NOW + MOST_GATHER_MS);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].fire.events_count, 11);
}

#[test]
fn an_event_while_a_run_of_the_same_thing_is_going_waits_for_it() {
    let tx = Tx::new();
    let trigger = tx.on_done(0, 5);
    observe(tx.store(), &done(&tx, NOW + 1, "first")).unwrap();
    let first = tick_at(&tx, NOW + 1)[0].fire.run_id.clone().unwrap();

    observe(tx.store(), &done(&tx, NOW + 2, "second")).unwrap();
    observe(tx.store(), &done(&tx, NOW + 3, "third")).unwrap();
    let fired = tick_at(&tx, NOW + 3);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].fire.outcome, "waiting");
    // Still waiting: nothing new to say.
    assert!(tick_at(&tx, NOW + 4).is_empty());
    let row = tx.store().trigger(&trigger.id).unwrap().unwrap();
    assert_eq!(
        standing(&row, &fires(&tx, &trigger.id)),
        TriggerStanding::Running
    );

    finish(&tx, &first, "succeeded");
    let fired = tick_at(&tx, NOW + 5);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].fire.outcome, "started");
    assert_eq!(
        fired[0].fire.events_count, 2,
        "one fire waits, with both events"
    );
    assert_eq!(fires(&tx, &trigger.id).len(), 2);
}

#[test]
fn a_run_someone_else_started_makes_it_wait_too() {
    let tx = Tx::new();
    tx.on_done(0, 3);
    let theirs = crate::workflow::runs::queue(
        tx.store(),
        &tx.fx.data_dir,
        &crate::workflow::runs::Request {
            workflow_id: "tell",
            workspace_id: tx.workspace_id(),
            inputs: &BTreeMap::new(),
            requested_by: "cli",
            unattended: false,
            origin_record: None,
            depth: 0,
            event: None,
        },
        &Accepting::default(),
    )
    .unwrap();
    observe(tx.store(), &done(&tx, NOW + 1, "mine")).unwrap();
    assert_eq!(tick_at(&tx, NOW + 1)[0].fire.outcome, "waiting");
    finish(&tx, &theirs, "succeeded");
    assert_eq!(tick_at(&tx, NOW + 2)[0].fire.outcome, "started");
}

#[test]
fn after_its_cap_it_stops_says_so_once_and_reset_lets_it_go_on() {
    let tx = Tx::new();
    let trigger = tx.on_done(0, 2);
    for round in 0..2 {
        observe(tx.store(), &done(&tx, NOW + round * 10 + 1, "round")).unwrap();
        let fired = tick_at(&tx, NOW + round * 10 + 1);
        assert_eq!(fired[0].fire.outcome, "started");
        finish(&tx, fired[0].fire.run_id.as_deref().unwrap(), "succeeded");
    }
    observe(tx.store(), &done(&tx, NOW + 30, "third")).unwrap();
    let fired = tick_at(&tx, NOW + 30);
    assert_eq!(fired[0].fire.outcome, "capped");
    let notice = fired[0].notice.as_ref().expect("told once");
    assert_eq!(notice.title, "Tell has stopped");

    // More events go onto the capped line, unannounced.
    observe(tx.store(), &done(&tx, NOW + 40, "fourth")).unwrap();
    assert!(tick_at(&tx, NOW + 40).is_empty());
    settle(tx.store()).unwrap();
    let lines = fires(&tx, &trigger.id);
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].outcome, "capped");
    assert_eq!(lines[0].events_count, 2);
    let row = tx.store().trigger(&trigger.id).unwrap().unwrap();
    assert_eq!(standing(&row, &lines), TriggerStanding::Capped);

    assert_eq!(reset(tx.store(), &trigger.id).unwrap(), 2);
    observe(tx.store(), &done(&tx, NOW + 50, "fifth")).unwrap();
    assert_eq!(tick_at(&tx, NOW + 50)[0].fire.outcome, "started");
}

#[test]
fn too_many_runs_in_an_hour_is_refused() {
    let tx = Tx::new();
    let trigger = tx.on_done(0, 100);
    for round in 0..RATE_PER_HOUR {
        observe(tx.store(), &done(&tx, NOW + round * 10 + 1, "again")).unwrap();
        let fired = tick_at(&tx, NOW + round * 10 + 1);
        finish(&tx, fired[0].fire.run_id.as_deref().unwrap(), "succeeded");
    }
    observe(tx.store(), &done(&tx, NOW + 1_000, "one more")).unwrap();
    let fired = tick_at(&tx, NOW + 1_000);
    assert_eq!(fired[0].fire.outcome, "refused");
    assert!(fired[0].notice.is_some());
    // An hour later the rate has room again.
    observe(tx.store(), &done(&tx, NOW + 3_700_000, "later")).unwrap();
    assert_eq!(tick_at(&tx, NOW + 3_700_000)[0].fire.outcome, "started");
    assert!(tx.store().trigger(&trigger.id).unwrap().unwrap().enabled);
}

#[test]
fn a_chain_of_triggered_runs_past_its_depth_is_refused() {
    let tx = Tx::new();
    tx.on_done(0, 3);
    let mut deep = done(&tx, NOW + 1, "deep");
    deep.depth = MOST_DEPTH + 1;
    observe(tx.store(), &deep).unwrap();
    let fired = tick_at(&tx, NOW + 1);
    assert_eq!(fired[0].fire.outcome, "refused");
    assert!(fired[0].fire.note.as_deref().unwrap().contains("loop"));

    let mut allowed = done(&tx, NOW + 2, "allowed");
    allowed.depth = MOST_DEPTH;
    observe(tx.store(), &allowed).unwrap();
    let fired = tick_at(&tx, NOW + 2);
    assert_eq!(fired[0].fire.outcome, "started");
    let run = fired[0].fire.run_id.clone().unwrap();
    let facts = Facts::parse(&tx.store().workflow_run(&run).unwrap().unwrap().context);
    assert_eq!(facts.depth, MOST_DEPTH);
}

#[test]
fn an_unapproved_test_waits_for_approval_and_the_user_is_told_once() {
    let tx = Tx::new();
    let inputs = BTreeMap::new();
    let filters = Filters::default();
    let events = [TriggerEvent::WorkspaceDone];
    let trigger = tx
        .try_add(&tx.draft(&events, &filters, ScheduleTarget::Test, "demo", &inputs))
        .unwrap();
    observe(tx.store(), &done(&tx, NOW + 1, "ready")).unwrap();
    let fired = tick_at(&tx, NOW + 1);
    assert_eq!(fired[0].fire.outcome, "started");
    let run = tx
        .store()
        .test_run(fired[0].fire.run_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "awaiting_approval");

    let settled = settle(tx.store()).unwrap();
    assert_eq!(settled.notices.len(), 1);
    assert_eq!(
        settled.notices[0].title,
        "Demo is waiting for your approval"
    );
    assert!(settle(tx.store()).unwrap().notices.is_empty(), "once");
    let row = tx.store().trigger(&trigger.id).unwrap().unwrap();
    assert_eq!(
        standing(&row, &fires(&tx, &trigger.id)),
        TriggerStanding::AwaitingApproval
    );

    // A second event finds the run waiting for approval in its way, and waits for it.
    observe(tx.store(), &done(&tx, NOW + 2, "again")).unwrap();
    assert_eq!(tick_at(&tx, NOW + 2)[0].fire.outcome, "waiting");
}

#[test]
fn a_workflow_whose_test_gained_a_step_a_person_does_is_refused_at_the_fire() {
    let tx = Tx::new();
    tx.fx.write_workflow("smoke", RUNS_THE_TEST_IN_TIME);
    let inputs = BTreeMap::new();
    let filters = Filters::default();
    let events = [TriggerEvent::WorkspaceDone];
    tx.try_add(&tx.draft(
        &events,
        &filters,
        ScheduleTarget::Workflow,
        "smoke",
        &inputs,
    ))
    .unwrap();
    tx.fx.write_test(
        "id: demo\nname: Demo\nversion: 1\nsteps:\n  - id: look\n    action: manual\n    prompt: Right?\n",
    );
    observe(tx.store(), &done(&tx, NOW + 1, "ready")).unwrap();
    let fired = tick_at(&tx, NOW + 1);
    assert_eq!(fired[0].fire.outcome, "refused");
    assert!(fired[0]
        .fire
        .note
        .as_deref()
        .unwrap()
        .contains("would wait for you"));
}

#[test]
fn a_finished_run_is_copied_onto_its_line_and_a_failure_is_told() {
    let tx = Tx::new();
    tx.on_done(0, 3);
    observe(tx.store(), &done(&tx, NOW + 1, "ready")).unwrap();
    let fired = tick_at(&tx, NOW + 1);
    let run = fired[0].fire.run_id.clone().unwrap();
    assert!(
        settle(tx.store()).unwrap().notices.is_empty(),
        "still going"
    );
    tx.store().start_workflow_run(&run, None).unwrap();
    tx.store()
        .finish_workflow_run(&run, "failed", Some("It broke."))
        .unwrap();
    let settled = settle(tx.store()).unwrap();
    assert!(settled.changed);
    assert_eq!(settled.notices.len(), 1);
    assert_eq!(settled.notices[0].title, "Tell failed");
    assert_eq!(settled.notices[0].body, "It broke.");
    let line = tx.store().trigger_fire(&fired[0].fire.id).unwrap().unwrap();
    assert_eq!(line.result.as_deref(), Some("failed"));
}

#[test]
fn a_trigger_set_to_never_tell_says_nothing() {
    let tx = Tx::new();
    let inputs = BTreeMap::new();
    let filters = Filters::default();
    let events = [TriggerEvent::WorkspaceDone];
    let draft = Draft {
        notify: ScheduleNotify::Never,
        cap: 1,
        ..tx.draft(&events, &filters, ScheduleTarget::Workflow, "tell", &inputs)
    };
    tx.try_add(&draft).unwrap();
    observe(tx.store(), &done(&tx, NOW + 1, "one")).unwrap();
    let run = tick_at(&tx, NOW + 1)[0].fire.run_id.clone().unwrap();
    finish(&tx, &run, "failed");
    assert!(settle(tx.store()).unwrap().notices.is_empty());
    observe(tx.store(), &done(&tx, NOW + 2, "two")).unwrap();
    let fired = tick_at(&tx, NOW + 2);
    assert_eq!(fired[0].fire.outcome, "capped");
    assert!(fired[0].notice.is_none());
}

#[test]
fn a_session_trigger_hears_its_own_agent_and_tells_no_one_when_it_is_not_running() {
    let tx = Tx::new();
    tx.agent("rec-1", "pty-1");
    tx.agent("rec-2", "pty-2");
    let inputs = BTreeMap::new();
    let filters = Filters::default();
    let events = [TriggerEvent::AgentSettled];
    let draft = Draft {
        session_id: Some("rec-1"),
        ..tx.draft(&events, &filters, ScheduleTarget::Workflow, "ping", &inputs)
    };
    let trigger = tx.try_add(&draft).unwrap();
    assert_eq!(
        observe(tx.store(), &settled(&tx, "rec-2", "pty-2", NOW + 1)).unwrap(),
        0
    );
    assert_eq!(
        observe(tx.store(), &settled(&tx, "rec-1", "pty-1", NOW + 2)).unwrap(),
        1
    );

    tx.store().end_session_by_pty("pty-1", Some(0)).unwrap();
    let fired = tick_at(&tx, NOW + 2);
    assert_eq!(fired[0].fire.outcome, "skipped");
    assert_eq!(fired[0].fire.subject, "session:rec-1");

    tx.store().mark_session_running("rec-1", "pty-1b").unwrap();
    observe(tx.store(), &settled(&tx, "rec-1", "pty-1b", NOW + 3)).unwrap();
    let fired = tick_at(&tx, NOW + 3);
    assert_eq!(fired[0].fire.outcome, "started");
    let run = fired[0].fire.run_id.clone().unwrap();
    let facts = Facts::parse(&tx.store().workflow_run(&run).unwrap().unwrap().context);
    assert_eq!(facts.origin_record.as_deref(), Some("rec-1"));
    assert_eq!(fires(&tx, &trigger.id).len(), 2);
}

#[test]
fn an_agent_a_workflow_started_is_that_workflows_and_sets_nothing_off() {
    let tx = Tx::new();
    tx.agent("reviewer", "pty-r");
    let inputs = BTreeMap::new();
    let filters = Filters::default();
    let events = [TriggerEvent::AgentSettled];
    tx.try_add(&tx.draft(&events, &filters, ScheduleTarget::Workflow, "ping", &inputs))
        .unwrap();
    // A run whose `start_session` step left the reviewer's terminal as its `session`.
    tx.fx.write_workflow(
        "starts",
        "id: starts\nname: Starts\nversion: 1\ntrigger:\n  kind: manual\nsteps:\n  - id: review\n    action: start_session\n    harness: claude\n",
    );
    let run = crate::workflow::runs::queue(
        tx.store(),
        &tx.fx.data_dir,
        &crate::workflow::runs::Request {
            workflow_id: "starts",
            workspace_id: tx.workspace_id(),
            inputs: &BTreeMap::new(),
            requested_by: "cli",
            unattended: false,
            origin_record: None,
            depth: 0,
            event: None,
        },
        &Accepting::default(),
    )
    .unwrap();
    tx.store().start_workflow_run(&run, None).unwrap();
    use crate::store::StepStatus;
    assert!(tx
        .store()
        .move_workflow_step(
            &run,
            "review",
            &[StepStatus::Pending],
            StepStatus::Succeeded,
            Some(r#"{"session":"pty-r","run":"run-reviewer"}"#),
            None,
        )
        .unwrap());
    assert_eq!(
        observe(tx.store(), &settled(&tx, "reviewer", "pty-r", NOW + 1)).unwrap(),
        0
    );
}

#[test]
fn going_back_to_work_lets_a_settling_go() {
    let tx = Tx::new();
    tx.agent("rec-1", "pty-1");
    let inputs = BTreeMap::new();
    let filters = Filters::default();
    let events = [TriggerEvent::AgentSettled];
    let draft = Draft {
        quiet_ms: 180_000,
        ..tx.draft(&events, &filters, ScheduleTarget::Workflow, "ping", &inputs)
    };
    let trigger = tx.try_add(&draft).unwrap();
    observe(tx.store(), &settled(&tx, "rec-1", "pty-1", NOW + 1)).unwrap();
    assert_eq!(went_busy(tx.store(), "rec-1").unwrap(), 1);
    assert!(tick_at(&tx, NOW + 200_000).is_empty());
    assert!(
        fires(&tx, &trigger.id).is_empty(),
        "no line for a turn that did not end"
    );
}

#[test]
fn a_finished_test_matches_its_filters() {
    let tx = Tx::new();
    let inputs = BTreeMap::new();
    let filters = Filters {
        test: Some("demo".into()),
        verdict: Some(VerdictFilter::Failed),
        ..Filters::default()
    };
    let events = [TriggerEvent::TestFinished];
    tx.try_add(&tx.draft(&events, &filters, ScheduleTarget::Workflow, "ping", &inputs))
        .unwrap();
    let ended = |test: &str, status: &str| Observation {
        event: TriggerEvent::TestFinished,
        workspace_id: tx.workspace_id().to_owned(),
        session_id: None,
        terminal: None,
        agent_run: None,
        at: NOW + 1,
        facts: BTreeMap::new(),
        depth: 1,
        target_id: Some(test.to_owned()),
        status: Some(status.to_owned()),
    };
    assert_eq!(observe(tx.store(), &ended("demo", "passed")).unwrap(), 0);
    assert_eq!(observe(tx.store(), &ended("other", "failed")).unwrap(), 0);
    assert_eq!(observe(tx.store(), &ended("demo", "errored")).unwrap(), 1);
}

#[test]
fn a_switched_off_trigger_lets_what_it_gathered_go() {
    let tx = Tx::new();
    let trigger = tx.on_done(60_000, 3);
    observe(tx.store(), &done(&tx, NOW + 1, "ready")).unwrap();
    crate::trigger::set_enabled(tx.store(), &trigger.id, false, NOW + 2).unwrap();
    assert!(tick_at(&tx, NOW + 120_000).is_empty());
    assert!(fires(&tx, &trigger.id).is_empty());
}

#[test]
fn new_commits_and_a_clean_worktree_are_looked_at_in_a_real_repository() {
    let tx = Tx::new();
    let git = crate::git::testing::git();
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    git.init(root).unwrap();
    git.run(root, &["checkout", "-b", "main"]).unwrap();
    git.initial_commit(root).unwrap();
    git.run(root, &["checkout", "-b", "fix"]).unwrap();
    let workspace = tx
        .store()
        .add_worktree(
            &tx.fx.project_id,
            "fix-real",
            &root.to_string_lossy(),
            Some("fix"),
            Some("main"),
        )
        .unwrap();
    tx.store()
        .add_session(&crate::store::NewSession {
            id: "rec-real",
            workspace_id: &workspace.id,
            harness_id: "claude",
            model: None,
            effort: None,
            harness_session_id: None,
            title: "Claude",
            forked_from: None,
            pty_session_id: "pty-real",
            prompt: None,
        })
        .unwrap();
    let (inputs, events) = (BTreeMap::new(), [TriggerEvent::AgentSettled]);
    let filters = Filters {
        new_commits: true,
        clean: true,
        ..Filters::default()
    };
    let trigger = crate::trigger::add(
        tx.store(),
        &tx.fx.data_dir,
        &Accepting::default(),
        &Draft {
            workspace_id: &workspace.id,
            cap: 10,
            ..tx.draft(&events, &filters, ScheduleTarget::Workflow, "ping", &inputs)
        },
        "app",
        NOW,
    )
    .unwrap();
    let settle_at = |at: i64| {
        let seen = Observation {
            workspace_id: workspace.id.clone(),
            terminal: Some("pty-real".into()),
            agent_run: None,
            ..settled(&tx, "rec-real", "pty-real", at)
        };
        observe(tx.store(), &seen).unwrap();
        let fired = tick(tx.store(), &tx.fx.data_dir, &Accepting::default(), &git, at).unwrap();
        for fired in &fired {
            if let Some(run) = &fired.fire.run_id {
                finish(&tx, run, "succeeded");
            }
        }
        fired
            .iter()
            .map(|f| f.fire.outcome.clone())
            .collect::<Vec<_>>()
    };

    // Nothing its base does not have yet: the turn ended, and that is all.
    assert!(settle_at(NOW + 1).is_empty());
    std::fs::write(root.join("a.txt"), "a").unwrap();
    git.commit_all(root, "a").unwrap();
    assert_eq!(settle_at(NOW + 2), ["started"]);
    // The same commit as last time: nothing new.
    assert!(settle_at(NOW + 3).is_empty());
    // New work, not committed: not clean.
    std::fs::write(root.join("b.txt"), "b").unwrap();
    assert!(settle_at(NOW + 4).is_empty());
    git.commit_all(root, "b").unwrap();
    assert_eq!(settle_at(NOW + 5), ["started"]);
    let lines = fires(&tx, &trigger.id);
    assert_eq!(lines.len(), 2, "a turn that did not count leaves no line");
    let head = git.commit_id(root, "HEAD").unwrap().unwrap();
    let facts: BTreeMap<String, String> = serde_json::from_str(&lines[0].facts).unwrap();
    assert_eq!(facts.get("head"), Some(&head));
}
