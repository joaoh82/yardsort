//! `pull.list` and `pull.describe`, against a real store with two projects and what a host
//! would have read of their pull requests.

use std::sync::Mutex;

use remote_proto::pull::{
    CheckState, PullDetail, PullListParams, PullPage, PullRef, PullReview, PullState, PullsOf,
    ReviewerState, MAX_CHECKS,
};

use super::*;
use crate::forge::{
    CheckCounts, Checks, Mergeable, PullRequest, PullRequestCheck, PullRequestDetails,
    PullRequestReview, PullRequestState, ReviewRequest, ReviewState,
};
use crate::remote::pull::{FollowedPull, ProjectPulls, PullRequests};

/// What a host read, by project, and what was asked of it.
#[derive(Default)]
struct Read {
    projects: BTreeMap<String, ProjectPulls>,
    asked: Mutex<Vec<(String, bool)>>,
}

impl PullRequests for Read {
    fn of_project(&self, project_id: &str, every_open: bool) -> ProjectPulls {
        self.asked
            .lock()
            .unwrap()
            .push((project_id.to_owned(), every_open));
        self.projects.get(project_id).cloned().unwrap_or_default()
    }
}

fn pr(number: u32, state: PullRequestState, updated: &str) -> PullRequest {
    PullRequest {
        number,
        url: format!("https://github.com/orchard/orchard/pull/{number}"),
        title: format!("Pull request {number}"),
        branch: format!("ys/branch-{number}"),
        state,
        draft: false,
        checks: Checks::None,
        details: Some(PullRequestDetails {
            base: "main".into(),
            head_oid: "a".repeat(40),
            base_oid: "b".repeat(40),
            additions: 184,
            deletions: 22,
            review: String::new(),
            updated_at: updated.into(),
            checks: Vec::new(),
            check_counts: CheckCounts::default(),
            mergeable: Mergeable::Mergeable,
            review_requests: Vec::new(),
            reviews: Vec::new(),
            cross_repository: false,
        }),
        author: Some("juno-p".into()),
        created_at: Some(1_700_000_000_000),
    }
}

fn check(name: &str, state: Checks) -> PullRequestCheck {
    PullRequestCheck {
        name: name.into(),
        state,
        workflow: Some("CI".into()),
        url: None,
    }
}

fn facade_with<'a>(f: &'a Fixture, read: &'a Read) -> HostFacade<'a> {
    HostFacade {
        pulls: Some(read),
        notifications: None,
        ..f.facade(None)
    }
}

fn list(facade: &HostFacade<'_>, grant: &Grant, params: PullListParams) -> Response {
    ask(facade, grant, Operation::PullList(params))
}

fn page_of(facade: &HostFacade<'_>, grant: &Grant, of: PullsOf) -> PullPage {
    match reply(list(
        facade,
        grant,
        PullListParams {
            of,
            ..Default::default()
        },
    )) {
        Reply::Pulls(page) => page,
        other => panic!("{other:?}"),
    }
}

fn describe(
    facade: &HostFacade<'_>,
    grant: &Grant,
    project: &str,
    number: u32,
    followed: Option<&str>,
) -> Response {
    ask(
        facade,
        grant,
        Operation::PullDescribe(PullRef {
            project: ProjectId::new(project.to_owned()).unwrap(),
            number,
            followed: followed.map(str::to_owned),
        }),
    )
}

fn detail(response: Response) -> PullDetail {
    match reply(response) {
        Reply::Pull(detail) => *detail,
        other => panic!("{other:?}"),
    }
}

fn refusal(response: Response) -> ErrorCode {
    match response.outcome {
        Outcome::Error(error) => error.code,
        Outcome::Result(reply) => panic!("answered: {reply:?}"),
    }
}

fn numbers(page: &PullPage) -> Vec<u32> {
    page.pulls.iter().map(|pull| pull.number).collect()
}

/// Alpha has an open pull request from its worktree and a merged one nobody's workspace
/// opened; beta has an open one from its worktree.
fn two_projects(f: &Fixture) -> Read {
    let mut read = Read::default();
    read.projects.insert(
        f.alpha.id.clone(),
        ProjectPulls {
            pull_requests: vec![
                pr(61, PullRequestState::Open, "2026-11-15T10:00:00Z"),
                pr(42, PullRequestState::Merged, "2026-11-14T10:00:00Z"),
            ],
            workspaces: [(f.alpha.worktree.clone(), vec![61])].into(),
            ..Default::default()
        },
    );
    read.projects.insert(
        f.beta.id.clone(),
        ProjectPulls {
            pull_requests: vec![pr(7, PullRequestState::Open, "2026-11-15T11:00:00Z")],
            workspaces: [(f.beta.worktree.clone(), vec![7])].into(),
            ..Default::default()
        },
    );
    read
}

#[test]
fn pull_requests_are_told_to_a_grant_that_reads_workspaces_and_only_of_the_projects_it_reaches() {
    let f = Fixture::new();
    let read = two_projects(&f);
    let facade = facade_with(&f, &read);

    let all = page_of(&facade, &everything(), PullsOf::Workspaces);
    assert_eq!(numbers(&all), [61, 7]);
    assert_eq!(all.open, 2);
    assert_eq!(all.next, None);
    assert!(!all.reading);
    let first = &all.pulls[0];
    assert_eq!(first.project.as_str(), f.alpha.id);
    assert_eq!(
        first
            .workspaces
            .iter()
            .map(|w| w.as_str())
            .collect::<Vec<_>>(),
        [f.alpha.worktree.as_str()]
    );
    assert_eq!(first.state, PullState::Open);
    assert_eq!(first.base.as_deref(), Some("main"));
    assert_eq!((first.additions, first.deletions), (Some(184), Some(22)));
    assert_eq!(first.author.as_deref(), Some("juno-p"));
    assert_eq!(first.updated_at, Some(1_794_736_800_000));

    // The open ones, most recently changed first, whoever opened them; the merged apart.
    assert_eq!(
        numbers(&page_of(&facade, &everything(), PullsOf::Open)),
        [7, 61]
    );
    assert_eq!(
        numbers(&page_of(&facade, &everything(), PullsOf::Merged)),
        [42]
    );
    assert!(page_of(&facade, &everything(), PullsOf::Closed)
        .pulls
        .is_empty());

    // A grant for one project hears of that project alone, in the count too.
    let alpha_only = only(&f.alpha.id, &[Scope::WorkspaceRead]);
    let page = page_of(&facade, &alpha_only, PullsOf::Open);
    assert_eq!(numbers(&page), [61]);
    assert_eq!(page.open, 1);
    assert_eq!(
        refusal(describe(&facade, &alpha_only, &f.beta.id, 7, None)),
        ErrorCode::NotFound
    );
    assert!(read
        .asked
        .lock()
        .unwrap()
        .iter()
        .filter(|(project, _)| *project == f.beta.id)
        .count()
        .eq(&4));

    // Reading workspaces is what it takes, and nothing less.
    let watcher = grant(&[Scope::ActivityRead], ProjectGrant::All);
    assert_eq!(
        refusal(list(&facade, &watcher, PullListParams::default())),
        ErrorCode::Forbidden
    );
    assert_eq!(
        refusal(describe(&facade, &watcher, &f.alpha.id, 61, None)),
        ErrorCode::Forbidden
    );
}

#[test]
fn a_host_that_reads_no_pull_requests_does_not_answer_and_does_not_say_it_does() {
    let f = Fixture::new();
    let facade = f.facade(None);
    assert_eq!(
        refusal(list(&facade, &everything(), PullListParams::default())),
        ErrorCode::Unsupported
    );
    assert_eq!(
        refusal(describe(&facade, &everything(), &f.alpha.id, 61, None)),
        ErrorCode::Unsupported
    );
    let says = |facade: &HostFacade<'_>| match reply(ask(
        facade,
        &everything(),
        Operation::HostDescribe,
    )) {
        Reply::Host(host) => host.capabilities,
        other => panic!("{other:?}"),
    };
    assert!(!says(&facade).iter().any(|op| op.starts_with("pull.")));
    let read = Read::default();
    let reading = facade_with(&f, &read);
    for op in PULL_OPERATIONS {
        assert!(says(&reading).iter().any(|said| said == op), "{op}");
    }
}

#[test]
fn a_workspaces_own_lead_in_the_hosts_order_and_one_followed_by_two_is_told_once() {
    let f = Fixture::new();
    let mut read = Read::default();
    let elsewhere = |id: &str, workspace: &str| FollowedPull {
        id: id.into(),
        workspace_id: workspace.into(),
        repo: "orchard/lib".into(),
        pull_request: PullRequest {
            url: "https://github.com/orchard/lib/pull/7".into(),
            ..pr(7, PullRequestState::Open, "2026-11-15T09:00:00Z")
        },
    };
    read.projects.insert(
        f.alpha.id.clone(),
        ProjectPulls {
            pull_requests: vec![
                pr(90, PullRequestState::Open, "2026-11-15T12:00:00Z"),
                pr(88, PullRequestState::Open, "2026-11-15T10:00:00Z"),
                pr(80, PullRequestState::Merged, "2026-11-10T10:00:00Z"),
            ],
            // The worktree shows 88 first, though 80 is the older and 90 the newest read.
            workspaces: [(f.alpha.worktree.clone(), vec![88, 80])].into(),
            followed: vec![
                elsewhere("follow-1", &f.alpha.worktree),
                elsewhere("follow-2", &f.alpha.local),
            ],
            reading: true,
        },
    );
    let facade = facade_with(&f, &read);

    let page = page_of(&facade, &everything(), PullsOf::Workspaces);
    assert!(page.reading);
    // One nobody's workspace opened is not among a workspace list's.
    let told: Vec<(u32, Option<&str>)> = page
        .pulls
        .iter()
        .map(|pull| (pull.number, pull.repo.as_deref()))
        .collect();
    assert!(!told.contains(&(90, None)));
    let of_worktree: Vec<(u32, Option<&str>)> = page
        .pulls
        .iter()
        .filter(|pull| {
            pull.workspaces
                .iter()
                .any(|w| w.as_str() == f.alpha.worktree)
        })
        .map(|pull| (pull.number, pull.repo.as_deref()))
        .collect();
    assert_eq!(
        of_worktree,
        [(88, None), (80, None), (7, Some("orchard/lib"))]
    );
    let followed: Vec<_> = page
        .pulls
        .iter()
        .filter(|pull| pull.followed.is_some())
        .collect();
    assert_eq!(followed.len(), 1);
    assert_eq!(followed[0].workspaces.len(), 2);
    // Open: the two of the project's own, and the one followed, counted once.
    assert_eq!(page.open, 3);

    // Either workspace's following of it names it; its number alone is the project's own.
    for id in ["follow-1", "follow-2"] {
        let one = detail(describe(&facade, &everything(), &f.alpha.id, 7, Some(id)));
        assert_eq!(one.pull.repo.as_deref(), Some("orchard/lib"));
        assert_eq!(
            one.pull.url.as_deref(),
            Some("https://github.com/orchard/lib/pull/7")
        );
    }
    assert_eq!(
        refusal(describe(&facade, &everything(), &f.alpha.id, 7, None)),
        ErrorCode::NotFound
    );
    assert_eq!(
        refusal(describe(
            &facade,
            &everything(),
            &f.alpha.id,
            88,
            Some("follow-1")
        )),
        ErrorCode::NotFound
    );

    // A workspace put away is not on show, and neither is what only it led to.
    f.store
        .set_workspace_archived(&f.alpha.worktree, true)
        .unwrap();
    let page = page_of(&facade, &everything(), PullsOf::Workspaces);
    assert_eq!(numbers(&page), [7]);
    assert_eq!(page.pulls[0].workspaces.len(), 1);
    // The project's own are still its own, in the list of every open one.
    assert_eq!(
        numbers(&page_of(&facade, &everything(), PullsOf::Open)),
        [90, 88, 7]
    );
}

#[test]
fn one_pull_request_is_described_with_its_checks_failures_first_and_its_reviewers() {
    let f = Fixture::new();
    let mut read = Read::default();
    let mut request = pr(61, PullRequestState::Open, "2026-11-15T10:00:00Z");
    {
        let details = request.details.as_mut().unwrap();
        details.checks = vec![
            check("build", Checks::Passing),
            check("test (ubuntu)", Checks::Failing),
            check("lint", Checks::Running),
            PullRequestCheck {
                workflow: None,
                ..check("netlify", Checks::Passing)
            },
            check("test (windows)", Checks::Failing),
        ];
        details.check_counts = CheckCounts {
            passed: 2,
            failed: 2,
            running: 1,
        };
        details.review = "CHANGES_REQUESTED".into();
        details.mergeable = Mergeable::Conflicting;
        details.reviews = vec![PullRequestReview {
            login: "mira-k".into(),
            state: ReviewState::ChangesRequested,
        }];
        details.review_requests = vec![ReviewRequest {
            name: "tomas-r".into(),
            team: false,
        }];
    }
    // Not a page on the web: a device is given no link to follow.
    let mut local = pr(62, PullRequestState::Closed, "2026-11-15T10:00:00Z");
    local.url = "file:///etc/passwd".into();
    // More checks than one message names.
    let mut many = pr(63, PullRequestState::Open, "2026-11-15T10:00:00Z");
    many.details.as_mut().unwrap().checks = (0..MAX_CHECKS + 5)
        .map(|n| {
            check(
                &format!("job {n}"),
                if n == MAX_CHECKS + 4 {
                    Checks::Failing
                } else {
                    Checks::Passing
                },
            )
        })
        .collect();
    read.projects.insert(
        f.alpha.id.clone(),
        ProjectPulls {
            pull_requests: vec![request, local, many],
            ..Default::default()
        },
    );
    let facade = facade_with(&f, &read);

    let one = detail(describe(&facade, &everything(), &f.alpha.id, 61, None));
    assert_eq!(
        one.checks
            .iter()
            .map(|check| (check.name.as_str(), check.state))
            .collect::<Vec<_>>(),
        [
            ("CI / test (ubuntu)", CheckState::Failed),
            ("CI / test (windows)", CheckState::Failed),
            ("CI / lint", CheckState::Running),
            ("CI / build", CheckState::Passed),
            ("netlify", CheckState::Passed),
        ]
    );
    assert_eq!(one.more_checks, 0);
    assert_eq!(
        (
            one.pull.checks.passed,
            one.pull.checks.failed,
            one.pull.checks.running
        ),
        (2, 2, 1)
    );
    assert_eq!(one.pull.review, PullReview::ChangesRequested);
    assert!(one.pull.conflicts);
    assert_eq!(one.reviews.len(), 1);
    assert_eq!(one.reviews[0].login, "mira-k");
    assert_eq!(one.reviews[0].state, ReviewerState::ChangesRequested);
    assert_eq!(one.waiting_on, ["tomas-r"]);

    assert_eq!(
        detail(describe(&facade, &everything(), &f.alpha.id, 62, None))
            .pull
            .url,
        None
    );

    let many = detail(describe(&facade, &everything(), &f.alpha.id, 63, None));
    assert_eq!(many.checks.len(), MAX_CHECKS);
    assert_eq!(many.more_checks, 5);
    // The one that failed is not among those left out.
    assert_eq!(many.checks[0].state, CheckState::Failed);

    assert_eq!(
        refusal(describe(&facade, &everything(), &f.alpha.id, 999, None)),
        ErrorCode::NotFound
    );
}

#[test]
fn many_pull_requests_come_a_page_at_a_time_each_within_a_frame_and_none_twice() {
    let f = Fixture::new();
    let mut read = Read::default();
    let long = "\u{1f600}".repeat(400);
    read.projects.insert(
        f.alpha.id.clone(),
        ProjectPulls {
            pull_requests: (1..=250)
                .map(|number| PullRequest {
                    title: long.clone(),
                    branch: long.clone(),
                    ..pr(number, PullRequestState::Open, "2026-11-15T10:00:00Z")
                })
                .collect(),
            ..Default::default()
        },
    );
    let facade = facade_with(&f, &read);

    let mut seen = Vec::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        // `ask` holds every answer to a frame.
        let page = match reply(list(
            &facade,
            &everything(),
            PullListParams {
                of: PullsOf::Open,
                cursor: cursor.take(),
                limit: Some(remote_proto::MAX_PAGE),
            },
        )) {
            Reply::Pulls(page) => page,
            other => panic!("{other:?}"),
        };
        assert_eq!(page.open, 250);
        seen.extend(numbers(&page));
        pages += 1;
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert!(pages > 1, "{pages} page");
    let mut sorted = seen.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted, (1..=250).collect::<Vec<u32>>());
    assert_eq!(seen.len(), 250);
    // The whole list of open ones is what asks the host to read every open one.
    assert!(read.asked.lock().unwrap().iter().all(|(_, every)| *every));
}

#[test]
fn the_longest_pull_request_a_host_could_read_is_still_one_message() {
    let f = Fixture::new();
    let mut read = Read::default();
    // A control character is the longest thing a character can be once written as JSON.
    let long = "\u{1}".repeat(2_000);
    let mut request = PullRequest {
        title: long.clone(),
        branch: long.clone(),
        author: Some(long.clone()),
        url: format!("https://github.com/{}", "\"".repeat(2_000)),
        ..pr(61, PullRequestState::Open, "2026-11-15T10:00:00Z")
    };
    {
        let details = request.details.as_mut().unwrap();
        details.base = long.clone();
        details.checks = (0..500)
            .map(|_| PullRequestCheck {
                workflow: Some(long.clone()),
                ..check(&long, Checks::Failing)
            })
            .collect();
        details.reviews = (0..100)
            .map(|_| PullRequestReview {
                login: long.clone(),
                state: ReviewState::Commented,
            })
            .collect();
        details.review_requests = (0..100)
            .map(|_| ReviewRequest {
                name: long.clone(),
                team: true,
            })
            .collect();
    }
    read.projects.insert(
        f.alpha.id.clone(),
        ProjectPulls {
            pull_requests: vec![request],
            workspaces: [(f.alpha.worktree.clone(), vec![61])].into(),
            ..Default::default()
        },
    );
    let facade = facade_with(&f, &read);

    // `ask` refuses an answer longer than a frame.
    let one = detail(describe(&facade, &everything(), &f.alpha.id, 61, None));
    assert_eq!(one.checks.len(), MAX_CHECKS);
    assert_eq!(one.more_checks, 460);
    // A link too long to be one is no link.
    assert_eq!(one.pull.url, None);
    assert_eq!(
        numbers(&page_of(&facade, &everything(), PullsOf::Open)),
        [61]
    );
}
