//! Driver tests that need no terminal, so they run on every platform: a cancel that lands while
//! a plan is being carried out, and a wait on the pull request.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pty_host::{PtyHost, SessionInfo};

use super::*;
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

    fn pr_posts(&self, _: &str, _: u32) -> Result<Vec<PrPost>, String> {
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
    Driver::default().tick(&store, &host, &hands).unwrap();

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

/// A forge whose answer the test sets, counting how often it is asked.
struct Forge {
    posts: Mutex<Vec<PrPost>>,
    asked: Mutex<u32>,
    notified: Mutex<Vec<String>>,
}

impl Hands for Forge {
    fn start(&self, _: &str, _: HarnessRequest) -> Result<SessionInfo, String> {
        unreachable!("this workflow starts no agent")
    }

    fn notify(&self, title: &str, _body: Option<&str>) -> Result<(), String> {
        self.notified.lock().unwrap().push(title.to_owned());
        Ok(())
    }

    fn handoff(&self, _: &str) -> Option<String> {
        None
    }

    fn pr_posts(&self, _: &str, number: u32) -> Result<Vec<PrPost>, String> {
        assert_eq!(number, 7);
        *self.asked.lock().unwrap() += 1;
        Ok(self.posts.lock().unwrap().clone())
    }
}

fn post(kind: crate::forge::PrPostKind, at_ms: i64, url: &str) -> PrPost {
    PrPost {
        kind,
        at_ms,
        url: Some(url.to_owned()),
    }
}

#[test]
fn a_review_posted_after_the_run_began_ends_the_wait_and_the_forge_is_not_asked_every_tick() {
    use crate::forge::{Checks, PrPostKind, PullRequest, PullRequestState};
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
    std::fs::write(folder.join("wait-review.yaml"), WAIT_FOR_A_REVIEW).unwrap();
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
    let run = queue(
        &store,
        dir.path(),
        &Request {
            workflow_id: "wait-review",
            workspace_id: &workspace.id,
            inputs: &BTreeMap::new(),
            requested_by: "cli",
        },
        &look,
    )
    .unwrap();

    // Before the run: an old review, and a comment. Neither is what it waits for.
    let before = crate::store::now_ms() - 60_000;
    let forge = Forge {
        posts: Mutex::new(vec![
            post(PrPostKind::Review, before, "https://github.com/o/r/pull/7"),
            post(
                PrPostKind::Comment,
                before,
                "https://github.com/o/r/pull/7#c1",
            ),
        ]),
        asked: Mutex::new(0),
        notified: Mutex::new(Vec::new()),
    };
    let host = PtyHost::new(Arc::new(|_| {}));
    let driver = Driver::new(SETTLE_BUSY_MS, Duration::from_secs(3600));
    for _ in 0..5 {
        driver.tick(&store, &host, &forge).unwrap();
    }
    let step = |id: &str| {
        store
            .workflow_steps(&run)
            .unwrap()
            .into_iter()
            .find(|s| s.step_id == id)
            .unwrap()
    };
    assert_eq!(step("posted").status, "waiting");
    assert_eq!(
        *forge.asked.lock().unwrap(),
        1,
        "once per interval, not once per tick"
    );

    // The review arrives; a driver asking every moment sees it at once.
    forge.posts.lock().unwrap().push(post(
        PrPostKind::Review,
        crate::store::now_ms(),
        "https://github.com/o/r/pull/7",
    ));
    let eager = Driver::new(SETTLE_BUSY_MS, Duration::ZERO);
    eager.tick(&store, &host, &forge).unwrap();
    assert_eq!(step("posted").status, "succeeded");
    assert_eq!(
        *forge.notified.lock().unwrap(),
        vec!["1 review on #7: https://github.com/o/r/pull/7".to_owned()]
    );
    assert_eq!(
        store.workflow_run(&run).unwrap().unwrap().status,
        "succeeded"
    );
}
