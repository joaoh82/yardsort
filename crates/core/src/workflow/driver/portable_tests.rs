//! Driver tests that need no terminal, so they run on every platform: a cancel that lands while
//! a plan is being carried out, and a wait on the pull request.

use std::collections::BTreeMap;
use std::path::PathBuf;
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
