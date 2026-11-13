//! Driver tests that need no terminal, so they run on every platform: a cancel that lands while
//! a plan is being carried out, and a wait on the pull request.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pty_host::{PtyHost, SessionInfo};

use super::*;
use crate::git::testing::git;
use crate::store::Store;
use crate::workflow::runs::{queue, Request};

/// Two steps with no needs: the engine plans both effects in one call.
const TWO_AT_ONCE: &str = r#"id: two
name: Two at once
version: 1
trigger:
  kind: manual
steps:
  - id: first
    action: notify
    title: first
  - id: second
    action: notify
    title: second
"#;

/// Notifies by remembering, and during the first notification cancels the run from a
/// connection of its own, as `ys workflow cancel` would from another process.
struct Cancelling {
    database: PathBuf,
    run: Mutex<Option<String>>,
    notified: Mutex<Vec<String>>,
}

impl Hands for Cancelling {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        unreachable!("this workflow starts no agent")
    }

    fn notify(&self, title: &str, _body: Option<&str>) -> Result<(), String> {
        let mut notified = self.notified.lock().unwrap();
        if notified.is_empty() {
            let run = self.run.lock().unwrap().clone().unwrap();
            let elsewhere = Store::open(&self.database).unwrap();
            assert!(elsewhere
                .cancel_workflow_run(&run, "Cancelled from ys.")
                .unwrap());
        }
        notified.push(title.to_owned());
        Ok(())
    }

    fn handoff(&self, _: &str) -> Option<String> {
        None
    }

    fn pr_posts(&self, _: &str, _: &crate::workflow::runs::PrFacts) -> PrAsk {
        unreachable!("this workflow waits on no pull request")
    }
}

#[test]
fn a_cancel_during_one_effect_stops_the_effects_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("yardsort.db");
    let store = Store::open(&database).unwrap();
    let project = store.add_project("app", "/code/app").unwrap();
    let workspace = store
        .add_worktree(
            &project.id,
            "fix-login",
            "/code/app-wt/fix-login",
            None,
            None,
        )
        .unwrap();
    let folder = crate::workflow::user_dir(dir.path());
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("two.yaml"), TWO_AT_ONCE).unwrap();
    let run = queue(
        &store,
        dir.path(),
        &Request {
            workflow_id: "two",
            workspace_id: &workspace.id,
            inputs: &BTreeMap::new(),
            requested_by: "cli",
            unattended: false,
            origin_record: None,
            depth: 0,
            event: None,
            tracked: None,
        },
        &crate::workflow::runs::Accepting::default(),
    )
    .unwrap();

    let hands = Cancelling {
        database,
        run: Mutex::new(Some(run.clone())),
        notified: Mutex::new(Vec::new()),
    };
    let host = PtyHost::new(Arc::new(|_| {}));
    Driver::default()
        .tick(&store, &host, &hands, &git())
        .unwrap();

    assert_eq!(
        *hands.notified.lock().unwrap(),
        vec!["first".to_owned()],
        "the second notification was planned before the cancel, and must not be shown after it"
    );
    let row = store.workflow_run(&run).unwrap().unwrap();
    assert_eq!(row.status, "cancelled");
    let steps = store.workflow_steps(&run).unwrap();
    let second = steps.iter().find(|s| s.step_id == "second").unwrap();
    assert_eq!(second.status, "cancelled");
}

/// Waits for a review, then says so.
const WAIT_FOR_A_REVIEW: &str = r#"id: wait-review
name: Wait for a review
version: 1
trigger:
  kind: manual
steps:
  - id: posted
    action: wait_pr_activity
    kind: review
    timeout: 10m
  - id: tell
    action: notify
    needs: [posted]
    title: "{{ steps.posted.count }} review on #{{ pr.number }}: {{ steps.posted.latest_url }}"
"#;

/// A forge whose answer the test sets, counting how often it is asked. With `hold`, each
/// question waits for the test to let it go, as a slow network would.
struct Forge {
    posts: Arc<Mutex<Vec<PrPost>>>,
    asked: Arc<Mutex<u32>>,
    hold: Option<Arc<Mutex<std::sync::mpsc::Receiver<()>>>>,
    notified: Mutex<Vec<String>>,
}

impl Forge {
    fn new(posts: Vec<PrPost>) -> Self {
        Self {
            posts: Arc::new(Mutex::new(posts)),
            asked: Arc::new(Mutex::new(0)),
            hold: None,
            notified: Mutex::new(Vec::new()),
        }
    }

    fn asked(&self) -> u32 {
        *self.asked.lock().unwrap()
    }
}

impl Hands for Forge {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        unreachable!("these workflows start no agent")
    }

    fn notify(&self, title: &str, _body: Option<&str>) -> Result<(), String> {
        self.notified.lock().unwrap().push(title.to_owned());
        Ok(())
    }

    fn handoff(&self, _: &str) -> Option<String> {
        None
    }

    fn pr_posts(&self, _: &str, pr: &crate::workflow::runs::PrFacts) -> PrAsk {
        assert_eq!(pr.number, 7);
        let (posts, asked, hold) = (
            Arc::clone(&self.posts),
            Arc::clone(&self.asked),
            self.hold.clone(),
        );
        Box::new(move || {
            if let Some(hold) = hold {
                let _ = hold.lock().unwrap().recv_timeout(Duration::from_secs(60));
            }
            *asked.lock().unwrap() += 1;
            Ok(posts.lock().unwrap().clone())
        })
    }
}

fn post(kind: crate::forge::PrPostKind, at_ms: i64, url: &str) -> PrPost {
    PrPost {
        kind,
        at_ms,
        url: Some(url.to_owned()),
    }
}

/// A profile with `workflows` in its folder and one workspace, whose pull request is #7.
struct Profile {
    dir: tempfile::TempDir,
    store: Store,
    workspace: String,
}

impl Profile {
    fn new(workflows: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
        let project = store.add_project("app", "/code/app").unwrap();
        let workspace = store
            .add_worktree(
                &project.id,
                "fix-login",
                "/code/app-wt/fix-login",
                None,
                None,
            )
            .unwrap();
        let folder = crate::workflow::user_dir(dir.path());
        std::fs::create_dir_all(&folder).unwrap();
        for (id, text) in workflows {
            std::fs::write(folder.join(format!("{id}.yaml")), text).unwrap();
        }
        Self {
            dir,
            store,
            workspace: workspace.id,
        }
    }

    fn queue(&self, workflow: &str) -> String {
        use crate::forge::{Checks, PullRequest, PullRequestState};
        let look = crate::workflow::runs::Accepting {
            pr: Some(PullRequest {
                number: 7,
                url: "https://github.com/o/r/pull/7".into(),
                title: "Fix the login".into(),
                branch: "fix-login".into(),
                state: PullRequestState::Open,
                draft: false,
                checks: Checks::None,
                details: None,
                author: None,
                created_at: None,
            }),
        };
        queue(
            &self.store,
            self.dir.path(),
            &Request {
                workflow_id: workflow,
                workspace_id: &self.workspace,
                inputs: &BTreeMap::new(),
                requested_by: "cli",
                unattended: false,
                origin_record: None,
                depth: 0,
                event: None,
                tracked: None,
            },
            &look,
        )
        .unwrap()
    }

    fn step(&self, run: &str, id: &str) -> crate::store::WorkflowStepRow {
        self.store
            .workflow_steps(run)
            .unwrap()
            .into_iter()
            .find(|s| s.step_id == id)
            .unwrap()
    }

    fn status(&self, run: &str) -> String {
        self.store.workflow_run(run).unwrap().unwrap().status
    }

    /// Tick until `done` holds, as the app's thread would.
    fn until(&self, driver: &Driver, hands: &dyn Hands, done: impl Fn() -> bool) {
        let host = PtyHost::new(Arc::new(|_| {}));
        let start = std::time::Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(10), "timed out");
            driver.tick(&self.store, &host, hands, &git()).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[test]
fn a_review_posted_after_the_run_began_ends_the_wait_and_the_forge_is_not_asked_every_tick() {
    use crate::forge::PrPostKind;
    let profile = Profile::new(&[("wait-review", WAIT_FOR_A_REVIEW)]);
    let run = profile.queue("wait-review");

    // Before the run: an old review, and a comment. Neither is what it waits for.
    let before = crate::store::now_ms() - 60_000;
    let forge = Forge::new(vec![
        post(PrPostKind::Review, before, "https://github.com/o/r/pull/7"),
        post(
            PrPostKind::Comment,
            before,
            "https://github.com/o/r/pull/7#c1",
        ),
    ]);
    let patient = Driver::new(SETTLE_BUSY_MS, Duration::from_secs(3600));
    profile.until(&patient, &forge, || forge.asked() == 1);
    let host = PtyHost::new(Arc::new(|_| {}));
    for _ in 0..5 {
        patient.tick(&profile.store, &host, &forge, &git()).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(profile.step(&run, "posted").status, "waiting");
    assert_eq!(forge.asked(), 1, "once per interval, not once per tick");

    // The review arrives; a driver asking every moment sees it.
    forge.posts.lock().unwrap().push(post(
        PrPostKind::Review,
        crate::store::now_ms(),
        "https://github.com/o/r/pull/7",
    ));
    let eager = Driver::new(SETTLE_BUSY_MS, Duration::ZERO);
    profile.until(&eager, &forge, || profile.status(&run) == "succeeded");
    assert_eq!(profile.step(&run, "posted").status, "succeeded");
    assert_eq!(
        *forge.notified.lock().unwrap(),
        vec!["1 review on #7: https://github.com/o/r/pull/7".to_owned()]
    );
}

/// Says so at once, needing nothing.
const RIGHT_AWAY: &str = r#"id: right-away
name: Right away
version: 1
trigger:
  kind: manual
steps:
  - id: tell
    action: notify
    title: right away
"#;

#[test]
fn a_forge_that_is_slow_to_answer_holds_up_no_other_run() {
    let profile = Profile::new(&[
        ("wait-review", WAIT_FOR_A_REVIEW),
        ("right-away", RIGHT_AWAY),
    ]);
    let waiting = profile.queue("wait-review");
    let (release, held) = std::sync::mpsc::channel();
    let mut forge = Forge::new(Vec::new());
    forge.hold = Some(Arc::new(Mutex::new(held)));
    let driver = Driver::new(SETTLE_BUSY_MS, Duration::ZERO);
    let host = PtyHost::new(Arc::new(|_| {}));

    // The first run's question to the forge goes out, and hangs.
    driver.tick(&profile.store, &host, &forge, &git()).unwrap();
    let other = profile.queue("right-away");
    let started = std::time::Instant::now();
    driver.tick(&profile.store, &host, &forge, &git()).unwrap();
    // Far short of the forge's minute, and far past what a loaded runner takes to do the rest.
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the tick waited {:?} on the forge",
        started.elapsed()
    );
    assert_eq!(
        profile.status(&other),
        "succeeded",
        "the other run went ahead while the forge was silent"
    );
    assert_eq!(profile.step(&waiting, "posted").status, "waiting");
    assert_eq!(
        forge.asked(),
        0,
        "the question is still out, and was not asked twice"
    );

    release.send(()).unwrap();
    profile.until(&driver, &forge, || forge.asked() >= 1);
}

#[test]
fn a_tick_says_when_a_run_changed_so_the_window_only_looks_when_there_is_something_new() {
    let profile = Profile::new(&[("right-away", RIGHT_AWAY)]);
    let forge = Forge::new(Vec::new());
    let driver = Driver::default();
    let host = PtyHost::new(Arc::new(|_| {}));
    assert!(
        !driver.tick(&profile.store, &host, &forge, &git()).unwrap(),
        "nothing to do"
    );
    let run = profile.queue("right-away");
    assert!(
        driver.tick(&profile.store, &host, &forge, &git()).unwrap(),
        "it started and finished"
    );
    assert_eq!(profile.status(&run), "succeeded");
    assert!(
        !driver.tick(&profile.store, &host, &forge, &git()).unwrap(),
        "and then nothing moved"
    );
}

#[test]
fn a_run_cancelled_from_ys_between_ticks_is_a_change_the_window_is_told_about() {
    let profile = Profile::new(&[("wait-review", WAIT_FOR_A_REVIEW)]);
    let run = profile.queue("wait-review");
    let forge = Forge::new(Vec::new());
    let driver = Driver::default();
    let host = PtyHost::new(Arc::new(|_| {}));
    assert!(
        driver.tick(&profile.store, &host, &forge, &git()).unwrap(),
        "it started, and waits"
    );
    assert!(
        !driver.tick(&profile.store, &host, &forge, &git()).unwrap(),
        "still waiting: no change"
    );

    // `ys workflow cancel`, from a connection of its own.
    let elsewhere = Store::open(&profile.dir.path().join("yardsort.db")).unwrap();
    assert!(elsewhere
        .cancel_workflow_run(&run, "Cancelled from ys.")
        .unwrap());
    assert!(
        driver.tick(&profile.store, &host, &forge, &git()).unwrap(),
        "the run is gone from the active ones, which is a change"
    );
    assert!(
        !driver.tick(&profile.store, &host, &forge, &git()).unwrap(),
        "and said once"
    );
}

/// A smoke test file that needs nothing but Node, so it is runnable without a browser or a set
/// environment — enough for `run_test` to accept and queue it.
const SMOKE_DEMO: &str = r#"id: demo
name: Demo
version: 1
steps:
  - id: version
    action: run
    program: node
    args: [--version]
"#;

/// One `run_test` step: the driver queues a smoke run in the workspace, marked as the workflow's,
/// and the step waits for it. No agent, no terminal, so this runs everywhere.
const RUN_A_TEST: &str = r#"id: smoke
name: Smoke
version: 1
trigger:
  kind: manual
steps:
  - id: run
    action: run_test
    test: demo
    timeout: 30m
"#;

/// Hands a `run_test`-only workflow never reaches: it queues the run through the store itself.
struct NoHands;
impl Hands for NoHands {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        unreachable!("no agent")
    }
    fn notify(&self, _: &str, _: Option<&str>) -> Result<(), String> {
        unreachable!("no notify")
    }
    fn handoff(&self, _: &str) -> Option<String> {
        None
    }
    fn pr_posts(&self, _: &str, _: &crate::workflow::runs::PrFacts) -> PrAsk {
        unreachable!("no pull request")
    }
}

#[test]
fn a_run_test_step_queues_a_run_marked_as_the_workflows_and_waits_for_it() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("yardsort.db");
    let store = Store::open(&database).unwrap();
    let project = store.add_project("app", "/code/app").unwrap();
    // A real worktree folder with the smoke test the step names.
    let worktree = dir.path().join("wt");
    std::fs::create_dir_all(crate::smoke::dir(&worktree)).unwrap();
    std::fs::write(crate::smoke::dir(&worktree).join("demo.yaml"), SMOKE_DEMO).unwrap();
    let workspace = store
        .add_worktree(
            &project.id,
            "fix-login",
            &worktree.to_string_lossy(),
            None,
            None,
        )
        .unwrap();
    let folder = crate::workflow::user_dir(dir.path());
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("smoke.yaml"), RUN_A_TEST).unwrap();
    let run = queue(
        &store,
        dir.path(),
        &Request {
            workflow_id: "smoke",
            workspace_id: &workspace.id,
            inputs: &BTreeMap::new(),
            requested_by: "cli",
            unattended: false,
            origin_record: None,
            depth: 0,
            event: None,
            tracked: None,
        },
        &crate::workflow::runs::Accepting::default(),
    )
    .unwrap();

    let host = PtyHost::new(Arc::new(|_| {}));
    Driver::default()
        .tick(&store, &host, &NoHands, &git())
        .unwrap();

    // The step queued a smoke run, in this workspace, marked as the workflow's.
    let runs = store.active_test_runs().unwrap();
    let queued = runs
        .iter()
        .find(|r| r.workspace_id.as_deref() == Some(workspace.id.as_str()))
        .expect("a smoke run was queued");
    assert_eq!(queued.requested_by, "workflow");
    // And the step waits for it, rather than finishing.
    let steps = store.workflow_steps(&run).unwrap();
    let step = steps.iter().find(|s| s.step_id == "run").unwrap();
    assert_eq!(step.status, "waiting");
    assert_eq!(
        super::engine::parse_outputs(&step.outputs)
            .get("run")
            .map(String::as_str),
        Some(queued.id.as_str()),
        "the step kept the run's id to wait on"
    );
}

/// What a schedule's run relies on. Nobody is there to approve a file or answer a step, and a
/// `run_test` step with no timeout would wait with its test for ever, the run active all the
/// while: so the step fails at once, with nothing queued, and says why.
#[test]
fn a_run_test_step_in_a_run_nobody_watches_fails_rather_than_wait_for_a_person() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
    let project = store.add_project("app", "/code/app").unwrap();
    let worktree = dir.path().join("wt");
    std::fs::create_dir_all(crate::smoke::dir(&worktree)).unwrap();
    std::fs::write(crate::smoke::dir(&worktree).join("demo.yaml"), SMOKE_DEMO).unwrap();
    let workspace = store
        .add_worktree(
            &project.id,
            "fix-login",
            &worktree.to_string_lossy(),
            None,
            None,
        )
        .unwrap();
    let folder = crate::workflow::user_dir(dir.path());
    std::fs::create_dir_all(&folder).unwrap();
    // The test is named by an input, so nothing could have known which before the run, and
    // the step has no timeout.
    let workflow = RUN_A_TEST
        .replace(
            "steps:",
            "inputs:\n  - id: which\n    kind: test\n    default: demo\nsteps:",
        )
        .replace("test: demo", "test: \"{{ inputs.which }}\"")
        .replace("    timeout: 30m\n", "");
    std::fs::write(folder.join("smoke.yaml"), workflow).unwrap();
    let start = |unattended: bool| {
        queue(
            &store,
            dir.path(),
            &Request {
                workflow_id: "smoke",
                workspace_id: &workspace.id,
                inputs: &BTreeMap::new(),
                requested_by: "app",
                unattended,
                origin_record: None,
                depth: 0,
                event: None,
                tracked: None,
            },
            &crate::workflow::runs::Accepting::default(),
        )
        .unwrap()
    };
    let host = PtyHost::new(Arc::new(|_| {}));
    let driver = Driver::default();

    // Nobody approved the file.
    let run = start(true);
    driver.tick(&store, &host, &NoHands, &git()).unwrap();
    driver.tick(&store, &host, &NoHands, &git()).unwrap();
    assert!(
        store.active_test_runs().unwrap().is_empty(),
        "no run is left waiting for an approval nobody is there to give"
    );
    let steps = store.workflow_steps(&run).unwrap();
    let step = steps.iter().find(|s| s.step_id == "run").unwrap();
    assert_eq!(step.status, "failed");
    let note = step.note.as_deref().unwrap();
    assert!(
        note.contains("new, or has changed since you approved it"),
        "{note}"
    );
    let ended = store.workflow_run(&run).unwrap().unwrap();
    assert_eq!(
        ended.status, "failed",
        "the run ends, so the next time can start one"
    );
    assert!(ended.error.as_deref().unwrap().contains("`run`"));

    // A step a person answers, approved or not.
    let manual =
        format!("{SMOKE_DEMO}  - id: look\n    action: manual\n    prompt: Does it look right?\n");
    std::fs::write(crate::smoke::dir(&worktree).join("demo.yaml"), &manual).unwrap();
    store
        .approve_test(
            &project.id,
            "demo",
            &crate::smoke::runs::content_hash(&manual),
        )
        .unwrap();
    let run = start(true);
    driver.tick(&store, &host, &NoHands, &git()).unwrap();
    driver.tick(&store, &host, &NoHands, &git()).unwrap();
    assert!(store.active_test_runs().unwrap().is_empty());
    let steps = store.workflow_steps(&run).unwrap();
    let note = steps[0].note.clone().unwrap();
    assert!(note.contains("a step a person does (`look`)"), "{note}");

    // Approved, with nothing for a person in it, it is queued like any other.
    std::fs::write(crate::smoke::dir(&worktree).join("demo.yaml"), SMOKE_DEMO).unwrap();
    store
        .approve_test(
            &project.id,
            "demo",
            &crate::smoke::runs::content_hash(SMOKE_DEMO),
        )
        .unwrap();
    let run = start(true);
    driver.tick(&store, &host, &NoHands, &git()).unwrap();
    let queued = store.active_test_runs().unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].status, "queued");
    let steps = store.workflow_steps(&run).unwrap();
    assert_eq!(steps[0].status, "waiting");
}

/// A smoke test that must pass before anything after it runs.
const GATED_TEST: &str = r#"id: gate
name: Gate
version: 1
trigger:
  kind: manual
steps:
  - id: run
    action: run_test
    test: demo
    gate: true
    timeout: 30m
  - id: tell
    action: notify
    needs: [run]
    title: Tests passed
    body: "{{ steps.run.summary }}"
"#;

/// Two smoke tests at once. One is a gate. The other does not need it, so a halt is what
/// stops it, and the test it queued should be cancelled.
const TWO_TESTS: &str = r#"id: gate
name: Gate
version: 1
trigger:
  kind: manual
steps:
  - id: gate
    action: run_test
    test: demo
    gate: true
    timeout: 30m
  - id: side
    action: run_test
    test: other
    timeout: 30m
"#;

const OTHER_SMOKE: &str = r#"id: other
name: Other
version: 1
steps:
  - id: version
    action: run
    program: node
    args: [--version]
"#;

/// Remembers each notification, so a test can see that a success step ran and a failure did not.
struct Told {
    sent: Mutex<Vec<(String, Option<String>)>>,
}

impl Hands for Told {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        unreachable!("this workflow starts no agent")
    }

    fn notify(&self, title: &str, body: Option<&str>) -> Result<(), String> {
        self.sent
            .lock()
            .unwrap()
            .push((title.to_owned(), body.map(str::to_owned)));
        Ok(())
    }

    fn handoff(&self, _: &str) -> Option<String> {
        None
    }

    fn pr_posts(&self, _: &str, _: &crate::workflow::runs::PrFacts) -> PrAsk {
        unreachable!("this workflow waits on no pull request")
    }
}

/// Queue `yaml` against the demo smoke test and tick once, so its test step is waiting.
fn queued_workflow(yaml: &str) -> (tempfile::TempDir, Store, String, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
    let project = store.add_project("app", "/code/app").unwrap();
    let worktree = dir.path().join("wt");
    std::fs::create_dir_all(crate::smoke::dir(&worktree)).unwrap();
    std::fs::write(crate::smoke::dir(&worktree).join("demo.yaml"), SMOKE_DEMO).unwrap();
    let workspace = store
        .add_worktree(
            &project.id,
            "fix-login",
            &worktree.to_string_lossy(),
            None,
            None,
        )
        .unwrap();
    let folder = crate::workflow::user_dir(dir.path());
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("gate.yaml"), yaml).unwrap();
    let run = queue(
        &store,
        dir.path(),
        &Request {
            workflow_id: "gate",
            workspace_id: &workspace.id,
            inputs: &BTreeMap::new(),
            requested_by: "cli",
            unattended: false,
            origin_record: None,
            depth: 0,
            event: None,
            tracked: None,
        },
        &crate::workflow::runs::Accepting::default(),
    )
    .unwrap();
    (dir, store, run, project.id, workspace.id)
}

fn finish_smoke(store: &Store, status: &str, error: Option<&str>) {
    let run = store
        .active_test_runs()
        .unwrap()
        .into_iter()
        .next()
        .expect("a smoke run was queued");
    if run.status == "awaiting_approval" {
        assert!(store.approve_test_run(&run.id).unwrap());
    }
    assert!(
        store.start_test_run(&run.id, 9, "out").unwrap(),
        "the smoke run was {}",
        run.status
    );
    assert!(store.finish_test_run(&run.id, status, error).unwrap());
}

fn finish_one(store: &Store, id: &str, status: &str, error: Option<&str>) {
    let run = store.test_run(id).unwrap().expect("the smoke run");
    if run.status == "awaiting_approval" {
        assert!(store.approve_test_run(id).unwrap());
    }
    assert!(
        store.start_test_run(id, 9, "out").unwrap(),
        "the smoke run was {}",
        run.status
    );
    assert!(store.finish_test_run(id, status, error).unwrap());
}

fn step_test_id(store: &Store, workflow_run: &str, step: &str) -> String {
    let row = store
        .workflow_steps(workflow_run)
        .unwrap()
        .into_iter()
        .find(|row| row.step_id == step)
        .unwrap_or_else(|| panic!("no step {step}"));
    crate::workflow::engine::parse_outputs(&row.outputs)
        .get("run")
        .cloned()
        .unwrap_or_else(|| panic!("{step} queued no smoke run: {}", row.outputs))
}

fn tick(store: &Store, hands: &dyn Hands) {
    let host = PtyHost::new(Arc::new(|_| {}));
    Driver::default().tick(store, &host, hands, &git()).unwrap();
}

#[test]
fn a_failed_verdict_on_a_gate_fails_the_run_and_a_schedule_tells_you_why() {
    let (_dir, store, run, project, workspace) = queued_workflow(GATED_TEST);
    let hands = Told {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    assert!(hands.sent.lock().unwrap().is_empty(), "still waiting");
    finish_smoke(&store, "failed", Some("wanted 200, got 500"));
    tick(&store, &hands);

    assert!(
        hands.sent.lock().unwrap().is_empty(),
        "the notify step was written for a pass, so a failure does not send it"
    );
    let ended = store.workflow_run(&run).unwrap().unwrap();
    assert_eq!(ended.status, "failed");
    let error = ended.error.unwrap();
    assert!(
        error.starts_with("Step `run` failed: failed  demo"),
        "{error}"
    );
    assert!(error.contains("steps passed"), "{error}");
    assert!(
        !error.contains('\n') && !error.contains("wanted 200"),
        "{error}"
    );
    let steps = store.workflow_steps(&run).unwrap();
    let smoke = steps.iter().find(|step| step.step_id == "run").unwrap();
    assert_eq!(smoke.status, "failed");
    let note = smoke.note.as_deref().unwrap();
    assert!(note.contains("The test failed:"), "{note}");
    assert!(note.contains("wanted 200, got 500"), "{note}");
    let outcome: serde_json::Value =
        serde_json::from_str(smoke.outcome.as_deref().unwrap()).unwrap();
    assert_eq!(outcome["verdict"], "failed");
    assert!(outcome["reason"]
        .as_str()
        .unwrap()
        .contains("wanted 200, got 500"));
    let tell = steps.iter().find(|step| step.step_id == "tell").unwrap();
    assert_eq!(tell.status, "skipped");

    let spec = crate::schedule::ScheduleSpec::Daily {
        time: crate::schedule::TimeOfDay { hour: 9, minute: 0 },
    }
    .to_json();
    for (name, notify) in [("Nightly", "failure"), ("Quiet", "never")] {
        let row = store
            .add_schedule(
                &crate::store::ScheduleFields {
                    name,
                    target_kind: "workflow",
                    target_id: "gate",
                    project_id: &project,
                    workspace_id: &workspace,
                    workspace_name: "fix-login",
                    inputs: "{}",
                    spec: &spec,
                    missed: "run_once",
                    notify,
                    next_at: Some(1),
                },
                "cli",
            )
            .unwrap();
        store
            .record_schedule_run(&crate::store::NewScheduleRun {
                schedule_id: &row.id,
                due_at: 1,
                fired_at: 1,
                outcome: "started",
                manual: false,
                run_id: Some(&run),
                note: None,
            })
            .unwrap();
    }
    let settled = crate::schedule::fire::settle(&store).unwrap();
    assert_eq!(settled.notices.len(), 1);
    assert_eq!(settled.notices[0].title, "Nightly failed");
    assert_eq!(settled.notices[0].body, error);
    let quiet = store
        .schedules()
        .unwrap()
        .into_iter()
        .find(|schedule| schedule.name == "Quiet")
        .unwrap();
    let history = store.schedule_runs(&quiet.id, 10).unwrap();
    assert_eq!(history[0].result.as_deref(), Some("failed"));
    assert!(history[0].notified_at.is_none());
}

#[test]
fn a_failed_gate_run_is_told_by_a_trigger_and_not_by_one_set_to_never() {
    let (_dir, store, run, project, workspace) = queued_workflow(GATED_TEST);
    tick(&store, &NoHands);
    finish_smoke(&store, "failed", Some("wanted 200, got 500"));
    tick(&store, &NoHands);
    let error = store.workflow_run(&run).unwrap().unwrap().error.unwrap();
    assert!(
        error.starts_with("Step `run` failed: failed  demo"),
        "{error}"
    );
    assert!(error.contains("steps passed"), "{error}");
    let note = store
        .workflow_steps(&run)
        .unwrap()
        .into_iter()
        .find(|step| step.step_id == "run")
        .unwrap()
        .note
        .unwrap();
    assert!(note.contains("wanted 200, got 500"), "{note}");

    for (name, notify) in [("On failure", "failure"), ("Quiet", "never")] {
        let row = store
            .add_trigger(
                &crate::store::TriggerFields {
                    name,
                    preset: None,
                    project_id: &project,
                    workspace_id: &workspace,
                    workspace_name: "fix-login",
                    session_id: None,
                    events: r#"["workspace.done"]"#,
                    filters: "{}",
                    target_kind: "workflow",
                    target_id: "gate",
                    inputs: "{}",
                    quiet_ms: 0,
                    cap: 3,
                    notify,
                },
                "cli",
                1,
            )
            .unwrap();
        let fire = store
            .open_new_trigger_fire(&crate::store::NewTriggerFire {
                trigger_id: &row.id,
                subject: "workspace:fix",
                event: "workspace.done",
                at: 1,
                due_at: 1,
                facts: "{}",
                depth: 0,
            })
            .unwrap();
        assert!(store
            .decide_trigger_fire(
                &fire.id,
                "gathering",
                &crate::store::FireDecision {
                    outcome: "started",
                    run_id: Some(&run),
                    note: None,
                    counted: true,
                    at: 2,
                },
            )
            .unwrap());
    }

    let settled = crate::trigger::fire::settle(&store).unwrap();
    assert_eq!(settled.notices.len(), 1);
    assert_eq!(settled.notices[0].title, "On failure failed");
    assert_eq!(settled.notices[0].body, error);
    assert!(
        !settled.notices[0].body.contains("wanted 200"),
        "the notification is the short line; the report stays on the step"
    );
    let quiet = store
        .triggers()
        .unwrap()
        .into_iter()
        .find(|trigger| trigger.name == "Quiet")
        .unwrap();
    let history = store.trigger_fires(&quiet.id, 10).unwrap();
    assert_eq!(history[0].result.as_deref(), Some("failed"));
    assert!(history[0].notified_at.is_none());
}

#[test]
fn a_failed_gate_cancels_the_smoke_test_a_parallel_step_is_waiting_on() {
    let (dir, store, run, _, _) = queued_workflow(TWO_TESTS);
    std::fs::write(
        crate::smoke::dir(&dir.path().join("wt")).join("other.yaml"),
        OTHER_SMOKE,
    )
    .unwrap();
    tick(&store, &NoHands);
    let gate_run = step_test_id(&store, &run, "gate");
    let side_run = step_test_id(&store, &run, "side");
    let side = store.test_run(&side_run).unwrap().unwrap();
    assert!(
        matches!(side.status.as_str(), "queued" | "awaiting_approval"),
        "the other test is still waiting, not {}",
        side.status
    );
    finish_one(&store, &gate_run, "failed", Some("wanted 200, got 500"));
    tick(&store, &NoHands);

    let steps = store.workflow_steps(&run).unwrap();
    let side_step = steps.iter().find(|step| step.step_id == "side").unwrap();
    assert_eq!(side_step.status, "skipped");
    assert_eq!(
        side_step.note.as_deref(),
        Some("Skipped: the gate `gate` failed.")
    );
    let ended = store.workflow_run(&run).unwrap().unwrap();
    assert_eq!(ended.status, "failed");
    let cancelled = store.test_run(&side_run).unwrap().unwrap();
    assert_eq!(cancelled.status, "cancelled");
    assert_eq!(
        cancelled.error.as_deref(),
        Some("The workflow stopped because a gate failed.")
    );
    let gate = store.test_run(&gate_run).unwrap().unwrap();
    assert_eq!(gate.status, "failed");
}

#[test]
fn a_halt_still_finishes_when_cancelling_the_smoke_test_fails() {
    let (dir, store, run, _, _) = queued_workflow(TWO_TESTS);
    std::fs::write(
        crate::smoke::dir(&dir.path().join("wt")).join("other.yaml"),
        OTHER_SMOKE,
    )
    .unwrap();
    tick(&store, &NoHands);
    let gate_run = step_test_id(&store, &run, "gate");
    let side_run = step_test_id(&store, &run, "side");
    // The cancel writes `status = 'cancelled'`. This makes that write fail, the way a broken
    // database would, so the halt has to finish the workflow anyway.
    rusqlite::Connection::open(dir.path().join("yardsort.db"))
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_cancel BEFORE UPDATE OF status ON test_runs
             WHEN NEW.status = 'cancelled'
             BEGIN
               SELECT RAISE(ABORT, 'cancel refused');
             END;",
        )
        .unwrap();
    finish_one(&store, &gate_run, "failed", Some("wanted 200, got 500"));
    tick(&store, &NoHands);

    let steps = store.workflow_steps(&run).unwrap();
    let side_step = steps.iter().find(|step| step.step_id == "side").unwrap();
    assert_eq!(side_step.status, "skipped");
    let ended = store.workflow_run(&run).unwrap().unwrap();
    assert_eq!(ended.status, "failed");
    let still = store.test_run(&side_run).unwrap().unwrap();
    assert_ne!(still.status, "cancelled");
}

#[test]
fn a_passing_verdict_on_a_gate_runs_the_step_after_it() {
    let (_dir, store, run, _, _) = queued_workflow(GATED_TEST);
    let hands = Told {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    finish_smoke(&store, "passed", None);
    tick(&store, &hands);
    let sent = hands.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "Tests passed");
    assert!(sent[0].1.as_deref().unwrap().contains("passed  demo"));
    let ended = store.workflow_run(&run).unwrap().unwrap();
    assert_eq!(ended.status, "succeeded");
    let smoke = store
        .workflow_steps(&run)
        .unwrap()
        .into_iter()
        .find(|step| step.step_id == "run")
        .unwrap();
    assert_eq!(smoke.note.as_deref(), Some("The test passed."));
}

#[test]
fn a_failed_verdict_without_a_gate_still_lets_the_run_succeed() {
    let open = GATED_TEST.replace("    gate: true\n", "");
    let (_dir, store, run, _, _) = queued_workflow(&open);
    let hands = Told {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    finish_smoke(&store, "failed", Some("wanted 200, got 500"));
    tick(&store, &hands);
    let smoke = store
        .workflow_steps(&run)
        .unwrap()
        .into_iter()
        .find(|step| step.step_id == "run")
        .unwrap();
    assert_eq!(smoke.status, "succeeded");
    assert_eq!(smoke.note.as_deref(), Some("The test failed."));
    let ended = store.workflow_run(&run).unwrap().unwrap();
    assert_eq!(ended.status, "succeeded");
    assert_eq!(hands.sent.lock().unwrap()[0].0, "Tests passed");
}

/// Remembers notifications, and fails the test if a conversation is read. A file or a piece of
/// text is not a transcript.
struct Filed {
    sent: Mutex<Vec<String>>,
}

impl Hands for Filed {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        unreachable!("this workflow starts no agent")
    }

    fn notify(&self, title: &str, _: Option<&str>) -> Result<(), String> {
        self.sent.lock().unwrap().push(title.to_owned());
        Ok(())
    }

    fn handoff(&self, _: &str) -> Option<String> {
        None
    }

    fn pr_posts(&self, _: &str, _: &crate::workflow::runs::PrFacts) -> PrAsk {
        unreachable!("this workflow waits on no pull request")
    }

    fn conversation(
        &self,
        _: &str,
        _: &str,
        _: bool,
        _: &str,
        _: &str,
    ) -> Result<Conversation, String> {
        panic!("a file artifact must not read a conversation");
    }
}

/// Starts one stand-in session and answers for its conversation.
struct Talks {
    sent: Mutex<Vec<String>>,
    asked: Mutex<Vec<(String, bool)>>,
}

impl Hands for Talks {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        Ok(SessionInfo {
            id: pty_host::SessionId("pty-9".into()),
            program: "claude".into(),
            args: Vec::new(),
            cwd: None,
            pid: None,
            size: pty_host::TermSize::DEFAULT,
            labels: BTreeMap::new(),
            state: pty_host::SessionState::Running,
            has_output: false,
            busy: false,
            busy_ms: 0,
            idle_ms: 0,
        })
    }

    fn notify(&self, title: &str, _: Option<&str>) -> Result<(), String> {
        self.sent.lock().unwrap().push(title.to_owned());
        Ok(())
    }

    fn handoff(&self, _: &str) -> Option<String> {
        None
    }

    fn pr_posts(&self, _: &str, _: &crate::workflow::runs::PrFacts) -> PrAsk {
        unreachable!("this workflow waits on no pull request")
    }

    fn conversation(
        &self,
        _: &str,
        pty: &str,
        raw: bool,
        _: &str,
        _: &str,
    ) -> Result<Conversation, String> {
        self.asked.lock().unwrap().push((pty.to_owned(), raw));
        Ok(Conversation::Ready(Some(
            if raw { "RAW TEXT" } else { "DIGEST" }.into(),
        )))
    }
}

/// Queue `yaml` in a real workspace folder and return that folder.
fn queue_folder(
    yaml: &str,
    id: &str,
    inputs: &BTreeMap<String, String>,
) -> (tempfile::TempDir, Store, String, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
    let project = store.add_project("app", "/code/app").unwrap();
    let worktree = dir.path().join("wt");
    std::fs::create_dir_all(&worktree).unwrap();
    let workspace = store
        .add_worktree(
            &project.id,
            "fix-login",
            &worktree.to_string_lossy(),
            None,
            None,
        )
        .unwrap();
    let folder = crate::workflow::user_dir(dir.path());
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(format!("{id}.yaml")), yaml).unwrap();
    let run = queue(
        &store,
        dir.path(),
        &Request {
            workflow_id: id,
            workspace_id: &workspace.id,
            inputs,
            requested_by: "cli",
            unattended: false,
            origin_record: None,
            depth: 0,
            event: None,
            tracked: None,
        },
        &crate::workflow::runs::Accepting::default(),
    )
    .unwrap();
    (dir, store, run, worktree)
}

const FILE_HANDOFF: &str = r#"id: hand
name: Hand
version: 1
trigger:
  kind: manual
steps:
  - id: draft
    action: notify
    title: Drafted
    produces:
      - name: spec
        path: SPEC.md
  - id: plan
    action: notify
    needs: [draft]
    consumes: [spec]
    title: "got {{ artifacts.spec }}"
"#;

#[test]
fn a_file_is_handed_to_the_next_step_and_kept_on_the_run() {
    let (_dir, store, run, worktree) = queue_folder(FILE_HANDOFF, "hand", &BTreeMap::new());
    std::fs::write(worktree.join("SPEC.md"), "Be kind.").unwrap();
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    assert_eq!(
        *hands.sent.lock().unwrap(),
        vec!["Drafted".to_owned(), "got Be kind.".to_owned()]
    );
    let saved = store.workflow_artifacts(&run).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "spec");
    assert_eq!(saved[0].step_id, "draft");
    assert_eq!(saved[0].source, "file");
    assert_eq!(saved[0].detail.as_deref(), Some("SPEC.md"));
    assert_eq!(saved[0].body, "Be kind.");
    assert!(!saved[0].truncated);
    assert_eq!(
        store.workflow_run(&run).unwrap().unwrap().status,
        "succeeded"
    );
}

#[test]
fn a_missing_file_fails_the_step_and_keeps_an_older_artifact() {
    let (_dir, store, run, _worktree) = queue_folder(FILE_HANDOFF, "hand", &BTreeMap::new());
    store
        .replace_step_artifacts(
            &run,
            "draft",
            &[crate::store::NewWorkflowArtifact {
                name: "spec".into(),
                source: "file".into(),
                detail: Some("SPEC.md".into()),
                body: "v1".into(),
                truncated: false,
            }],
        )
        .unwrap();
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    assert_eq!(*hands.sent.lock().unwrap(), vec!["Drafted".to_owned()]);
    let draft = store
        .workflow_steps(&run)
        .unwrap()
        .into_iter()
        .find(|step| step.step_id == "draft")
        .unwrap();
    assert_eq!(draft.status, "failed");
    assert!(
        !draft.outputs.contains("artifact_pending"),
        "a failed capture does not keep the pending marker: {}",
        draft.outputs
    );
    assert!(draft.note.unwrap().contains("not in the workspace"));
    let plan = store
        .workflow_steps(&run)
        .unwrap()
        .into_iter()
        .find(|step| step.step_id == "plan")
        .unwrap();
    assert_eq!(plan.status, "skipped");
    assert_eq!(store.workflow_artifacts(&run).unwrap()[0].body, "v1");
}

#[test]
fn a_rendered_secret_path_is_not_copied() {
    let mut inputs = BTreeMap::new();
    inputs.insert("file".into(), ".env".into());
    let yaml = r#"id: hand
name: Hand
version: 1
trigger:
  kind: manual
inputs:
  - id: file
    kind: text
steps:
  - id: draft
    action: notify
    title: Drafted
    produces:
      - name: spec
        path: "{{ inputs.file }}"
"#;
    let (_dir, store, run, worktree) = queue_folder(yaml, "hand", &inputs);
    std::fs::write(worktree.join(".env"), "TOKEN=1").unwrap();
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    let draft = store
        .workflow_steps(&run)
        .unwrap()
        .into_iter()
        .find(|step| step.step_id == "draft")
        .unwrap();
    assert_eq!(draft.status, "failed");
    assert!(draft.note.unwrap().contains("secret"));
    assert!(store.workflow_artifacts(&run).unwrap().is_empty());
}

const TALK: &str = r#"id: talk
name: Talk
version: 1
trigger:
  kind: manual
steps:
  - id: start
    action: start_session
    harness: claude
    prompt: Hello
  - id: note
    action: notify
    needs: [start]
    title: Noted
    produces:
      - name: discussion
        from: transcript
        session: "{{ steps.start.session }}"
      - name: rawtalk
        from: transcript
        session: "{{ steps.start.session }}"
        raw: true
  - id: say
    action: notify
    needs: [note]
    consumes: [discussion, rawtalk]
    title: "{{ artifacts.discussion }} / {{ artifacts.rawtalk }}"
"#;

#[test]
fn a_transcript_is_the_digest_unless_the_step_asks_for_raw() {
    let (_dir, store, run, _worktree) = queue_folder(TALK, "talk", &BTreeMap::new());
    let hands = Talks {
        sent: Mutex::new(Vec::new()),
        asked: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    assert_eq!(
        *hands.asked.lock().unwrap(),
        vec![("pty-9".to_owned(), false), ("pty-9".to_owned(), true)]
    );
    assert_eq!(
        hands.sent.lock().unwrap().last().map(String::as_str),
        Some("DIGEST / RAW TEXT")
    );
    let saved = store.workflow_artifacts(&run).unwrap();
    assert_eq!(saved.len(), 2);
    assert_eq!(
        saved
            .iter()
            .find(|row| row.name == "discussion")
            .unwrap()
            .detail
            .as_deref(),
        Some("digest")
    );
    assert_eq!(
        saved.iter().find(|row| row.name == "rawtalk").unwrap().body,
        "RAW TEXT"
    );
}

const SIGN_OFF: &str = r#"id: sign
name: Sign
version: 1
trigger:
  kind: manual
steps:
  - id: brief
    action: notify
    title: Brief
    produces:
      - name: plan
        text: Ship it
  - id: sign-off
    needs: [brief]
    action: approve
    consumes: [plan]
    prompt: Look at the plan.
  - id: next
    needs: [sign-off]
    action: notify
    title: Go
"#;

fn step(store: &Store, run: &str, id: &str) -> crate::store::WorkflowStepRow {
    store
        .workflow_steps(run)
        .unwrap()
        .into_iter()
        .find(|step| step.step_id == id)
        .unwrap_or_else(|| panic!("no step {id}"))
}

#[test]
fn approving_continues_and_the_next_step_does_not_start_while_it_waits() {
    let (_dir, store, run, _worktree) = queue_folder(SIGN_OFF, "sign", &BTreeMap::new());
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "sign-off").status, "waiting");
    assert_eq!(step(&store, &run, "next").status, "pending");
    assert_eq!(store.workflow_run(&run).unwrap().unwrap().status, "running");
    assert!(
        hands
            .sent
            .lock()
            .unwrap()
            .iter()
            .any(|title| title == "Waiting for your approval"),
        "a run a person started is told: {:?}",
        hands.sent.lock().unwrap()
    );
    assert!(recover_is_a_no_op(&store, &run));
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "sign-off").status, "waiting");
    assert_eq!(step(&store, &run, "next").status, "pending");
    assert_eq!(
        store
            .record_workflow_approval(&run, "sign-off", "approved", Some("ship it"), "ada", None)
            .unwrap(),
        crate::store::ApprovalWrite::Recorded
    );
    assert_eq!(
        store
            .record_workflow_approval(&run, "sign-off", "rejected", Some("no"), "ada", None)
            .unwrap(),
        crate::store::ApprovalWrite::AlreadyDecided
    );
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "sign-off").status, "succeeded");
    let outcome: serde_json::Value =
        serde_json::from_str(step(&store, &run, "sign-off").outcome.unwrap().as_str()).unwrap();
    assert_eq!(outcome["decision"], "approved");
    assert_eq!(outcome["feedback"], "ship it");
    assert_eq!(outcome["actor"], "ada");
    assert_eq!(step(&store, &run, "next").status, "succeeded");
    assert_eq!(
        store.workflow_run(&run).unwrap().unwrap().status,
        "succeeded"
    );
    assert!(hands.sent.lock().unwrap().iter().any(|title| title == "Go"));
}

#[test]
fn rejecting_stops_the_run_and_a_request_for_changes_is_stored_apart_from_it() {
    let (_dir, store, run, _worktree) = queue_folder(SIGN_OFF, "sign", &BTreeMap::new());
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    assert_eq!(
        store
            .record_workflow_approval(&run, "sign-off", "changes_requested", None, "ada", None)
            .unwrap(),
        crate::store::ApprovalWrite::FeedbackRequired
    );
    assert_eq!(step(&store, &run, "sign-off").status, "waiting");
    store
        .record_workflow_approval(
            &run,
            "sign-off",
            "changes_requested",
            Some("split the migration"),
            "ada",
            None,
        )
        .unwrap();
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "sign-off").status, "failed");
    assert_eq!(step(&store, &run, "next").status, "skipped");
    assert_eq!(store.workflow_run(&run).unwrap().unwrap().status, "failed");
    let outcome: serde_json::Value =
        serde_json::from_str(step(&store, &run, "sign-off").outcome.unwrap().as_str()).unwrap();
    assert_eq!(outcome["decision"], "changes_requested");
    assert_ne!(outcome["decision"], "rejected");
    let saved = store.workflow_artifacts(&run).unwrap();
    let changes = saved
        .iter()
        .find(|row| row.name == "changes-sign-off")
        .expect("the request is an artifact a later revision can consume");
    assert_eq!(changes.body, "split the migration");
    assert_eq!(changes.source, "decision");
    assert!(!hands.sent.lock().unwrap().iter().any(|title| title == "Go"));
}

#[test]
fn a_decision_on_a_finished_run_is_refused() {
    let (_dir, store, run, _worktree) = queue_folder(SIGN_OFF, "sign", &BTreeMap::new());
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    assert!(store.cancel_workflow_run(&run, "Cancelled.").unwrap());
    let refused = store
        .record_workflow_approval(&run, "sign-off", "approved", None, "ada", None)
        .unwrap();
    assert!(
        matches!(refused, crate::store::ApprovalWrite::RunEnded { .. }),
        "{refused:?}"
    );
}

#[test]
fn a_schedule_set_to_never_is_not_told_that_an_approval_is_waiting() {
    let (_dir, store, run, _worktree) = queue_folder(SIGN_OFF, "sign", &BTreeMap::new());
    let row = store.workflow_run(&run).unwrap().unwrap();
    let spec = crate::schedule::ScheduleSpec::Daily {
        time: crate::schedule::TimeOfDay { hour: 9, minute: 0 },
    }
    .to_json();
    let schedule = store
        .add_schedule(
            &crate::store::ScheduleFields {
                name: "Nightly",
                target_kind: "workflow",
                target_id: "sign",
                project_id: row.project_id.as_deref().unwrap(),
                workspace_id: row.workspace_id.as_deref().unwrap(),
                workspace_name: &row.workspace_name,
                inputs: "{}",
                spec: &spec,
                missed: "run_once",
                notify: "never",
                next_at: Some(1),
            },
            "cli",
        )
        .unwrap();
    store
        .record_schedule_run(&crate::store::NewScheduleRun {
            schedule_id: &schedule.id,
            due_at: 1,
            fired_at: 1,
            outcome: "started",
            manual: false,
            run_id: Some(&run),
            note: None,
        })
        .unwrap();
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "sign-off").status, "waiting");
    assert!(
        !hands
            .sent
            .lock()
            .unwrap()
            .iter()
            .any(|title| title == "Waiting for your approval"),
        "notify never stays quiet: {:?}",
        hands.sent.lock().unwrap()
    );
}

#[test]
fn a_capture_that_fails_after_a_restart_drops_the_pending_marker() {
    let (_dir, store, run, _worktree) = queue_folder(FILE_HANDOFF, "hand", &BTreeMap::new());
    store
        .replace_step_artifacts(
            &run,
            "draft",
            &[crate::store::NewWorkflowArtifact {
                name: "spec".into(),
                source: "file".into(),
                detail: Some("SPEC.md".into()),
                body: "v1".into(),
                truncated: false,
            }],
        )
        .unwrap();
    assert!(store.start_workflow_run(&run, None).unwrap());
    assert!(store
        .move_workflow_step(
            &run,
            "draft",
            &[crate::store::StepStatus::Pending],
            crate::store::StepStatus::Running,
            Some(r#"{"artifact_pending":"1"}"#),
            Some("Told."),
            None,
            false,
        )
        .unwrap());
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    let draft = step(&store, &run, "draft");
    assert_eq!(draft.status, "failed");
    assert!(
        !draft.outputs.contains("artifact_pending"),
        "{}",
        draft.outputs
    );
    assert_eq!(store.workflow_artifacts(&run).unwrap()[0].body, "v1");
    assert_eq!(step(&store, &run, "plan").status, "skipped");
}

/// Returns Later once, then the digest, so a slow transcript is retried instead of failed.
struct SlowTalk {
    calls: Mutex<u32>,
}

impl Hands for SlowTalk {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        Ok(SessionInfo {
            id: pty_host::SessionId("pty-9".into()),
            program: "claude".into(),
            args: Vec::new(),
            cwd: None,
            pid: None,
            size: pty_host::TermSize::DEFAULT,
            labels: BTreeMap::new(),
            state: pty_host::SessionState::Running,
            has_output: false,
            busy: false,
            busy_ms: 0,
            idle_ms: 0,
        })
    }

    fn notify(&self, _: &str, _: Option<&str>) -> Result<(), String> {
        Ok(())
    }

    fn handoff(&self, _: &str) -> Option<String> {
        None
    }

    fn pr_posts(&self, _: &str, _: &crate::workflow::runs::PrFacts) -> PrAsk {
        unreachable!("this workflow waits on no pull request")
    }

    fn conversation(
        &self,
        _: &str,
        _: &str,
        _: bool,
        _: &str,
        _: &str,
    ) -> Result<Conversation, String> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        if *calls == 1 {
            Ok(Conversation::Later)
        } else {
            Ok(Conversation::Ready(Some("DIGEST".into())))
        }
    }
}

#[test]
fn a_transcript_that_is_not_ready_is_read_again_on_the_next_tick() {
    let yaml = r#"id: talk
name: Talk
version: 1
trigger:
  kind: manual
steps:
  - id: start
    action: start_session
    harness: claude
    prompt: Hello
  - id: note
    action: notify
    needs: [start]
    title: Noted
    produces:
      - name: discussion
        from: transcript
        session: "{{ steps.start.session }}"
"#;
    let (_dir, store, run, _worktree) = queue_folder(yaml, "talk", &BTreeMap::new());
    let hands = SlowTalk {
        calls: Mutex::new(0),
    };
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "note").status, "waiting");
    assert!(
        step(&store, &run, "note")
            .outputs
            .contains("artifact_pending"),
        "the read is resumed, not started as a new action: {}",
        step(&store, &run, "note").outputs
    );
    assert_eq!(store.workflow_run(&run).unwrap().unwrap().status, "running");
    assert_eq!(*hands.calls.lock().unwrap(), 1);
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "note").status, "succeeded");
    assert_eq!(store.workflow_artifacts(&run).unwrap()[0].body, "DIGEST");
    assert!(*hands.calls.lock().unwrap() >= 2);
}

fn recover_is_a_no_op(store: &Store, run: &str) -> bool {
    crate::workflow::engine::recover(&store.workflow_steps(run).unwrap()).is_empty()
}

fn notice_count(hands: &Filed, title: &str) -> usize {
    hands
        .sent
        .lock()
        .unwrap()
        .iter()
        .filter(|sent| sent.as_str() == title)
        .count()
}

fn tick_until(store: &Store, hands: &dyn Hands, when: StopAfter) {
    let host = PtyHost::new(Arc::new(|_| {}));
    let driver = Driver::default();
    driver.testing_stop_after(when);
    driver.tick(store, &host, hands, &git()).unwrap();
}

/// A fresh process, with no code in memory, must not approve a gate or start the next step.
#[test]
fn a_restart_while_an_approval_is_waiting_does_not_skip_the_gate() {
    let (_dir, store, run, _worktree) = queue_folder(SIGN_OFF, "sign", &BTreeMap::new());
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "sign-off").status, "waiting");
    assert_eq!(notice_count(&hands, "Waiting for your approval"), 1);
    let open = crate::workflow::waits::list_open(&store).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(
        open[0].blocked_line(),
        "Blocked: waiting for your approval."
    );
    assert_eq!(
        open[0].action_needed,
        "Approve, reject, or request changes."
    );
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "sign-off").status, "waiting");
    assert_eq!(step(&store, &run, "next").status, "pending");
    assert_eq!(store.workflow_run(&run).unwrap().unwrap().status, "running");
    assert_eq!(notice_count(&hands, "Waiting for your approval"), 1);
    assert_eq!(notice_count(&hands, "Approval code replaced"), 1);
    assert_eq!(notice_count(&hands, "Go"), 0);
    assert!(step(&store, &run, "sign-off").outcome.is_none());
}

/// A decision written while nothing was ticking is applied once, and the next effect once.
#[test]
fn a_decision_recorded_while_the_app_was_closed_is_applied_once() {
    let (_dir, store, run, _worktree) = queue_folder(SIGN_OFF, "sign", &BTreeMap::new());
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    assert_eq!(
        store
            .record_workflow_approval(&run, "sign-off", "approved", Some("ship it"), "ada", None)
            .unwrap(),
        crate::store::ApprovalWrite::Recorded
    );
    assert_eq!(step(&store, &run, "sign-off").status, "waiting");
    assert!(crate::workflow::waits::list_open(&store)
        .unwrap()
        .is_empty());
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "sign-off").status, "succeeded");
    assert_eq!(step(&store, &run, "next").status, "succeeded");
    assert_eq!(notice_count(&hands, "Go"), 1);
    assert_eq!(notice_count(&hands, "Waiting for your approval"), 1);
    assert_eq!(notice_count(&hands, "Approval code replaced"), 0);
    tick(&store, &hands);
    assert_eq!(notice_count(&hands, "Go"), 1);
    let outcome: serde_json::Value =
        serde_json::from_str(step(&store, &run, "sign-off").outcome.unwrap().as_str()).unwrap();
    assert_eq!(outcome["decision"], "approved");
    assert_eq!(outcome["actor"], "ada");
}

#[test]
fn a_crash_after_the_decision_is_applied_does_not_start_the_next_effect() {
    let (_dir, store, run, _worktree) = queue_folder(SIGN_OFF, "sign", &BTreeMap::new());
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    store
        .record_workflow_approval(&run, "sign-off", "approved", Some("ship it"), "ada", None)
        .unwrap();
    tick_until(&store, &hands, StopAfter::AfterDecisionApplied);
    assert_eq!(step(&store, &run, "sign-off").status, "succeeded");
    assert_eq!(step(&store, &run, "next").status, "pending");
    assert!(store
        .workflow_wait(&run, "sign-off")
        .unwrap()
        .unwrap()
        .applied_at
        .is_some());
    assert_eq!(notice_count(&hands, "Go"), 0);
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "next").status, "succeeded");
    assert_eq!(notice_count(&hands, "Go"), 1);
    tick(&store, &hands);
    assert_eq!(notice_count(&hands, "Go"), 1);
    assert_eq!(
        store
            .workflow_artifacts(&run)
            .unwrap()
            .iter()
            .filter(|row| row.name == "plan")
            .count(),
        1
    );
}

#[test]
fn a_decision_and_its_artifact_are_one_change_and_a_restart_does_not_write_it_again() {
    let (_dir, store, run, _worktree) = queue_folder(SIGN_OFF, "sign", &BTreeMap::new());
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    store
        .record_workflow_approval(
            &run,
            "sign-off",
            "changes_requested",
            Some("split the migration"),
            "ada",
            None,
        )
        .unwrap();
    tick_until(&store, &hands, StopAfter::AfterDecisionApplied);
    assert_eq!(step(&store, &run, "sign-off").status, "failed");
    assert_eq!(step(&store, &run, "next").status, "pending");
    let saved = store.workflow_artifacts(&run).unwrap();
    let changes: Vec<_> = saved
        .iter()
        .filter(|row| row.name == "changes-sign-off")
        .collect();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].body, "split the migration");
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "next").status, "skipped");
    assert_eq!(store.workflow_run(&run).unwrap().unwrap().status, "failed");
    assert_eq!(notice_count(&hands, "Go"), 0);
    tick(&store, &hands);
    assert_eq!(
        store
            .workflow_artifacts(&run)
            .unwrap()
            .iter()
            .filter(|row| row.name == "changes-sign-off")
            .count(),
        1
    );
    assert_eq!(notice_count(&hands, "Go"), 0);
}

#[test]
fn a_crash_after_the_next_step_is_claimed_does_not_fire_it() {
    let (_dir, store, run, _worktree) = queue_folder(SIGN_OFF, "sign", &BTreeMap::new());
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    store
        .record_workflow_approval(&run, "sign-off", "approved", Some("ship it"), "ada", None)
        .unwrap();
    tick_until(&store, &hands, StopAfter::AfterNextClaim);
    assert_eq!(step(&store, &run, "next").status, "running");
    assert_eq!(notice_count(&hands, "Go"), 0);
    let outcome: serde_json::Value =
        serde_json::from_str(step(&store, &run, "sign-off").outcome.unwrap().as_str()).unwrap();
    assert_eq!(outcome["decision"], "approved");
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "next").status, "failed");
    assert_eq!(
        step(&store, &run, "next").note.as_deref(),
        Some("Yardsort stopped while this step was running.")
    );
    assert_eq!(notice_count(&hands, "Go"), 0);
    let kept: serde_json::Value =
        serde_json::from_str(step(&store, &run, "sign-off").outcome.unwrap().as_str()).unwrap();
    assert_eq!(kept["decision"], "approved");
    assert_eq!(
        store
            .workflow_artifacts(&run)
            .unwrap()
            .iter()
            .filter(|row| row.name == "plan")
            .count(),
        1
    );
    tick(&store, &hands);
    assert_eq!(notice_count(&hands, "Go"), 0);
}

/// Remembers the notification body and whether the code was offered.
struct CodeNotice {
    sent: Mutex<Vec<(String, String)>>,
    code: Mutex<Option<String>>,
    show: bool,
}

impl Hands for CodeNotice {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        unreachable!("this workflow starts no agent")
    }

    fn notify(&self, title: &str, body: Option<&str>) -> Result<(), String> {
        self.sent
            .lock()
            .unwrap()
            .push((title.to_owned(), body.unwrap_or("").to_owned()));
        Ok(())
    }

    fn handoff(&self, _: &str) -> Option<String> {
        None
    }

    fn pr_posts(&self, _: &str, _: &crate::workflow::runs::PrFacts) -> PrAsk {
        unreachable!("this workflow waits on no pull request")
    }

    fn include_approval_code(&self) -> bool {
        self.show
    }

    fn approval_code(&self, _: &str, _: &str, code: &str) {
        *self.code.lock().unwrap() = Some(code.to_owned());
    }
}

#[test]
fn the_waiting_notice_can_leave_the_code_out() {
    let (_dir, store, _run, _worktree) = queue_folder(SIGN_OFF, "sign", &BTreeMap::new());
    let hidden = CodeNotice {
        sent: Mutex::new(Vec::new()),
        code: Mutex::new(None),
        show: false,
    };
    tick(&store, &hidden);
    let body = hidden
        .sent
        .lock()
        .unwrap()
        .iter()
        .find(|notice| notice.0 == "Waiting for your approval")
        .expect("told")
        .1
        .clone();
    let code = hidden
        .code
        .lock()
        .unwrap()
        .clone()
        .expect("a code was issued");
    assert!(body.contains("Open Yardsort for the code."));
    assert!(!body.contains(&code));

    let (_dir, store, _run, _worktree) = queue_folder(SIGN_OFF, "sign", &BTreeMap::new());
    let shown = CodeNotice {
        sent: Mutex::new(Vec::new()),
        code: Mutex::new(None),
        show: true,
    };
    tick(&store, &shown);
    let body = shown
        .sent
        .lock()
        .unwrap()
        .iter()
        .find(|notice| notice.0 == "Waiting for your approval")
        .expect("told")
        .1
        .clone();
    let code = shown
        .code
        .lock()
        .unwrap()
        .clone()
        .expect("a code was issued");
    assert!(body.contains(&format!("Your code is {code}.")));
    assert!(!body.contains("Open Yardsort for the code."));
}

/// Remembers every one-time code the driver issues, in order.
struct Codes {
    sent: Mutex<Vec<String>>,
    codes: Mutex<Vec<String>>,
}

impl Hands for Codes {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        unreachable!("this workflow starts no agent")
    }

    fn notify(&self, title: &str, _: Option<&str>) -> Result<(), String> {
        self.sent.lock().unwrap().push(title.to_owned());
        Ok(())
    }

    fn handoff(&self, _: &str) -> Option<String> {
        None
    }

    fn pr_posts(&self, _: &str, _: &crate::workflow::runs::PrFacts) -> PrAsk {
        unreachable!("this workflow waits on no pull request")
    }

    fn approval_code(&self, _: &str, _: &str, code: &str) {
        self.codes.lock().unwrap().push(code.to_owned());
    }
}

const LOOP_APPROVAL: &str = r#"id: loop
name: Loop
version: 1
trigger:
  kind: manual
steps:
  - id: plan
    action: notify
    title: Plan
  - id: sign-off
    needs: [plan]
    action: approve
    prompt: Look again.
    on:
      approved: done
      rejected: stop
      changes_requested: plan
    max_attempts: 3
  - id: done
    needs: [sign-off]
    action: notify
    title: Done
"#;

#[test]
fn a_loop_through_an_approval_needs_a_new_code_and_applies_the_rewind_once() {
    let (_dir, store, run, _worktree) = queue_folder(LOOP_APPROVAL, "loop", &BTreeMap::new());
    let hands = Codes {
        sent: Mutex::new(Vec::new()),
        codes: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "sign-off").status, "waiting");
    assert_eq!(step(&store, &run, "sign-off").attempts, 1);
    let first = hands.codes.lock().unwrap()[0].clone();
    assert_eq!(
        crate::workflow::waits::decide_with_code(
            &store,
            &run,
            "sign-off",
            "changes_requested",
            Some("split the migration"),
            "ada",
            &first,
        )
        .unwrap(),
        crate::store::ApprovalWrite::Recorded
    );

    // The plan exists and the decision is stored. The process dies before the rewind is applied.
    tick_until(&store, &hands, StopAfter::BeforeRewindApplied);
    assert_eq!(step(&store, &run, "sign-off").status, "waiting");
    assert_eq!(step(&store, &run, "plan").status, "succeeded");
    assert_eq!(step(&store, &run, "sign-off").attempts, 1);
    assert!(store.workflow_approval(&run, "sign-off").unwrap().is_some());
    assert!(store.workflow_artifacts(&run).unwrap().is_empty());

    tick(&store, &hands);
    assert_eq!(step(&store, &run, "plan").status, "pending");
    assert_eq!(step(&store, &run, "sign-off").status, "pending");
    assert_eq!(step(&store, &run, "sign-off").attempts, 1);
    assert!(store.workflow_approval(&run, "sign-off").unwrap().is_none());
    let saved = store.workflow_artifacts(&run).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "changes-sign-off");
    assert_eq!(saved[0].body, "split the migration");
    let history = store.archived_approvals(&run, "sign-off").unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].decision, "changes_requested");
    assert_eq!(history[0].actor, "ada");
    assert_eq!(history[0].feedback.as_deref(), Some("split the migration"));

    tick(&store, &hands);
    assert_eq!(step(&store, &run, "plan").status, "succeeded");
    assert_eq!(step(&store, &run, "sign-off").status, "waiting");
    assert_eq!(step(&store, &run, "sign-off").attempts, 2);
    assert_eq!(hands.codes.lock().unwrap().len(), 2);
    let second = hands.codes.lock().unwrap()[1].clone();
    assert_ne!(first, second);
    assert_eq!(
        store.archived_approvals(&run, "sign-off").unwrap().len(),
        1,
        "the rewind was applied again"
    );
    assert_eq!(
        crate::workflow::waits::decide_with_code(
            &store, &run, "sign-off", "approved", None, "ada", &first,
        )
        .unwrap(),
        crate::store::ApprovalWrite::WrongCode
    );
    assert_eq!(
        crate::workflow::waits::decide_with_code(
            &store, &run, "sign-off", "approved", None, "ada", &second,
        )
        .unwrap(),
        crate::store::ApprovalWrite::Recorded
    );
}

#[test]
fn a_rewind_cancels_the_smoke_test_a_parallel_step_is_still_running() {
    let yaml = r#"id: gate
name: Gate
version: 1
trigger:
  kind: manual
steps:
  - id: implement
    action: notify
    title: Build
  - id: verify
    needs: [implement]
    action: run_test
    test: demo
    on:
      passed: stop
      failed: implement
    max_attempts: 3
  - id: side
    needs: [implement]
    action: run_test
    test: other
"#;
    let (dir, store, run, _, _) = queued_workflow(yaml);
    std::fs::write(
        crate::smoke::dir(&dir.path().join("wt")).join("other.yaml"),
        OTHER_SMOKE,
    )
    .unwrap();
    let hands = Told {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    let verify_run = step_test_id(&store, &run, "verify");
    let side_run = step_test_id(&store, &run, "side");
    finish_one(&store, &verify_run, "failed", Some("the button is missing"));
    tick(&store, &hands);
    assert_eq!(step(&store, &run, "implement").status, "pending");
    assert_eq!(step(&store, &run, "side").status, "pending");
    assert_eq!(step(&store, &run, "verify").status, "pending");
    let cancelled = store.test_run(&side_run).unwrap().unwrap();
    assert_eq!(cancelled.status, "cancelled");
    assert_eq!(
        cancelled.error.as_deref(),
        Some("The workflow is running this step again.")
    );
    assert_eq!(
        store.test_run(&verify_run).unwrap().unwrap().status,
        "failed"
    );
}

const CLARIFY_YAML: &str = r#"id: clarify
name: Clarify
version: 1
trigger:
  kind: manual
steps:
  - id: start
    action: start_session
    harness: claude
    prompt: Clarify the problem.
  - id: discover
    needs: [start]
    action: wait_session
    session: "{{ steps.start.session }}"
    until: settled
    clarify: true
    max_rounds: 3
    produces:
      - name: brief
        from: brief
  - id: next
    needs: [discover]
    action: notify
    title: Next
    consumes: [brief]
    body: "{{ artifacts.brief }}"
"#;

/// A question printed by the stand-in. It is never asked, because nothing called `ys workflow ask`.
const PRINTED_QUESTION: &str = "What is the timeout for the login?";

struct QuietHost {
    pastes: Mutex<Vec<String>>,
    /// The next paste is refused because someone else holds the session.
    hold: AtomicBool,
}

impl QuietHost {
    fn new() -> Self {
        Self {
            pastes: Mutex::new(Vec::new()),
            hold: AtomicBool::new(false),
        }
    }

    fn session() -> SessionInfo {
        let mut labels = BTreeMap::new();
        labels.insert(crate::launch::RUN_LABEL.to_owned(), "activity-1".to_owned());
        SessionInfo {
            id: pty_host::SessionId("pty-9".into()),
            program: "claude".into(),
            args: Vec::new(),
            cwd: None,
            pid: Some(7),
            size: pty_host::TermSize::DEFAULT,
            labels,
            state: pty_host::SessionState::Running,
            has_output: true,
            busy: false,
            busy_ms: 0,
            idle_ms: 1_000,
        }
    }
}

impl pty_host::TerminalHost for QuietHost {
    fn spawn(&self, _: pty_host::LaunchPlan) -> pty_host::Result<SessionInfo> {
        Ok(Self::session())
    }
    fn attach(
        &self,
        _: &pty_host::SessionId,
        _: pty_host::OutputSink,
    ) -> pty_host::Result<pty_host::AttachmentId> {
        Err(pty_host::HostError::UnknownSession(pty_host::SessionId(
            "pty-9".into(),
        )))
    }
    fn detach(&self, _: &pty_host::SessionId, _: pty_host::AttachmentId) -> pty_host::Result<()> {
        Ok(())
    }
    fn write(&self, _: &pty_host::SessionId, _: &[u8]) -> pty_host::Result<()> {
        Ok(())
    }
    fn paste(&self, _: &pty_host::SessionId, text: &str) -> pty_host::Result<()> {
        self.pastes.lock().unwrap().push(text.to_owned());
        Ok(())
    }
    fn resize(&self, _: &pty_host::SessionId, _: pty_host::TermSize) -> pty_host::Result<()> {
        Ok(())
    }
    fn kill(&self, _: &pty_host::SessionId) -> pty_host::Result<()> {
        Ok(())
    }
    fn remove(&self, _: &pty_host::SessionId) -> pty_host::Result<()> {
        Ok(())
    }
    fn info(&self, id: &pty_host::SessionId) -> pty_host::Result<SessionInfo> {
        if id.0 == "pty-9" {
            Ok(Self::session())
        } else {
            Err(pty_host::HostError::UnknownSession(id.clone()))
        }
    }
    fn list(&self) -> Vec<SessionInfo> {
        vec![Self::session()]
    }
    fn type_unattended(
        &self,
        id: &pty_host::SessionId,
        text: &str,
        paste: bool,
    ) -> pty_host::Result<()> {
        if id.0 != "pty-9" {
            return Err(pty_host::HostError::UnknownSession(id.clone()));
        }
        if paste && self.hold.load(Ordering::SeqCst) {
            return Err(pty_host::HostError::ControlHeld {
                id: id.clone(),
                by: pty_host::Controller::Leased {
                    holder: "other".into(),
                },
            });
        }
        if paste {
            self.pastes.lock().unwrap().push(text.to_owned());
        }
        Ok(())
    }
}

struct Asking {
    notices: Mutex<Vec<(String, String)>>,
    codes: Mutex<Vec<String>>,
}

impl Asking {
    fn new() -> Self {
        Self {
            notices: Mutex::new(Vec::new()),
            codes: Mutex::new(Vec::new()),
        }
    }
}

impl Hands for Asking {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        Ok(QuietHost::session())
    }
    fn notify(&self, title: &str, body: Option<&str>) -> Result<(), String> {
        self.notices
            .lock()
            .unwrap()
            .push((title.to_owned(), body.unwrap_or("").to_owned()));
        Ok(())
    }
    fn handoff(&self, _: &str) -> Option<String> {
        None
    }
    fn pr_posts(&self, _: &str, _: &crate::workflow::runs::PrFacts) -> PrAsk {
        unreachable!("this workflow waits on no pull request")
    }
    fn conversation(
        &self,
        _: &str,
        _: &str,
        _: bool,
        _: &str,
        _: &str,
    ) -> Result<Conversation, String> {
        Ok(Conversation::Ready(Some(PRINTED_QUESTION.to_owned())))
    }

    fn approval_code(&self, _: &str, _: &str, code: &str) {
        self.codes.lock().unwrap().push(code.to_owned());
    }
}

fn clarify_run(yaml: &str) -> (tempfile::TempDir, Store, String) {
    let (dir, store, run, _) = queue_folder(yaml, "clarify", &BTreeMap::new());
    (dir, store, run)
}

fn quiet(driver: &Driver) {
    driver.observe(&pty_host::HostEvent::Quiet {
        id: pty_host::SessionId("pty-9".into()),
        busy_ms: 1,
    });
}

#[test]
fn a_clarification_round_pauses_and_the_same_session_receives_the_answer() {
    let (_dir, store, run) = clarify_run(CLARIFY_YAML);
    let host = QuietHost::new();
    let hands = Asking::new();
    let driver = Driver::new(1, Duration::from_secs(3600));
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(step(&store, &run, "start").status, "succeeded");
    assert_eq!(
        crate::workflow::engine::parse_outputs(&step(&store, &run, "start").outputs).get("session"),
        Some(&"pty-9".to_owned())
    );
    let located = crate::workflow::clarify::locate(&store, "activity-1", None)
        .unwrap()
        .expect("the activity run names the clarifying step");
    assert_eq!(located.step_id, "discover");
    assert_eq!(
        crate::workflow::clarify::ask(
            &store,
            &run,
            "discover",
            &["What should the timeout be?".to_owned()]
        )
        .unwrap(),
        crate::workflow::clarify::AskWrite::Recorded { round: 1 }
    );
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(step(&store, &run, "discover").status, "waiting");
    assert_eq!(
        crate::workflow::waits::waiting_on(&store, &run)
            .unwrap()
            .as_deref(),
        Some("your answer")
    );
    assert!(hands
        .notices
        .lock()
        .unwrap()
        .iter()
        .any(|(title, _)| { title == crate::workflow::waits::TITLE_ANSWER }));
    // Settled, and the terminal has the question as text. That is not a finished phase.
    quiet(&driver);
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(step(&store, &run, "discover").status, "waiting");
    assert_eq!(
        store
            .answer_workflow_wait(&run, "discover", "30 seconds", "ada", None)
            .unwrap(),
        crate::workflow::waits::AnswerWrite::Recorded
    );
    driver.tick(&store, &host, &hands, &git()).unwrap();
    let pastes = host.pastes.lock().unwrap().clone();
    assert_eq!(pastes.len(), 1, "{pastes:?}");
    assert!(pastes[0].contains("30 seconds"), "{pastes:?}");
    assert!(
        pastes[0].contains("What should the timeout be?"),
        "{pastes:?}"
    );
    assert_eq!(
        crate::workflow::engine::parse_outputs(&step(&store, &run, "start").outputs).get("session"),
        Some(&"pty-9".to_owned())
    );
    let round = store
        .clarification_rounds(&run, "discover")
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert!(round.applied_at.is_some());
    assert!(round.typed_at.is_some(), "typed only after the keystrokes");
    assert_eq!(step(&store, &run, "discover").status, "waiting");
    assert_eq!(
        crate::workflow::clarify::summary(
            &store,
            &run,
            "discover",
            "The timeout should be 30 seconds."
        )
        .unwrap(),
        crate::workflow::clarify::SummaryWrite::Recorded
    );
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(host.pastes.lock().unwrap().len(), 1);
    assert_eq!(step(&store, &run, "discover").status, "succeeded");
    let brief = store
        .workflow_artifacts(&run)
        .unwrap()
        .into_iter()
        .find(|artifact| artifact.name == "brief")
        .expect("the brief");
    assert!(brief.body.contains("30 seconds"), "{}", brief.body);
    assert!(
        brief.body.contains("What should the timeout be?"),
        "{}",
        brief.body
    );
    assert!(
        brief.body.contains("The timeout should be 30 seconds."),
        "{}",
        brief.body
    );
    assert!(!brief.body.contains(PRINTED_QUESTION), "{}", brief.body);
    assert_eq!(step(&store, &run, "next").status, "succeeded");
    assert!(hands.notices.lock().unwrap().iter().any(|(title, body)| {
        title == "Next"
            && body.contains("30 seconds")
            && body.contains("What should the timeout be?")
    }));
}

#[test]
fn a_question_under_notify_never_still_gets_a_code() {
    let (_dir, store, run) = clarify_run(CLARIFY_YAML);
    let row = store.workflow_run(&run).unwrap().unwrap();
    let spec = crate::schedule::ScheduleSpec::Daily {
        time: crate::schedule::TimeOfDay { hour: 9, minute: 0 },
    }
    .to_json();
    let schedule = store
        .add_schedule(
            &crate::store::ScheduleFields {
                name: "Nightly",
                target_kind: "workflow",
                target_id: "clarify",
                project_id: row.project_id.as_deref().unwrap(),
                workspace_id: row.workspace_id.as_deref().unwrap(),
                workspace_name: &row.workspace_name,
                inputs: "{}",
                spec: &spec,
                missed: "run_once",
                notify: "never",
                next_at: Some(1),
            },
            "cli",
        )
        .unwrap();
    store
        .record_schedule_run(&crate::store::NewScheduleRun {
            schedule_id: &schedule.id,
            due_at: 1,
            fired_at: 1,
            outcome: "started",
            manual: false,
            run_id: Some(&run),
            note: None,
        })
        .unwrap();
    let host = QuietHost::new();
    let hands = Asking::new();
    let driver = Driver::new(1, Duration::from_secs(3600));
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(
        crate::workflow::clarify::ask(
            &store,
            &run,
            "discover",
            &["What should the timeout be?".to_owned()]
        )
        .unwrap(),
        crate::workflow::clarify::AskWrite::Recorded { round: 1 }
    );
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert!(
        !hands
            .notices
            .lock()
            .unwrap()
            .iter()
            .any(|(title, _)| title == crate::workflow::waits::TITLE_ANSWER),
        "notify never stays quiet: {:?}",
        hands.notices.lock().unwrap()
    );
    assert!(store.answer_code_present(&run, "discover").unwrap());
    let code = hands
        .codes
        .lock()
        .unwrap()
        .last()
        .cloned()
        .expect("the window was given a code");
    assert_eq!(
        crate::workflow::waits::answer_with_code(
            &store,
            &run,
            "discover",
            "30 seconds",
            "ada",
            &code
        )
        .unwrap(),
        crate::workflow::waits::AnswerWrite::Recorded
    );
}

#[test]
fn a_summary_is_refused_while_a_question_is_open() {
    let (_dir, store, run) = clarify_run(CLARIFY_YAML);
    let host = QuietHost::new();
    let hands = Asking::new();
    let driver = Driver::new(1, Duration::from_secs(3600));
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(
        crate::workflow::clarify::ask(
            &store,
            &run,
            "discover",
            &["What should the timeout be?".to_owned()]
        )
        .unwrap(),
        crate::workflow::clarify::AskWrite::Recorded { round: 1 }
    );
    let written = crate::workflow::clarify::summary(
        &store,
        &run,
        "discover",
        "The timeout should be 30 seconds.",
    )
    .unwrap();
    assert_eq!(
        written,
        crate::workflow::clarify::SummaryWrite::AlreadyWaiting
    );
    let message = crate::workflow::clarify::summary_refusal(&written, &run, "discover")
        .expect("the refusal is told");
    assert!(message.contains("still waiting for an answer"), "{message}");
    assert!(store
        .clarification_summary(&run, "discover")
        .unwrap()
        .is_none());
}

#[test]
fn a_question_only_printed_in_the_session_does_not_pause_the_step() {
    let (_dir, store, run) = clarify_run(CLARIFY_YAML);
    let host = QuietHost::new();
    let hands = Asking::new();
    let driver = Driver::new(1, Duration::from_secs(3600));
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert!(store.workflow_wait(&run, "discover").unwrap().is_none());
    quiet(&driver);
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(
        step(&store, &run, "discover").status,
        "waiting",
        "a quiet session does not finish a clarifying step"
    );
    assert!(store.workflow_wait(&run, "discover").unwrap().is_none());
    assert!(host.pastes.lock().unwrap().is_empty());
    crate::workflow::clarify::summary(&store, &run, "discover", "Nothing to ask.").unwrap();
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(step(&store, &run, "discover").status, "succeeded");
    let brief = store
        .workflow_artifacts(&run)
        .unwrap()
        .into_iter()
        .find(|artifact| artifact.name == "brief")
        .unwrap();
    assert!(
        brief.body.contains("No questions were asked"),
        "{}",
        brief.body
    );
    assert!(!brief.body.contains(PRINTED_QUESTION), "{}", brief.body);
    assert!(brief.body.contains("Nothing to ask."), "{}", brief.body);
}

#[test]
fn clarification_can_take_more_than_one_round() {
    let (_dir, store, run) = clarify_run(CLARIFY_YAML);
    let host = QuietHost::new();
    let hands = Asking::new();
    let driver = Driver::new(1, Duration::from_secs(3600));
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(
        crate::workflow::clarify::ask(&store, &run, "discover", &["First?".to_owned()]).unwrap(),
        crate::workflow::clarify::AskWrite::Recorded { round: 1 }
    );
    store
        .answer_workflow_wait(&run, "discover", "one", "ada", None)
        .unwrap();
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(
        crate::workflow::clarify::ask(&store, &run, "discover", &["Second?".to_owned()]).unwrap(),
        crate::workflow::clarify::AskWrite::Recorded { round: 2 }
    );
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(
        crate::workflow::waits::waiting_on(&store, &run)
            .unwrap()
            .as_deref(),
        Some("your answer")
    );
    store
        .answer_workflow_wait(&run, "discover", "two", "ada", None)
        .unwrap();
    driver.tick(&store, &host, &hands, &git()).unwrap();
    let pastes = host.pastes.lock().unwrap().clone();
    assert_eq!(pastes.len(), 2, "{pastes:?}");
    assert!(pastes[0].contains("one") && pastes[0].contains("First?"));
    assert!(pastes[1].contains("two") && pastes[1].contains("Second?"));
    crate::workflow::clarify::summary(&store, &run, "discover", "Both answers are in.").unwrap();
    quiet(&driver);
    driver.tick(&store, &host, &hands, &git()).unwrap();
    let brief = store
        .workflow_artifacts(&run)
        .unwrap()
        .into_iter()
        .find(|artifact| artifact.name == "brief")
        .unwrap();
    assert!(brief.body.contains("First?"), "{}", brief.body);
    assert!(brief.body.contains("one"), "{}", brief.body);
    assert!(brief.body.contains("Second?"), "{}", brief.body);
    assert!(brief.body.contains("two"), "{}", brief.body);
    assert!(
        brief.body.contains("Both answers are in."),
        "{}",
        brief.body
    );
}

#[test]
fn a_clarification_past_the_round_limit_fails_the_step() {
    let yaml = CLARIFY_YAML.replace("max_rounds: 3", "max_rounds: 1");
    let (_dir, store, run) = clarify_run(&yaml);
    let host = QuietHost::new();
    let hands = Asking::new();
    let driver = Driver::new(1, Duration::from_secs(3600));
    driver.tick(&store, &host, &hands, &git()).unwrap();
    crate::workflow::clarify::ask(&store, &run, "discover", &["Only once?".to_owned()]).unwrap();
    store
        .answer_workflow_wait(&run, "discover", "yes", "ada", None)
        .unwrap();
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(
        crate::workflow::clarify::ask(&store, &run, "discover", &["Again?".to_owned()]).unwrap(),
        crate::workflow::clarify::AskWrite::TooMany { asked: 2, limit: 1 }
    );
    assert!(crate::workflow::waits::waiting_on(&store, &run)
        .unwrap()
        .is_none());
    driver.tick(&store, &host, &hands, &git()).unwrap();
    let discover = step(&store, &run, "discover");
    assert_eq!(discover.status, "failed");
    assert!(
        discover.note.unwrap().contains("past the limit (1)"),
        "the limit was not named"
    );
    assert_eq!(host.pastes.lock().unwrap().len(), 1);
}

#[test]
fn an_answer_is_typed_once_across_a_restart() {
    let (_dir, store, run) = clarify_run(CLARIFY_YAML);
    let host = QuietHost::new();
    let hands = Asking::new();
    let driver = Driver::new(1, Duration::from_secs(3600));
    driver.tick(&store, &host, &hands, &git()).unwrap();
    crate::workflow::clarify::ask(&store, &run, "discover", &["Which name?".to_owned()]).unwrap();
    store
        .answer_workflow_wait(&run, "discover", "Ship it", "ada", None)
        .unwrap();

    driver.testing_stop_after(StopAfter::BeforeAnswerApplied);
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert!(host.pastes.lock().unwrap().is_empty());
    assert!(store.clarification_rounds(&run, "discover").unwrap()[0]
        .applied_at
        .is_none());

    let again = Driver::new(1, Duration::from_secs(3600));
    again.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(host.pastes.lock().unwrap().len(), 1);
    assert!(host.pastes.lock().unwrap()[0].contains("Ship it"));
    let rounds = store.clarification_rounds(&run, "discover").unwrap();
    let applied = rounds[0].applied_at.expect("the answer was claimed");
    assert!(rounds[0].typed_at.is_some());
    let restarted = Driver::new(1, Duration::from_secs(3600));
    restarted.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(host.pastes.lock().unwrap().len(), 1);
    assert_eq!(
        store.clarification_rounds(&run, "discover").unwrap()[0].applied_at,
        Some(applied)
    );

    let (_dir, store, run) = clarify_run(CLARIFY_YAML);
    let host = QuietHost::new();
    let driver = Driver::new(1, Duration::from_secs(3600));
    driver.tick(&store, &host, &hands, &git()).unwrap();
    crate::workflow::clarify::ask(&store, &run, "discover", &["Which name?".to_owned()]).unwrap();
    store
        .answer_workflow_wait(&run, "discover", "Keep it", "ada", None)
        .unwrap();
    driver.testing_stop_after(StopAfter::AfterAnswerApplied);
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert!(
        host.pastes.lock().unwrap().is_empty(),
        "the process died after the claim and before the keystrokes"
    );
    let claimed = store.clarification_rounds(&run, "discover").unwrap();
    assert!(claimed[0].applied_at.is_some());
    assert!(claimed[0].typed_at.is_none());
    let resumed = Driver::new(1, Duration::from_secs(3600));
    resumed.tick(&store, &host, &hands, &git()).unwrap();
    assert!(
        host.pastes.lock().unwrap().is_empty(),
        "a restart after the claim does not type the answer"
    );
    let discover = step(&store, &run, "discover");
    assert_eq!(discover.status, "failed");
    assert!(
        discover
            .note
            .unwrap()
            .contains("may not have reached the session"),
        "the step says the answer may not have arrived"
    );
}

#[test]
fn a_busy_session_releases_the_claim_and_the_next_tick_types_the_answer() {
    let (_dir, store, run) = clarify_run(CLARIFY_YAML);
    let host = QuietHost::new();
    host.hold.store(true, Ordering::SeqCst);
    let hands = Asking::new();
    let driver = Driver::new(1, Duration::from_secs(3600));
    driver.tick(&store, &host, &hands, &git()).unwrap();
    crate::workflow::clarify::ask(&store, &run, "discover", &["Which name?".to_owned()]).unwrap();
    store
        .answer_workflow_wait(&run, "discover", "Hold on", "ada", None)
        .unwrap();
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert!(
        host.pastes.lock().unwrap().is_empty(),
        "a held session is not typed into"
    );
    let round = store.clarification_rounds(&run, "discover").unwrap();
    assert!(round[0].applied_at.is_none(), "the claim was rolled back");
    assert!(round[0].typed_at.is_none());
    assert_eq!(step(&store, &run, "discover").status, "waiting");
    host.hold.store(false, Ordering::SeqCst);
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert_eq!(host.pastes.lock().unwrap().len(), 1);
    assert!(host.pastes.lock().unwrap()[0].contains("Hold on"));
    let round = store.clarification_rounds(&run, "discover").unwrap();
    assert!(round[0].applied_at.is_some());
    assert!(round[0].typed_at.is_some());
}

#[test]
fn a_cancelled_or_halted_run_is_not_typed_an_answer() {
    let (_dir, store, run) = clarify_run(CLARIFY_YAML);
    let host = QuietHost::new();
    let hands = Asking::new();
    let driver = Driver::new(1, Duration::from_secs(3600));
    driver.tick(&store, &host, &hands, &git()).unwrap();
    crate::workflow::clarify::ask(&store, &run, "discover", &["Which name?".to_owned()]).unwrap();
    store
        .answer_workflow_wait(&run, "discover", "Too late", "ada", None)
        .unwrap();
    assert!(store
        .cancel_workflow_run(&run, "Cancelled from ys.")
        .unwrap());
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert!(host.pastes.lock().unwrap().is_empty());
    assert!(!store
        .claim_clarification_answer(&run, "discover", 1, "note")
        .unwrap());

    let (_dir, store, run) = clarify_run(CLARIFY_YAML);
    let host = QuietHost::new();
    let driver = Driver::new(1, Duration::from_secs(3600));
    driver.tick(&store, &host, &hands, &git()).unwrap();
    crate::workflow::clarify::ask(&store, &run, "discover", &["Which name?".to_owned()]).unwrap();
    store
        .answer_workflow_wait(&run, "discover", "Stopped", "ada", None)
        .unwrap();
    assert!(store
        .finish_workflow_run(&run, "failed", Some("a gate failed"))
        .unwrap());
    driver.tick(&store, &host, &hands, &git()).unwrap();
    assert!(host.pastes.lock().unwrap().is_empty());
    assert!(!store
        .claim_clarification_answer(&run, "discover", 1, "note")
        .unwrap());
}

/// Waits for a review, then names the pull request. Queued before the branch has one.
const FIND_THE_PULL_REQUEST: &str = r##"id: find-pr
name: Find the pull request
version: 1
trigger:
  kind: manual
steps:
  - id: posted
    action: wait_pr_activity
    kind: review
  - id: tell
    action: notify
    needs: [posted]
    title: "#{{ pr.number }}"
"##;

/// Names the pull request in the first step, so a missing one fails that step.
const NAME_THE_PULL_REQUEST: &str = r##"id: name-pr
name: Name the pull request
version: 1
trigger:
  kind: manual
steps:
  - id: say
    action: notify
    title: "#{{ pr.number }}"
"##;

/// The answer `open_pull_request` gives, changed by the test once the wait is known to be holding.
struct LatePr {
    answer: Mutex<Option<crate::workflow::runs::PrFacts>>,
    posts: Mutex<Vec<PrPost>>,
    notified: Mutex<Vec<String>>,
}

impl Hands for LatePr {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        unreachable!("these workflows start no agent")
    }

    fn notify(&self, title: &str, _body: Option<&str>) -> Result<(), String> {
        self.notified.lock().unwrap().push(title.to_owned());
        Ok(())
    }

    fn handoff(&self, _: &str) -> Option<String> {
        None
    }

    fn pr_posts(&self, _: &str, pr: &crate::workflow::runs::PrFacts) -> PrAsk {
        assert_eq!(pr.number, 9);
        let posts = self.posts.lock().unwrap().clone();
        Box::new(move || Ok(posts))
    }

    fn open_pull_request(&self, _: &str) -> OpenPr {
        let found = self.answer.lock().unwrap().clone();
        Box::new(move || Ok(found))
    }
}

fn queue_without_pull_request(profile: &Profile, workflow: &str) -> String {
    queue(
        &profile.store,
        profile.dir.path(),
        &Request {
            workflow_id: workflow,
            workspace_id: &profile.workspace,
            inputs: &BTreeMap::new(),
            requested_by: "cli",
            unattended: false,
            origin_record: None,
            depth: 0,
            event: None,
            tracked: None,
        },
        &crate::workflow::runs::Accepting::default(),
    )
    .unwrap()
}

#[test]
fn a_run_queued_without_a_pull_request_waits_then_follows_the_one_it_finds() {
    let profile = Profile::new(&[("find-pr", FIND_THE_PULL_REQUEST)]);
    let run = queue_without_pull_request(&profile, "find-pr");
    let hands = LatePr {
        answer: Mutex::new(None),
        posts: Mutex::new(Vec::new()),
        notified: Mutex::new(Vec::new()),
    };
    let driver = Driver::new(1, Duration::ZERO);
    profile.until(&driver, &hands, || {
        profile.step(&run, "posted").status == "waiting"
    });
    let host = PtyHost::new(Arc::new(|_| {}));
    driver.tick(&profile.store, &host, &hands, &git()).unwrap();
    // That tick may have a lookup in flight. Let it land before the answer changes, or a late
    // "none" can overwrite the pull request the next question finds.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(profile.step(&run, "posted").status, "waiting");
    assert!(
        crate::workflow::runs::Facts::parse(
            &profile.store.workflow_run(&run).unwrap().unwrap().context
        )
        .pr
        .is_none(),
        "the wait must not invent a pull request"
    );

    *hands.answer.lock().unwrap() = Some(crate::workflow::runs::PrFacts {
        number: 9,
        url: "https://github.com/o/r/pull/9".into(),
        title: "Add a logout button".into(),
        elsewhere: None,
    });
    *hands.posts.lock().unwrap() = vec![post(
        crate::forge::PrPostKind::Review,
        9_000_000_000_000,
        "https://github.com/o/r/pull/9#pullrequestreview-1",
    )];
    profile.until(&driver, &hands, || profile.status(&run) == "succeeded");
    assert_eq!(hands.notified.lock().unwrap().as_slice(), ["#9".to_owned()]);
    let facts = crate::workflow::runs::Facts::parse(
        &profile.store.workflow_run(&run).unwrap().unwrap().context,
    );
    assert_eq!(facts.pr.as_ref().map(|pr| pr.number), Some(9));
}

#[test]
fn a_step_that_names_the_pull_request_with_nothing_waiting_before_it_is_not_queued() {
    let profile = Profile::new(&[("name-pr", NAME_THE_PULL_REQUEST)]);
    let error = queue(
        &profile.store,
        profile.dir.path(),
        &Request {
            workflow_id: "name-pr",
            workspace_id: &profile.workspace,
            inputs: &BTreeMap::new(),
            requested_by: "cli",
            unattended: false,
            origin_record: None,
            depth: 0,
            event: None,
            tracked: None,
        },
        &crate::workflow::runs::Accepting::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, "pull_request_missing");
    assert!(
        error.message.contains("Open one first"),
        "{}",
        error.message
    );
    assert!(profile.store.workflow_runs(None, 10).unwrap().is_empty());
}

/// Counts lookups that fail, so a second tick can be seen asking again.
struct FailingLookup {
    asked: std::sync::atomic::AtomicU32,
}

impl Hands for FailingLookup {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        unreachable!("this workflow starts no agent")
    }

    fn notify(&self, _: &str, _: Option<&str>) -> Result<(), String> {
        Ok(())
    }

    fn handoff(&self, _: &str) -> Option<String> {
        None
    }

    fn pr_posts(&self, _: &str, _: &crate::workflow::runs::PrFacts) -> PrAsk {
        Box::new(|| Ok(Vec::new()))
    }

    fn open_pull_request(&self, _: &str) -> OpenPr {
        self.asked.fetch_add(1, Ordering::Relaxed);
        Box::new(|| Err("gh is not logged in.".into()))
    }
}

#[test]
fn a_failed_pull_request_lookup_is_not_retried_on_the_next_tick() {
    let profile = Profile::new(&[("find-pr", FIND_THE_PULL_REQUEST)]);
    queue_without_pull_request(&profile, "find-pr");
    let hands = FailingLookup {
        asked: std::sync::atomic::AtomicU32::new(0),
    };
    let driver = Driver::new(1, Duration::from_secs(3600));
    let host = PtyHost::new(Arc::new(|_| {}));
    driver.tick(&profile.store, &host, &hands, &git()).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(hands.asked.load(Ordering::Relaxed), 1);
    driver.tick(&profile.store, &host, &hands, &git()).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        hands.asked.load(Ordering::Relaxed),
        1,
        "a failed lookup was asked again before the interval"
    );
}

/// Writes a note the following wait captures. A stale file must not survive the start.
const REWRITE_NOTES: &str = r#"id: notes
name: Notes
version: 1
trigger:
  kind: manual
steps:
  - id: write
    action: start_session
    harness: claude
    prompt: write the plan
  - id: saved
    needs: [write]
    action: wait_session
    session: "{{ steps.write.session }}"
    until: settled
    produces:
      - name: plan
        path: factory/plan.md
      - name: source
        path: src.md
"#;

struct Starting {
    notified: Mutex<Vec<String>>,
}

impl Hands for Starting {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        Ok(QuietHost::session())
    }

    fn notify(&self, title: &str, _: Option<&str>) -> Result<(), String> {
        self.notified.lock().unwrap().push(title.to_owned());
        Ok(())
    }

    fn handoff(&self, _: &str) -> Option<String> {
        None
    }

    fn pr_posts(&self, _: &str, _: &crate::workflow::runs::PrFacts) -> PrAsk {
        Box::new(|| Ok(Vec::new()))
    }
}

#[test]
fn starting_a_session_clears_an_untracked_factory_note_and_not_the_shared_exclude() {
    let scratch = tempfile::tempdir().unwrap();
    let repo = scratch.path().join("main");
    let linked = scratch.path().join("linked");
    std::fs::create_dir(&repo).unwrap();
    let git = git();
    git.run(&repo, &["init"]).unwrap();
    std::fs::write(repo.join("src.md"), "the project's file").unwrap();
    git.run(&repo, &["add", "src.md"]).unwrap();
    git.run(&repo, &["commit", "-m", "init"]).unwrap();
    git.run(
        &repo,
        &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
    )
    .unwrap();
    std::fs::create_dir_all(linked.join("factory")).unwrap();
    std::fs::write(linked.join("factory/plan.md"), "stale plan").unwrap();

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
    let project = store.add_project("app", "/code/app").unwrap();
    let workspace = store
        .add_worktree(
            &project.id,
            "fix-login",
            linked.to_str().unwrap(),
            Some("feature"),
            None,
        )
        .unwrap();
    let folder = crate::workflow::user_dir(dir.path());
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("notes.yaml"), REWRITE_NOTES).unwrap();
    queue(
        &store,
        dir.path(),
        &Request {
            workflow_id: "notes",
            workspace_id: &workspace.id,
            inputs: &BTreeMap::new(),
            requested_by: "cli",
            unattended: false,
            origin_record: None,
            depth: 0,
            event: None,
            tracked: None,
        },
        &crate::workflow::runs::Accepting::default(),
    )
    .unwrap();

    let host = PtyHost::new(Arc::new(|_| {}));
    let hands = Starting {
        notified: Mutex::new(Vec::new()),
    };
    let driver = Driver::default();
    driver.tick(&store, &host, &hands, &git).unwrap();
    assert!(
        !linked.join("factory/plan.md").exists(),
        "the stale plan was still there when the session started"
    );
    assert_eq!(
        std::fs::read_to_string(linked.join("src.md")).unwrap(),
        "the project's file",
        "a tracked file was deleted with the notes"
    );
    let gitignore = std::fs::read_to_string(linked.join("factory/.gitignore")).unwrap();
    assert!(
        gitignore.lines().any(|line| line.trim() == "*"),
        "factory/.gitignore did not ignore the notes: {gitignore}"
    );
    assert!(
        git.run(&linked, &["check-ignore", "factory/plan.md"])
            .is_ok(),
        "the linked worktree does not ignore factory/"
    );
    assert!(
        git.run(&repo, &["check-ignore", "factory/plan.md"])
            .is_err(),
        "the main checkout ignores factory/ too"
    );
    let exclude = git
        .run(
            &repo,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "info/exclude",
            ],
        )
        .unwrap();
    let shared = std::fs::read_to_string(exclude.trim()).unwrap_or_default();
    assert!(
        !shared.lines().any(|line| line.trim() == "factory/"),
        "the shared exclude lists factory/: {shared}"
    );

    // The next tick must not write the gitignore again.
    std::fs::remove_file(linked.join("factory/.gitignore")).unwrap();
    driver.tick(&store, &host, &hands, &git).unwrap();
    assert!(
        !linked.join("factory/.gitignore").exists(),
        "a second tick wrote factory/.gitignore again"
    );
}

/// A committed note under `factory/` is the project's. Clearing a rewrite must leave it.
const REWRITE_TRACKED: &str = r#"id: notes
name: Notes
version: 1
trigger:
  kind: manual
steps:
  - id: write
    action: start_session
    harness: claude
    prompt: write the plan
  - id: saved
    needs: [write]
    action: wait_session
    session: "{{ steps.write.session }}"
    until: settled
    produces:
      - name: kept
        path: factory/kept.md
"#;

#[test]
fn a_tracked_file_under_factory_survives_the_rewrite_clear() {
    let repo = tempfile::tempdir().unwrap();
    let git = git();
    git.run(repo.path(), &["init"]).unwrap();
    std::fs::create_dir_all(repo.path().join("factory")).unwrap();
    std::fs::write(repo.path().join("factory/kept.md"), "committed note").unwrap();
    git.run(repo.path(), &["add", "factory/kept.md"]).unwrap();
    git.run(repo.path(), &["commit", "-m", "keep the note"])
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
    let project = store.add_project("app", "/code/app").unwrap();
    let workspace = store
        .add_worktree(
            &project.id,
            "fix-login",
            repo.path().to_str().unwrap(),
            Some("main"),
            None,
        )
        .unwrap();
    let folder = crate::workflow::user_dir(dir.path());
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("notes.yaml"), REWRITE_TRACKED).unwrap();
    queue(
        &store,
        dir.path(),
        &Request {
            workflow_id: "notes",
            workspace_id: &workspace.id,
            inputs: &BTreeMap::new(),
            requested_by: "cli",
            unattended: false,
            origin_record: None,
            depth: 0,
            event: None,
            tracked: None,
        },
        &crate::workflow::runs::Accepting::default(),
    )
    .unwrap();

    let host = PtyHost::new(Arc::new(|_| {}));
    let hands = Starting {
        notified: Mutex::new(Vec::new()),
    };
    Driver::default().tick(&store, &host, &hands, &git).unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.path().join("factory/kept.md")).unwrap(),
        "committed note",
        "a tracked file under factory/ was deleted"
    );
}

/// A step that succeeds, a test that gates, and a step after it: what a failed run looks like
/// when it is taken up again.
const HELLO_THEN_GATE: &str = r#"id: gate
name: Gate
version: 1
trigger:
  kind: manual
steps:
  - id: hello
    action: notify
    title: Starting
  - id: run
    action: run_test
    needs: [hello]
    test: demo
    gate: true
    timeout: 30m
  - id: tell
    action: notify
    needs: [run]
    title: Tests passed
"#;

#[test]
fn a_failed_run_continues_from_the_step_that_failed_and_keeps_the_steps_before_it() {
    let (_dir, store, run, _, _) = queued_workflow(HELLO_THEN_GATE);
    let hands = Told {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    let first_test = step_test_id(&store, &run, "run");
    finish_smoke(&store, "failed", Some("wanted 200, got 500"));
    tick(&store, &hands);
    let failed = store.workflow_run(&run).unwrap().unwrap();
    assert_eq!(failed.status, "failed");
    assert!(failed.error.is_some());
    assert_eq!(step(&store, &run, "run").status, "failed");
    assert_eq!(step(&store, &run, "tell").status, "skipped");

    let again = crate::workflow::runs::continue_failed(&store, &run).unwrap();
    assert_eq!(again, ["run", "tell"]);
    let going = store.workflow_run(&run).unwrap().unwrap();
    assert_eq!(going.status, "running");
    assert_eq!(going.error, None);
    assert_eq!(going.ended_at, None);
    assert_eq!(step(&store, &run, "hello").status, "succeeded");
    let reset = step(&store, &run, "run");
    assert_eq!(reset.status, "pending");
    assert_eq!(reset.note, None);
    assert_eq!(reset.outcome, None, "the failed verdict does not stand");

    tick(&store, &hands);
    let second_test = step_test_id(&store, &run, "run");
    assert_ne!(
        second_test, first_test,
        "the test is run again, not read again"
    );
    finish_smoke(&store, "passed", None);
    tick(&store, &hands);

    assert_eq!(
        store.workflow_run(&run).unwrap().unwrap().status,
        "succeeded"
    );
    let titles: Vec<String> = hands
        .sent
        .lock()
        .unwrap()
        .iter()
        .map(|(title, _)| title.clone())
        .collect();
    assert_eq!(
        titles,
        ["Starting", "Tests passed"],
        "the step that had succeeded is not carried out a second time"
    );
    assert_eq!(step(&store, &run, "hello").attempts, 1);
    assert_eq!(step(&store, &run, "run").attempts, 2);
}

#[test]
fn continuing_after_a_test_that_never_ended_cancels_that_test_and_runs_a_new_one() {
    let (_dir, store, run, _, _) = queued_workflow(HELLO_THEN_GATE);
    let hands = Told {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    let stuck = step_test_id(&store, &run, "run");
    // What the engine does once the step's timeout has passed with the test still going.
    assert!(store
        .move_workflow_step(
            &run,
            "run",
            &[StepStatus::Waiting],
            StepStatus::Failed,
            None,
            Some("The test run did not end within 30m."),
            None,
            false,
        )
        .unwrap());
    tick(&store, &hands);
    let failed = store.workflow_run(&run).unwrap().unwrap();
    assert_eq!(failed.status, "failed");
    assert_eq!(
        failed.error.as_deref(),
        Some("Step `run` failed: The test run did not end within 30m.")
    );
    assert_eq!(step(&store, &run, "tell").status, "skipped");

    crate::workflow::runs::continue_failed(&store, &run).unwrap();
    let old = store.test_run(&stuck).unwrap().unwrap();
    assert_eq!(old.status, "cancelled");
    assert_eq!(
        old.error.as_deref(),
        Some("The workflow is running this step again.")
    );

    tick(&store, &hands);
    assert_ne!(step_test_id(&store, &run, "run"), stuck);
    assert_eq!(step(&store, &run, "run").status, "waiting");
    assert_eq!(step(&store, &run, "tell").status, "pending");
}

#[test]
fn only_a_failed_run_continues_and_never_beside_another_run_of_the_same_workflow() {
    let (dir, store, run, _, workspace) = queued_workflow(HELLO_THEN_GATE);
    let hands = Told {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);

    let refused = crate::workflow::runs::continue_failed(&store, &run).unwrap_err();
    assert_eq!(refused.code, "workflow_run_not_failed");
    assert!(
        refused.message.contains("is running"),
        "{}",
        refused.message
    );
    assert_eq!(step(&store, &run, "run").status, "waiting");

    let nothing = crate::workflow::runs::continue_failed(&store, "no-such-run").unwrap_err();
    assert_eq!(nothing.code, "workflow_run_not_found");

    finish_smoke(&store, "failed", None);
    tick(&store, &hands);
    assert_eq!(store.workflow_run(&run).unwrap().unwrap().status, "failed");
    let other = queue(
        &store,
        dir.path(),
        &Request {
            workflow_id: "gate",
            workspace_id: &workspace,
            inputs: &BTreeMap::new(),
            requested_by: "cli",
            unattended: false,
            origin_record: None,
            depth: 0,
            event: None,
            tracked: None,
        },
        &crate::workflow::runs::Accepting::default(),
    )
    .unwrap();

    let refused = crate::workflow::runs::continue_failed(&store, &run).unwrap_err();
    assert_eq!(refused.code, "workflow_already_running");
    assert!(
        refused.message.contains(crate::memory::short_id(&other)),
        "{}",
        refused.message
    );
    assert_eq!(store.workflow_run(&run).unwrap().unwrap().status, "failed");
    assert_eq!(
        step(&store, &run, "run").status,
        "failed",
        "a refusal changes nothing"
    );
}

#[test]
fn a_rejected_approval_asks_again_when_the_run_continues() {
    let (_dir, store, run, _worktree) = queue_folder(SIGN_OFF, "sign", &BTreeMap::new());
    let hands = Filed {
        sent: Mutex::new(Vec::new()),
    };
    tick(&store, &hands);
    store
        .record_workflow_approval(&run, "sign-off", "rejected", Some("not yet"), "ada", None)
        .unwrap();
    tick(&store, &hands);
    assert_eq!(store.workflow_run(&run).unwrap().unwrap().status, "failed");

    let again = crate::workflow::runs::continue_failed(&store, &run).unwrap();
    assert_eq!(again, ["sign-off", "next"]);
    tick(&store, &hands);
    assert_eq!(
        step(&store, &run, "sign-off").status,
        "waiting",
        "the earlier rejection is not applied to the new attempt"
    );
    assert_eq!(step(&store, &run, "brief").attempts, 1);

    assert_eq!(
        store
            .record_workflow_approval(&run, "sign-off", "approved", None, "ada", None)
            .unwrap(),
        crate::store::ApprovalWrite::Recorded
    );
    tick(&store, &hands);
    assert_eq!(
        store.workflow_run(&run).unwrap().unwrap().status,
        "succeeded"
    );
}

#[test]
fn a_continued_run_that_fails_again_tells_the_phone_again() {
    let (_dir, store, run, _, _) = queued_workflow(HELLO_THEN_GATE);
    let hands = Told {
        sent: Mutex::new(Vec::new()),
    };
    let failures = |store: &Store| {
        crate::notify::pending(store, now_ms())
            .unwrap()
            .into_iter()
            .filter(|notice| notice.category == "failure")
            .count()
    };
    tick(&store, &hands);
    finish_smoke(&store, "failed", Some("wanted 200, got 500"));
    tick(&store, &hands);
    assert_eq!(store.workflow_run(&run).unwrap().unwrap().status, "failed");
    assert_eq!(failures(&store), 1);

    // The second failure ends at a later time than the first, as it does outside a test.
    std::thread::sleep(Duration::from_millis(5));
    crate::workflow::runs::continue_failed(&store, &run).unwrap();
    tick(&store, &hands);
    finish_smoke(&store, "failed", Some("wanted 200, got 500"));
    tick(&store, &hands);
    assert_eq!(store.workflow_run(&run).unwrap().unwrap().status, "failed");
    assert_eq!(
        failures(&store),
        2,
        "the same run failing a second time is not a retry of the first notice"
    );
}
