//! What a trigger may hold, and the fixture the firing tests share. The fixture is the
//! schedules' — a real store in a temp data folder, a worktree with a smoke test, a workflow in
//! the user's folder — with what a trigger adds: workflows that read `{{ event.* }}`, and agent
//! sessions.

use std::collections::BTreeMap;

use super::*;
use crate::schedule::tests::Fx;
use crate::store::{NewRun, NewSession};
use crate::workflow::runs::Accepting;

/// A made-up now, in milliseconds.
pub(crate) const NOW: i64 = 1_800_000_000_000;

/// Tells the user what the agent said when it was done.
pub(crate) const TELL: &str = r#"id: tell
name: Tell
version: 1
trigger:
  kind: manual
steps:
  - id: tell
    action: notify
    title: "Done: {{ event.note }} ({{ event.count }}, {{ event.kind }})"
"#;

/// Uses a field no event of this version leaves.
const WANTS_A_LINK: &str = r#"id: link
name: Link
version: 1
trigger:
  kind: manual
steps:
  - id: tell
    action: notify
    title: "{{ event.url }}"
"#;

/// Runs the worktree's smoke test with a timeout, as a trigger requires.
pub(crate) const RUNS_THE_TEST_IN_TIME: &str = r#"id: smoke
name: Smoke
version: 1
trigger:
  kind: manual
steps:
  - id: check
    action: run_test
    test: demo
    timeout: 30m
"#;

pub(crate) struct Tx {
    pub fx: Fx,
}

impl Tx {
    pub fn new() -> Self {
        let fx = Fx::new();
        fx.write_workflow("tell", TELL);
        Self { fx }
    }

    pub fn store(&self) -> &Store {
        &self.fx.store
    }

    pub fn workspace_id(&self) -> &str {
        &self.fx.workspace.id
    }

    pub fn draft<'a>(
        &'a self,
        events: &'a [TriggerEvent],
        filters: &'a Filters,
        target: ScheduleTarget,
        target_id: &'a str,
        inputs: &'a BTreeMap<String, String>,
    ) -> Draft<'a> {
        Draft {
            name: None,
            preset: None,
            workspace_id: &self.fx.workspace.id,
            session_id: None,
            events,
            filters,
            target,
            target_id,
            inputs,
            quiet_ms: 0,
            cap: 3,
            notify: ScheduleNotify::default(),
        }
    }

    pub fn try_add(&self, draft: &Draft<'_>) -> IpcResult<TriggerRow> {
        add(
            self.store(),
            &self.fx.data_dir,
            &Accepting::default(),
            draft,
            "app",
            NOW,
        )
    }

    /// A trigger on `workspace.done` that runs `tell`, with this quiet time and cap.
    pub fn on_done(&self, quiet_ms: i64, cap: i64) -> TriggerRow {
        let inputs = BTreeMap::new();
        let filters = Filters::default();
        let events = [TriggerEvent::WorkspaceDone];
        let mut draft = self.draft(&events, &filters, ScheduleTarget::Workflow, "tell", &inputs);
        draft.quiet_ms = quiet_ms;
        draft.cap = cap;
        self.try_add(&draft).unwrap()
    }

    /// An agent session in the workspace, running in `terminal`, with its agent run.
    pub fn agent(&self, record: &str, terminal: &str) {
        self.store()
            .add_session(&NewSession {
                id: record,
                workspace_id: self.workspace_id(),
                harness_id: "claude",
                model: None,
                effort: None,
                harness_session_id: None,
                title: "Claude",
                forked_from: None,
                pty_session_id: terminal,
                prompt: None,
            })
            .unwrap();
        let run = format!("run-{record}");
        self.store()
            .add_run(&NewRun {
                id: &run,
                workspace_id: self.workspace_id(),
                session_id: Some(record),
                kind: "harness",
                harness_id: Some("claude"),
                harness_session_id: None,
                launched_by: "app",
            })
            .unwrap();
        self.store().run_spawned(&run, terminal).unwrap();
    }
}

fn no_inputs() -> BTreeMap<String, String> {
    BTreeMap::new()
}

#[test]
fn a_new_trigger_is_on_named_after_its_target_and_counts_from_now() {
    let tx = Tx::new();
    let trigger = tx.on_done(0, 3);
    assert_eq!(trigger.name, "Tell");
    assert!(trigger.enabled);
    assert_eq!(trigger.since, NOW);
    assert_eq!(events_of(&trigger), vec![TriggerEvent::WorkspaceDone]);
    assert_eq!(
        describe(&trigger),
        "When an agent says its work is done, run the workflow `tell`"
    );
    assert!(listens_for(tx.store(), tx.workspace_id(), TriggerEvent::WorkspaceDone).unwrap());
    assert!(!listens_for(tx.store(), tx.workspace_id(), TriggerEvent::AgentSettled).unwrap());
}

#[test]
fn an_agents_events_and_the_workspaces_do_not_share_a_trigger() {
    let tx = Tx::new();
    let (filters, inputs) = (Filters::default(), no_inputs());
    let events = [TriggerEvent::AgentSettled, TriggerEvent::WorkspaceDone];
    let error = tx
        .try_add(&tx.draft(&events, &filters, ScheduleTarget::Workflow, "tell", &inputs))
        .unwrap_err();
    assert_eq!(error.code, "trigger_events");
}

#[test]
fn a_filter_for_an_event_it_does_not_listen_for_is_refused() {
    let tx = Tx::new();
    let filters = Filters {
        new_commits: true,
        ..Filters::default()
    };
    let inputs = no_inputs();
    let events = [TriggerEvent::WorkspaceDone];
    let error = tx
        .try_add(&tx.draft(&events, &filters, ScheduleTarget::Workflow, "tell", &inputs))
        .unwrap_err();
    assert_eq!(error.code, "trigger_filters");
    assert!(error.message.contains("agent.settled"), "{}", error.message);
}

#[test]
fn a_workflow_that_reads_what_its_events_do_not_leave_is_refused() {
    let tx = Tx::new();
    tx.fx.write_workflow("link", WANTS_A_LINK);
    let (filters, inputs) = (Filters::default(), no_inputs());
    let events = [TriggerEvent::WorkspaceDone];
    let error = tx
        .try_add(&tx.draft(&events, &filters, ScheduleTarget::Workflow, "link", &inputs))
        .unwrap_err();
    assert_eq!(error.code, "trigger_variables");
    assert!(error.message.contains("event.url"), "{}", error.message);
    // `note` is what a done signal leaves, so the workflow that reads it is fine.
    tx.try_add(&tx.draft(&events, &filters, ScheduleTarget::Workflow, "tell", &inputs))
        .unwrap();
}

#[test]
fn a_run_test_step_without_a_timeout_is_refused_and_one_with_a_timeout_is_not() {
    let tx = Tx::new();
    let (filters, inputs) = (Filters::default(), no_inputs());
    let events = [TriggerEvent::WorkspaceDone];
    tx.fx
        .write_workflow("smoke", crate::schedule::tests::RUNS_THE_TEST);
    let error = tx
        .try_add(&tx.draft(
            &events,
            &filters,
            ScheduleTarget::Workflow,
            "smoke",
            &inputs,
        ))
        .unwrap_err();
    assert_eq!(error.code, "trigger_needs_person");
    assert!(error.message.contains("timeout"), "{}", error.message);

    // The file is not approved, and that is fine: the run waits for the user, with a timeout.
    tx.fx.write_workflow("smoke", RUNS_THE_TEST_IN_TIME);
    tx.try_add(&tx.draft(
        &events,
        &filters,
        ScheduleTarget::Workflow,
        "smoke",
        &inputs,
    ))
    .unwrap();
}

#[test]
fn a_test_with_a_step_a_person_does_cannot_be_triggered() {
    let tx = Tx::new();
    tx.fx.write_test(
        "id: demo\nname: Demo\nversion: 1\nsteps:\n  - id: look\n    action: manual\n    prompt: Right?\n",
    );
    let (filters, inputs) = (Filters::default(), no_inputs());
    let events = [TriggerEvent::WorkspaceDone];
    let error = tx
        .try_add(&tx.draft(&events, &filters, ScheduleTarget::Test, "demo", &inputs))
        .unwrap_err();
    assert_eq!(error.code, "trigger_needs_person");
}

#[test]
fn an_unapproved_test_can_be_triggered() {
    let tx = Tx::new();
    let (filters, inputs) = (Filters::default(), no_inputs());
    let events = [TriggerEvent::WorkspaceDone];
    let trigger = tx
        .try_add(&tx.draft(&events, &filters, ScheduleTarget::Test, "demo", &inputs))
        .unwrap();
    assert_eq!(trigger.name, "Demo");
}

#[test]
fn a_session_trigger_names_a_session_of_its_own_workspace() {
    let tx = Tx::new();
    let (filters, inputs) = (Filters::default(), no_inputs());
    let events = [TriggerEvent::AgentSettled];
    let mut draft = tx.draft(&events, &filters, ScheduleTarget::Workflow, "ping", &inputs);
    draft.session_id = Some("nobody");
    assert_eq!(tx.try_add(&draft).unwrap_err().code, "session_not_found");
    tx.agent("rec-1", "pty-1");
    draft.session_id = Some("rec-1");
    let trigger = tx.try_add(&draft).unwrap();
    assert_eq!(trigger.session_id.as_deref(), Some("rec-1"));
}

#[test]
fn quiet_and_cap_are_bounded() {
    let tx = Tx::new();
    let (filters, inputs) = (Filters::default(), no_inputs());
    let events = [TriggerEvent::WorkspaceDone];
    let mut draft = tx.draft(&events, &filters, ScheduleTarget::Workflow, "tell", &inputs);
    draft.quiet_ms = MOST_QUIET_MS + 1;
    assert_eq!(tx.try_add(&draft).unwrap_err().code, "trigger_quiet");
    draft.quiet_ms = 0;
    draft.cap = 0;
    assert_eq!(tx.try_add(&draft).unwrap_err().code, "trigger_cap");
}

#[test]
fn switched_back_on_it_counts_from_then() {
    let tx = Tx::new();
    let trigger = tx.on_done(0, 3);
    let off = set_enabled(tx.store(), &trigger.id, false, NOW + 10).unwrap();
    assert!(!off.enabled);
    assert_eq!(standing(&off, &[]), TriggerStanding::Off);
    let on = set_enabled(tx.store(), &trigger.id, true, NOW + 20).unwrap();
    assert!(on.enabled);
    assert_eq!(on.since, NOW + 20);
}

#[test]
fn a_workflow_reads_event_variables_and_a_run_by_hand_is_told_they_are_empty() {
    let tx = Tx::new();
    let entry = crate::workflow::find(&tx.fx.data_dir, "tell").unwrap();
    assert!(entry.runnable(), "{:?}", entry.problems);
    let workflow = entry.workflow.unwrap();
    assert_eq!(
        event_fields(&workflow),
        BTreeSet::from(["count".to_owned(), "kind".to_owned(), "note".to_owned()])
    );
    let git = crate::git::testing::git();
    let place = crate::workflow::runs::Place::load(tx.store(), tx.workspace_id(), &git)
        .unwrap()
        .unwrap();
    let notes = crate::workflow::runs::empty_variables(&workflow, &place, true);
    assert!(
        notes.iter().any(|note| note.contains("event.note")),
        "{notes:?}"
    );
}

#[test]
fn every_preset_saves_with_its_one_answer() {
    let tx = Tx::new();
    for preset in presets::PRESETS {
        let mut inputs = BTreeMap::new();
        if let Some((input, _)) = preset.asks {
            let answer = if input == "suite" { "demo" } else { "claude" };
            inputs.insert(input.to_owned(), answer.to_owned());
        }
        let filters = Filters::default();
        let draft = Draft {
            preset: Some(preset.id),
            quiet_ms: preset.quiet_ms(),
            ..tx.draft(
                preset.events,
                &filters,
                ScheduleTarget::Workflow,
                preset.workflow,
                &inputs,
            )
        };
        let trigger = tx
            .try_add(&draft)
            .unwrap_or_else(|e| panic!("{}: {}", preset.id, e.message));
        assert_eq!(
            trigger.name,
            crate::workflow::find(&tx.fx.data_dir, preset.workflow)
                .unwrap()
                .name
        );
        assert_eq!(trigger.preset.as_deref(), Some(preset.id));
    }
    assert_eq!(
        presets::find("address-reviews").unwrap().quiet_ms(),
        2 * 60_000
    );
}
