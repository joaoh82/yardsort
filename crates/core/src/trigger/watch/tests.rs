//! What a look at a pull request is news of, and what becomes of that news: a real store, a
//! made-up clock, and a forge answered from a table.

use std::cell::RefCell;
use std::collections::BTreeMap;

use super::*;
use crate::forge::{CheckCounts, PullRequest, PullRequestState, ReviewState, WatchPost};
use crate::schedule::ScheduleTarget;
use crate::trigger::fire::{observe, tick};
use crate::trigger::tests::{Tx, NOW};
use crate::trigger::{Filters, FromFilter, ReviewStateFilter};
use crate::workflow::runs::Facts;

fn pr(number: u32) -> PrLook {
    PrLook {
        number,
        url: format!("https://github.com/acme/tool/pull/{number}"),
        title: "Fix login".into(),
        state: PullRequestState::Open,
        draft: false,
        branch: "fix-login".into(),
        head: "c1".into(),
        check_counts: CheckCounts::default(),
        failed_checks: vec![],
        more_failed: false,
        mergeable: Mergeable::Unknown,
        posts: vec![],
        viewer: Some("me".into()),
    }
}

fn post(id: &str, kind: WatchPostKind, author: &str, at: i64) -> WatchPost {
    WatchPost {
        id: id.into(),
        kind,
        author: Some(author.into()),
        state: (kind == WatchPostKind::Review).then_some(ReviewState::Commented),
        at,
        url: Some(format!("https://github.com/acme/tool/pull/7#{id}")),
    }
}

fn seeded() -> PrWatchRow {
    PrWatchRow {
        workspace_id: "w".into(),
        seeded: true,
        ..PrWatchRow::default()
    }
}

fn events(seen: &[Observation]) -> Vec<(&'static str, Option<&str>)> {
    seen.iter()
        .map(|o| (o.event.as_str(), o.status.as_deref()))
        .collect()
}

#[test]
fn what_is_true_at_the_first_look_is_recorded_not_told() {
    let mut open = pr(7);
    open.check_counts.failed = 1;
    open.failed_checks = vec!["lint".into()];
    open.mergeable = Mergeable::Conflicting;
    open.posts = vec![post("review:1", WatchPostKind::Review, "alice", NOW - 5)];
    let unseeded = PrWatchRow {
        workspace_id: "w".into(),
        ..PrWatchRow::default()
    };
    let (seen, watch) = compare(&unseeded, Some(&open), NOW);
    // The review is told; `observe` drops it for a trigger newer than it.
    assert_eq!(events(&seen), [("pr.review", None)]);
    assert!(watch.seeded);
    assert_eq!(watch.number, Some(7));
    assert_eq!(watch.checks.as_deref(), Some("failed"));
    assert_eq!(watch.checks_head.as_deref(), Some("c1"));
    assert_eq!(watch.conflict_head.as_deref(), Some("c1"));
    // The same again is nothing.
    let (again, _) = compare(&watch, Some(&open), NOW + 60_000);
    assert!(again.is_empty(), "{again:?}");
}

#[test]
fn a_pull_request_opening_is_told_once_and_a_draft_says_so() {
    let (seen, watch) = compare(&seeded(), None, NOW);
    assert!(seen.is_empty());
    assert_eq!(watch.number, None);

    let mut draft = pr(7);
    draft.draft = true;
    let (seen, watch) = compare(&watch, Some(&draft), NOW + 1);
    assert_eq!(events(&seen), [("pr.opened", Some("draft"))]);
    assert_eq!(seen[0].facts.get("number").map(String::as_str), Some("7"));
    let (seen, watch) = compare(&watch, Some(&draft), NOW + 2);
    assert!(seen.is_empty());
    let (seen, _) = compare(&watch, Some(&pr(7)), NOW + 3);
    assert_eq!(events(&seen), [("pr.opened", Some("ready"))]);
}

#[test]
fn finished_checks_are_told_once_per_commit_and_verdict() {
    let mut running = pr(7);
    running.check_counts = CheckCounts {
        passed: 1,
        failed: 1,
        running: 1,
    };
    let mut watch = seeded();
    watch.number = Some(7);
    // One has failed, but another is still running: not finished.
    let (seen, watch) = compare(&watch, Some(&running), NOW);
    assert!(seen.is_empty());

    let mut failed = running.clone();
    failed.check_counts.running = 0;
    failed.check_counts.failed = 2;
    failed.failed_checks = vec!["lint".into(), "test (ubuntu)".into()];
    let (seen, watch) = compare(&watch, Some(&failed), NOW + 1);
    assert_eq!(events(&seen), [("pr.checks_failed", None)]);
    assert_eq!(
        seen[0].facts.get("summary").map(String::as_str),
        Some("2 checks failed: lint, test (ubuntu)")
    );
    assert_eq!(seen[0].facts.get("head").map(String::as_str), Some("c1"));
    let (seen, watch) = compare(&watch, Some(&failed), NOW + 2);
    assert!(seen.is_empty(), "the same failure on the same commit");

    // A push: the same failure on a new commit is news again.
    let mut pushed = failed.clone();
    pushed.head = "c2".into();
    let (seen, watch) = compare(&watch, Some(&pushed), NOW + 3);
    assert_eq!(events(&seen), [("pr.checks_failed", None)]);
    let mut fixed = pushed.clone();
    fixed.check_counts = CheckCounts {
        passed: 3,
        failed: 0,
        running: 0,
    };
    let (seen, _) = compare(&watch, Some(&fixed), NOW + 4);
    assert_eq!(events(&seen), [("pr.checks_passed", None)]);
}

#[test]
fn a_conflict_is_told_once_per_commit_and_unknown_is_no_answer() {
    let mut watch = seeded();
    watch.number = Some(7);
    let mut conflicting = pr(7);
    conflicting.mergeable = Mergeable::Conflicting;
    let (seen, watch) = compare(&watch, Some(&conflicting), NOW);
    assert_eq!(events(&seen), [("pr.conflicted", None)]);
    let (seen, watch) = compare(&watch, Some(&pr(7)), NOW + 1);
    assert!(seen.is_empty(), "unknown changes nothing");
    let (seen, watch) = compare(&watch, Some(&conflicting), NOW + 2);
    assert!(seen.is_empty());
    let mut clean = pr(7);
    clean.mergeable = Mergeable::Mergeable;
    let (_, watch) = compare(&watch, Some(&clean), NOW + 3);
    let (seen, _) = compare(&watch, Some(&conflicting), NOW + 4);
    assert_eq!(
        events(&seen),
        [("pr.conflicted", None)],
        "it merged cleanly in between, so this is a new conflict"
    );
}

#[test]
fn each_post_is_told_once_with_whose_it_is() {
    let mut watch = seeded();
    watch.number = Some(7);
    let mut open = pr(7);
    open.posts = vec![
        post("review:1", WatchPostKind::Review, "alice", NOW - 2),
        post("reply:2", WatchPostKind::Reply, "me", NOW - 1),
    ];
    let (seen, watch) = compare(&watch, Some(&open), NOW);
    assert_eq!(events(&seen), [("pr.review", None), ("pr.comment", None)]);
    assert_eq!(seen[0].at, NOW - 2, "a post is as old as it is");
    assert!(!seen[0].post.as_ref().unwrap().mine);
    assert!(seen[1].post.as_ref().unwrap().mine);
    assert_eq!(
        seen[0].facts.get("summary").map(String::as_str),
        Some("1 review from alice")
    );
    open.posts
        .push(post("comment:3", WatchPostKind::Comment, "bob", NOW + 1));
    let (seen, _) = compare(&watch, Some(&open), NOW + 2);
    assert_eq!(events(&seen), [("pr.comment", None)]);
}

#[test]
fn merged_is_told_once_and_the_next_pull_request_opens_afresh() {
    let (_, watch) = compare(&seeded(), Some(&pr(7)), NOW);
    let mut merged = pr(7);
    merged.state = PullRequestState::Merged;
    let (seen, watch) = compare(&watch, Some(&merged), NOW + 1);
    assert_eq!(events(&seen), [("pr.merged", None)]);
    assert_eq!(
        seen[0].facts.get("title").map(String::as_str),
        Some("Fix login")
    );
    assert!(watch.closed);
    // The branch has no open one for a while.
    let (seen, watch) = compare(&watch, None, NOW + 2);
    assert!(seen.is_empty());
    assert!(
        watch.closed,
        "still waiting for a reopening or the next one"
    );
    let (seen, _) = compare(&watch, Some(&pr(8)), NOW + 3);
    assert_eq!(events(&seen), [("pr.opened", Some("open"))]);
}

#[test]
fn closed_and_reopened_carries_on_where_it_was() {
    let mut open = pr(7);
    open.posts = vec![post("review:1", WatchPostKind::Review, "alice", NOW - 1)];
    open.check_counts.failed = 1;
    open.failed_checks = vec!["lint".into()];
    open.mergeable = Mergeable::Conflicting;
    let (_, watch) = compare(&seeded(), Some(&open), NOW);
    let mut closed = open.clone();
    closed.state = PullRequestState::Closed;
    let (seen, watch) = compare(&watch, Some(&closed), NOW + 1);
    assert_eq!(events(&seen), [("pr.closed", None)]);
    assert_eq!(watch.number, Some(7), "kept, for a reopening");
    assert!(watch.seen.contains("review:1"));

    // Reopened: the same review, the same red checks, the same conflict are not news, and it
    // has not opened.
    let (seen, watch) = compare(&watch, Some(&open), NOW + 2);
    assert!(seen.is_empty(), "{seen:?}");
    assert!(!watch.closed);
    open.posts
        .push(post("comment:2", WatchPostKind::Comment, "bob", NOW + 3));
    let (seen, _) = compare(&watch, Some(&open), NOW + 4);
    assert_eq!(events(&seen), [("pr.comment", None)]);
}

#[test]
fn more_failures_than_are_named_say_so() {
    let mut watch = seeded();
    watch.number = Some(7);
    let mut failed = pr(7);
    failed.check_counts.failed = 150;
    failed.failed_checks = vec!["lint".into()];
    failed.more_failed = true;
    let (seen, _) = compare(&watch, Some(&failed), NOW);
    assert_eq!(
        seen[0].facts.get("summary").map(String::as_str),
        Some("150 checks failed: lint and others")
    );
}

#[test]
fn gathered_posts_add_up_and_name_each_author_once() {
    let one = BTreeMap::from([
        ("reviews".to_owned(), "1".to_owned()),
        ("authors".to_owned(), "alice".to_owned()),
    ]);
    let two = gather_posts(
        &one,
        &BTreeMap::from([
            ("comments".to_owned(), "1".to_owned()),
            ("authors".to_owned(), "coderabbitai[bot]".to_owned()),
        ]),
    );
    let three = gather_posts(
        &two,
        &BTreeMap::from([
            ("comments".to_owned(), "1".to_owned()),
            ("authors".to_owned(), "alice".to_owned()),
        ]),
    );
    assert_eq!(
        three.get("summary").map(String::as_str),
        Some("1 review and 2 comments from alice and coderabbitai[bot]")
    );
    assert_eq!(
        three.get("authors").map(String::as_str),
        Some("alice, coderabbitai[bot]")
    );
}

/// A forge answered from a table: the pull request a watch sees, and the open one a run's
/// `{{ pr.* }}` is read from.
#[derive(Default)]
struct Forge {
    pr: RefCell<Option<PrLook>>,
    down: RefCell<bool>,
    /// The number each look asked about, `None` for the branch's open one.
    asked: RefCell<Vec<Option<u32>>>,
}

impl Forge {
    fn set(&self, pr: PrLook) {
        *self.pr.borrow_mut() = Some(pr);
    }
}

impl Look for Forge {
    fn harness(&self, wanted: &str) -> Result<String, String> {
        Ok(wanted.to_owned())
    }

    fn pull_request(&self, _workspace: &WorkspaceRow) -> Result<Option<PullRequest>, String> {
        Ok(self.pr.borrow().as_ref().map(|look| PullRequest {
            number: look.number,
            url: look.url.clone(),
            title: look.title.clone(),
            branch: look.branch.clone(),
            state: look.state,
            draft: look.draft,
            checks: look.check_counts.verdict(),
            details: None,
            author: None,
            created_at: None,
        }))
    }

    fn watch_pull_request(
        &self,
        _workspace: &WorkspaceRow,
        number: Option<u32>,
    ) -> Result<Option<PrLook>, String> {
        self.asked.borrow_mut().push(number);
        if *self.down.borrow() {
            return Err("GitHub did not answer.".into());
        }
        Ok(self.pr.borrow().clone())
    }
}

impl Tx {
    /// A trigger on reviews and comments that runs `address-review`, with the workspace's own
    /// agent running for it to tell.
    fn on_posts(&self, forge: &Forge, filters: &Filters) -> crate::store::TriggerRow {
        self.agent("rec-1", "pty-1");
        self.on_posts_alone(forge, filters)
    }

    /// The same, with no agent.
    fn on_posts_alone(&self, forge: &Forge, filters: &Filters) -> crate::store::TriggerRow {
        let inputs = BTreeMap::new();
        let events = [TriggerEvent::PrReview, TriggerEvent::PrComment];
        let mut draft = self.draft(
            &events,
            filters,
            ScheduleTarget::Workflow,
            "address-review",
            &inputs,
        );
        draft.quiet_ms = 2 * 60_000;
        crate::trigger::add(self.store(), &self.fx.data_dir, forge, &draft, "app", NOW).unwrap()
    }

    fn look(&self, forge: &Forge, now: i64) -> usize {
        let git = crate::git::testing::git();
        let mut taken = 0;
        for seen in look_at(self.store(), forge, &git, &self.fx.workspace, now).unwrap() {
            taken += observe(self.store(), &seen).unwrap();
        }
        taken
    }

    fn tick(&self, forge: &Forge, now: i64) -> Vec<crate::trigger::fire::Fired> {
        let git = crate::git::testing::git();
        tick(self.store(), &self.fx.data_dir, forge, &git, now).unwrap()
    }
}

#[test]
fn saving_seeds_the_watch_so_what_was_there_is_not_news() {
    let tx = Tx::new();
    let forge = Forge::default();
    let mut open = pr(7);
    open.posts = vec![post(
        "review:1",
        WatchPostKind::Review,
        "alice",
        NOW - 60_000,
    )];
    open.check_counts.failed = 1;
    forge.set(open.clone());
    tx.on_posts(&forge, &Filters::default());
    let watch = tx.store().pr_watch(tx.workspace_id()).unwrap().unwrap();
    assert!(watch.seeded);
    assert!(watch.seen.contains("review:1"));
    assert_eq!(tx.look(&forge, NOW + 60_000), 0);
    assert!(tx.tick(&forge, NOW + 10 * 60_000).is_empty());
}

#[test]
fn reviews_from_others_become_one_run_of_address_review_and_mine_do_not_count() {
    let tx = Tx::new();
    let forge = Forge::default();
    forge.set(pr(7));
    let trigger = tx.on_posts(&forge, &Filters::default());

    let mut open = pr(7);
    open.posts = vec![
        post("review:1", WatchPostKind::Review, "alice", NOW + 1_000),
        post("reply:2", WatchPostKind::Reply, "me", NOW + 2_000),
        post(
            "comment:3",
            WatchPostKind::Comment,
            "coderabbitai[bot]",
            NOW + 3_000,
        ),
    ];
    forge.set(open);
    assert_eq!(
        tx.look(&forge, NOW + 60_000),
        2,
        "my own reply is not counted"
    );
    // Two minutes after the last of them.
    assert!(tx.tick(&forge, NOW + 3_000 + 2 * 60_000 - 1).is_empty());
    let fired = tx.tick(&forge, NOW + 3_000 + 2 * 60_000);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].fire.outcome, "started");
    assert_eq!(fired[0].fire.subject, "pr:7");

    let run = tx
        .store()
        .workflow_run(fired[0].fire.run_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(run.workflow_id, "address-review");
    let facts = Facts::parse(&run.context);
    assert_eq!(facts.pr.as_ref().map(|pr| pr.number), Some(7));
    assert_eq!(
        facts.event.get("summary").map(String::as_str),
        Some("1 review and 1 comment from alice and coderabbitai[bot]")
    );
    assert_eq!(facts.event.get("count").map(String::as_str), Some("2"));
    assert_eq!(
        tx.store().trigger(&trigger.id).unwrap().unwrap().id,
        trigger.id
    );
}

#[test]
fn from_anyone_counts_mine_but_not_while_its_own_run_is_going() {
    let tx = Tx::new();
    let forge = Forge::default();
    forge.set(pr(7));
    let filters = Filters {
        from: Some(FromFilter::Anyone),
        ..Filters::default()
    };
    tx.on_posts(&forge, &filters);
    let mut open = pr(7);
    open.posts = vec![post("comment:1", WatchPostKind::Comment, "me", NOW + 1_000)];
    forge.set(open.clone());
    assert_eq!(tx.look(&forge, NOW + 60_000), 1);
    let fired = tx.tick(&forge, NOW + 10 * 60_000);
    assert_eq!(fired[0].fire.outcome, "started");

    // The agent answers, as me, while the run is going: not counted.
    open.posts.push(post(
        "reply:2",
        WatchPostKind::Reply,
        "me",
        NOW + 11 * 60_000,
    ));
    forge.set(open.clone());
    assert_eq!(tx.look(&forge, NOW + 12 * 60_000), 0);
}

#[test]
fn bots_and_states_can_be_left_out() {
    let tx = Tx::new();
    let forge = Forge::default();
    forge.set(pr(7));
    let filters = Filters {
        no_bots: true,
        states: vec![ReviewStateFilter::ChangesRequested],
        ..Filters::default()
    };
    tx.on_posts(&forge, &filters);
    let mut asks = post("review:2", WatchPostKind::Review, "alice", NOW + 2_000);
    asks.state = Some(ReviewState::ChangesRequested);
    let mut open = pr(7);
    open.posts = vec![
        post("review:1", WatchPostKind::Review, "alice", NOW + 1_000),
        asks,
        post(
            "comment:3",
            WatchPostKind::Comment,
            "coderabbitai[bot]",
            NOW + 3_000,
        ),
        post("comment:4", WatchPostKind::Comment, "bob", NOW + 4_000),
    ];
    forge.set(open);
    assert_eq!(
        tx.look(&forge, NOW + 60_000),
        2,
        "the review asking for changes, and bob's comment"
    );
}

#[test]
fn a_forge_that_does_not_answer_is_written_down_and_news_of_nothing() {
    let tx = Tx::new();
    let forge = Forge::default();
    forge.set(pr(7));
    tx.on_posts(&forge, &Filters::default());
    *forge.down.borrow_mut() = true;
    assert_eq!(tx.look(&forge, NOW + 60_000), 0);
    let watch = tx.store().pr_watch(tx.workspace_id()).unwrap().unwrap();
    assert_eq!(watch.problem.as_deref(), Some("GitHub did not answer."));
    assert_eq!(watch.looked_at, NOW + 60_000);
    assert_eq!(watch.number, Some(7), "what was seen before stays");
}

#[test]
fn a_watch_is_due_once_a_minute_and_goes_when_nothing_listens() {
    let tx = Tx::new();
    let forge = Forge::default();
    forge.set(pr(7));
    let trigger = tx.on_posts(&forge, &Filters::default());
    assert!(
        due(tx.store(), NOW + 1).unwrap().is_empty(),
        "seeded just now"
    );
    assert_eq!(due(tx.store(), NOW + 60_000).unwrap().len(), 1);
    crate::trigger::set_enabled(tx.store(), &trigger.id, false, NOW + 2).unwrap();
    assert!(due(tx.store(), NOW + 60_000).unwrap().is_empty());
    assert!(tx.store().pr_watch(tx.workspace_id()).unwrap().is_none());
}

#[test]
fn a_workflow_that_needs_the_open_pull_request_cannot_run_when_it_merged() {
    let tx = Tx::new();
    let forge = Forge::default();
    let inputs = BTreeMap::from([("reviewer".to_owned(), "claude".to_owned())]);
    let filters = Filters::default();
    let events = [TriggerEvent::PrMerged];
    // It reads `{{ pr.number }}` and waits for activity on it.
    let draft = tx.draft(
        &events,
        &filters,
        ScheduleTarget::Workflow,
        "code-review",
        &inputs,
    );
    let error =
        crate::trigger::add(tx.store(), &tx.fx.data_dir, &forge, &draft, "app", NOW).unwrap_err();
    assert_eq!(error.code, "trigger_pull_request_gone");
    assert!(error.message.contains("event.url"), "{}", error.message);
}

#[test]
fn with_no_agent_to_tell_the_fire_is_skipped_and_the_user_told_once() {
    let tx = Tx::new();
    let forge = Forge::default();
    forge.set(pr(7));
    let trigger = tx.on_posts_alone(&forge, &Filters::default());
    let mut open = pr(7);
    open.posts = vec![post(
        "review:1",
        WatchPostKind::Review,
        "alice",
        NOW + 1_000,
    )];
    forge.set(open.clone());
    tx.look(&forge, NOW + 60_000);
    let fired = tx.tick(&forge, NOW + 10 * 60_000);
    assert_eq!(fired[0].fire.outcome, "skipped");
    assert!(
        fired[0].fire.run_id.is_none(),
        "no run that would skip every step"
    );
    let notice = fired[0].notice.as_ref().expect("told");
    assert_eq!(notice.title, "Address reviews had no one to tell");
    assert!(
        notice.body.contains("No agent was running"),
        "{}",
        notice.body
    );

    open.posts.push(post(
        "review:2",
        WatchPostKind::Review,
        "bob",
        NOW + 11 * 60_000,
    ));
    forge.set(open);
    tx.look(&forge, NOW + 12 * 60_000);
    let again = tx.tick(&forge, NOW + 20 * 60_000);
    assert_eq!(again[0].fire.outcome, "skipped");
    assert!(again[0].notice.is_none(), "said once while it lasts");
    assert_eq!(
        tx.store()
            .counted_trigger_runs(&trigger.id, "pr:7")
            .unwrap(),
        0,
        "a fire with no one to tell is not a round"
    );
}

#[test]
fn a_closed_watch_asks_for_the_branchs_open_pull_request() {
    let tx = Tx::new();
    let forge = Forge::default();
    forge.set(pr(7));
    tx.on_posts(&forge, &Filters::default());
    let mut closed = pr(7);
    closed.state = PullRequestState::Closed;
    forge.set(closed);
    forge.asked.borrow_mut().clear();
    tx.look(&forge, NOW + 60_000);
    assert!(
        tx.store()
            .pr_watch(tx.workspace_id())
            .unwrap()
            .unwrap()
            .closed
    );
    tx.look(&forge, NOW + 120_000);
    assert_eq!(
        *forge.asked.borrow(),
        [Some(7), None],
        "by number while open, then the branch's open one"
    );
}
