//! The firing rules, with a made-up clock. Nothing here starts a process: a run is queued
//! through the real doors into a real store, and what the tests read is the rows.

use chrono::Duration;

use super::*;
use crate::schedule::tests::{nov, time, Fx};
use crate::schedule::{set_enabled, IfMissed, ScheduleSpec, ScheduleTarget};
use crate::store::Store;
use crate::workflow::runs::Accepting;

fn daily() -> ScheduleSpec {
    ScheduleSpec::Daily { time: time(9, 0) }
}

fn fire(fx: &Fx, now: &chrono::DateTime<chrono_tz::Tz>) -> Vec<Fired> {
    tick(&fx.store, &fx.data_dir, &Accepting::default(), now).unwrap()
}

/// The one line a tick wrote.
fn only(fired: Vec<Fired>) -> ScheduleRunRow {
    assert_eq!(fired.len(), 1, "{fired:?}");
    fired.into_iter().next().unwrap().run
}

#[test]
fn nothing_fires_before_its_time() {
    let fx = Fx::new();
    fx.add(ScheduleTarget::Workflow, &daily(), &nov(5, 10, 0));
    assert!(fire(&fx, &nov(6, 8, 59)).is_empty());
    assert!(fx.store.active_workflow_runs().unwrap().is_empty());
}

#[test]
fn a_workflow_is_queued_when_its_time_comes_and_the_schedule_moves_on() {
    let fx = Fx::new();
    let schedule = fx.add(ScheduleTarget::Workflow, &daily(), &nov(5, 10, 0));
    let due = nov(6, 9, 0);

    // The tick that finds it runs a second after the time, as a tick does.
    let line = only(fire(&fx, &(due + Duration::seconds(1))));
    assert_eq!(line.outcome, "started");
    assert_eq!(line.due_at, due.timestamp_millis());
    assert!(!line.manual);

    let runs = fx.store.active_workflow_runs().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(Some(runs[0].id.as_str()), line.run_id.as_deref());
    assert_eq!(runs[0].workflow_id, "ping");
    assert_eq!(
        runs[0].workspace_id.as_deref(),
        Some(fx.workspace.id.as_str())
    );
    assert_eq!(runs[0].requested_by, "app");

    let after = fx.schedule(&schedule.id);
    assert!(after.enabled);
    assert_eq!(after.next_at, Some(nov(7, 9, 0).timestamp_millis()));
}

#[test]
fn a_time_is_acted_on_once_by_a_second_tick_or_a_second_store() {
    let fx = Fx::new();
    fx.add(ScheduleTarget::Workflow, &daily(), &nov(5, 10, 0));
    let now = nov(6, 9, 0);
    assert_eq!(fire(&fx, &now).len(), 1);
    assert!(fire(&fx, &now).is_empty());

    let elsewhere = Store::open(&fx.database).unwrap();
    assert!(tick(&elsewhere, &fx.data_dir, &Accepting::default(), &now)
        .unwrap()
        .is_empty());
    assert_eq!(fx.store.active_workflow_runs().unwrap().len(), 1);
}

#[test]
fn a_time_that_comes_while_the_last_run_is_still_going_is_skipped() {
    let fx = Fx::new();
    let schedule = fx.add(
        ScheduleTarget::Workflow,
        &ScheduleSpec::Every { minutes: 10 },
        &nov(5, 10, 0),
    );
    assert_eq!(only(fire(&fx, &nov(5, 10, 10))).outcome, "started");

    // Nobody carries the run out, so it is still queued ten minutes later.
    let line = only(fire(&fx, &nov(5, 10, 20)));
    assert_eq!(line.outcome, "skipped");
    assert_eq!(line.run_id, None);
    assert_eq!(
        line.note.as_deref(),
        Some("The previous run was still going.")
    );
    assert_eq!(
        fx.store.active_workflow_runs().unwrap().len(),
        1,
        "nothing piled up"
    );

    // It is not paused for that: the time after is asked again, and by then the run is over.
    let run = fx.store.active_workflow_runs().unwrap().remove(0);
    fx.store
        .finish_workflow_run(&run.id, "succeeded", None)
        .unwrap();
    assert_eq!(only(fire(&fx, &nov(5, 10, 30))).outcome, "started");
    assert!(fx.schedule(&schedule.id).enabled);
}

#[test]
fn a_missed_time_runs_once_however_many_were_missed() {
    let fx = Fx::new();
    let schedule = fx.add(ScheduleTarget::Workflow, &daily(), &nov(5, 10, 0));

    // The laptop was closed for a week. Seven nine o'clocks passed.
    let opened = nov(13, 14, 0);
    let line = only(fire(&fx, &opened));
    assert_eq!(line.outcome, "started");
    assert_eq!(
        line.due_at,
        nov(6, 9, 0).timestamp_millis(),
        "the first one missed"
    );
    assert_eq!(line.fired_at, opened.timestamp_millis());
    assert_eq!(fx.store.active_workflow_runs().unwrap().len(), 1);

    // And the next is tomorrow's, not another of the missed ones.
    assert_eq!(
        fx.schedule(&schedule.id).next_at,
        Some(nov(14, 9, 0).timestamp_millis())
    );
    assert!(fire(&fx, &(opened + Duration::seconds(1))).is_empty());
}

#[test]
fn a_missed_time_is_let_go_when_the_schedule_skips_those() {
    let fx = Fx::new();
    let schedule = fx
        .try_add(
            ScheduleTarget::Workflow,
            &daily(),
            IfMissed::Skip,
            &nov(5, 10, 0),
        )
        .unwrap();

    let line = only(fire(&fx, &nov(13, 14, 0)));
    assert_eq!(line.outcome, "missed");
    assert_eq!(line.run_id, None);
    assert!(fx.store.active_workflow_runs().unwrap().is_empty());
    assert_eq!(
        fx.schedule(&schedule.id).next_at,
        Some(nov(14, 9, 0).timestamp_millis())
    );

    // On time the next morning, it runs.
    assert_eq!(only(fire(&fx, &nov(14, 9, 0))).outcome, "started");
}

#[test]
fn a_few_minutes_late_is_not_missed() {
    let fx = Fx::new();
    fx.try_add(
        ScheduleTarget::Workflow,
        &daily(),
        IfMissed::Skip,
        &nov(5, 10, 0),
    )
    .unwrap();
    let late = nov(6, 9, 0) + Duration::milliseconds(LATE_AFTER_MS);
    assert_eq!(only(fire(&fx, &late)).outcome, "started");
}

#[test]
fn a_once_fires_and_is_done() {
    let fx = Fx::new();
    let once = ScheduleSpec::Once {
        at: nov(5, 18, 0).timestamp_millis(),
    };
    let schedule = fx.add(ScheduleTarget::Workflow, &once, &nov(5, 10, 0));
    assert_eq!(only(fire(&fx, &nov(5, 18, 0))).outcome, "started");

    let done = fx.schedule(&schedule.id);
    assert!(!done.enabled);
    assert_eq!(done.next_at, None);
    assert!(fire(&fx, &nov(6, 18, 0)).is_empty());
}

#[test]
fn a_once_missed_and_skipped_is_done_without_running() {
    let fx = Fx::new();
    let once = ScheduleSpec::Once {
        at: nov(5, 18, 0).timestamp_millis(),
    };
    let schedule = fx
        .try_add(
            ScheduleTarget::Workflow,
            &once,
            IfMissed::Skip,
            &nov(5, 10, 0),
        )
        .unwrap();
    assert_eq!(only(fire(&fx, &nov(6, 8, 0))).outcome, "missed");
    assert!(!fx.schedule(&schedule.id).enabled);
    assert!(fx.store.active_workflow_runs().unwrap().is_empty());
}

#[test]
fn an_archived_workspace_is_refused_and_asked_again_next_time() {
    let fx = Fx::new();
    let schedule = fx.add(ScheduleTarget::Workflow, &daily(), &nov(5, 10, 0));
    fx.store
        .set_workspace_archived(&fx.workspace.id, true)
        .unwrap();

    let line = only(fire(&fx, &nov(6, 9, 0)));
    assert_eq!(line.outcome, "refused");
    assert!(
        line.note.as_deref().unwrap().contains("is archived"),
        "{:?}",
        line.note
    );
    assert!(fx.store.active_workflow_runs().unwrap().is_empty());
    assert!(
        fx.schedule(&schedule.id).enabled,
        "restoring it is all it takes"
    );

    fx.store
        .set_workspace_archived(&fx.workspace.id, false)
        .unwrap();
    assert_eq!(only(fire(&fx, &nov(7, 9, 0))).outcome, "started");
}

#[test]
fn a_deleted_workspace_pauses_the_schedule() {
    let fx = Fx::new();
    let schedule = fx.add(ScheduleTarget::Workflow, &daily(), &nov(5, 10, 0));
    assert!(fx.store.remove_worktree(&fx.workspace.id).unwrap());

    let line = only(fire(&fx, &nov(6, 9, 0)));
    assert_eq!(line.outcome, "refused");
    assert!(line.note.as_deref().unwrap().contains("`fix` was deleted"));
    assert!(!fx.schedule(&schedule.id).enabled);
    assert!(fire(&fx, &nov(7, 9, 0)).is_empty());

    // And it cannot be switched back on until it is given somewhere to run.
    let refused = set_enabled(&fx.store, &schedule.id, true, &nov(7, 10, 0)).unwrap_err();
    assert_eq!(refused.code, "workspace_not_found");
}

#[test]
fn a_workflow_that_is_no_longer_there_is_refused() {
    let fx = Fx::new();
    fx.add(ScheduleTarget::Workflow, &daily(), &nov(5, 10, 0));
    std::fs::remove_file(crate::workflow::user_dir(&fx.data_dir).join("ping.yaml")).unwrap();
    let line = only(fire(&fx, &nov(6, 9, 0)));
    assert_eq!(line.outcome, "refused");
    assert_eq!(line.note.as_deref(), Some("There is no workflow `ping`."));
}

#[test]
fn a_scheduled_test_nobody_approved_waits_for_approval_and_the_user_is_told_once() {
    let fx = Fx::new();
    fx.add(ScheduleTarget::Test, &daily(), &nov(5, 10, 0));
    let line = only(fire(&fx, &nov(6, 9, 0)));
    assert_eq!(line.outcome, "started");

    let run = fx
        .store
        .test_run(line.run_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        run.status, "awaiting_approval",
        "a schedule approves nothing"
    );
    assert_eq!(run.requested_by, "app");

    let told = settle(&fx.store).unwrap();
    assert!(told.changed);
    assert_eq!(told.notices.len(), 1);
    assert_eq!(told.notices[0].title, "Demo is waiting for your approval");
    assert!(told.notices[0].body.contains("`demo`"));
    assert_eq!(
        settle(&fx.store).unwrap(),
        Settled::default(),
        "once, not every second"
    );

    // Approved in the app, it is an ordinary queued run, and the line is still open.
    assert!(fx.store.approve_test_run(&run.id).unwrap());
    assert_eq!(settle(&fx.store).unwrap(), Settled::default());
    assert_eq!(fx.store.unsettled_schedule_runs().unwrap().len(), 1);
}

#[test]
fn an_approved_test_is_queued_straight_away() {
    let fx = Fx::new();
    fx.add(ScheduleTarget::Test, &daily(), &nov(5, 10, 0));
    // Approve the file as it is, the way a first run in the app does.
    let first = only(fire(&fx, &nov(6, 9, 0)));
    let run_id = first.run_id.unwrap();
    fx.store.approve_test_run(&run_id).unwrap();
    fx.store.cancel_test_run(&run_id, "Cancelled.").unwrap();

    let second = only(fire(&fx, &nov(7, 9, 0)));
    let run = fx
        .store
        .test_run(second.run_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "queued");

    // Until someone changes the file: then it is shown again.
    fx.store.cancel_test_run(&run.id, "Cancelled.").unwrap();
    fx.write_test(&crate::schedule::tests::DEMO.replace("--version", "--help"));
    let third = only(fire(&fx, &nov(8, 9, 0)));
    let run = fx
        .store
        .test_run(third.run_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "awaiting_approval");
}

#[test]
fn a_test_that_gained_a_step_a_person_does_is_refused_at_its_time() {
    let fx = Fx::new();
    fx.add(ScheduleTarget::Test, &daily(), &nov(5, 10, 0));
    fx.write_test(&format!(
        "{}  - id: look\n    action: manual\n    prompt: Does it look right?\n",
        crate::schedule::tests::DEMO
    ));
    let line = only(fire(&fx, &nov(6, 9, 0)));
    assert_eq!(line.outcome, "refused");
    assert!(line
        .note
        .as_deref()
        .unwrap()
        .contains("a step a person does"));
    assert!(fx.store.active_test_runs().unwrap().is_empty());
}

/// Fire a workflow schedule with this `notify`, end its run this way, and settle.
fn ended(notify: &str, status: &str, error: Option<&str>) -> Settled {
    let fx = Fx::new();
    let schedule = fx.add(ScheduleTarget::Workflow, &daily(), &nov(5, 10, 0));
    let fields = crate::store::ScheduleFields {
        name: "Nightly",
        target_kind: &schedule.target_kind,
        target_id: &schedule.target_id,
        project_id: &schedule.project_id,
        workspace_id: &fx.workspace.id,
        workspace_name: &schedule.workspace_name,
        inputs: &schedule.inputs,
        spec: &schedule.spec,
        missed: &schedule.missed,
        notify,
        next_at: schedule.next_at,
    };
    fx.store.update_schedule(&schedule.id, &fields).unwrap();

    let line = only(fire(&fx, &nov(6, 9, 0)));
    let run_id = line.run_id.unwrap();
    assert_eq!(
        settle(&fx.store).unwrap(),
        Settled::default(),
        "still going"
    );
    fx.store.start_workflow_run(&run_id, None).unwrap();
    fx.store
        .finish_workflow_run(&run_id, status, error)
        .unwrap();

    let settled = settle(&fx.store).unwrap();
    assert!(settled.changed);
    let history = fx.store.schedule_runs(&schedule.id, 10).unwrap();
    assert_eq!(history[0].result.as_deref(), Some(status));
    assert_eq!(
        history[0].notified_at.is_some(),
        !settled.notices.is_empty()
    );
    assert_eq!(
        settle(&fx.store).unwrap(),
        Settled::default(),
        "settled once"
    );
    settled
}

#[test]
fn a_failure_is_told_by_default_and_a_success_is_not() {
    let failed = ended(
        "failure",
        "failed",
        Some("The agent did not settle in time."),
    );
    assert_eq!(failed.notices.len(), 1);
    assert_eq!(failed.notices[0].title, "Nightly failed");
    assert_eq!(failed.notices[0].body, "The agent did not settle in time.");

    assert!(ended("failure", "succeeded", None).notices.is_empty());
    // Cancelling is something a person did; it is not news to them.
    assert!(ended("failure", "cancelled", Some("Cancelled from ys."))
        .notices
        .is_empty());
}

#[test]
fn always_tells_every_ending_and_never_tells_none() {
    let finished = ended("always", "succeeded", None);
    assert_eq!(finished.notices[0].title, "Nightly finished");
    assert_eq!(finished.notices[0].body, "The workflow `ping` in `fix`.");
    assert_eq!(
        ended("always", "cancelled", Some("Cancelled from ys.")).notices[0].title,
        "Nightly was cancelled"
    );
    assert!(ended("never", "failed", Some("Broke.")).notices.is_empty());
}

#[test]
fn run_now_starts_a_run_and_leaves_the_next_time_alone() {
    let fx = Fx::new();
    let schedule = fx.add(ScheduleTarget::Workflow, &daily(), &nov(5, 10, 0));
    let now = nov(5, 12, 0).timestamp_millis();
    let fired = run_now(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        &schedule.id,
        now,
    )
    .unwrap();
    assert_eq!(fired.run.outcome, "started");
    assert!(fired.run.manual);
    assert_eq!(fx.store.active_workflow_runs().unwrap().len(), 1);
    assert_eq!(fx.schedule(&schedule.id).next_at, schedule.next_at);

    // A second press while that run is going is refused to the person, and not written down.
    let again = run_now(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        &schedule.id,
        now,
    )
    .unwrap_err();
    assert_eq!(again.code, "workflow_already_running");
    assert_eq!(fx.store.schedule_runs(&schedule.id, 10).unwrap().len(), 1);

    let missing = run_now(&fx.store, &fx.data_dir, &Accepting::default(), "nope", now).unwrap_err();
    assert_eq!(missing.code, "schedule_not_found");
}

#[test]
fn a_change_of_zone_moves_times_of_day_and_leaves_intervals_alone() {
    let fx = Fx::new();
    let at_nine = fx.add(ScheduleTarget::Workflow, &daily(), &nov(5, 10, 0));
    let every = fx.add(
        ScheduleTarget::Test,
        &ScheduleSpec::Every { minutes: 90 },
        &nov(5, 10, 0),
    );
    assert!(
        !rezone(&fx.store, &nov(5, 10, 0)).unwrap(),
        "same zone, nothing to move"
    );

    // The same moment on a machine now in New York, where it is five in the morning: the next
    // nine o'clock is today's there, which is two in the afternoon in Lisbon.
    let moved_to = nov(5, 10, 0).with_timezone(&chrono_tz::America::New_York);
    assert!(rezone(&fx.store, &moved_to).unwrap());
    assert_eq!(
        fx.schedule(&at_nine.id).next_at,
        Some(nov(5, 14, 0).timestamp_millis())
    );
    assert_eq!(fx.schedule(&every.id).next_at, every.next_at);
}

#[test]
fn a_test_approved_where_it_was_scheduled_runs_without_waiting_unless_it_changed() {
    use crate::smoke::runs::{approve_seen, content_hash};
    let fx = Fx::new();
    fx.add(ScheduleTarget::Test, &daily(), &nov(5, 10, 0));
    let shown = content_hash(crate::schedule::tests::DEMO);

    // The dialog showed the file as it is: saving approves it, and the first run goes.
    assert!(approve_seen(&fx.store, &fx.workspace.id, "demo", &shown).unwrap());
    let first = only(fire(&fx, &nov(6, 9, 0)));
    let run = fx
        .store
        .test_run(first.run_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "queued");
    fx.store.cancel_test_run(&run.id, "Cancelled.").unwrap();

    // An agent edits the file between the dialog reading it and Save: nothing is approved.
    fx.write_test(&crate::schedule::tests::DEMO.replace("--version", "--eval"));
    assert!(!approve_seen(&fx.store, &fx.workspace.id, "demo", &shown).unwrap());
    let second = only(fire(&fx, &nov(7, 9, 0)));
    let run = fx
        .store
        .test_run(second.run_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "awaiting_approval");
    assert!(!approve_seen(&fx.store, &fx.workspace.id, "no-such-test", &shown).unwrap());
}
