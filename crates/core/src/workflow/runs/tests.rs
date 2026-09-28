use super::*;
use crate::forge::PullRequestState;
use crate::store::StepStatus;

const DEMO: &str = r#"id: demo
name: Demo
version: 1
trigger:
  kind: manual
inputs:
  - id: who
    kind: harness
    required: true
  - id: depth
    kind: choice
    options: [quick, deep]
    default: quick
  - id: note
    kind: text
steps:
  - id: start
    action: start_session
    harness: "{{ inputs.who }}"
    prompt: "{{ inputs.note }}"
  - id: tell
    action: notify
    needs: [start]
    title: Started
"#;

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    workspace: WorkspaceRow,
    look: Looks,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_memory();
        let project = store.add_project("app", "/code/app").unwrap();
        let workspace = store
            .add_worktree(
                &project.id,
                "fix-login",
                "/code/app-wt/fix-login",
                Some("fix-login"),
                Some("main"),
            )
            .unwrap();
        let folder = crate::workflow::user_dir(dir.path());
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("demo.yaml"), DEMO).unwrap();
        Self {
            dir,
            store,
            workspace,
            look: Looks::default(),
        }
    }

    fn queue(&self, workflow: &str, inputs: &[(&str, &str)]) -> IpcResult<String> {
        let inputs: BTreeMap<String, String> = inputs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        queue(
            &self.store,
            self.dir.path(),
            &Request {
                workflow_id: workflow,
                workspace_id: &self.workspace.id,
                inputs: &inputs,
                requested_by: "cli",
            },
            &self.look,
        )
    }
}

/// Two agents are installed; ids are matched without regard to case, as `ys` does. The forge
/// answers with `pr`, or `forge_down` as its error.
#[derive(Default)]
struct Looks {
    pr: Option<PullRequest>,
    forge_down: Option<String>,
    asked: std::cell::Cell<u32>,
}

impl Look for Looks {
    fn harness(&self, wanted: &str) -> Result<String, String> {
        ["claude", "codex"]
            .into_iter()
            .find(|id| id.eq_ignore_ascii_case(wanted))
            .map(str::to_owned)
            .ok_or_else(|| format!("No harness called {wanted:?}."))
    }

    fn pull_request(&self, _workspace: &WorkspaceRow) -> Result<Option<PullRequest>, String> {
        self.asked.set(self.asked.get() + 1);
        match &self.forge_down {
            Some(why) => Err(why.clone()),
            None => Ok(self.pr.clone()),
        }
    }
}

fn pull_request(number: u32, state: crate::forge::PullRequestState) -> PullRequest {
    PullRequest {
        number,
        url: format!("https://github.com/o/r/pull/{number}"),
        title: "Fix the login".into(),
        branch: "fix-login".into(),
        state,
        draft: false,
        checks: crate::forge::Checks::None,
        details: None,
        created_at: None,
    }
}

#[test]
fn a_run_is_queued_with_its_inputs_its_file_and_every_step_pending() {
    let fx = Fixture::new();
    let id = fx
        .queue("demo", &[("who", "Claude"), ("note", "  hi  ")])
        .unwrap();
    let run = fx.store.workflow_run(&id).unwrap().unwrap();
    assert_eq!(run.status, "queued");
    assert_eq!(run.workflow_id, "demo");
    assert_eq!(run.definition, DEMO, "the file is kept as it was");
    assert_eq!(run.workspace_name, "fix-login");
    assert_eq!(run.requested_by, "cli");
    let inputs: Inputs = serde_json::from_str(&run.inputs).unwrap();
    assert_eq!(inputs["who"], "claude", "the harness as Yardsort spells it");
    assert_eq!(inputs["depth"], "quick", "the default");
    assert_eq!(inputs["note"], "hi");
    let steps = fx.store.workflow_steps(&id).unwrap();
    let listed: Vec<(&str, &str, &str)> = steps
        .iter()
        .map(|s| (s.step_id.as_str(), s.action.as_str(), s.status.as_str()))
        .collect();
    assert_eq!(
        listed,
        vec![
            ("start", "start_session", "pending"),
            ("tell", "notify", "pending")
        ]
    );
}

#[test]
fn inputs_are_checked_before_anything_is_written() {
    let fx = Fixture::new();
    let cases: &[(&[(&str, &str)], &str)] = &[
        (&[], "`demo` needs `who` (who)."),
        (
            &[("who", "claude"), ("colour", "red")],
            "`demo` has no input `colour`.",
        ),
        (
            &[("who", "claude"), ("depth", "medium")],
            "`medium` is not one of the answers",
        ),
        (&[("who", "gemini")], "No harness called \"gemini\"."),
    ];
    for (inputs, expected) in cases {
        let error = fx.queue("demo", inputs).unwrap_err();
        assert_eq!(error.code, "workflow_inputs");
        assert!(
            error.message.contains(expected),
            "{inputs:?}: {}",
            error.message
        );
    }
    assert!(fx.store.workflow_runs(None, 10).unwrap().is_empty());
}

#[test]
fn a_named_harness_in_a_step_is_checked_too() {
    let fx = Fixture::new();
    let folder = crate::workflow::user_dir(fx.dir.path());
    let fixed = DEMO
        .replace("id: demo", "id: fixed")
        .replace("\"{{ inputs.who }}\"", "gemini");
    std::fs::write(folder.join("fixed.yaml"), fixed).unwrap();
    let error = fx.queue("fixed", &[("who", "claude")]).unwrap_err();
    assert_eq!(error.code, "harness_not_found");
}

#[test]
fn a_workflow_that_uses_the_pull_request_needs_one_open_and_keeps_it() {
    let mut fx = Fixture::new();
    let error = fx
        .queue("code-review", &[("reviewer", "claude")])
        .unwrap_err();
    assert_eq!(error.code, "pull_request_missing");
    assert!(
        error.message.contains("has no open one"),
        "{}",
        error.message
    );

    fx.look.pr = Some(pull_request(7, PullRequestState::Merged));
    let error = fx
        .queue("code-review", &[("reviewer", "claude")])
        .unwrap_err();
    assert_eq!(
        error.code, "pull_request_missing",
        "a merged one is not open"
    );

    fx.look.forge_down = Some("gh is not logged in.".into());
    let error = fx
        .queue("code-review", &[("reviewer", "claude")])
        .unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        ("pull_request_unknown", "gh is not logged in.")
    );
    assert!(fx.store.workflow_runs(None, 10).unwrap().is_empty());

    fx.look.forge_down = None;
    fx.look.pr = Some(pull_request(7, PullRequestState::Open));
    let id = fx.queue("code-review", &[("reviewer", "claude")]).unwrap();
    let run = fx.store.workflow_run(&id).unwrap().unwrap();
    let facts = Facts::parse(&run.context);
    assert_eq!(
        facts.pr,
        Some(PrFacts {
            number: 7,
            url: "https://github.com/o/r/pull/7".into(),
            title: "Fix the login".into(),
        })
    );
    let var = |path: &str| facts.var(&path.split('.').map(str::to_owned).collect::<Vec<_>>());
    assert_eq!(var("pr.number").as_deref(), Some("7"));
    assert_eq!(var("pr.title").as_deref(), Some("Fix the login"));
}

#[test]
fn the_forge_is_not_asked_when_a_workflow_does_not_need_it() {
    let fx = Fixture::new();
    fx.queue("demo", &[("who", "claude")]).unwrap();
    assert_eq!(fx.look.asked.get(), 0);
    let run = &fx.store.workflow_runs(None, 1).unwrap()[0];
    assert_eq!(run.context, "{}");
}

#[test]
fn a_broken_or_missing_workflow_or_an_archived_workspace_is_refused() {
    let fx = Fixture::new();
    assert_eq!(
        fx.queue("nope", &[]).unwrap_err().code,
        "workflow_not_found"
    );
    let folder = crate::workflow::user_dir(fx.dir.path());
    std::fs::write(folder.join("broken.yaml"), "id: broken\n").unwrap();
    assert_eq!(
        fx.queue("broken", &[]).unwrap_err().code,
        "workflow_invalid"
    );
    fx.store
        .set_workspace_archived(&fx.workspace.id, true)
        .unwrap();
    let error = fx.queue("demo", &[("who", "claude")]).unwrap_err();
    assert_eq!(error.code, "workspace_archived");
}

#[test]
fn one_run_at_a_time_per_workflow_and_workspace() {
    let fx = Fixture::new();
    let first = fx.queue("demo", &[("who", "claude")]).unwrap();
    let error = fx.queue("demo", &[("who", "claude")]).unwrap_err();
    assert_eq!(error.code, "workflow_already_running");
    assert!(
        error.message.contains(crate::memory::short_id(&first)),
        "{}",
        error.message
    );
    fx.store
        .cancel_workflow_run(&first, "changed my mind")
        .unwrap();
    assert!(
        fx.queue("demo", &[("who", "claude")]).is_ok(),
        "once it ends, another may start"
    );
}

#[test]
fn a_cancelled_run_cannot_be_moved_on_by_a_driver_that_had_not_heard() {
    let fx = Fixture::new();
    let id = fx.queue("demo", &[("who", "claude")]).unwrap();
    assert!(fx.store.start_workflow_run(&id, None).unwrap());
    assert!(fx
        .store
        .move_workflow_step(
            &id,
            "start",
            &[StepStatus::Pending],
            StepStatus::Running,
            None,
            None
        )
        .unwrap());
    assert!(fx
        .store
        .cancel_workflow_run(&id, "cancelled from ys")
        .unwrap());
    let run = fx.store.workflow_run(&id).unwrap().unwrap();
    assert_eq!(run.status, "cancelled");
    assert_eq!(run.error.as_deref(), Some("cancelled from ys"));
    let steps = fx.store.workflow_steps(&id).unwrap();
    assert!(steps.iter().all(|s| s.status == "cancelled"), "{steps:?}");
    // The driver finishes what it was doing and reports it: too late, nothing changes.
    assert!(!fx
        .store
        .move_workflow_step(
            &id,
            "start",
            &[StepStatus::Running],
            StepStatus::Succeeded,
            None,
            None
        )
        .unwrap());
    assert!(!fx
        .store
        .finish_workflow_run(&id, "succeeded", None)
        .unwrap());
    assert!(!fx.store.cancel_workflow_run(&id, "again").unwrap());
    assert_eq!(
        fx.store.workflow_run(&id).unwrap().unwrap().status,
        "cancelled"
    );
}

#[test]
fn a_step_move_keeps_its_first_start_time_and_records_what_it_left() {
    let fx = Fixture::new();
    let id = fx.queue("demo", &[("who", "claude")]).unwrap();
    fx.store.start_workflow_run(&id, None).unwrap();
    let moved = |from, to, outputs| {
        fx.store
            .move_workflow_step(&id, "start", &[from], to, outputs, None)
            .unwrap()
    };
    assert!(moved(StepStatus::Pending, StepStatus::Running, None));
    let started = fx.store.workflow_steps(&id).unwrap()[0].started_at;
    assert!(started.is_some());
    assert!(
        !moved(StepStatus::Pending, StepStatus::Running, None),
        "not pending any more"
    );
    assert!(moved(
        StepStatus::Running,
        StepStatus::Succeeded,
        Some(r#"{"session":"pty-1"}"#)
    ));
    let step = &fx.store.workflow_steps(&id).unwrap()[0];
    assert_eq!(step.started_at, started);
    assert!(step.ended_at.is_some());
    assert_eq!(step.outputs, r#"{"session":"pty-1"}"#);
    // A step skipped straight from pending never started.
    assert!(fx
        .store
        .move_workflow_step(
            &id,
            "tell",
            &[StepStatus::Pending],
            StepStatus::Skipped,
            None,
            Some("x")
        )
        .unwrap());
    let tell = &fx.store.workflow_steps(&id).unwrap()[1];
    assert_eq!((tell.started_at, tell.note.as_deref()), (None, Some("x")));
}

#[test]
fn a_run_knows_its_project_and_workspace() {
    let fx = Fixture::new();
    let place = Place::load(&fx.store, &fx.workspace.id).unwrap().unwrap();
    let var = |path: &str| place.var(&path.split('.').map(str::to_owned).collect::<Vec<_>>());
    assert_eq!(var("project.name").as_deref(), Some("app"));
    assert_eq!(var("workspace.branch").as_deref(), Some("fix-login"));
    assert_eq!(var("workspace.base_branch").as_deref(), Some("main"));
    assert_eq!(
        var("workspace.task"),
        None,
        "no agent has been started there yet"
    );
    assert_eq!(var("memory"), None, "not a place variable");
}
