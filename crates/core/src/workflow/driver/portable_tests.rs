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
    assert!(error.contains("Step `run` failed:"), "{error}");
    assert!(error.contains("The test failed:"), "{error}");
    assert!(error.contains("wanted 200, got 500"), "{error}");
    let steps = store.workflow_steps(&run).unwrap();
    let smoke = steps.iter().find(|step| step.step_id == "run").unwrap();
    assert_eq!(smoke.status, "failed");
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
