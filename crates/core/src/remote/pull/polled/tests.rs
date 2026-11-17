use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::forge::PullRequestState;

fn reading(numbers: &[u32]) -> Reading {
    Reading {
        pull_requests: numbers
            .iter()
            .map(|n| pr(*n, PullRequestState::Open))
            .collect(),
        workspaces: BTreeMap::new(),
        every_open: false,
    }
}

fn pr(number: u32, state: PullRequestState) -> PullRequest {
    PullRequest {
        number,
        url: format!("https://github.com/o/r/pull/{number}"),
        title: format!("Pull request {number}"),
        branch: format!("ys/b-{number}"),
        state,
        draft: false,
        checks: crate::forge::Checks::None,
        details: None,
        author: None,
        created_at: None,
    }
}

/// Waits for the reading on its thread to be over.
fn settle(polled: &Polled, project: &str) {
    let until = Instant::now() + Duration::from_secs(10);
    while polled.marks.lock().unwrap().reading.contains(project) {
        assert!(Instant::now() < until, "the reading never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_first_question_starts_a_reading_and_the_next_has_what_it_found() {
    let asked = Arc::new(AtomicUsize::new(0));
    let polled = Polled::new(Arc::new({
        let asked = Arc::clone(&asked);
        move |_: &str, _: bool| {
            asked.fetch_add(1, Ordering::SeqCst);
            Some(reading(&[7, 3]))
        }
    }));
    let first = polled.of_project("p", false);
    assert!(first.pull_requests.is_empty());
    settle(&polled, "p");
    let second = polled.of_project("p", false);
    assert!(!second.reading);
    let numbers: Vec<u32> = second.pull_requests.iter().map(|pr| pr.number).collect();
    assert_eq!(numbers, [7, 3]);
    // Fresh, so not read again.
    assert_eq!(asked.load(Ordering::SeqCst), 1);
}

#[test]
fn a_project_is_read_again_once_what_was_read_is_old_and_not_while_it_is_being_read() {
    let polled = Polled::new(Arc::new(|_: &str, _: bool| Some(reading(&[1]))));
    let start = Instant::now();
    let after = |seconds: u64| start + Duration::from_secs(seconds);
    assert_eq!(polled.begin("p", false, start), Some(false));
    // Asked again while `gh` is still away.
    assert_eq!(polled.begin("p", true, start), None);
    keep(&polled.marks, "p", Some(reading(&[1])), start);
    polled.marks.lock().unwrap().reading.remove("p");
    assert_eq!(polled.begin("p", false, after(30)), None);
    assert_eq!(polled.begin("p", false, after(45)), Some(false));
}

#[test]
fn a_reading_that_found_nothing_is_tried_again_in_a_while_and_not_at_every_question() {
    let polled = Polled::new(Arc::new(|_: &str, _: bool| None));
    let start = Instant::now();
    let after = |seconds: u64| start + Duration::from_secs(seconds);
    assert_eq!(polled.begin("p", false, start), Some(false));
    keep(&polled.marks, "p", None, start);
    polled.marks.lock().unwrap().reading.remove("p");
    assert_eq!(polled.begin("p", false, after(4)), None);
    assert_eq!(polled.begin("p", false, after(10)), Some(false));
}

#[test]
fn every_open_one_is_asked_for_when_wanted_and_kept_through_a_reading_of_the_newest() {
    let polled = Polled::new(Arc::new(|_: &str, _: bool| None));
    let start = Instant::now();
    let after = |seconds: u64| start + Duration::from_secs(seconds);
    keep(&polled.marks, "p", Some(reading(&[9])), start);
    // Fresh, but every open one was never read.
    assert_eq!(polled.begin("p", true, after(1)), Some(true));
    polled.marks.lock().unwrap().reading.remove("p");
    let mut full = reading(&[9, 2]);
    full.every_open = true;
    keep(&polled.marks, "p", Some(full), after(2));
    // The newest, read again: 2 is older than any of them, and still open.
    keep(&polled.marks, "p", Some(reading(&[10, 9])), after(50));
    let numbers: Vec<u32> = polled.marks.lock().unwrap().kept["p"]
        .found
        .pull_requests
        .iter()
        .map(|pr| pr.number)
        .collect();
    assert_eq!(numbers, [10, 9, 2]);
    // A workspace list does not ask for them.
    assert_eq!(polled.begin("p", false, after(51)), None);
}

#[test]
fn a_reading_that_panics_is_still_over() {
    let polled = Polled::new(Arc::new(|_: &str, _: bool| -> Option<Reading> {
        panic!("gh went away")
    }));
    polled.of_project("p", false);
    settle(&polled, "p");
    assert!(!polled.marks.lock().unwrap().kept.contains_key("p"));
}

#[cfg(unix)]
#[test]
fn read_asks_gh_in_the_project_and_shares_its_pull_requests_out_among_the_workspaces() {
    let dir = tempfile::tempdir().unwrap();
    let base = crate::git::normalize(dir.path());
    let git = crate::git::testing::git();
    let store = Store::in_memory();
    let root = base.join("orchard");
    std::fs::create_dir_all(&root).unwrap();
    git.init(&root).unwrap();
    std::fs::write(root.join("README.md"), "hello\n").unwrap();
    git.commit_all(&root, "first").unwrap();
    let main = git.run(&root, &["branch", "--show-current"]).unwrap();
    let project = store
        .add_project("orchard", &root.to_string_lossy())
        .unwrap();
    let tree = base.join("orchard-wt");
    git.worktree_add(&root, &tree, "ys/fix-login", &main)
        .unwrap();
    let worktree = store
        .add_worktree(
            &project.id,
            "fix",
            &tree.to_string_lossy(),
            Some("ys/fix-login"),
            Some(&main),
        )
        .unwrap();

    let asked = base.join("asked");
    let script = base.join("gh");
    crate::program::testing::executable(
        &script,
        format!(
            "#!/bin/sh\npwd >> '{asked}'\necho \"$@\" >> '{asked}'\necho '{answer}'\n",
            asked = asked.display(),
            answer = r#"[{"number":42,"url":"https://github.com/o/r/pull/42","title":"Fix login","headRefName":"ys/fix-login","state":"OPEN","isDraft":false},{"number":5,"url":"https://github.com/o/r/pull/5","title":"Other","headRefName":"ys/other","state":"MERGED","isDraft":false}]"#
        ),
    );
    let env = crate::env::ShellEnv {
        vars: std::env::vars().collect(),
        source: crate::env::EnvSource::Process,
        warning: None,
    };
    let gh = Gh::at(script, &env);

    let found = read(&store, &git, Some(&gh), &project.id, false).unwrap();
    let numbers: Vec<u32> = found.pull_requests.iter().map(|pr| pr.number).collect();
    assert_eq!(numbers, [42, 5]);
    assert_eq!(found.workspaces.get(&worktree.id), Some(&vec![42]));
    let asked = std::fs::read_to_string(&asked).unwrap();
    let mut lines = asked.lines();
    assert_eq!(Path::new(lines.next().unwrap()), root);
    assert!(lines
        .next()
        .unwrap()
        .starts_with("pr list --state all --limit 50"));

    // Without gh: a reading of none, not a failure.
    let none = read(&store, &git, None, &project.id, false).unwrap();
    assert!(none.pull_requests.is_empty());
    // A project that is not there: nothing to keep.
    assert!(read(&store, &git, Some(&gh), "gone", false).is_none());
}
