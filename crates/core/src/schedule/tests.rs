//! What a schedule may hold, and the fixture the firing tests share: a real store in a temp
//! data folder, a real worktree folder with a smoke test in it, a workflow in the user's
//! folder, and a clock that is whatever the test says it is.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, NaiveDate, TimeZone};
use chrono_tz::Tz;

use super::*;
use crate::store::{ScheduleRow, Store, WorkspaceRow};
use crate::workflow::runs::Accepting;

pub(crate) const ZONE: Tz = chrono_tz::Europe::Lisbon;

/// A wall-clock time in [`ZONE`], in November 2026.
pub(crate) fn nov(day: u32, hour: u32, minute: u32) -> DateTime<Tz> {
    ZONE.from_local_datetime(
        &NaiveDate::from_ymd_opt(2026, 11, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap(),
    )
    .earliest()
    .unwrap()
}

pub(crate) fn time(hour: u8, minute: u8) -> TimeOfDay {
    TimeOfDay { hour, minute }
}

/// One step that needs no agent and no terminal.
const PING: &str = r#"id: ping
name: Ping
version: 1
trigger:
  kind: manual
inputs:
  - id: note
    kind: text
  - id: depth
    kind: choice
    options: [quick, deep]
    default: quick
steps:
  - id: tell
    action: notify
    title: "{{ inputs.note }}"
"#;

pub(crate) const DEMO: &str = r#"id: demo
name: Demo
version: 1
steps:
  - id: version
    action: run
    program: node
    args: [--version]
"#;

const ASKS_A_PERSON: &str = r#"id: demo
name: Demo
version: 1
steps:
  - id: version
    action: run
    program: node
    args: [--version]
  - id: look
    action: manual
    prompt: Does it look right?
"#;

pub(crate) struct Fx {
    _dir: tempfile::TempDir,
    pub data_dir: PathBuf,
    pub database: PathBuf,
    pub store: Store,
    pub project_id: String,
    pub workspace: WorkspaceRow,
}

impl Fx {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let database = data_dir.join("yardsort.db");
        let store = Store::open(&database).unwrap();
        let project = store
            .add_project("app", &dir.path().join("code").to_string_lossy())
            .unwrap();
        let worktree = dir.path().join("wt");
        std::fs::create_dir_all(crate::smoke::dir(&worktree)).unwrap();
        std::fs::write(crate::smoke::dir(&worktree).join("demo.yaml"), DEMO).unwrap();
        let workspace = store
            .add_worktree(&project.id, "fix", &worktree.to_string_lossy(), None, None)
            .unwrap();
        let folder = crate::workflow::user_dir(&data_dir);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("ping.yaml"), PING).unwrap();
        Self {
            _dir: dir,
            data_dir,
            database,
            store,
            project_id: project.id,
            workspace,
        }
    }

    /// Replace the worktree's smoke test file.
    pub fn write_test(&self, text: &str) {
        let folder = crate::smoke::dir(std::path::Path::new(&self.workspace.path));
        std::fs::write(folder.join("demo.yaml"), text).unwrap();
    }

    pub fn try_add(
        &self,
        target: ScheduleTarget,
        spec: &ScheduleSpec,
        missed: IfMissed,
        now: &DateTime<Tz>,
    ) -> IpcResult<ScheduleRow> {
        let target_id = match target {
            ScheduleTarget::Workflow => "ping",
            ScheduleTarget::Test => "demo",
        };
        add(
            &self.store,
            &self.data_dir,
            &Accepting::default(),
            &Draft {
                name: None,
                target,
                target_id,
                workspace_id: &self.workspace.id,
                inputs: &BTreeMap::new(),
                spec,
                missed,
                notify: ScheduleNotify::default(),
            },
            "app",
            now,
        )
    }

    pub fn add(
        &self,
        target: ScheduleTarget,
        spec: &ScheduleSpec,
        now: &DateTime<Tz>,
    ) -> ScheduleRow {
        self.try_add(target, spec, IfMissed::default(), now)
            .unwrap()
    }

    pub fn schedule(&self, id: &str) -> ScheduleRow {
        self.store.schedule(id).unwrap().unwrap()
    }
}

fn daily() -> ScheduleSpec {
    ScheduleSpec::Daily { time: time(9, 0) }
}

#[test]
fn a_new_schedule_is_on_named_after_its_target_and_knows_its_first_time() {
    let fx = Fx::new();
    let schedule = fx.add(ScheduleTarget::Test, &daily(), &nov(5, 10, 0));
    assert_eq!(schedule.name, "Demo");
    assert!(schedule.enabled);
    assert_eq!(
        schedule.next_at,
        Some(nov(6, 9, 0).timestamp_millis()),
        "ten o'clock is past nine, so the first time is tomorrow's"
    );
    assert_eq!(schedule.workspace_name, "fix");
    assert_eq!(schedule.project_id, fx.project_id);
    assert_eq!(ScheduleSpec::parse(&schedule.spec), Some(daily()));
    assert_eq!(
        (schedule.missed.as_str(), schedule.notify.as_str()),
        ("run_once", "failure")
    );
}

#[test]
fn a_schedule_is_refused_for_what_its_run_would_be_refused_for() {
    let fx = Fx::new();
    let now = nov(5, 10, 0);
    let inputs = BTreeMap::new();
    let draft = |target, target_id, workspace_id| Draft {
        name: Some("  Mine  "),
        target,
        target_id,
        workspace_id,
        inputs: &inputs,
        spec: &ScheduleSpec::Every { minutes: 30 },
        missed: IfMissed::Skip,
        notify: ScheduleNotify::Always,
    };
    let code = |draft: &Draft<'_>| {
        add(
            &fx.store,
            &fx.data_dir,
            &Accepting::default(),
            draft,
            "cli",
            &now,
        )
        .unwrap_err()
        .code
    };
    assert_eq!(
        code(&draft(ScheduleTarget::Workflow, "nope", &fx.workspace.id)),
        "workflow_not_found"
    );
    assert_eq!(
        code(&draft(ScheduleTarget::Test, "nope", &fx.workspace.id)),
        "test_not_found"
    );
    assert_eq!(
        code(&draft(ScheduleTarget::Test, "demo", "no-such-workspace")),
        "workspace_not_found"
    );

    let unknown: BTreeMap<String, String> = [("colour".to_owned(), "red".to_owned())].into();
    let mut asked = draft(ScheduleTarget::Workflow, "ping", &fx.workspace.id);
    asked.inputs = &unknown;
    assert_eq!(code(&asked), "workflow_inputs");
    asked.target = ScheduleTarget::Test;
    asked.target_id = "demo";
    assert_eq!(code(&asked), "test_inputs");

    fx.store
        .set_workspace_archived(&fx.workspace.id, true)
        .unwrap();
    assert_eq!(
        code(&draft(ScheduleTarget::Test, "demo", &fx.workspace.id)),
        "workspace_archived"
    );
    assert!(
        fx.store.schedules().unwrap().is_empty(),
        "a refusal writes nothing"
    );

    fx.store
        .set_workspace_archived(&fx.workspace.id, false)
        .unwrap();
    let kept = add(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        &draft(ScheduleTarget::Workflow, "ping", &fx.workspace.id),
        "cli",
        &now,
    )
    .unwrap();
    assert_eq!(
        (
            kept.name.as_str(),
            kept.missed.as_str(),
            kept.notify.as_str(),
            kept.created_by.as_str()
        ),
        ("Mine", "skip", "always", "cli")
    );
}

#[test]
fn a_test_with_a_step_a_person_does_cannot_be_scheduled() {
    let fx = Fx::new();
    fx.write_test(ASKS_A_PERSON);
    let refused = fx
        .try_add(
            ScheduleTarget::Test,
            &daily(),
            IfMissed::default(),
            &nov(5, 10, 0),
        )
        .unwrap_err();
    assert_eq!(refused.code, "schedule_needs_person");
    assert!(refused.message.contains("`look`"), "{}", refused.message);
}

#[test]
fn a_spec_is_checked_in_words() {
    let now = nov(5, 10, 0);
    let problem = |spec: ScheduleSpec| check(&spec, &now).unwrap_err();
    assert_eq!(
        problem(ScheduleSpec::Once {
            at: nov(5, 9, 59).timestamp_millis()
        }),
        "That time has already passed."
    );
    assert!(problem(ScheduleSpec::Every { minutes: 0 }).contains("every minute"));
    assert!(problem(ScheduleSpec::Daily { time: time(24, 0) }).contains("`24:00`"));
    assert!(problem(ScheduleSpec::Weekly {
        days: vec![],
        time: time(9, 0)
    })
    .contains("at least one day"));
    assert!(problem(ScheduleSpec::Monthly {
        day: Some(31),
        time: time(9, 0)
    })
    .contains("1 to 28"));
    assert!(problem(ScheduleSpec::Cron {
        expr: "nope".to_owned()
    })
    .contains("has 1 field;"));
    assert_eq!(
        problem(ScheduleSpec::Cron {
            expr: "0 0 30 2 *".to_owned()
        }),
        "That never comes."
    );

    for fine in [
        ScheduleSpec::Once {
            at: nov(5, 10, 1).timestamp_millis(),
        },
        ScheduleSpec::Every { minutes: 1 },
        ScheduleSpec::Monthly {
            day: None,
            time: time(23, 59),
        },
        ScheduleSpec::Cron {
            expr: "*/5 9-17 * * mon-fri".to_owned(),
        },
    ] {
        assert_eq!(check(&fine, &now), Ok(()), "{fine:?}");
    }
}

#[test]
fn a_spec_reads_the_way_it_was_written() {
    let words = |spec: ScheduleSpec| describe(&spec, &ZONE);
    assert_eq!(
        words(ScheduleSpec::Once {
            at: nov(5, 18, 0).timestamp_millis()
        }),
        "5 Nov 2026 at 18:00"
    );
    assert_eq!(words(ScheduleSpec::Every { minutes: 1 }), "Every minute");
    assert_eq!(
        words(ScheduleSpec::Every { minutes: 45 }),
        "Every 45 minutes"
    );
    assert_eq!(words(ScheduleSpec::Every { minutes: 60 }), "Every hour");
    assert_eq!(words(ScheduleSpec::Every { minutes: 120 }), "Every 2 hours");
    assert_eq!(words(ScheduleSpec::Every { minutes: 1440 }), "Every day");
    assert_eq!(words(ScheduleSpec::Every { minutes: 2880 }), "Every 2 days");
    assert_eq!(words(daily()), "Daily at 09:00");
    assert_eq!(
        words(ScheduleSpec::Weekly {
            days: vec![
                Weekday::Fri,
                Weekday::Mon,
                Weekday::Thu,
                Weekday::Tue,
                Weekday::Wed
            ],
            time: time(9, 0)
        }),
        "Weekdays at 09:00"
    );
    assert_eq!(
        words(ScheduleSpec::Weekly {
            days: vec![Weekday::Sun, Weekday::Sat],
            time: time(10, 30)
        }),
        "Weekends at 10:30"
    );
    assert_eq!(
        words(ScheduleSpec::Weekly {
            days: vec![Weekday::Thu, Weekday::Mon],
            time: time(7, 5)
        }),
        "Mon, Thu at 07:05"
    );
    assert_eq!(
        words(ScheduleSpec::Monthly {
            day: Some(1),
            time: time(6, 0)
        }),
        "Monthly on the 1st at 06:00"
    );
    assert_eq!(
        words(ScheduleSpec::Monthly {
            day: Some(22),
            time: time(6, 0)
        }),
        "Monthly on the 22nd at 06:00"
    );
    assert_eq!(
        words(ScheduleSpec::Monthly {
            day: None,
            time: time(6, 0)
        }),
        "Monthly on the last day at 06:00"
    );
    assert_eq!(
        words(ScheduleSpec::Cron {
            expr: " 0 9 * * 1-5 ".to_owned()
        }),
        "Cron `0 9 * * 1-5`"
    );
}

#[test]
fn a_spec_survives_the_row_it_is_kept_in() {
    for spec in [
        ScheduleSpec::Once {
            at: 1_800_000_000_000,
        },
        ScheduleSpec::Every { minutes: 15 },
        daily(),
        ScheduleSpec::Weekly {
            days: vec![Weekday::Mon, Weekday::Sun],
            time: time(18, 30),
        },
        ScheduleSpec::Monthly {
            day: None,
            time: time(0, 0),
        },
        ScheduleSpec::Cron {
            expr: "0 3 * * *".to_owned(),
        },
    ] {
        assert_eq!(ScheduleSpec::parse(&spec.to_json()), Some(spec));
    }
    assert_eq!(
        ScheduleSpec::Daily { time: time(9, 0) }.to_json(),
        r#"{"kind":"daily","time":{"hour":9,"minute":0}}"#
    );
    assert_eq!(ScheduleSpec::parse(r#"{"kind":"hourly"}"#), None);
}

#[test]
fn times_and_weekdays_are_read_as_people_write_them() {
    assert_eq!(TimeOfDay::parse("9:00"), Some(time(9, 0)));
    assert_eq!(TimeOfDay::parse(" 18:30 "), Some(time(18, 30)));
    for bad in ["24:00", "9:60", "9", "9:5", "nine", "-1:00", "09:000"] {
        assert_eq!(TimeOfDay::parse(bad), None, "{bad}");
    }
    assert_eq!(Weekday::parse("mon"), Some(Weekday::Mon));
    assert_eq!(Weekday::parse("Sunday"), Some(Weekday::Sun));
    assert_eq!(Weekday::parse("WED"), Some(Weekday::Wed));
    assert_eq!(Weekday::parse("someday"), None);
}

#[test]
fn changing_a_schedule_works_its_next_time_out_again() {
    let fx = Fx::new();
    let schedule = fx.add(ScheduleTarget::Test, &daily(), &nov(5, 10, 0));
    let inputs: BTreeMap<String, String> = [("note".to_owned(), "hello".to_owned())].into();
    let changed = change(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        &schedule.id,
        &Draft {
            name: Some("Evening ping"),
            target: ScheduleTarget::Workflow,
            target_id: "ping",
            workspace_id: &fx.workspace.id,
            inputs: &inputs,
            spec: &ScheduleSpec::Daily { time: time(18, 0) },
            missed: IfMissed::Skip,
            notify: ScheduleNotify::Never,
        },
        &nov(5, 10, 0),
    )
    .unwrap();
    assert_eq!(changed.id, schedule.id);
    assert_eq!(changed.name, "Evening ping");
    assert_eq!(changed.target_kind, "workflow");
    assert_eq!(changed.inputs, r#"{"note":"hello"}"#);
    assert_eq!(changed.next_at, Some(nov(5, 18, 0).timestamp_millis()));

    let missing = change(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        "no-such-schedule",
        &Draft {
            name: None,
            target: ScheduleTarget::Workflow,
            target_id: "ping",
            workspace_id: &fx.workspace.id,
            inputs: &BTreeMap::new(),
            spec: &daily(),
            missed: IfMissed::default(),
            notify: ScheduleNotify::default(),
        },
        &nov(5, 10, 0),
    )
    .unwrap_err();
    assert_eq!(missing.code, "schedule_not_found");
}

#[test]
fn a_schedule_switched_back_on_starts_from_now() {
    let fx = Fx::new();
    let schedule = fx.add(ScheduleTarget::Test, &daily(), &nov(5, 10, 0));
    let off = set_enabled(&fx.store, &schedule.id, false, &nov(5, 11, 0)).unwrap();
    assert!(!off.enabled);

    // Three days later: the next time is the next nine o'clock, not one of those that passed.
    let on = set_enabled(&fx.store, &schedule.id, true, &nov(8, 12, 0)).unwrap();
    assert!(on.enabled);
    assert_eq!(on.next_at, Some(nov(9, 9, 0).timestamp_millis()));
}

#[test]
fn a_once_whose_time_has_passed_cannot_be_switched_back_on() {
    let fx = Fx::new();
    let once = ScheduleSpec::Once {
        at: nov(5, 18, 0).timestamp_millis(),
    };
    let schedule = fx.add(ScheduleTarget::Test, &once, &nov(5, 10, 0));
    set_enabled(&fx.store, &schedule.id, false, &nov(5, 11, 0)).unwrap();
    let refused = set_enabled(&fx.store, &schedule.id, true, &nov(6, 0, 0)).unwrap_err();
    assert_eq!(refused.code, "schedule_when");
    assert!(!fx.schedule(&schedule.id).enabled);
}

#[test]
fn where_a_schedule_stands_follows_its_newest_time() {
    use crate::schedule::fire::{settle, tick};
    let fx = Fx::new();
    let schedule = fx.add(ScheduleTarget::Test, &daily(), &nov(5, 10, 0));
    let stands = |fx: &Fx| {
        let row = fx.schedule(&schedule.id);
        let history = fx.store.schedule_runs(&schedule.id, 1).unwrap();
        standing(&row, history.first())
    };
    assert_eq!(stands(&fx), ScheduleStanding::Waiting);

    // Its time comes; nobody has approved the file.
    tick(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        &nov(6, 9, 0),
    )
    .unwrap();
    assert_eq!(stands(&fx), ScheduleStanding::Running);
    settle(&fx.store).unwrap();
    assert_eq!(stands(&fx), ScheduleStanding::AwaitingApproval);
    assert!(stands(&fx).needs_attention());

    // Approved, carried out, passed.
    let run = fx.store.active_test_runs().unwrap().remove(0);
    fx.store.approve_test_run(&run.id).unwrap();
    fx.store.start_test_run(&run.id, 4000, "/tmp/out").unwrap();
    fx.store.finish_test_run(&run.id, "passed", None).unwrap();
    settle(&fx.store).unwrap();
    assert_eq!(stands(&fx), ScheduleStanding::Waiting);

    // Archived, the next time is refused; switched off, that is what it says.
    fx.store
        .set_workspace_archived(&fx.workspace.id, true)
        .unwrap();
    tick(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        &nov(7, 9, 0),
    )
    .unwrap();
    assert_eq!(stands(&fx), ScheduleStanding::Refused);
    set_enabled(&fx.store, &schedule.id, false, &nov(7, 10, 0)).unwrap();
    assert_eq!(stands(&fx), ScheduleStanding::Off);

    // Its workspace deleted, nothing else about it matters.
    fx.store.remove_worktree(&fx.workspace.id).unwrap();
    assert_eq!(stands(&fx), ScheduleStanding::NoWorkspace);
}

#[test]
fn a_once_that_has_fired_is_done() {
    use crate::schedule::fire::tick;
    let fx = Fx::new();
    let once = ScheduleSpec::Once {
        at: nov(5, 18, 0).timestamp_millis(),
    };
    let schedule = fx.add(ScheduleTarget::Workflow, &once, &nov(5, 10, 0));
    tick(
        &fx.store,
        &fx.data_dir,
        &Accepting::default(),
        &nov(5, 18, 0),
    )
    .unwrap();
    // Fired, and its run still going: that is what it says, not that it is done with.
    let history = fx.store.schedule_runs(&schedule.id, 1).unwrap();
    assert_eq!(
        standing(&fx.schedule(&schedule.id), history.first()),
        ScheduleStanding::Running
    );
    let run = fx.store.active_workflow_runs().unwrap().remove(0);
    fx.store
        .finish_workflow_run(&run.id, "succeeded", None)
        .unwrap();
    crate::schedule::fire::settle(&fx.store).unwrap();
    let history = fx.store.schedule_runs(&schedule.id, 1).unwrap();
    assert_eq!(
        standing(&fx.schedule(&schedule.id), history.first()),
        ScheduleStanding::Done
    );
}
