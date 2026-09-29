//! The driver against a real program in a real PTY: a shell script stands in for the agent. It
//! prints for a moment — working — then reads one line and writes it to a file, so the test can
//! see exactly what was typed into it. Unix-only, as the host's own tests that wait on output
//! mid-run are.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pty_host::{LaunchPlan, PtyHost, SessionId, SessionInfo, TermSize};

use super::*;
use crate::git::testing::git;
use crate::launch::WORKSPACE_LABEL;
use crate::store::Store;
use crate::workflow::runs::{queue, Request};

const WORKFLOW: &str = r#"id: shell-demo
name: Shell demo
version: 1
trigger:
  kind: manual
inputs:
  - id: who
    kind: harness
    required: true
steps:
  - id: start
    action: start_session
    harness: "{{ inputs.who }}"
    prompt: "Work on {{ workspace.branch }}"
  - id: settle
    action: wait_session
    needs: [start]
    session: "{{ steps.start.session }}"
    timeout: 1m
  - id: nudge
    action: send_to_session
    needs: [settle]
    session: "{{ steps.start.session }}"
    prompt: Now write the tests.
    timeout: 1m
  - id: tell
    action: notify
    needs: [nudge]
    title: "{{ inputs.who }} got it on {{ workspace.name }}"
"#;

/// Busy for about 600ms, then waits for a line and keeps it.
const AGENT: &str = r#"for i in 1 2 3 4 5 6; do echo "working $i"; sleep 0.1; done
read line
printf '%s' "$line" > "$KEPT"
sleep 30"#;

const LIMIT: Duration = Duration::from_secs(20);

/// Busy for about 600ms, then keeps two lines, working for a moment after the first.
const TWO_LINES: &str = r#"for i in 1 2 3 4 5 6; do echo "working $i"; sleep 0.1; done
read one
printf '%s\n' "$one" >> "$KEPT"
for i in 1 2 3 4 5 6; do echo "on it $i"; sleep 0.1; done
read two
printf '%s\n' "$two" >> "$KEPT"
sleep 30"#;

/// Two messages for one agent, both due the moment it settles.
const TWO_SENDS: &str = r#"id: two-sends
name: Two sends
version: 1
trigger:
  kind: manual
steps:
  - id: start
    action: start_session
    harness: shell
  - id: settle
    action: wait_session
    needs: [start]
    session: "{{ steps.start.session }}"
    timeout: 1m
  - id: one
    action: send_to_session
    needs: [settle]
    session: "{{ steps.start.session }}"
    prompt: first
    timeout: 1m
  - id: two
    action: send_to_session
    needs: [settle]
    session: "{{ steps.start.session }}"
    prompt: second
    timeout: 1m
"#;

/// Starts the script instead of an agent, and remembers what it was asked.
struct ScriptHands {
    host: Arc<PtyHost>,
    script: &'static str,
    kept: PathBuf,
    fail_with: Option<String>,
    started: Mutex<Vec<HarnessRequest>>,
    notified: Mutex<Vec<String>>,
}

impl Hands for ScriptHands {
    fn start(&self, workspace_id: &str, request: HarnessRequest) -> Result<SessionInfo, String> {
        self.started.lock().unwrap().push(request);
        if let Some(why) = &self.fail_with {
            return Err(why.clone());
        }
        let labels: BTreeMap<String, String> =
            [(WORKSPACE_LABEL.to_owned(), workspace_id.to_owned())].into();
        self.host
            .spawn(LaunchPlan {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), self.script.into()],
                cwd: None,
                env: vec![("KEPT".into(), self.kept.display().to_string())],
                clear_env: false,
                size: TermSize { cols: 80, rows: 24 },
                labels,
                prompt: None,
            })
            .map_err(|e| e.to_string())
    }

    fn notify(&self, title: &str, _body: Option<&str>) -> Result<(), String> {
        self.notified.lock().unwrap().push(title.to_owned());
        Ok(())
    }

    fn handoff(&self, _workspace_id: &str) -> Option<String> {
        None
    }

    fn pr_posts(&self, _workspace_id: &str, _number: u32) -> PrAsk {
        Box::new(|| Ok(Vec::new()))
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    workspace_id: String,
    host: Arc<PtyHost>,
    driver: Arc<Driver>,
    hands: ScriptHands,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
        let project = store.add_project("app", "/code/app").unwrap();
        let workspace = store
            .add_worktree(
                &project.id,
                "fix-login",
                &dir.path().display().to_string(),
                Some("fix-login"),
                Some("main"),
            )
            .unwrap();
        let folder = crate::workflow::user_dir(dir.path());
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("shell-demo.yaml"), WORKFLOW).unwrap();
        std::fs::write(folder.join("two-sends.yaml"), TWO_SENDS).unwrap();

        // The host's events reach the driver as the app wires them. Quiet comes sooner than in
        // the app, and so does "settled", so the test takes seconds, not minutes.
        let driver = Arc::new(Driver::new(200, Duration::ZERO));
        let hearing = Arc::clone(&driver);
        let host = Arc::new(
            PtyHost::new(Arc::new(move |event| {
                hearing.observe(&event);
            }))
            .with_quiet_after(Duration::from_millis(300)),
        );
        let hands = ScriptHands {
            host: Arc::clone(&host),
            script: AGENT,
            kept: dir.path().join("kept.txt"),
            fail_with: None,
            started: Mutex::new(Vec::new()),
            notified: Mutex::new(Vec::new()),
        };
        Self {
            dir,
            store,
            workspace_id: workspace.id,
            host,
            driver,
            hands,
        }
    }

    fn queue(&self) -> String {
        self.queue_workflow("shell-demo", &[("who", "shell")])
    }

    fn queue_workflow(&self, workflow: &str, given: &[(&str, &str)]) -> String {
        let inputs: BTreeMap<String, String> = given
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        queue(
            &self.store,
            self.dir.path(),
            &Request {
                workflow_id: workflow,
                workspace_id: &self.workspace_id,
                inputs: &inputs,
                requested_by: "cli",
            },
            &crate::workflow::runs::Accepting::default(),
        )
        .unwrap()
    }

    fn tick(&mut self) {
        self.driver
            .tick(&self.store, self.host.as_ref(), &self.hands, &git())
            .unwrap();
    }

    /// Tick, as the app's thread does, until `done` holds.
    fn until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let start = Instant::now();
        while !done(self) {
            assert!(start.elapsed() < LIMIT, "timed out waiting until {what}");
            self.tick();
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn step(&self, run: &str, step: &str) -> crate::store::WorkflowStepRow {
        self.store
            .workflow_steps(run)
            .unwrap()
            .into_iter()
            .find(|s| s.step_id == step)
            .unwrap()
    }

    fn run_status(&self, run: &str) -> String {
        self.store.workflow_run(run).unwrap().unwrap().status
    }

    fn session_of(&self, run: &str) -> SessionId {
        let outputs = engine::parse_outputs(&self.step(run, "start").outputs);
        SessionId(outputs["session"].clone())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for session in self.host.list() {
            let _ = self.host.kill(&session.id);
        }
    }
}

#[test]
fn a_run_starts_an_agent_waits_for_it_to_settle_types_to_it_and_says_so() {
    let mut fx = Fixture::new();
    let run = fx.queue();
    fx.until("the run ends", |fx| {
        !fx.store.workflow_run(&run).unwrap().unwrap().active()
    });

    assert_eq!(
        fx.run_status(&run),
        "succeeded",
        "{:?}",
        fx.store.workflow_run(&run)
    );
    for step in ["start", "settle", "nudge", "tell"] {
        assert_eq!(fx.step(&run, step).status, "succeeded", "{step}");
    }
    let started = fx.hands.started.lock().unwrap().clone();
    assert_eq!(started.len(), 1, "one agent, started once");
    assert_eq!(started[0].id, "shell");
    assert_eq!(started[0].prompt.as_deref(), Some("Work on fix-login"));
    assert!(!started[0].skip_memory);
    assert_eq!(
        engine::parse_outputs(&fx.step(&run, "settle").outputs)["outcome"],
        "settled"
    );
    assert_eq!(
        std::fs::read_to_string(&fx.hands.kept).unwrap(),
        "Now write the tests.",
        "the message was typed and submitted"
    );
    assert_eq!(
        *fx.hands.notified.lock().unwrap(),
        vec!["shell got it on fix-login".to_owned()]
    );
    // Running the workflow does not end the agent; it is the user's now.
    let session = fx.session_of(&run);
    assert!(matches!(
        fx.host.info(&session).unwrap().state,
        pty_host::SessionState::Running
    ));
}

#[test]
fn a_cancelled_run_stops_but_leaves_its_agent_running() {
    let mut fx = Fixture::new();
    let run = fx.queue();
    fx.until("the agent is being waited for", |fx| {
        fx.step(&run, "settle").status == "waiting"
    });
    assert!(fx
        .store
        .cancel_workflow_run(&run, "Cancelled from ys.")
        .unwrap());
    // Let the agent settle: a driver that had not heard of the cancel would type to it now.
    std::thread::sleep(Duration::from_millis(1_500));
    for _ in 0..5 {
        fx.tick();
    }
    assert_eq!(fx.run_status(&run), "cancelled");
    assert_eq!(fx.step(&run, "nudge").status, "cancelled");
    assert!(
        !fx.hands.kept.exists(),
        "nothing was typed after the cancel"
    );
    assert!(fx.hands.notified.lock().unwrap().is_empty());
    let session = fx.session_of(&run);
    assert!(matches!(
        fx.host.info(&session).unwrap().state,
        pty_host::SessionState::Running
    ));
}

#[test]
fn an_agent_that_cannot_start_fails_the_run_with_the_reason() {
    let mut fx = Fixture::new();
    fx.hands.fail_with = Some("`shell` was not found on PATH.".into());
    let run = fx.queue();
    fx.until("the run ends", |fx| {
        !fx.store.workflow_run(&run).unwrap().unwrap().active()
    });
    assert_eq!(fx.run_status(&run), "failed");
    assert_eq!(
        fx.store
            .workflow_run(&run)
            .unwrap()
            .unwrap()
            .error
            .as_deref(),
        Some("Step `start` failed: `shell` was not found on PATH.")
    );
    assert_eq!(fx.step(&run, "tell").status, "skipped");
    assert!(fx.hands.notified.lock().unwrap().is_empty());
}

#[test]
fn a_step_left_running_by_a_previous_app_fails_and_is_not_done_twice() {
    let mut fx = Fixture::new();
    let run = fx.queue();
    // The last app took the run and started the agent, then quit before recording it.
    assert!(fx.store.start_workflow_run(&run, None).unwrap());
    assert!(fx
        .store
        .move_workflow_step(
            &run,
            "start",
            &[StepStatus::Pending],
            StepStatus::Running,
            None,
            None
        )
        .unwrap());
    fx.tick();
    assert_eq!(fx.run_status(&run), "failed");
    assert_eq!(
        fx.step(&run, "start").note.as_deref(),
        Some("Yardsort stopped while this step was running.")
    );
    assert!(
        fx.hands.started.lock().unwrap().is_empty(),
        "no second agent"
    );
}

#[test]
fn two_messages_due_at_once_are_typed_one_quiet_moment_at_a_time() {
    let mut fx = Fixture::new();
    fx.hands.script = TWO_LINES;
    let run = fx.queue_workflow("two-sends", &[]);
    fx.until("the agent settles", |fx| {
        fx.step(&run, "settle").status == "succeeded"
    });
    // Both messages were due in the tick the agent settled. The first went; the second must
    // not have gone on the strength of the same look at a quiet agent.
    assert_eq!(fx.step(&run, "one").status, "succeeded");
    assert_eq!(fx.step(&run, "two").status, "waiting");

    fx.until("the second message is typed", |fx| {
        fx.step(&run, "two").status == "succeeded"
    });
    assert_eq!(
        std::fs::read_to_string(&fx.hands.kept).unwrap(),
        "first\nsecond\n",
        "each message arrived whole, in order"
    );
    let one_ended = fx.step(&run, "one").ended_at.unwrap();
    let two_ended = fx.step(&run, "two").ended_at.unwrap();
    assert!(
        two_ended - one_ended >= 600,
        "the second waited out the agent's work on the first ({}ms)",
        two_ended - one_ended
    );
    fx.until("the run ends", |fx| {
        !fx.store.workflow_run(&run).unwrap().unwrap().active()
    });
    assert_eq!(fx.run_status(&run), "succeeded");
}

/// The review's last word goes to the workspace's own agent.
const TELL_THE_AUTHOR: &str = r#"id: tell-author
name: Tell the author
version: 1
trigger:
  kind: manual
steps:
  - id: review
    action: start_session
    harness: shell
  - id: settle
    action: wait_session
    needs: [review]
    session: "{{ steps.review.session }}"
    timeout: 1m
  - id: tell_author
    action: send_to_session
    needs: [settle]
    session: origin
    prompt: the review is in
    timeout: 1m
"#;

/// Waits for a line and keeps it in `$HEARD`.
const LISTENER: &str = r#"read line
printf '%s' "$line" > "$HEARD"
sleep 30"#;

#[test]
fn the_workspaces_own_agent_is_told_not_a_shell_and_not_the_runs_own_reviewer() {
    let mut fx = Fixture::new();
    let folder = crate::workflow::user_dir(fx.dir.path());
    std::fs::write(folder.join("tell-author.yaml"), TELL_THE_AUTHOR).unwrap();
    let spawn = |labels: &[(&str, &str)], heard: &std::path::Path| {
        fx.host
            .spawn(LaunchPlan {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), LISTENER.into()],
                cwd: None,
                env: vec![("HEARD".into(), heard.display().to_string())],
                clear_env: false,
                size: TermSize { cols: 80, rows: 24 },
                labels: labels
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
                prompt: None,
            })
            .unwrap()
    };
    // The author: an agent conversation Yardsort recorded in this workspace.
    let author_heard = fx.dir.path().join("author.txt");
    let author = spawn(
        &[
            (WORKSPACE_LABEL, fx.workspace_id.as_str()),
            (crate::launch::HARNESS_LABEL, "claude"),
            (crate::launch::RECORD_LABEL, "record-author"),
        ],
        &author_heard,
    );
    fx.store
        .add_session(&crate::store::NewSession {
            id: "record-author",
            workspace_id: &fx.workspace_id,
            harness_id: "claude",
            model: None,
            effort: None,
            harness_session_id: None,
            title: "Fix the login",
            forked_from: None,
            pty_session_id: &author.id.0,
            prompt: Some("Fix the login"),
        })
        .unwrap();
    // A shell opened after it, in the same workspace: newer, but not an agent.
    let shell_heard = fx.dir.path().join("shell.txt");
    spawn(&[(WORKSPACE_LABEL, fx.workspace_id.as_str())], &shell_heard);

    let run = fx.queue_workflow("tell-author", &[]);
    fx.until("the run ends", |fx| {
        !fx.store.workflow_run(&run).unwrap().unwrap().active()
    });
    assert_eq!(
        fx.run_status(&run),
        "succeeded",
        "{:?}",
        fx.store.workflow_steps(&run)
    );
    let facts = Facts::parse(&fx.store.workflow_run(&run).unwrap().unwrap().context);
    assert_eq!(facts.origin.as_deref(), Some(author.id.0.as_str()));
    assert_eq!(
        std::fs::read_to_string(&author_heard).unwrap(),
        "the review is in"
    );
    assert!(!shell_heard.exists(), "the shell was not told");
    assert!(
        !fx.hands.kept.exists(),
        "the reviewer the run started, the newest agent of all, was not told either"
    );
}
