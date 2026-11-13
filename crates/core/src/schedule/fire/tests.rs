//! The firing rules, with a made-up clock. Nothing here starts a process: a run is queued
//! through the real doors into a real store, and what the tests read is the rows.

use chrono::Duration;

use super::*;
use crate::schedule::tests::{nov, time, Fx, DEMO, RUNS_THE_TEST};
use crate::schedule::{set_enabled, IfMissed, ScheduleSpec, ScheduleStanding, ScheduleTarget};
use crate::store::Store;
use crate::workflow::runs::Accepting;

#[test]
fn a_failure_notice_is_the_first_line_kept_short() {
    let long = format!("Step `smoke` failed: {}\nthen the report", "x".repeat(400));
    let body = failure_notice(&long);
    let mut lines = body.lines();
    let first = lines.next().unwrap();
    assert_eq!(first.chars().count(), 203, "{first}");
    assert!(first.ends_with("..."));
    assert_eq!(lines.next(), Some("Open Yardsort for the report."));
    assert!(lines.next().is_none());
    assert_eq!(failure_notice("It broke."), "It broke.");
    assert_eq!(
        failure_notice("It broke.\nthen the report"),
        "It broke.\nOpen Yardsort for the report."
    );
}

fn daily() -> ScheduleSpec {
    ScheduleSpec::Daily { time: time(9, 0) }
}

fn fire(fx: &Fx, now: &chrono::DateTime<chrono_tz::Tz>) -> Vec<Fired> {
    tick(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        &fx.workspaces(),
        now,
    )
    .unwrap()
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
    assert!(tick(
        &elsewhere,
        &fx.data_dir,
        &Accepting::default(),
        &fx.workspaces(),
        &now
    )
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
    assert_eq!(
        fx.stands(&schedule.id),
        ScheduleStanding::Running,
        "its run is still going, whatever the newest line says"
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
fn a_once_missed_and_skipped_never_runs_and_says_so() {
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
    let after = fx.schedule(&schedule.id);
    assert!(!after.enabled);
    assert!(fx.store.active_workflow_runs().unwrap().is_empty());
    assert_eq!(
        fx.stands(&schedule.id),
        ScheduleStanding::Missed,
        "not done: it never ran"
    );
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

/// The blocking case: a workflow's `run_test` step with no timeout waits for as long as its
/// test does, and a test whose file nobody approved waits for a person. Started by a schedule,
/// the run would never end, and every later time would be skipped behind it.
#[test]
fn a_workflow_whose_test_would_wait_for_a_person_is_refused_at_its_time() {
    let fx = Fx::new();
    fx.write_workflow("smoke", RUNS_THE_TEST);
    fx.approve_test();
    let schedule = fx
        .try_add_of(
            ScheduleTarget::Workflow,
            "smoke",
            &daily(),
            IfMissed::default(),
            &nov(5, 10, 0),
        )
        .unwrap();

    // The file changes after the schedule was saved: an agent edited it, or a pull brought a
    // new version.
    fx.write_test(&DEMO.replace("--version", "--help"));
    let mut fired = fire(&fx, &nov(6, 9, 0));
    assert_eq!(fired.len(), 1);
    let first = fired.remove(0);
    assert_eq!(first.run.outcome, "refused");
    let note = first.run.note.as_deref().unwrap();
    assert!(note.contains("has changed since you approved it"), "{note}");
    assert!(
        fx.store.active_workflow_runs().unwrap().is_empty(),
        "no run was started to sit waiting"
    );
    assert!(fx.store.active_test_runs().unwrap().is_empty());
    // The user is told, since no run will end to tell them.
    let notice = first.notice.expect("told");
    assert_eq!(notice.title, "Smoke could not start");
    assert_eq!(notice.body, note);
    assert!(
        fx.schedule(&schedule.id).enabled,
        "approving is all it takes"
    );

    // Still not approved the next day: refused again, and not told again.
    let mut fired = fire(&fx, &nov(7, 9, 0));
    let second = fired.remove(0);
    assert_eq!(second.run.outcome, "refused");
    assert_eq!(second.notice, None, "once while it lasts");

    // Approved, it goes, and the run knows nobody is watching it.
    fx.approve_test();
    let third = only(fire(&fx, &nov(8, 9, 0)));
    assert_eq!(third.outcome, "started");
    let run = fx.store.active_workflow_runs().unwrap().remove(0);
    assert!(crate::workflow::runs::Facts::parse(&run.context).unattended);
}

#[test]
fn run_now_is_attended_so_its_workflow_may_wait_for_the_person_who_asked() {
    let fx = Fx::new();
    fx.write_workflow("smoke", RUNS_THE_TEST);
    fx.approve_test();
    let schedule = fx
        .try_add_of(
            ScheduleTarget::Workflow,
            "smoke",
            &daily(),
            IfMissed::default(),
            &nov(5, 10, 0),
        )
        .unwrap();
    fx.write_test(&DEMO.replace("--version", "--help"));

    let now = nov(5, 12, 0).timestamp_millis();
    let fired = run_now(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        &fx.workspaces(),
        &schedule.id,
        now,
    )
    .unwrap();
    assert_eq!(fired.run.outcome, "started");
    let run = fx.store.active_workflow_runs().unwrap().remove(0);
    assert!(!crate::workflow::runs::Facts::parse(&run.context).unattended);
}

#[test]
fn a_refusal_is_told_once_while_it_lasts_and_never_when_the_schedule_says_never() {
    let fx = Fx::new();
    let schedule = fx.add(ScheduleTarget::Workflow, &daily(), &nov(5, 10, 0));
    fx.store
        .set_workspace_archived(&fx.workspace.id, true)
        .unwrap();
    let told = |day| fire(&fx, &nov(day, 9, 0)).remove(0).notice;

    let first = told(6).expect("told the first time");
    assert_eq!(first.schedule_id, schedule.id);
    assert_eq!(first.title, "Ping could not start");
    assert!(first.body.contains("is archived"), "{}", first.body);
    assert_eq!(told(7), None, "not at every time that comes");

    // It starts again, and is refused again later: that is news.
    fx.store
        .set_workspace_archived(&fx.workspace.id, false)
        .unwrap();
    assert_eq!(told(8), None, "a run that started has nothing to tell yet");
    fx.store
        .set_workspace_archived(&fx.workspace.id, true)
        .unwrap();
    assert!(told(9).is_some());

    // A schedule that tells nothing tells nothing.
    let quiet = Fx::new();
    let schedule = quiet.add(ScheduleTarget::Workflow, &daily(), &nov(5, 10, 0));
    let row = quiet.schedule(&schedule.id);
    quiet
        .store
        .update_schedule(
            &row.id,
            &crate::store::ScheduleFields {
                name: &row.name,
                target_kind: &row.target_kind,
                target_id: &row.target_id,
                project_id: &row.project_id,
                workspace_id: row.workspace_id.as_deref(),
                new_workspace: false,
                base_branch: None,
                keep: 5,
                workspace_name: &row.workspace_name,
                inputs: &row.inputs,
                spec: &row.spec,
                missed: &row.missed,
                notify: "never",
                next_at: row.next_at,
            },
        )
        .unwrap();
    quiet
        .store
        .set_workspace_archived(&quiet.workspace.id, true)
        .unwrap();
    let mut fired = fire(&quiet, &nov(6, 9, 0));
    let line = fired.remove(0);
    assert_eq!(line.run.outcome, "refused");
    assert_eq!(line.notice, None);
}

#[test]
fn a_schedule_whose_next_time_cannot_be_worked_out_stays_on_and_says_it_was_refused() {
    let fx = Fx::new();
    let schedule = fx.add(ScheduleTarget::Workflow, &daily(), &nov(5, 10, 0));
    // What a schedule saved by a later Yardsort, or a cron this one reads differently, looks
    // like from here.
    let row = fx.schedule(&schedule.id);
    fx.store
        .update_schedule(
            &row.id,
            &crate::store::ScheduleFields {
                name: &row.name,
                target_kind: &row.target_kind,
                target_id: &row.target_id,
                project_id: &row.project_id,
                workspace_id: row.workspace_id.as_deref(),
                new_workspace: false,
                base_branch: None,
                keep: 5,
                workspace_name: &row.workspace_name,
                inputs: &row.inputs,
                spec: r#"{"kind":"cron","expr":"every so often"}"#,
                missed: &row.missed,
                notify: &row.notify,
                next_at: row.next_at,
            },
        )
        .unwrap();

    let mut fired = fire(&fx, &nov(6, 9, 0));
    let line = fired.remove(0);
    assert_eq!(line.run.outcome, "refused");
    let note = line.run.note.as_deref().unwrap();
    assert!(note.contains("a cron expression here has five"), "{note}");
    assert!(note.contains("until you edit it"), "{note}");
    assert!(line.notice.is_some());
    assert!(
        fx.store.active_workflow_runs().unwrap().is_empty(),
        "nothing ran"
    );

    // Not switched off, which is how a `once` that is done looks.
    let after = fx.schedule(&schedule.id);
    assert!(after.enabled);
    assert_eq!(after.next_at, None);
    assert_eq!(fx.stands(&schedule.id), ScheduleStanding::Refused);
    assert!(
        fire(&fx, &nov(7, 9, 0)).is_empty(),
        "and it is not asked every second"
    );
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
fn a_run_still_waiting_for_approval_is_said_however_many_times_are_skipped_behind_it() {
    let fx = Fx::new();
    let schedule = fx.add(ScheduleTarget::Test, &daily(), &nov(5, 10, 0));
    let first = only(fire(&fx, &nov(6, 9, 0)));
    settle(&fx.store).unwrap();
    assert_eq!(fx.stands(&schedule.id), ScheduleStanding::AwaitingApproval);

    // Nobody approves it, so each morning after is skipped: the run is in the way. The
    // schedule goes on saying what it waits for, and goes on being counted as needing the user.
    for day in [7, 8] {
        let line = only(fire(&fx, &nov(day, 9, 0)));
        assert_eq!(line.outcome, "skipped");
        assert_eq!(settle(&fx.store).unwrap(), Settled::default());
        let stands = fx.stands(&schedule.id);
        assert_eq!(stands, ScheduleStanding::AwaitingApproval, "on day {day}");
        assert!(stands.needs_attention());
    }

    // Approved and carried out, it is waiting for its next time again.
    let run_id = first.run_id.unwrap();
    fx.store.approve_test_run(&run_id).unwrap();
    assert_eq!(fx.stands(&schedule.id), ScheduleStanding::AwaitingApproval);
    fx.store.start_test_run(&run_id, 4000, "/tmp/out").unwrap();
    fx.store.finish_test_run(&run_id, "passed", None).unwrap();
    settle(&fx.store).unwrap();
    assert_eq!(fx.stands(&schedule.id), ScheduleStanding::Waiting);
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
        workspace_id: Some(&fx.workspace.id),
        new_workspace: false,
        base_branch: None,
        keep: 5,
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
        &fx.workspaces(),
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
        &fx.workspaces(),
        &schedule.id,
        now,
    )
    .unwrap_err();
    assert_eq!(again.code, "workflow_already_running");
    assert_eq!(fx.store.schedule_runs(&schedule.id, 10).unwrap().len(), 1);

    let missing = run_now(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        &fx.workspaces(),
        "nope",
        now,
    )
    .unwrap_err();
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

// --- a new workspace for each run ------------------------------------------------------------

/// A schedule of `ping` every ten minutes that makes its own workspaces, keeping `keep`.
fn add_each_run(fx: &Fx, keep: Option<u32>) -> IpcResult<ScheduleRow> {
    add_each_run_of(fx, ScheduleTarget::Workflow, "ping", keep)
}

fn add_each_run_of(
    fx: &Fx,
    target: ScheduleTarget,
    target_id: &str,
    keep: Option<u32>,
) -> IpcResult<ScheduleRow> {
    crate::schedule::add(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        &crate::schedule::Draft {
            name: None,
            target,
            target_id,
            workspace_id: "",
            new_workspace: Some(crate::schedule::EachRun {
                project_id: &fx.project_id,
                base: None,
                name: None,
                keep,
            }),
            inputs: &std::collections::BTreeMap::new(),
            spec: &ScheduleSpec::Every { minutes: 10 },
            missed: IfMissed::default(),
            notify: crate::schedule::ScheduleNotify::default(),
        },
        "app",
        &nov(5, 10, 0),
    )
}

/// The workspaces made in the fixture's project: every one with a branch.
fn made(fx: &Fx) -> Vec<WorkspaceRow> {
    fx.store
        .workspaces()
        .unwrap()
        .into_iter()
        .filter(|w| w.branch.is_some())
        .collect()
}

use crate::store::WorkspaceRow;

#[test]
fn a_schedule_for_a_new_workspace_makes_one_each_time_it_fires() {
    let fx = Fx::with_repository();
    let schedule = add_each_run(&fx, None).unwrap();
    assert!(schedule.new_workspace);
    assert_eq!(schedule.workspace_id, None);
    assert_eq!(schedule.keep, 5);
    assert_eq!(fx.stands(&schedule.id), ScheduleStanding::Waiting);
    assert!(made(&fx).is_empty(), "saving it makes nothing");

    let first = only(fire(&fx, &nov(5, 10, 10)));
    assert_eq!(first.outcome, "started", "{:?}", first.note);
    // Nobody carries the first run out, and the second time starts all the same: its run is
    // in a workspace of its own.
    let second = only(fire(&fx, &nov(5, 10, 20)));
    assert_eq!(second.outcome, "started", "{:?}", second.note);

    let names: Vec<String> = made(&fx).into_iter().map(|w| w.name).collect();
    assert_eq!(names, ["ping", "ping-2"]);
    let runs = fx.store.active_workflow_runs().unwrap();
    assert_eq!(runs.len(), 2);
    let made_ids: Vec<String> = made(&fx).into_iter().map(|w| w.id).collect();
    for run in &runs {
        assert!(made_ids.contains(run.workspace_id.as_ref().unwrap()));
    }
    assert_ne!(runs[0].workspace_id, runs[1].workspace_id);
    assert_eq!(
        fx.store.schedule_workspaces(&schedule.id).unwrap(),
        ["ping", "ping-2"]
    );
    assert!(fx.schedule(&schedule.id).enabled);
}

#[test]
fn a_schedule_makes_no_more_workspaces_than_it_keeps_and_removes_none() {
    let fx = Fx::with_repository();
    let schedule = add_each_run(&fx, Some(2)).unwrap();
    assert_eq!(only(fire(&fx, &nov(5, 10, 10))).outcome, "started");
    assert_eq!(only(fire(&fx, &nov(5, 10, 20))).outcome, "started");

    // Both runs end. Their workspaces stay: ending a run removes nothing.
    for run in fx.store.active_workflow_runs().unwrap() {
        fx.store
            .finish_workflow_run(&run.id, "succeeded", None)
            .unwrap();
    }
    settle(&fx.store).unwrap();

    // The third time finds both still there.
    let fired = fire(&fx, &nov(5, 10, 30));
    let notice = fired[0].notice.clone().expect("told once");
    let line = only(fired);
    assert_eq!(line.outcome, "refused");
    let note = line.note.unwrap();
    assert!(
        note.contains("`ping`") && note.contains("`ping-2`"),
        "{note}"
    );
    assert_eq!(notice.body, note);
    assert_eq!(made(&fx).len(), 2, "nothing made, nothing removed");
    for workspace in made(&fx) {
        assert!(std::path::Path::new(&workspace.path).is_dir());
    }
    assert_eq!(fx.stands(&schedule.id), ScheduleStanding::Refused);
    // It goes on asking: the user making room is all it waits for.
    assert!(fx.schedule(&schedule.id).enabled);

    // Refused again, and not told again.
    let fired = fire(&fx, &nov(5, 10, 40));
    assert!(fired[0].notice.is_none());
    assert_eq!(only(fired).outcome, "refused");

    // Run now is refused to the person the same way.
    let pressed = run_now(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        &fx.workspaces(),
        &schedule.id,
        nov(5, 10, 41).timestamp_millis(),
    )
    .unwrap_err();
    assert_eq!(pressed.code, "schedule_workspaces_full");

    // One archived, the next time has room.
    let first = made(&fx).remove(0);
    fx.store.set_workspace_archived(&first.id, true).unwrap();
    let line = only(fire(&fx, &nov(5, 10, 50)));
    assert_eq!(line.outcome, "started", "{:?}", line.note);
    assert_eq!(fx.store.schedule_workspaces(&schedule.id).unwrap().len(), 2);
}

#[test]
fn a_workspace_kept_after_its_setup_failed_counts_as_one_the_schedule_made() {
    let fx = Fx::with_repository();
    fx.store
        .set_project_automation(
            &fx.project_id,
            r#"{"copyFiles":[],"setup":{"program":"yardsort-no-such-program","args":[]},"run":null}"#,
        )
        .unwrap();
    let schedule = add_each_run(&fx, Some(1)).unwrap();

    let line = only(fire(&fx, &nov(5, 10, 10)));
    assert_eq!(line.outcome, "refused");
    assert!(line.note.unwrap().starts_with("Workspace kept at "));
    assert_eq!(made(&fx).len(), 1, "kept, as the composer keeps it");
    assert!(fx.store.active_workflow_runs().unwrap().is_empty());

    // It is the one this schedule keeps, so the next time makes no second.
    let line = only(fire(&fx, &nov(5, 10, 20)));
    assert_eq!(line.outcome, "refused");
    assert!(line.note.unwrap().contains("still there"));
    assert_eq!(made(&fx).len(), 1);
    assert_eq!(fx.store.schedule_workspaces(&schedule.id).unwrap().len(), 1);
}

#[test]
fn what_cannot_run_in_a_workspace_made_for_it_is_refused_when_the_schedule_is_saved() {
    let fx = Fx::with_repository();
    // A smoke test is a file in a workspace that is there.
    let test = add_each_run_of(&fx, ScheduleTarget::Test, "demo", None).unwrap_err();
    assert_eq!(test.code, "schedule_target");

    // A workflow that needs an open pull request when it starts would be refused every time.
    fx.write_workflow(
        "review",
        "id: review\nname: Review\nversion: 1\ntrigger:\n  kind: manual\nsteps:\n  - id: tell\n    action: notify\n    title: \"{{ pr.number }}\"\n",
    );
    let review = add_each_run_of(&fx, ScheduleTarget::Workflow, "review", None).unwrap_err();
    assert_eq!(review.code, "pull_request_missing");

    for keep in [0, 51] {
        assert_eq!(
            add_each_run(&fx, Some(keep)).unwrap_err().code,
            "schedule_keep"
        );
    }
    assert!(fx.store.schedules().unwrap().is_empty());
    assert!(made(&fx).is_empty());
}

#[test]
fn a_notice_about_a_run_in_a_made_workspace_names_that_workspace() {
    let fx = Fx::with_repository();
    let schedule = add_each_run(&fx, None).unwrap();
    fx.store
        .update_schedule(
            &schedule.id,
            &crate::store::ScheduleFields {
                name: &schedule.name,
                target_kind: &schedule.target_kind,
                target_id: &schedule.target_id,
                project_id: &schedule.project_id,
                workspace_id: None,
                workspace_name: "",
                new_workspace: true,
                base_branch: None,
                keep: 5,
                inputs: &schedule.inputs,
                spec: &schedule.spec,
                missed: &schedule.missed,
                notify: "always",
                next_at: schedule.next_at,
            },
        )
        .unwrap();
    let line = only(fire(&fx, &nov(5, 10, 10)));
    let run_id = line.run_id.unwrap();
    fx.store
        .finish_workflow_run(&run_id, "succeeded", None)
        .unwrap();
    let settled = settle(&fx.store).unwrap();
    assert_eq!(settled.notices.len(), 1);
    assert_eq!(settled.notices[0].body, "The workflow `ping` in `ping`.");
}

#[test]
fn a_run_that_is_still_making_its_workspace_holds_a_slot_against_a_second() {
    let fx = Fx::with_repository();
    let schedule = add_each_run(&fx, Some(1)).unwrap();
    // Another run of it, in this process or another, has taken the one slot and is in the
    // project's setup: its workspace is not there to be counted yet.
    let far_past = 0;
    let slot = fx
        .store
        .reserve_schedule_workspace(&schedule.id, schedule.keep, far_past)
        .unwrap()
        .expect("the one slot is free");
    assert_eq!(
        fx.store
            .reserve_schedule_workspace(&schedule.id, schedule.keep, far_past)
            .unwrap(),
        None,
        "taking the last slot twice"
    );

    let pressed = run_now(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        &fx.workspaces(),
        &schedule.id,
        nov(5, 10, 5).timestamp_millis(),
    )
    .unwrap_err();
    assert_eq!(pressed.code, "schedule_workspaces_full");
    assert!(pressed.message.contains("is making one now"), "{pressed:?}");
    assert!(made(&fx).is_empty(), "no second workspace beside the first");

    // That run made nothing after all and gave its slot back: the next has room.
    fx.store.release_schedule_workspace(&slot).unwrap();
    assert_eq!(only(fire(&fx, &nov(5, 10, 10))).outcome, "started");
    assert_eq!(made(&fx).len(), 1);

    // A refusal that makes nothing holds no slot: the workflow is gone, twice, and the slot
    // the first workspace holds is the only one counted.
    fx.store
        .set_workspace_archived(&made(&fx)[0].id, true)
        .unwrap();
    std::fs::remove_file(crate::workflow::user_dir(&fx.data_dir).join("ping.yaml")).unwrap();
    assert_eq!(only(fire(&fx, &nov(5, 10, 20))).outcome, "refused");
    assert_eq!(only(fire(&fx, &nov(5, 10, 30))).outcome, "refused");
    assert!(fx
        .store
        .reserve_schedule_workspace(&schedule.id, schedule.keep, far_past)
        .unwrap()
        .is_some());
}

#[test]
fn a_slot_left_by_a_process_that_died_is_dropped_and_a_filled_one_is_not() {
    let fx = Fx::with_repository();
    let schedule = add_each_run(&fx, Some(1)).unwrap();
    let slot = fx
        .store
        .reserve_schedule_workspace(&schedule.id, 1, 0)
        .unwrap()
        .unwrap();
    // Not stale yet: it was taken after the line.
    assert!(fx
        .store
        .reserve_schedule_workspace(&schedule.id, 1, 0)
        .unwrap()
        .is_none());
    // Taken before the line and never filled: dropped, and its place taken.
    let later = crate::store::now_ms() + 1;
    let second = fx
        .store
        .reserve_schedule_workspace(&schedule.id, 1, later)
        .unwrap()
        .expect("the stale slot is dropped");
    assert_ne!(slot, second);

    // Filled, it is a workspace, however old: it holds its place until archived or deleted.
    fx.store
        .fill_schedule_workspace(&second, &fx.workspace.id)
        .unwrap();
    fx.store.release_schedule_workspace(&second).unwrap();
    let much_later = crate::store::now_ms() + 10_000;
    assert!(fx
        .store
        .reserve_schedule_workspace(&schedule.id, 1, much_later)
        .unwrap()
        .is_none());
    assert_eq!(fx.store.schedule_workspaces(&schedule.id).unwrap(), ["fix"]);
}

#[test]
fn a_failed_setup_claims_its_own_workspace_and_no_other_made_meanwhile() {
    let fx = Fx::with_repository();
    fx.store
        .set_project_automation(
            &fx.project_id,
            r#"{"copyFiles":[],"setup":{"program":"yardsort-no-such-program","args":[]},"run":null}"#,
        )
        .unwrap();
    let schedule = add_each_run(&fx, Some(2)).unwrap();

    // The workspace a call made is said even when its setup fails and the call is an error.
    let seen = std::cell::RefCell::new(Vec::new());
    let other =
        fx.workspaces()
            .create_named_noting(&fx.project_id, None, "personal-work", &|workspace| {
                seen.borrow_mut().push(workspace.name.clone())
            });
    assert_eq!(other.unwrap_err().code, "workspace_setup_failed");
    assert_eq!(*seen.borrow(), ["personal-work"]);

    // While the schedule's attempt is under way, someone else goes on adding workspaces to
    // the same project, from another connection as `ys` would. A look at the whole project
    // before and after the attempt would take those for the schedule's.
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let adding = {
        let (stop, database, project) = (stop.clone(), fx.database.clone(), fx.project_id.clone());
        std::thread::spawn(move || {
            let elsewhere = Store::open(&database).unwrap();
            let mut added = 0;
            let mut tried = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) && tried < 2_000 {
                tried += 1;
                let name = format!("theirs-{tried}");
                // A busy database is not what this is about: try the next.
                let path = format!("/nowhere/{name}");
                if elsewhere
                    .add_worktree(&project, &name, &path, None, None)
                    .is_ok()
                {
                    added += 1;
                }
            }
            added
        })
    };
    // Let it get going, so some are added before the attempt and some during it.
    while fx.store.workspaces().unwrap().len() < 5 {
        std::thread::yield_now();
    }
    let line = only(fire(&fx, &nov(5, 10, 10)));
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(adding.join().unwrap() > 0);
    assert_eq!(line.outcome, "refused");
    assert_eq!(
        fx.store.schedule_workspaces(&schedule.id).unwrap(),
        ["ping"],
        "only the workspace this schedule made"
    );
}
