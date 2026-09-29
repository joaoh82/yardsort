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

    fn pr_posts(&self, _: &str, _: u32) -> PrAsk {
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

    fn pr_posts(&self, _: &str, number: u32) -> PrAsk {
        assert_eq!(number, 7);
        let (posts, asked, hold) = (
            Arc::clone(&self.posts),
            Arc::clone(&self.asked),
            self.hold.clone(),
        );
        Box::new(move || {
            if let Some(hold) = hold {
                let _ = hold.lock().unwrap().recv_timeout(Duration::from_secs(10));
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
    assert!(
        started.elapsed() < Duration::from_secs(1),
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
