//! `ys pr …` — every project's open pull requests, and a workspace on one of them.
//!
//! What the app's Pull requests view reads, asked of `gh` when the command is run: the same
//! query (`forge::Gh::open_pull_requests`), the same review rules as the view's filters, and the
//! same way of getting a pull request's branch into the repository
//! (`forge::checkout::prepare_branch`). Nothing is kept between runs and the app need not be
//! open.
//!
//! It is how the Floor Manager answers "which code reviews need attention", and how it gets a
//! pull request somebody else opened into a workspace a review can run in. Nothing here writes
//! to GitHub.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use yardsort_core::forge::checkout::{self, prepare_branch};
use yardsort_core::forge::{
    self, Checks, Gh, Mergeable, OpenPullRequests, PullRequest, PullRequestState, ReviewState,
};
use yardsort_core::store::{ProjectRow, WorkspaceRow};
use yardsort_core::tasks::github::{coverage, Coverage};
use yardsort_core::workspaces::Workspaces;

use crate::{table, Failure, Output, Yardsort};

/// How many open pull requests one repository is read for: what the Pull requests view reads.
const CAP: usize = 200;

/// How long `gh` may take over one page. It answers in a second or two.
const PAGE_LIMIT: Duration = Duration::from_secs(30);

/// Where a pull request stands with its reviewers: the view's review filter.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Reviews {
    /// Nobody has reviewed it yet.
    None,
    /// GitHub says a review is required.
    Required,
    Approved,
    /// Changes were requested.
    Changes,
    /// You have reviewed it.
    ByMe,
    /// You have not reviewed it, and it is not yours.
    NotByMe,
    /// Your review was asked for, by name, and you have not given it.
    AwaitingMe,
}

#[derive(clap::Subcommand)]
pub enum Command {
    /// Open pull requests across every project on GitHub, most recently updated first.
    ///
    /// Each comes with where it stands with its reviewers and its checks, the workspace it is in
    /// if one is, and what about it needs attention.
    List {
        /// Only this project's, by name or id.
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
        /// Only those where the reviews stand this way. `by-me`, `not-by-me` and `awaiting-me`
        /// mean whoever `gh` is logged in as.
        #[arg(long, value_enum)]
        reviews: Option<Reviews>,
        /// Only those that need attention: failing checks, conflicts, changes requested, a
        /// review asked of you, or no review at all on one that is not a draft.
        #[arg(long)]
        attention: bool,
        /// Only those opened by this login. `@me` is whoever `gh` is logged in as.
        #[arg(long, value_name = "LOGIN")]
        author: Option<String>,
        /// Leave drafts out.
        #[arg(long)]
        no_drafts: bool,
        /// At most this many.
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    /// Open a workspace on a pull request's branch, for reviewing it: no new branch, and no
    /// agent started.
    ///
    /// The branch is fetched from the project's remote and a worktree made for it, as the
    /// Pull requests view's Start workspace does. One from a fork is `pr/<number>`, following
    /// the pull request. A workspace that already has the branch is said, and nothing is made.
    Checkout {
        /// Which one: `12` or `#12`.
        number: String,
        /// The project, by name or id. Default: the one this is run in.
        #[arg(long, value_name = "PROJECT")]
        project: Option<String>,
    },
}

pub fn run(ys: &Yardsort, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::List {
            project,
            reviews,
            attention,
            author,
            no_drafts,
            limit,
        } => list(
            ys,
            project.as_deref(),
            &Wanted {
                reviews,
                attention,
                author,
                no_drafts,
            },
            limit,
            out,
        ),
        Command::Checkout { number, project } => checkout(ys, &number, project.as_deref(), out),
    }
}

/// What `list` lets through.
struct Wanted {
    reviews: Option<Reviews>,
    attention: bool,
    author: Option<String>,
    no_drafts: bool,
}

/// One pull request, as `--json` gives it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    project: String,
    project_id: String,
    number: u32,
    title: String,
    url: String,
    branch: String,
    author: Option<String>,
    draft: bool,
    /// `approved`, `changesRequested`, `reviewRequired`, or `null` when nothing decides it.
    review: Option<&'static str>,
    /// Each reviewer's latest review: `{ login, state }`.
    reviews: Vec<forge::PullRequestReview>,
    /// Reviews asked for and not given: `{ name, team }`.
    review_requests: Vec<forge::ReviewRequest>,
    checks: Checks,
    check_counts: Option<forge::CheckCounts>,
    mergeable: Option<Mergeable>,
    updated_at: Option<String>,
    /// The Yardsort workspace on its branch, by name, if there is one.
    workspace: Option<String>,
    /// Why it needs attention: `checksFailing`, `conflicts`, `changesRequested`, `awaitingYou`,
    /// `unreviewed`. Empty when nothing does.
    attention: Vec<&'static str>,
    /// Who `gh` is logged in as, which the review filters about "me" go by.
    viewer: Option<String>,
}

fn list(
    ys: &Yardsort,
    project: Option<&str>,
    wanted: &Wanted,
    limit: Option<usize>,
    out: &Output,
) -> Result<(), Failure> {
    let projects = match project {
        Some(wanted) => vec![ys.project(wanted)?],
        None => ys.store.projects()?,
    };
    let gh = gh(ys)?;
    let git = ys.git()?;
    // Each repository is asked by itself, and one slow repository should not hold up the rest.
    let answers: Vec<(ProjectRow, Result<OpenPullRequests, String>)> =
        std::thread::scope(|scope| {
            let asking: Vec<_> = projects
                .into_iter()
                .map(|project| {
                    let (gh, git) = (&gh, &git);
                    scope.spawn(move || {
                        let answer = ask(gh, git, &project);
                        (project, answer)
                    })
                })
                .collect();
            asking
                .into_iter()
                .filter_map(|handle| handle.join().ok())
                .collect()
        });

    let workspaces = ys.store.workspaces()?;
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    for (project, answer) in answers {
        let found = match answer {
            Ok(found) => found,
            Err(why) => {
                problems.push(format!("{}: {why}", project.name));
                continue;
            }
        };
        if let Some(problem) = &found.problem {
            problems.push(format!("{}: {problem}", project.name));
        }
        if let Some(total) = found.total {
            if total as usize > found.pull_requests.len() && found.answered {
                problems.push(format!(
                    "{}: showing the {} most recently updated of {total} open.",
                    project.name,
                    found.pull_requests.len()
                ));
            }
        }
        for pr in found.pull_requests {
            let row = row_of(&project, &workspaces, pr, found.viewer.clone());
            if lets_through(&row, wanted) {
                rows.push(row);
            }
        }
    }
    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    if let Some(limit) = limit {
        rows.truncate(limit);
    }
    for problem in &problems {
        eprintln!("{problem}");
    }

    out.emit(&rows, || {
        if rows.is_empty() {
            sayln!(
                "No open pull requests{}.",
                if filtered(wanted) { " match" } else { "" }
            );
            return;
        }
        let mut lines = vec![vec![
            "PROJECT".to_owned(),
            "PR".to_owned(),
            "STATE".to_owned(),
            "REVIEW".to_owned(),
            "CHECKS".to_owned(),
            "AUTHOR".to_owned(),
            "ATTENTION".to_owned(),
            "TITLE".to_owned(),
        ]];
        for row in &rows {
            lines.push(vec![
                row.project.clone(),
                format!("#{}", row.number),
                if row.draft { "draft" } else { "open" }.to_owned(),
                match row.review {
                    Some("approved") => "approved",
                    Some("changesRequested") => "changes requested",
                    Some(_) => "review required",
                    None if row.reviews.is_empty() => "none",
                    None => "commented",
                }
                .to_owned(),
                checks(row),
                row.author.clone().unwrap_or_else(|| "ghost".to_owned()),
                if row.attention.is_empty() {
                    "-".to_owned()
                } else {
                    row.attention
                        .iter()
                        .map(|reason| plain(reason))
                        .collect::<Vec<_>>()
                        .join(", ")
                },
                match &row.workspace {
                    Some(name) => format!("{} (in {name})", row.title),
                    None => row.title.clone(),
                },
            ]);
        }
        table(&lines);
    })
}

/// The open pull requests of one project, or why there are none to read.
fn ask(
    gh: &Gh,
    git: &yardsort_core::git::Git,
    project: &ProjectRow,
) -> Result<OpenPullRequests, String> {
    let root = Path::new(&project.root_path);
    if !root.is_dir() {
        return Err(format!("its folder is missing, {}", root.display()));
    }
    match coverage(forge::repo_at(git, root).as_ref()) {
        Coverage::Covered(host) => {
            Ok(gh.open_pull_requests(root, host.as_deref(), CAP, PAGE_LIMIT))
        }
        Coverage::NoRemote => {
            Err("no remote on GitHub, so it has no pull requests to read.".into())
        }
        Coverage::OtherForge(_) => {
            Err("not on GitHub; pull requests are read for GitHub only.".into())
        }
    }
}

fn row_of(
    project: &ProjectRow,
    workspaces: &[WorkspaceRow],
    pr: PullRequest,
    viewer: Option<String>,
) -> Row {
    let details = pr.details.as_ref();
    let workspace = workspaces
        .iter()
        .find(|w| {
            w.project_id == project.id
                && !w.archived
                && !w.forgotten
                && (w.branch.as_deref() == Some(pr.branch.as_str())
                    || w.branch.as_deref() == Some(format!("pr/{}", pr.number).as_str()))
        })
        .map(|w| w.name.clone());
    let review = decision(&pr);
    let mut attention = Vec::new();
    if pr.checks == Checks::Failing {
        attention.push("checksFailing");
    }
    if details.is_some_and(|d| d.mergeable == Mergeable::Conflicting) {
        attention.push("conflicts");
    }
    if review == Some("changesRequested") {
        attention.push("changesRequested");
    }
    if details.is_some_and(|d| {
        d.review_requests
            .iter()
            .any(|request| !request.team && same(Some(&request.name), viewer.as_deref()))
    }) {
        attention.push("awaitingYou");
    }
    if !pr.draft && details.is_some_and(|d| d.reviews.is_empty()) {
        attention.push("unreviewed");
    }
    Row {
        project: project.name.clone(),
        project_id: project.id.clone(),
        number: pr.number,
        title: pr.title.clone(),
        url: pr.url.clone(),
        branch: pr.branch.clone(),
        author: pr.author.clone(),
        draft: pr.draft,
        review,
        reviews: details.map(|d| d.reviews.clone()).unwrap_or_default(),
        review_requests: details
            .map(|d| d.review_requests.clone())
            .unwrap_or_default(),
        checks: pr.checks,
        check_counts: details.map(|d| d.check_counts),
        mergeable: details.map(|d| d.mergeable),
        updated_at: details.map(|d| d.updated_at.clone()),
        workspace,
        attention,
        viewer,
    }
}

/// The forge's own decision when it has one. A repository with no review rule has none, and
/// then the latest reviews decide: any request for changes outranks any approval. The same rule
/// as `reviewDecision` in the Pull requests view.
fn decision(pr: &PullRequest) -> Option<&'static str> {
    let details = pr.details.as_ref()?;
    match details.review.as_str() {
        "APPROVED" => return Some("approved"),
        "CHANGES_REQUESTED" => return Some("changesRequested"),
        "REVIEW_REQUIRED" => return Some("reviewRequired"),
        _ => {}
    }
    if details
        .reviews
        .iter()
        .any(|r| r.state == ReviewState::ChangesRequested)
    {
        Some("changesRequested")
    } else if details
        .reviews
        .iter()
        .any(|r| r.state == ReviewState::Approved)
    {
        Some("approved")
    } else {
        None
    }
}

fn same(a: Option<&str>, b: Option<&str>) -> bool {
    matches!((a, b), (Some(a), Some(b)) if a.eq_ignore_ascii_case(b))
}

fn lets_through(row: &Row, wanted: &Wanted) -> bool {
    if wanted.no_drafts && row.draft {
        return false;
    }
    if wanted.attention && row.attention.is_empty() {
        return false;
    }
    match wanted.author.as_deref() {
        Some("@me") if !same(row.author.as_deref(), row.viewer.as_deref()) => return false,
        Some(login) if login != "@me" && !same(row.author.as_deref(), Some(login)) => return false,
        _ => {}
    }
    let you = row.viewer.as_deref();
    let by_you = row.reviews.iter().any(|r| same(Some(&r.login), you));
    match wanted.reviews {
        None => true,
        Some(Reviews::None) => row.reviews.is_empty(),
        Some(Reviews::Required) => row.review == Some("reviewRequired"),
        Some(Reviews::Approved) => row.review == Some("approved"),
        Some(Reviews::Changes) => row.review == Some("changesRequested"),
        Some(Reviews::ByMe) => by_you,
        // Your own pull request is not one you have failed to review.
        Some(Reviews::NotByMe) => you.is_some() && !by_you && !same(row.author.as_deref(), you),
        Some(Reviews::AwaitingMe) => row
            .review_requests
            .iter()
            .any(|request| !request.team && same(Some(&request.name), you)),
    }
}

fn filtered(wanted: &Wanted) -> bool {
    wanted.reviews.is_some() || wanted.attention || wanted.author.is_some() || wanted.no_drafts
}

fn plain(reason: &str) -> &'static str {
    match reason {
        "checksFailing" => "checks failing",
        "conflicts" => "conflicts",
        "changesRequested" => "changes requested",
        "awaitingYou" => "your review asked",
        _ => "not reviewed",
    }
}

/// `✓ 12/12`, as the Pull requests view shows them, in words a terminal of any font can show.
fn checks(row: &Row) -> String {
    let Some(counts) = row.check_counts.filter(|c| c.total() > 0) else {
        return "-".to_owned();
    };
    let word = match row.checks {
        Checks::Failing => "failing",
        Checks::Running => "running",
        Checks::Passing => "passing",
        Checks::None => "-",
    };
    format!("{word} {}/{}", counts.passed, counts.total())
}

/// What `checkout` made, or found.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckedOut {
    id: String,
    name: String,
    branch: Option<String>,
    path: String,
    project: String,
    pull_request: u32,
    url: String,
    /// A workspace already had the branch, and nothing was made.
    existing: bool,
    /// Commits the pull request has that the workspace's branch does not. For a workspace that
    /// was already there it is fetched and counted again; `None` when that could not be done.
    behind: Option<u32>,
    /// The pull request comes from a fork: the branch is `pr/<number>`.
    fork: bool,
}

fn checkout(
    ys: &Yardsort,
    number: &str,
    project: Option<&str>,
    out: &Output,
) -> Result<(), Failure> {
    let number: u32 = number
        .trim()
        .trim_start_matches('#')
        .parse()
        .map_err(|_| Failure::new(format!("{number:?} is not a pull request number.")))?;
    let project = super::task::project_of(ys, project)?;
    let root = Path::new(&project.root_path);
    let gh = gh(ys)?;
    let pr = gh
        .pull_request(root, number)
        .map_err(|e| Failure::new(format!("Cannot read #{number} in {}: {e}", project.name)))?;
    if pr.state != PullRequestState::Open {
        return Err(Failure::new(format!(
            "#{number} in {} is {}, not open.",
            project.name,
            match pr.state {
                PullRequestState::Merged => "merged",
                _ => "closed",
            }
        )));
    }
    let fork = pr.details.as_ref().is_some_and(|d| d.cross_repository);
    let branch = if fork {
        format!("pr/{number}")
    } else {
        pr.branch.clone()
    };
    let existing = ys.store.workspaces()?.into_iter().find(|w| {
        w.project_id == project.id
            && !w.archived
            && !w.forgotten
            && w.branch.as_deref() == Some(branch.as_str())
    });
    let git = ys.git()?;
    let (workspace, behind, existing) = match existing {
        // Left exactly as it is, but measured: a second review of a pull request that moved on
        // would otherwise read old code without anyone knowing.
        Some(workspace) => match checkout::behind(&git, root, &pr, &branch) {
            Ok(behind) => (workspace, Some(behind), true),
            Err(e) => {
                eprintln!(
                    "Could not check whether {} is behind #{number}: {}",
                    workspace.name, e.message
                );
                (workspace, None, true)
            }
        },
        None => {
            let prepared = prepare_branch(&git, root, &pr)?;
            let worktree_root = ys.worktree_root()?;
            let workspace = Workspaces {
                env: ys.env(),
                store: &ys.store,
                git: &git,
                worktree_root: &worktree_root,
                settings: &ys.settings.workspaces,
            }
            .open_branch(&project.id, &prepared.branch)?;
            (workspace, Some(prepared.behind), false)
        }
    };
    let made = CheckedOut {
        id: workspace.id.clone(),
        name: workspace.name.clone(),
        branch: workspace.branch.clone(),
        path: workspace.path.clone(),
        project: project.name.clone(),
        pull_request: number,
        url: pr.url.clone(),
        existing,
        behind,
        fork,
    };
    out.emit(&made, || {
        if existing {
            sayln!("#{number} is already in {}", made.name);
        } else {
            sayln!("created  {}", made.name);
        }
        sayln!("  branch {}", made.branch.as_deref().unwrap_or("-"));
        sayln!("  path   {}", made.path);
        let stale = behind.unwrap_or(0);
        if stale > 0 {
            sayln!(
                "\n{} is {stale} commit{} behind the pull request. It was left as it was: bring \
                 it up to date before a review, or the review reads the old code.",
                if existing {
                    "The workspace"
                } else {
                    "The branch was already here and"
                },
                if stale == 1 { "" } else { "s" }
            );
        }
        if fork {
            sayln!(
                "\nIt comes from a fork, so its branch follows the pull request. The built-in \
                 code review finds a pull request by its branch and cannot review this one; \
                 start a reviewer in the workspace instead."
            );
        } else {
            sayln!(
                "\nTo have an agent review it:\n  ys workflow run code-review --workspace {}",
                made.name
            );
        }
    })
}

fn gh(ys: &Yardsort) -> Result<Gh, Failure> {
    Gh::find(ys.env()).ok_or_else(|| {
        Failure::new(
            "Pull requests are read with the GitHub CLI, and `gh` is not installed or not on \
             PATH. Get it from https://cli.github.com, then run `gh auth login`.",
        )
    })
}
