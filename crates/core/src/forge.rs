//! The forge: where a pushed branch becomes a pull request, and where CI says what it thinks.
//!
//! Two levels, because only one of them can be relied on:
//!
//! - **The remote URL** is always there. Parsed into a host, an owner and a repository name it
//!   gives a *compare* page — a link that opens the forge's own "open a pull request" form with
//!   the branches already filled in. No account, no token, no extra program: this is the floor,
//!   and it works for GitHub, GitLab, Bitbucket and the Gitea family.
//! - **[`gh`](Gh)**, when the user has it and has logged in, does the rest: it opens the pull
//!   request without leaving Yardsort, and it can say what the pull request and its checks are
//!   doing afterwards.
//!
//! Yardsort holds no forge credentials of its own. `gh` already keeps the user's, in the way
//! they expect and can revoke; a second copy in our credential store would be one more secret to
//! leak. It follows that everything here degrades to the link when `gh` is absent or logged out,
//! and that is a supported state rather than an error to report.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::env::ShellEnv;
use crate::git::HeadHistory;
use crate::program::Program;

#[derive(Debug, thiserror::Error)]
pub enum ForgeError {
    #[error("gh is not installed, or not on PATH")]
    NotInstalled,
    #[error("`gh {command}` failed: {stderr}")]
    Failed { command: String, stderr: String },
    #[error("could not read what gh printed: {0}")]
    Unreadable(String),
    #[error("could not run gh: {0}")]
    Io(#[from] std::io::Error),
}

impl ForgeError {
    /// The stable code the webview switches on.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotInstalled => "gh_not_installed",
            Self::Failed { .. } => "gh_failed",
            Self::Unreadable(_) => "gh_unreadable",
            Self::Io(_) => "gh_io",
        }
    }

    /// Whether this is `gh` saying "log in first" rather than anything being broken.
    pub fn is_logged_out(&self) -> bool {
        match self {
            Self::Failed { stderr, .. } => {
                let said = stderr.to_ascii_lowercase();
                said.contains("gh auth login")
                    || said.contains("authentication required")
                    || said.contains("not logged into")
            }
            _ => false,
        }
    }
}

pub type ForgeResult<T> = Result<T, ForgeError>;

/// Which forge a host is, as far as the shape of its URLs goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ForgeKind {
    GitHub,
    GitLab,
    Bitbucket,
    /// Gitea, Forgejo, and anything else self-hosted: assumed to speak GitHub's URL shapes,
    /// which the Gitea family does. A wrong guess costs a link that 404s, not data.
    Unknown,
}

/// A repository on a forge, as read out of a remote URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub host: String,
    /// The owner, which on GitLab may itself contain `/` for subgroups.
    pub owner: String,
    pub name: String,
    pub kind: ForgeKind,
}

impl Repo {
    /// The forge's "open a pull request" form, with both branches filled in.
    pub fn compare_url(&self, base: &str, head: &str) -> String {
        let Self {
            host, owner, name, ..
        } = self;
        let base = encode(base);
        let head = encode(head);
        match self.kind {
            ForgeKind::GitLab => format!(
                "https://{host}/{owner}/{name}/-/merge_requests/new\
                 ?merge_request%5Bsource_branch%5D={head}\
                 &merge_request%5Btarget_branch%5D={base}"
            ),
            ForgeKind::Bitbucket => {
                format!("https://{host}/{owner}/{name}/pull-requests/new?source={head}&dest={base}")
            }
            // Gitea and Forgejo use GitHub's compare path too.
            ForgeKind::GitHub | ForgeKind::Unknown => {
                format!("https://{host}/{owner}/{name}/compare/{base}...{head}?expand=1")
            }
        }
    }
}

/// Percent-encode the characters a branch name may legally contain that a URL may not.
/// Git already forbids most of what would matter; `#`, `?` and `%` are the ones left.
fn encode(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '%' => "%25".to_owned(),
            '#' => "%23".to_owned(),
            '?' => "%3F".to_owned(),
            '&' => "%26".to_owned(),
            ' ' => "%20".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

fn kind_of(host: &str) -> ForgeKind {
    let host = host.to_ascii_lowercase();
    if host == "github.com" || host.starts_with("github.") || host.ends_with(".github.com") {
        ForgeKind::GitHub
    } else if host.contains("gitlab") {
        ForgeKind::GitLab
    } else if host.contains("bitbucket") {
        ForgeKind::Bitbucket
    } else {
        ForgeKind::Unknown
    }
}

/// The remote a push from `root` would go to, read as a repository on a forge. `None` with no
/// remote, and for one that is a path on disk.
pub fn repo_at(git: &crate::git::Git, root: &Path) -> Option<Repo> {
    let remote = git.push_remote(root).ok()??;
    let url = git.remote_url(root, &remote).ok()??;
    parse_remote(&url)
}

/// Read a git remote URL as a repository on a forge.
///
/// Handles the three shapes git hands out — `git@host:owner/repo.git`,
/// `ssh://git@host/owner/repo.git` and `https://host/owner/repo.git` — and returns `None` for
/// anything else, a local path most of all.
pub fn parse_remote(url: &str) -> Option<Repo> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url.split_once("://").map(|(_, rest)| rest) {
        // scheme://[user@]host[:port]/owner/repo
        let rest = rest.split_once('@').map_or(rest, |(_, rest)| rest);
        let (authority, path) = rest.split_once('/')?;
        (
            authority.split_once(':').map_or(authority, |(h, _)| h),
            path,
        )
    } else {
        // [user@]host:owner/repo — git's scp-like form. A Windows path (`C:\…`) has no `@`
        // and no `/`, so it falls out at the split below rather than being taken for a host.
        let rest = url.split_once('@').map_or(url, |(_, rest)| rest);
        let (host, path) = rest.split_once(':')?;
        (host, path)
    };

    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.rsplit_once('/')?;
    if host.is_empty() || owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(Repo {
        host: host.to_owned(),
        owner: owner.trim_matches('/').to_owned(),
        name: name.to_owned(),
        kind: kind_of(host),
    })
}

/// What CI says about a pull request's head commit, rolled up into the one thing a row can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Checks {
    /// Nothing is configured, or nothing has reported yet.
    None,
    Running,
    Passing,
    Failing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PullRequestState {
    Open,
    Merged,
    Closed,
}

/// How a pull request's checks divide up: what a row in a list shows as _12/12_.
///
/// The one place a check's state is read, so the verdict on a workspace's badge and the numbers
/// in the pull request list cannot disagree: [`Checks`] is worked out from these.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CheckCounts {
    pub passed: u32,
    pub failed: u32,
    /// Queued, in progress, or waiting on something: not finished either way.
    pub running: u32,
}

impl CheckCounts {
    pub fn total(&self) -> u32 {
        self.passed + self.failed + self.running
    }

    /// One failure outranks everything — a green summary hiding a red check is the one answer
    /// that would make this worse than not showing it at all. Anything still running outranks
    /// success, so "passing" always means *finished* and passing.
    pub fn verdict(&self) -> Checks {
        if self.failed > 0 {
            Checks::Failing
        } else if self.running > 0 {
            Checks::Running
        } else if self.passed > 0 {
            Checks::Passing
        } else {
            Checks::None
        }
    }

    /// Count `count` checks that reported `verdict`. A cancelled check is a failure; a skipped
    /// or neutral one passes, once it has finished.
    fn add(&mut self, verdict: &str, finished: bool, count: u32) {
        match verdict.to_ascii_uppercase().as_str() {
            "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED"
            | "STARTUP_FAILURE" => self.failed += count,
            "SUCCESS" | "NEUTRAL" | "SKIPPED" if finished => self.passed += count,
            _ => self.running += count,
        }
    }
}

/// Someone a review was asked of and who has not answered yet: a person, or a team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    /// A login, or a team's slug.
    pub name: String,
    pub team: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
}

/// A reviewer's latest word on a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestReview {
    pub login: String,
    pub state: ReviewState,
}

/// A pull request, as much of it as a workspace row, the panel and the pull request list need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u32,
    pub url: String,
    pub title: String,
    /// The branch it would merge, which is how a workspace finds its own.
    pub branch: String,
    pub state: PullRequestState,
    pub draft: bool,
    pub checks: Checks,
    pub details: Option<PullRequestDetails>,
    /// Who opened it, by login. `None` for an account that no longer exists.
    pub author: Option<String>,
    /// When it was opened, in epoch milliseconds. A row shows its age from it, and it is how an
    /// attempt is matched to the pull request opened during its life (see `crate::outcomes`).
    /// A whole number of milliseconds fits a JavaScript number with room to spare, which is
    /// what the window is told it is.
    #[specta(type = Option<f64>)]
    pub created_at: Option<i64>,
}

/// Detail shared by the sidebar card and toolbar, from the same project-wide query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestDetails {
    pub base: String,
    pub head_oid: String,
    /// The commit of the base branch the forge measures the pull request against. For one
    /// merged long ago it is the base as it was then, which is what still gives its diff back.
    pub base_oid: String,
    pub additions: u32,
    pub deletions: u32,
    pub review: String,
    pub updated_at: String,
    /// Each check by name. Empty when the pull request came from the list of open ones, which
    /// asks the forge for [`check_counts`](Self::check_counts) alone: see [`Gh::open_page`].
    pub checks: Vec<PullRequestCheck>,
    pub check_counts: CheckCounts,
    /// Whether it merges cleanly into its base, as the forge last worked it out.
    pub mergeable: Mergeable,
    /// Reviews asked for and not given yet.
    pub review_requests: Vec<ReviewRequest>,
    /// Each reviewer's latest review.
    pub reviews: Vec<PullRequestReview>,
    /// Its branch lives in a fork, not in the repository it would merge into.
    pub cross_repository: bool,
}

/// GitHub's answer to "can this be merged without conflicts". It works it out lazily, after a
/// push to either side, so `Unknown` is an ordinary answer for a while and not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Mergeable {
    Mergeable,
    Conflicting,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestCheck {
    pub name: String,
    pub state: Checks,
    /// The workflow a check run belongs to — `CI` for `CI / test`. A commit status has none.
    pub workflow: Option<String>,
    /// Where the forge shows this run: its log, or whatever a commit status points at.
    pub url: Option<String>,
}

/// One pull request in full, asked about by itself: what the Pull requests view's Summary
/// shows. The list has most of [`PullRequest`] already; this adds the words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestSummary {
    /// As the forge has it this moment, with every check by name and link.
    pub pull_request: PullRequest,
    /// The description, as its author wrote it: Markdown. Empty when there is none.
    pub body: String,
    pub changed_files: u32,
    /// The conversation: comments and reviews as they were given, oldest first. Comments made
    /// on particular lines are not among them — `gh pr view` does not return those.
    pub posts: Vec<PullRequestPost>,
}

/// Which side of a diff a comment on a line is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DiffSide {
    /// The file as it was: a removed or unchanged line of the old version.
    Left,
    /// The file as the pull request has it.
    Right,
}

impl DiffSide {
    fn as_api(self) -> &'static str {
        match self {
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
        }
    }
}

/// A comment on particular lines of a pull request's diff, as the forge's API lists them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LineComment {
    /// The forge's id, as text: it does not fit a JavaScript number.
    pub id: String,
    pub path: String,
    /// The line it is on now, in the file as the pull request has it (or as it was, for
    /// [`DiffSide::Left`]). `None` when the diff has moved on from under it, or when it is about
    /// the file as a whole.
    pub line: Option<u32>,
    /// The first line of a range, when it is on several.
    pub start_line: Option<u32>,
    pub side: DiffSide,
    /// The line it was made on, which is all that is left of where an outdated one went.
    pub original_line: Option<u32>,
    /// The diff has changed since and the forge no longer places it on a line.
    pub outdated: bool,
    /// About the file as a whole, not a line of it.
    pub whole_file: bool,
    pub author: Option<String>,
    /// In epoch milliseconds.
    #[specta(type = f64)]
    pub at: i64,
    /// Markdown.
    pub body: String,
    pub url: Option<String>,
    /// The comment this one answers, when it is a reply.
    pub in_reply_to: Option<String>,
}

/// Where a comment on lines goes: what the forge needs to place it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LinePlace {
    /// The head commit the lines are of, as the diff was read.
    pub commit: String,
    pub path: String,
    pub side: DiffSide,
    /// The last line, or the only one.
    pub line: u32,
    /// The first line of a range.
    pub start_line: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PullRequestPostKind {
    Comment,
    Review,
}

/// A comment on a pull request's conversation, or a review of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestPost {
    pub kind: PullRequestPostKind,
    /// By login. `None` for an account that no longer exists.
    pub author: Option<String>,
    /// When it was posted, in epoch milliseconds.
    #[specta(type = f64)]
    pub at: i64,
    /// Markdown. Often empty for a review, which may be a verdict and nothing else, or only
    /// comments on lines.
    pub body: String,
    /// Where to read it on the forge. A review has no address of its own, and gets the pull
    /// request's.
    pub url: Option<String>,
    /// What a review concluded. `None` for a comment.
    pub review: Option<ReviewState>,
    /// The forge has hidden it — as spam, off-topic, outdated — and says why. Its words are
    /// then not shown here either.
    pub hidden: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MergeMethod {
    Squash,
    Merge,
    Rebase,
}

/// Branches may be reused. An open PR wins; otherwise show the newest number,
/// independently of the order returned by the forge.
pub fn pull_request_for<'a>(requests: &'a [PullRequest], branch: &str) -> Option<&'a PullRequest> {
    requests
        .iter()
        .filter(|pr| pr.branch == branch)
        .max_by_key(|pr| (pr.state == PullRequestState::Open, pr.number))
}

/// The pull requests that came out of one workspace, by number, newest first.
///
/// `branch` is what the workspace has checked out now, and `own` the branch Yardsort made it
/// with. A pull request belongs to the workspace when its head is a commit made there, or when
/// its branch was checked out there before it was opened. The time matters because branch
/// names are reused: an older workspace's pull request on the same name is not this one's.
///
/// The one [`pull_request_for`] picks for `branch` is always included, opened before or not:
/// that is what the workspace has always shown, an adopted worktree's pull request included.
pub fn pull_requests_from(
    requests: &[PullRequest],
    branch: Option<&str>,
    own: Option<&str>,
    history: &HeadHistory,
) -> Vec<u32> {
    let checked_out_at = |name: &str| {
        let first = history
            .branches
            .iter()
            .find(|(checked_out, _)| checked_out == name)
            .map(|(_, at)| *at);
        // The branch Yardsort made the worktree on has been there since its birth, which no
        // `checkout` line says: nothing was checked out to get there. Any other branch, the
        // one checked out now included, has been there since its first `checkout` line.
        let from_birth = (Some(name) == own).then_some(history.since).flatten();
        match (first, from_birth) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    };
    let current = branch.and_then(|branch| pull_request_for(requests, branch));
    let mut found: Vec<&PullRequest> = requests
        .iter()
        .filter(|pr| {
            Some(pr.number) == current.map(|c| c.number)
                || pr
                    .details
                    .as_ref()
                    .is_some_and(|d| history.commits.contains(&d.head_oid))
                || match (checked_out_at(&pr.branch), pr.created_at) {
                    (Some(at), Some(opened)) => opened + CLOCK_SKEW_MS >= at,
                    _ => false,
                }
        })
        .collect();
    found.sort_by_key(|pr| std::cmp::Reverse(pr.number));
    found.dedup_by_key(|pr| pr.number);
    found.into_iter().map(|pr| pr.number).collect()
}

/// How far the forge's clock and this machine's may disagree. A pull request opened a moment
/// after its branch was checked out must not look as if it came first.
const CLOCK_SKEW_MS: i64 = 2 * 60 * 1000;

/// A review or a comment on a pull request: what a workflow waiting on one looks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrPost {
    pub kind: PrPostKind,
    /// When it was posted, in epoch milliseconds.
    pub at_ms: i64,
    /// Where to read it, when the forge says.
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrPostKind {
    /// A review: approve, comment or request changes, with any line comments in it.
    Review,
    /// A comment on the pull request's conversation.
    Comment,
}

/// Reviews and comments out of `gh pr view --json reviews,comments,url`. A review that is still
/// pending — started, not submitted — has no time and is not a post yet.
pub fn pr_posts(json: &str) -> ForgeResult<Vec<PrPost>> {
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|e| ForgeError::Unreadable(e.to_string()))?;
    let pr_url = parsed.get("url").and_then(|v| v.as_str());
    let list = |key: &str| {
        parsed
            .get(key)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    };
    let mut posts = Vec::new();
    for review in list("reviews") {
        let at = review.get("submittedAt").and_then(|t| t.as_str());
        let Some(at_ms) = at.and_then(crate::activity::iso_to_ms) else {
            continue;
        };
        posts.push(PrPost {
            kind: PrPostKind::Review,
            at_ms,
            url: pr_url.map(str::to_owned),
        });
    }
    for comment in list("comments") {
        let at = comment.get("createdAt").and_then(|t| t.as_str());
        let Some(at_ms) = at.and_then(crate::activity::iso_to_ms) else {
            continue;
        };
        posts.push(PrPost {
            kind: PrPostKind::Comment,
            at_ms,
            url: comment
                .get("url")
                .and_then(|v| v.as_str())
                .or(pr_url)
                .map(str::to_owned),
        });
    }
    posts.sort_by_key(|post| post.at_ms);
    Ok(posts)
}

/// The `gh` command line, found on the user's `PATH`.
pub struct Gh {
    program: Program,
}

impl Gh {
    pub fn find(env: &ShellEnv) -> Option<Self> {
        Program::find(env, "gh").map(|program| Self { program })
    }

    /// The `gh` at `path`, rather than the one on `PATH`.
    pub fn at(path: std::path::PathBuf, env: &ShellEnv) -> Self {
        Self {
            program: Program::at(path, env),
        }
    }

    pub fn path(&self) -> &Path {
        self.program.path()
    }

    fn command(&self, cwd: &Path, args: &[&str]) -> std::process::Command {
        let mut command = self.program.command(cwd);
        command
            .args(args)
            // gh dresses its output up for a terminal otherwise, and asks questions.
            .env("GH_PROMPT_DISABLED", "1")
            .env("NO_COLOR", "1")
            .env("CLICOLOR", "0");
        command
    }

    pub(crate) fn run(&self, cwd: &Path, args: &[&str]) -> ForgeResult<String> {
        let output = self.command(cwd, args).output()?;
        Self::answer(args, &output)
    }

    /// As [`Gh::run`], but `gh` gets `limit` to answer and is stopped after that, so nothing
    /// waiting on it waits for ever on a network that went away.
    pub(crate) fn run_within(
        &self,
        cwd: &Path,
        args: &[&str],
        limit: Duration,
    ) -> ForgeResult<String> {
        use std::io::Read;
        let mut child = self
            .command(cwd, args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        // Read both pipes as the program writes, or a full pipe would stop it before it ends.
        let drain = |pipe: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                if let Some(mut pipe) = pipe {
                    let _ = pipe.read_to_end(&mut bytes);
                }
                bytes
            })
        };
        let stdout = drain(
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        let stderr = drain(
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        );
        let deadline = std::time::Instant::now() + limit;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ForgeError::Failed {
                    command: args.join(" "),
                    stderr: format!("no answer within {} s", limit.as_secs()),
                });
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let output = std::process::Output {
            status,
            stdout: stdout.join().unwrap_or_default(),
            stderr: stderr.join().unwrap_or_default(),
        };
        Self::answer(args, &output)
    }

    fn answer(args: &[&str], output: &std::process::Output) -> ForgeResult<String> {
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        } else {
            Err(ForgeError::Failed {
                command: args.join(" "),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }

    /// Every pull request `gh` will tell us about for the repository at `root`, newest first.
    ///
    /// One call answers for every workspace in the project, which is why it is a list and not a
    /// lookup per branch: a project with a dozen workspaces would otherwise be a dozen round
    /// trips every time the window came back into focus.
    pub fn pull_requests(&self, root: &Path, limit: u32) -> ForgeResult<Vec<PullRequest>> {
        let limit = limit.to_string();
        let out = self.run(
            root,
            &[
                "pr",
                "list",
                "--state",
                "all",
                "--limit",
                &limit,
                "--json",
                PULL_REQUEST_FIELDS,
            ],
        )?;
        pull_requests(&out)
    }

    /// The open pull requests whose head is `branch`, asked of the forge by branch.
    ///
    /// Where [`Gh::pull_requests`] is a bounded overview for the sidebar, this is the answer to
    /// "does this branch have one": an older pull request behind fifty newer ones is still
    /// found.
    pub fn open_pull_requests_for(
        &self,
        root: &Path,
        branch: &str,
    ) -> ForgeResult<Vec<PullRequest>> {
        let out = self.run(
            root,
            &[
                "pr",
                "list",
                "--head",
                branch,
                "--state",
                "open",
                "--limit",
                "20",
                "--json",
                PULL_REQUEST_FIELDS,
            ],
        )?;
        pull_requests(&out)
    }

    /// Pull request `number`, asked of the forge now rather than out of the project's list:
    /// for an action that must not rest on an answer half a minute old.
    pub fn pull_request(&self, root: &Path, number: u32) -> ForgeResult<PullRequest> {
        let out = self.run(root, &Self::pull_request_args(&number.to_string()))?;
        Self::read_pull_request(&out)
    }

    /// [`Gh::pull_request`], stopped after `limit`: for a background poll, which must not hang
    /// on a network that went away.
    pub fn pull_request_within(
        &self,
        root: &Path,
        number: u32,
        limit: Duration,
    ) -> ForgeResult<PullRequest> {
        let number = number.to_string();
        let out = self.run_within(root, &Self::pull_request_args(&number), limit)?;
        Self::read_pull_request(&out)
    }

    fn pull_request_args(number: &str) -> [&str; 5] {
        ["pr", "view", number, "--json", PULL_REQUEST_FIELDS]
    }

    fn read_pull_request(out: &str) -> ForgeResult<PullRequest> {
        let row: serde_json::Value =
            serde_json::from_str(out).map_err(|e| ForgeError::Unreadable(e.to_string()))?;
        pull_request(&row)
            .ok_or_else(|| ForgeError::Unreadable("expected a pull request".to_owned()))
    }

    /// Pull request `number` in full — its description, every check with its link, and the
    /// conversation — asked of the forge now. `limit` is how long `gh` may take.
    pub fn pull_request_summary(
        &self,
        root: &Path,
        number: u32,
        limit: Duration,
    ) -> ForgeResult<PullRequestSummary> {
        let fields = format!("{PULL_REQUEST_FIELDS},body,comments,reviews,changedFiles");
        let out = self.run_within(
            root,
            &["pr", "view", &number.to_string(), "--json", &fields],
            limit,
        )?;
        pull_request_summary(&out)
    }

    /// The reviews and comments on pull request `number`, oldest first.
    /// `limit` is how long `gh` may take; after that it is stopped and this is an error.
    pub fn pr_posts(&self, root: &Path, number: u32, limit: Duration) -> ForgeResult<Vec<PrPost>> {
        let out = self.run_within(
            root,
            &[
                "pr",
                "view",
                &number.to_string(),
                "--json",
                "url,reviews,comments",
            ],
            limit,
        )?;
        pr_posts(&out)
    }

    /// Merge exactly the head the user confirmed. Never delete branches or bypass protections.
    pub fn merge_pull_request(
        &self,
        root: &Path,
        number: u32,
        head: &str,
        method: MergeMethod,
    ) -> ForgeResult<()> {
        let flag = match method {
            MergeMethod::Squash => "--squash",
            MergeMethod::Merge => "--merge",
            MergeMethod::Rebase => "--rebase",
        };
        self.run(
            root,
            &[
                "pr",
                "merge",
                &number.to_string(),
                flag,
                "--match-head-commit",
                head,
            ],
        )?;
        Ok(())
    }

    /// Open a pull request for the branch checked out at `root`, and return its URL.
    pub fn create_pull_request(
        &self,
        root: &Path,
        base: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> ForgeResult<String> {
        let mut args = vec![
            "pr", "create", "--base", base, "--title", title, "--body", body,
        ];
        if draft {
            args.push("--draft");
        }
        let out = self.run(root, &args)?;
        // gh prints the URL on the last line, after whatever else it felt like saying.
        Ok(out
            .lines()
            .rev()
            .find(|line| line.starts_with("http"))
            .unwrap_or(&out)
            .trim()
            .to_owned())
    }

    /// Run `gh` with `stdin` on its standard input: for text that is the user's words, which
    /// never go in an argument — an argument list has a length Windows caps, and a shell is not
    /// involved either way, but a `--body` of a few thousand characters is a thing to avoid.
    pub(crate) fn run_with_stdin(
        &self,
        cwd: &Path,
        args: &[&str],
        stdin: &str,
    ) -> ForgeResult<String> {
        use std::io::Write;
        let mut child = self
            .command(cwd, args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        if let Some(mut pipe) = child.stdin.take() {
            // A program that has already failed may have closed its end; what matters then is
            // what it says, which `wait_with_output` reads.
            let _ = pipe.write_all(stdin.as_bytes());
        }
        let output = child.wait_with_output()?;
        Self::answer(args, &output)
    }

    /// Post `body` as a comment on pull request `number`'s conversation.
    pub fn comment(&self, root: &Path, number: u32, body: &str) -> ForgeResult<()> {
        self.run_with_stdin(
            root,
            &["pr", "comment", &number.to_string(), "--body-file", "-"],
            body,
        )?;
        Ok(())
    }

    /// Every comment made on lines of pull request `number`'s diff, oldest first. `gh pr view`
    /// does not return these; the forge's API does, a page at a time.
    pub fn line_comments(&self, root: &Path, number: u32) -> ForgeResult<Vec<LineComment>> {
        let endpoint = format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments");
        let out = self.run(root, &["api", "--paginate", "--slurp", &endpoint])?;
        line_comments(&out)
    }

    /// Post `body` as a comment on the lines `place` names, on pull request `number`.
    pub fn line_comment(
        &self,
        root: &Path,
        number: u32,
        place: &LinePlace,
        body: &str,
    ) -> ForgeResult<()> {
        let endpoint = format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments");
        let commit = format!("commit_id={}", place.commit);
        let path = format!("path={}", place.path);
        let side = format!("side={}", place.side.as_api());
        let line = format!("line={}", place.line);
        // `-f` sends a string as it is; `-F` turns a number into one. `body=@-` is read from
        // standard input.
        let mut args = vec![
            "api", "-X", "POST", &endpoint, "-F", "body=@-", "-f", &commit, "-f", &path, "-f",
            &side, "-F", &line,
        ];
        let start = place.start_line.map(|start| format!("start_line={start}"));
        let start_side = format!("start_side={}", place.side.as_api());
        if let Some(start) = &start {
            args.extend(["-F", start, "-f", &start_side]);
        }
        self.run_with_stdin(root, &args, body)?;
        Ok(())
    }

    /// Close pull request `number` without merging it. Its branch is left where it is, and no
    /// comment is posted.
    pub fn close_pull_request(&self, root: &Path, number: u32) -> ForgeResult<()> {
        self.run(root, &["pr", "close", &number.to_string()])?;
        Ok(())
    }

    /// Open a closed pull request again.
    pub fn reopen_pull_request(&self, root: &Path, number: u32) -> ForgeResult<()> {
        self.run(root, &["pr", "reopen", &number.to_string()])?;
        Ok(())
    }

    /// One page of the repository's open pull requests, most recently updated first: the page
    /// after the cursor `after`, or the first.
    ///
    /// Our own query through `gh api graphql` rather than `gh pr list`, which asks for every
    /// check of every pull request a hundred at a time — more than GitHub answers in its ten
    /// seconds on a repository with real CI. This asks for fifty and for the checks *counted*.
    /// The measurements are in `docs/design/22-pull-requests.md`.
    ///
    /// `{owner}` and `{repo}` are `gh`'s own placeholders, so the repository is the one
    /// `gh pr list` would have picked in the same folder. `host` is needed for a GitHub that is
    /// not github.com: `gh api` asks its default host otherwise, whatever the remote says.
    pub fn open_page(
        &self,
        root: &Path,
        host: Option<&str>,
        after: Option<&str>,
        limit: Duration,
    ) -> ForgeResult<OpenPage> {
        let after = after.map(|cursor| format!("after={cursor}"));
        let mut extra = Vec::new();
        if let Some(after) = &after {
            extra.extend(["-f", after.as_str()]);
        }
        let out = self.graphql(root, host, OPEN_QUERY, &extra, limit)?;
        open_page(&out)
    }

    /// Ask GitHub a question of our own through `gh api graphql`, about the repository of the
    /// folder `root`. `limit` is how long `gh` may take.
    ///
    /// `{owner}` and `{repo}` are `gh`'s own placeholders, filled in from the folder's remote,
    /// so the repository is the one `gh pr list` would have picked there — in a clone of a
    /// fork, the parent. `host` is needed for a GitHub that is not github.com: `gh api` asks
    /// its default host otherwise, whatever the remote says. `extra` is further arguments, for
    /// the query's other variables.
    pub(crate) fn graphql(
        &self,
        root: &Path,
        host: Option<&str>,
        query: &str,
        extra: &[&str],
        limit: Duration,
    ) -> ForgeResult<String> {
        let query = format!("query={query}");
        let mut args = vec!["api", "graphql"];
        if let Some(host) = host {
            args.extend(["--hostname", host]);
        }
        // `-F` fills the placeholders in; `-f` sends the rest exactly as written.
        args.extend(["-F", "owner={owner}", "-F", "name={repo}", "-f", &query]);
        args.extend(extra);
        self.run_within(root, &args, limit)
            .map_err(|error| match error {
                // The command is the whole query otherwise, which helps nobody read the reason.
                ForgeError::Failed { stderr, .. } => ForgeError::Failed {
                    command: "api graphql".to_owned(),
                    stderr,
                },
                other => other,
            })
    }

    /// Every open pull request, a page at a time, up to `cap`.
    ///
    /// Never an `Err`: a page that fails ends the reading and says why, and the pages before it
    /// are kept. Half a list with the reason beside it is worth more than none.
    pub fn open_pull_requests(
        &self,
        root: &Path,
        host: Option<&str>,
        cap: usize,
        limit: Duration,
    ) -> OpenPullRequests {
        let mut found = OpenPullRequests::default();
        let mut after: Option<String> = None;
        loop {
            let page = match self.open_page(root, host, after.as_deref(), limit) {
                Ok(page) => page,
                Err(error) => {
                    found.logged_out = error.is_logged_out();
                    found.problem = Some(error.to_string());
                    break;
                }
            };
            found.answered = true;
            found.total = page.total.or(found.total);
            found.viewer = page.viewer.or(found.viewer);
            for pr in page.pull_requests {
                // Ordered by when they last changed, so one that changed between two pages can
                // turn up on both.
                if !found
                    .pull_requests
                    .iter()
                    .any(|seen| seen.number == pr.number)
                {
                    found.pull_requests.push(pr);
                }
            }
            if found.pull_requests.len() >= cap {
                found.pull_requests.truncate(cap);
                break;
            }
            match page.next {
                Some(cursor) => after = Some(cursor),
                None => break,
            }
        }
        found
    }
}

/// Read what `gh pr view --json` printed for [`Gh::pull_request_summary`].
pub fn pull_request_summary(json: &str) -> ForgeResult<PullRequestSummary> {
    let row: serde_json::Value =
        serde_json::from_str(json).map_err(|e| ForgeError::Unreadable(e.to_string()))?;
    let pull_request = pull_request(&row)
        .ok_or_else(|| ForgeError::Unreadable("expected a pull request".to_owned()))?;
    let author = |post: &serde_json::Value| {
        post.get("author")
            .and_then(|author| author.get("login"))
            .and_then(|login| login.as_str())
            .filter(|login| !login.is_empty())
            .map(str::to_owned)
    };
    let at = |post: &serde_json::Value, key: &str| {
        post.get(key)
            .and_then(|t| t.as_str())
            .and_then(crate::activity::iso_to_ms)
    };

    let mut posts = Vec::new();
    for comment in items(row.get("comments")) {
        let Some(at) = at(comment, "createdAt") else {
            continue;
        };
        let hidden = comment
            .get("isMinimized")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            .then(|| string_field(comment, "minimizedReason").to_ascii_lowercase());
        posts.push(PullRequestPost {
            kind: PullRequestPostKind::Comment,
            author: author(comment),
            at,
            // What the forge hid is not shown by another door.
            body: match hidden {
                Some(_) => String::new(),
                None => string_field(comment, "body"),
            },
            url: comment
                .get("url")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            review: None,
            hidden,
        });
    }
    for given in items(row.get("reviews")) {
        // A review still being written has no time and no verdict: it has not been given.
        let (Some(at), Some(state)) = (
            at(given, "submittedAt"),
            review(given).map(|review| review.state),
        ) else {
            continue;
        };
        posts.push(PullRequestPost {
            kind: PullRequestPostKind::Review,
            author: author(given),
            at,
            body: string_field(given, "body"),
            url: Some(pull_request.url.clone()),
            review: Some(state),
            hidden: None,
        });
    }
    posts.sort_by_key(|post| post.at);

    Ok(PullRequestSummary {
        body: string_field(&row, "body"),
        changed_files: row
            .get("changedFiles")
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0),
        posts,
        pull_request,
    })
}

/// Read what `gh api --paginate --slurp` printed for the comments on a pull request's lines:
/// an array of pages, each an array of comments.
pub fn line_comments(json: &str) -> ForgeResult<Vec<LineComment>> {
    let pages: serde_json::Value =
        serde_json::from_str(json).map_err(|e| ForgeError::Unreadable(e.to_string()))?;
    let pages = pages
        .as_array()
        .ok_or_else(|| ForgeError::Unreadable("expected pages of comments".to_owned()))?;
    let number = |v: Option<&serde_json::Value>| {
        v.and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())
    };
    let id = |v: Option<&serde_json::Value>| v.and_then(|v| v.as_u64()).map(|v| v.to_string());
    let mut comments = Vec::new();
    for comment in pages.iter().flat_map(|page| items(Some(page))) {
        let (Some(id), Some(at)) = (
            id(comment.get("id")),
            comment
                .get("created_at")
                .and_then(|t| t.as_str())
                .and_then(crate::activity::iso_to_ms),
        ) else {
            continue;
        };
        let whole_file = comment
            .get("subject_type")
            .and_then(|v| v.as_str())
            .is_some_and(|kind| kind == "file");
        let line = number(comment.get("line"));
        comments.push(LineComment {
            id: id.clone(),
            path: string_field(comment, "path"),
            line,
            start_line: number(comment.get("start_line")),
            side: match comment.get("side").and_then(|v| v.as_str()) {
                Some("LEFT") => DiffSide::Left,
                _ => DiffSide::Right,
            },
            original_line: number(comment.get("original_line")),
            // The forge drops `position` once the diff no longer has the line.
            outdated: !whole_file && comment.get("position").is_none_or(|p| p.is_null()),
            whole_file,
            author: comment
                .get("user")
                .and_then(|user| user.get("login"))
                .and_then(|login| login.as_str())
                .map(str::to_owned),
            at,
            body: string_field(comment, "body"),
            url: comment
                .get("html_url")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            in_reply_to: match comment.get("in_reply_to_id") {
                Some(reply) if !reply.is_null() => reply.as_u64().map(|v| v.to_string()),
                _ => None,
            },
        });
    }
    comments.sort_by_key(|comment| comment.at);
    Ok(comments)
}

/// The open pull requests of a repository, fifty to the page. The fields are the ones
/// [`PULL_REQUEST_FIELDS`] asks `gh` for, except the checks: counted by state, not listed.
const OPEN_QUERY: &str = "query($owner:String!,$name:String!,$after:String){\
repository(owner:$owner,name:$name){\
pullRequests(states:OPEN,first:50,after:$after,orderBy:{field:UPDATED_AT,direction:DESC}){\
totalCount pageInfo{hasNextPage endCursor} \
nodes{number url title isDraft state createdAt updatedAt headRefName headRefOid baseRefName \
baseRefOid isCrossRepository additions deletions mergeable reviewDecision author{login} \
reviewRequests(first:10){nodes{requestedReviewer{__typename ...on User{login} ...on Team{slug}}}} \
latestReviews(first:10){nodes{state author{login}}} \
commits(last:1){nodes{commit{statusCheckRollup{contexts(first:1){\
checkRunCountsByState{state count} statusContextCountsByState{state count}}}}}}}}} \
viewer{login}}";

/// One page of [`Gh::open_page`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenPage {
    pub pull_requests: Vec<PullRequest>,
    /// How many open pull requests the repository has, on every page.
    pub total: Option<u32>,
    /// The cursor to ask for the next page with. `None` on the last one.
    pub next: Option<String>,
    /// Who `gh` is logged in as.
    pub viewer: Option<String>,
}

/// What [`Gh::open_pull_requests`] managed to read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenPullRequests {
    pub pull_requests: Vec<PullRequest>,
    pub total: Option<u32>,
    pub viewer: Option<String>,
    /// At least one page arrived. When none did, whatever was known before is still the best
    /// there is.
    pub answered: bool,
    /// Why the reading stopped short, when it did.
    pub problem: Option<String>,
    pub logged_out: bool,
}

/// Read one answer to [`OPEN_QUERY`].
pub fn open_page(json: &str) -> ForgeResult<OpenPage> {
    let unreadable = |what: &str| ForgeError::Unreadable(what.to_owned());
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|e| ForgeError::Unreadable(e.to_string()))?;
    let data = parsed
        .get("data")
        .ok_or_else(|| unreadable("expected data"))?;
    let list = data
        .get("repository")
        .and_then(|repository| repository.get("pullRequests"))
        .filter(|list| !list.is_null())
        .ok_or_else(|| unreadable("expected a repository's pull requests"))?;
    let nodes = |value: Option<&serde_json::Value>| -> Vec<serde_json::Value> {
        value
            .and_then(|v| v.get("nodes"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    };
    let pull_requests = nodes(Some(list))
        .iter()
        .filter_map(|node| {
            let requests: Vec<serde_json::Value> = nodes(node.get("reviewRequests"))
                .iter()
                .filter_map(|request| request.get("requestedReviewer").cloned())
                .collect();
            let commits = nodes(node.get("commits"));
            let contexts = commits
                .first()
                .and_then(|node| node.get("commit"))
                .and_then(|commit| commit.get("statusCheckRollup"))
                .and_then(|rollup| rollup.get("contexts"));
            let mut counts = CheckCounts::default();
            for key in ["checkRunCountsByState", "statusContextCountsByState"] {
                for by_state in items(contexts.and_then(|contexts| contexts.get(key))) {
                    let state = by_state.get("state").and_then(|s| s.as_str());
                    let count = by_state
                        .get("count")
                        .and_then(|c| c.as_u64())
                        .and_then(|c| u32::try_from(c).ok());
                    if let (Some(state), Some(count)) = (state, count) {
                        counts.add(state, true, count);
                    }
                }
            }
            read(
                node,
                vec![],
                counts,
                &requests,
                &nodes(node.get("latestReviews")),
            )
        })
        .collect();
    let page = list.get("pageInfo");
    let more = page
        .and_then(|page| page.get("hasNextPage"))
        .and_then(|more| more.as_bool())
        .unwrap_or(false);
    Ok(OpenPage {
        pull_requests,
        total: list
            .get("totalCount")
            .and_then(|total| total.as_u64())
            .and_then(|total| u32::try_from(total).ok()),
        next: page
            .and_then(|page| page.get("endCursor"))
            .and_then(|cursor| cursor.as_str())
            .filter(|_| more)
            .map(str::to_owned),
        viewer: data
            .get("viewer")
            .and_then(|viewer| viewer.get("login"))
            .and_then(|login| login.as_str())
            .map(str::to_owned),
    })
}

/// What is asked of `gh` about each pull request.
const PULL_REQUEST_FIELDS: &str = "number,url,title,headRefName,state,isDraft,statusCheckRollup,\
createdAt,baseRefName,headRefOid,baseRefOid,additions,deletions,reviewDecision,updatedAt,\
mergeable,author,reviewRequests,latestReviews,isCrossRepository";

fn pull_requests(out: &str) -> ForgeResult<Vec<PullRequest>> {
    let parsed: serde_json::Value =
        serde_json::from_str(out).map_err(|e| ForgeError::Unreadable(e.to_string()))?;
    let rows = parsed
        .as_array()
        .ok_or_else(|| ForgeError::Unreadable("expected a list of pull requests".to_owned()))?;
    Ok(rows.iter().filter_map(pull_request).collect())
}

/// A pull request out of `gh pr list --json` or `gh pr view --json`: every check by name.
pub(crate) fn pull_request(row: &serde_json::Value) -> Option<PullRequest> {
    let rollup = row.get("statusCheckRollup");
    let checks = rollup
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .map(|check| {
            let text = |key: &str| {
                check
                    .get(key)
                    .and_then(|v| v.as_str())
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned)
            };
            PullRequestCheck {
                // A check run has a name; a commit status has a context instead.
                name: text("name")
                    .or_else(|| text("context"))
                    .unwrap_or_else(|| "Check".to_owned()),
                state: roll_up(Some(&serde_json::json!([check]))),
                workflow: text("workflowName"),
                url: text("detailsUrl").or_else(|| text("targetUrl")),
            }
        })
        .collect();
    read(
        row,
        checks,
        count(rollup),
        items(row.get("reviewRequests")),
        items(row.get("latestReviews")),
    )
}

/// What a row from `gh` and a node from our own query have in common, which is nearly all of
/// it. They differ in how the checks arrive and in how the two review lists are wrapped.
fn read(
    row: &serde_json::Value,
    checks: Vec<PullRequestCheck>,
    check_counts: CheckCounts,
    review_requests: &[serde_json::Value],
    reviews: &[serde_json::Value],
) -> Option<PullRequest> {
    Some(PullRequest {
        number: u32::try_from(row.get("number")?.as_u64()?).ok()?,
        url: row.get("url")?.as_str()?.to_owned(),
        title: row
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_owned(),
        branch: row.get("headRefName")?.as_str()?.to_owned(),
        state: match row.get("state").and_then(|s| s.as_str()).unwrap_or("OPEN") {
            "MERGED" => PullRequestState::Merged,
            "CLOSED" => PullRequestState::Closed,
            _ => PullRequestState::Open,
        },
        draft: row
            .get("isDraft")
            .and_then(|d| d.as_bool())
            .unwrap_or(false),
        checks: check_counts.verdict(),
        details: Some(PullRequestDetails {
            base: string_field(row, "baseRefName"),
            head_oid: string_field(row, "headRefOid"),
            base_oid: string_field(row, "baseRefOid"),
            additions: row
                .get("additions")
                .and_then(|v| v.as_u64())
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(0),
            deletions: row
                .get("deletions")
                .and_then(|v| v.as_u64())
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(0),
            review: string_field(row, "reviewDecision"),
            updated_at: string_field(row, "updatedAt"),
            checks,
            check_counts,
            mergeable: match row.get("mergeable").and_then(|v| v.as_str()) {
                Some("MERGEABLE") => Mergeable::Mergeable,
                Some("CONFLICTING") => Mergeable::Conflicting,
                _ => Mergeable::Unknown,
            },
            review_requests: review_requests.iter().filter_map(review_request).collect(),
            reviews: reviews.iter().filter_map(review).collect(),
            cross_repository: row
                .get("isCrossRepository")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }),
        author: row
            .get("author")
            .and_then(|author| author.get("login"))
            .and_then(|login| login.as_str())
            .filter(|login| !login.is_empty())
            .map(str::to_owned),
        created_at: row
            .get("createdAt")
            .and_then(|t| t.as_str())
            .and_then(crate::activity::iso_to_ms),
    })
}

fn items(list: Option<&serde_json::Value>) -> &[serde_json::Value] {
    list.and_then(|v| v.as_array()).map_or(&[], Vec::as_slice)
}

/// A person or a team a review was asked of. A team has a slug where a person has a login.
fn review_request(reviewer: &serde_json::Value) -> Option<ReviewRequest> {
    let text = |key: &str| reviewer.get(key).and_then(|v| v.as_str());
    match text("login") {
        Some(login) => Some(ReviewRequest {
            name: login.to_owned(),
            team: false,
        }),
        None => text("slug")
            .or_else(|| text("name"))
            .map(|team| ReviewRequest {
                name: team.to_owned(),
                team: true,
            }),
    }
}

/// A reviewer's latest review. One still being written has not been given, and is not one.
fn review(review: &serde_json::Value) -> Option<PullRequestReview> {
    let login = review.get("author")?.get("login")?.as_str()?;
    let state = match review.get("state")?.as_str()? {
        "APPROVED" => ReviewState::Approved,
        "CHANGES_REQUESTED" => ReviewState::ChangesRequested,
        "COMMENTED" => ReviewState::Commented,
        "DISMISSED" => ReviewState::Dismissed,
        _ => return None,
    };
    Some(PullRequestReview {
        login: login.to_owned(),
        state,
    })
}

fn string_field(row: &serde_json::Value, key: &str) -> String {
    row.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned()
}

/// Reduce every check on the head commit to one verdict: see [`CheckCounts::verdict`].
fn roll_up(rollup: Option<&serde_json::Value>) -> Checks {
    count(rollup).verdict()
}

/// Count the checks `gh` lists on the head commit, by what became of each.
fn count(rollup: Option<&serde_json::Value>) -> CheckCounts {
    let mut counts = CheckCounts::default();
    for check in rollup
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
    {
        // A check run reports `status` then `conclusion`; a commit status only has `state`.
        let status = check.get("status").and_then(|s| s.as_str());
        let verdict = check
            .get("conclusion")
            .and_then(|c| c.as_str())
            .or_else(|| check.get("state").and_then(|s| s.as_str()))
            .unwrap_or("");
        let finished = status.is_none_or(|status| status.eq_ignore_ascii_case("COMPLETED"));
        counts.add(verdict, finished, 1);
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_gh_that_never_answers_is_stopped_at_the_limit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("gh");
        std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = crate::env::ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        };
        let gh = Gh::at(script, &env);
        let started = std::time::Instant::now();
        let error = gh
            .pr_posts(dir.path(), 7, Duration::from_millis(300))
            .unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waited {:?}",
            started.elapsed()
        );
        assert!(error.to_string().contains("no answer within"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_gh_that_answers_in_time_is_read_whole() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("gh");
        std::fs::write(
            &script,
            "#!/bin/sh\necho '{\"url\":\"u\",\"reviews\":[{\"submittedAt\":\"2026-09-28T16:17:20Z\"}],\"comments\":[]}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = crate::env::ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        };
        let posts = Gh::at(script, &env)
            .pr_posts(dir.path(), 7, Duration::from_secs(10))
            .unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].kind, PrPostKind::Review);
    }

    #[test]
    fn reviews_and_comments_come_back_with_their_times_oldest_first() {
        let json = r#"{
            "url": "https://github.com/o/r/pull/7",
            "reviews": [
                {"author": {"login": "bot"}, "state": "COMMENTED", "body": "Two things.",
                 "submittedAt": "2026-09-28T16:17:20Z"},
                {"author": {"login": "bot"}, "state": "PENDING", "body": "", "submittedAt": null}
            ],
            "comments": [
                {"author": {"login": "me"}, "body": "Thanks", "createdAt": "2026-09-28T16:10:00Z",
                 "url": "https://github.com/o/r/pull/7#issuecomment-1"}
            ]
        }"#;
        let posts = pr_posts(json).unwrap();
        assert_eq!(
            posts.len(),
            2,
            "a pending review is not posted yet: {posts:?}"
        );
        assert_eq!(posts[0].kind, PrPostKind::Comment);
        assert_eq!(
            posts[0].url.as_deref(),
            Some("https://github.com/o/r/pull/7#issuecomment-1")
        );
        assert_eq!(posts[1].kind, PrPostKind::Review);
        assert_eq!(
            posts[1].url.as_deref(),
            Some("https://github.com/o/r/pull/7")
        );
        assert_eq!(
            posts[1].at_ms - posts[0].at_ms,
            7 * 60_000 + 20_000,
            "times are exact to the second"
        );
        assert!(pr_posts("{}").unwrap().is_empty());
        assert!(pr_posts("not json").is_err());
    }

    #[test]
    fn reads_the_shapes_git_hands_out() {
        let expected = Repo {
            host: "github.com".into(),
            owner: "joaoh82".into(),
            name: "yardsort".into(),
            kind: ForgeKind::GitHub,
        };
        for url in [
            "git@github.com:joaoh82/yardsort.git",
            "git@github.com:joaoh82/yardsort",
            "ssh://git@github.com/joaoh82/yardsort.git",
            "https://github.com/joaoh82/yardsort.git",
            "https://github.com/joaoh82/yardsort",
            "https://joaoh82@github.com/joaoh82/yardsort.git",
        ] {
            assert_eq!(parse_remote(url).as_ref(), Some(&expected), "{url}");
        }
    }

    #[test]
    fn keeps_a_gitlab_subgroup_in_the_owner() {
        let repo = parse_remote("git@gitlab.com:group/sub/thing.git").expect("parsed");
        assert_eq!(repo.owner, "group/sub");
        assert_eq!(repo.name, "thing");
        assert_eq!(repo.kind, ForgeKind::GitLab);
    }

    #[test]
    fn a_local_remote_is_not_a_forge() {
        for url in [
            "/srv/git/thing.git",
            "../sibling",
            r"C:\repos\thing",
            "file:///srv/git/thing.git",
        ] {
            assert_eq!(parse_remote(url), None, "{url}");
        }
    }

    #[test]
    fn a_self_hosted_host_gets_githubs_urls() {
        let repo = parse_remote("git@git.example.org:team/thing.git").expect("parsed");
        assert_eq!(repo.kind, ForgeKind::Unknown);
        assert_eq!(
            repo.compare_url("main", "ys/fix"),
            "https://git.example.org/team/thing/compare/main...ys/fix?expand=1"
        );
    }

    #[test]
    fn each_forge_gets_its_own_form() {
        let github = parse_remote("git@github.com:o/r.git").expect("parsed");
        assert_eq!(
            github.compare_url("main", "ys/fix"),
            "https://github.com/o/r/compare/main...ys/fix?expand=1"
        );
        let gitlab = parse_remote("git@gitlab.com:o/r.git").expect("parsed");
        assert!(gitlab
            .compare_url("main", "ys/fix")
            .contains("merge_request%5Bsource_branch%5D=ys/fix"));
        let bitbucket = parse_remote("git@bitbucket.org:o/r.git").expect("parsed");
        assert_eq!(
            bitbucket.compare_url("main", "ys/fix"),
            "https://bitbucket.org/o/r/pull-requests/new?source=ys/fix&dest=main"
        );
    }

    #[test]
    fn a_branch_name_cannot_break_out_of_the_query() {
        let repo = parse_remote("git@github.com:o/r.git").expect("parsed");
        assert_eq!(
            repo.compare_url("main", "ys/fix#1"),
            "https://github.com/o/r/compare/main...ys/fix%231?expand=1"
        );
    }

    fn rollup(json: &str) -> Checks {
        roll_up(Some(&serde_json::from_str(json).expect("json")))
    }

    #[test]
    fn one_failure_outranks_every_success() {
        assert_eq!(
            rollup(
                r#"[{"status":"COMPLETED","conclusion":"SUCCESS"},
                    {"status":"COMPLETED","conclusion":"FAILURE"}]"#
            ),
            Checks::Failing
        );
    }

    #[test]
    fn anything_still_running_outranks_success() {
        assert_eq!(
            rollup(
                r#"[{"status":"COMPLETED","conclusion":"SUCCESS"},
                    {"status":"IN_PROGRESS","conclusion":null}]"#
            ),
            Checks::Running
        );
    }

    #[test]
    fn a_skipped_check_still_counts_as_passing() {
        assert_eq!(
            rollup(
                r#"[{"status":"COMPLETED","conclusion":"SKIPPED"},
                    {"status":"COMPLETED","conclusion":"SUCCESS"}]"#
            ),
            Checks::Passing
        );
    }

    #[test]
    fn a_commit_status_reports_state_instead() {
        assert_eq!(
            rollup(r#"[{"context":"ci","state":"SUCCESS"}]"#),
            Checks::Passing
        );
        assert_eq!(
            rollup(r#"[{"context":"ci","state":"PENDING"}]"#),
            Checks::Running
        );
        assert_eq!(
            rollup(r#"[{"context":"ci","state":"ERROR"}]"#),
            Checks::Failing
        );
    }

    #[test]
    fn no_checks_at_all_says_nothing() {
        assert_eq!(rollup("[]"), Checks::None);
        assert_eq!(roll_up(None), Checks::None);
        assert_eq!(roll_up(Some(&serde_json::Value::Null)), Checks::None);
    }

    #[test]
    fn reads_what_gh_prints() {
        let rows: serde_json::Value = serde_json::from_str(
            r#"[{"number":42,"url":"https://github.com/o/r/pull/42","title":"Fix login",
                 "headRefName":"ys/fix-login","state":"OPEN","isDraft":true,
                 "baseRefName":"main","headRefOid":"abc123","additions":12,"deletions":3,
                 "reviewDecision":"APPROVED","updatedAt":"2026-09-28T10:00:00Z",
                 "mergeable":"CONFLICTING",
                 "statusCheckRollup":[{"name":"test","status":"COMPLETED","conclusion":"SUCCESS"}]}]"#,
        )
        .expect("json");
        let pr = pull_request(&rows[0]).expect("a pull request");
        assert_eq!(pr.number, 42);
        assert_eq!(pr.branch, "ys/fix-login");
        assert_eq!(pr.state, PullRequestState::Open);
        assert!(pr.draft);
        assert_eq!(pr.checks, Checks::Passing);
        let details = pr.details.unwrap();
        assert_eq!(details.base, "main");
        assert_eq!(details.head_oid, "abc123");
        assert_eq!((details.additions, details.deletions), (12, 3));
        assert_eq!(details.review, "APPROVED");
        assert_eq!(details.updated_at, "2026-09-28T10:00:00Z");
        assert_eq!(details.mergeable, Mergeable::Conflicting);
        assert_eq!(
            details.checks,
            vec![PullRequestCheck {
                name: "test".into(),
                state: Checks::Passing,
                workflow: None,
                url: None,
            }]
        );
        assert_eq!(details.check_counts.passed, 1);
        assert!(details.review_requests.is_empty() && details.reviews.is_empty());
        assert!(!details.cross_repository);
        assert_eq!(pr.author, None, "not asked for, not invented");
    }

    fn opened(number: u32, branch: &str, at: i64, head: &str) -> PullRequest {
        let row = serde_json::json!({
            "number": number, "url": format!("https://github.com/o/r/pull/{number}"),
            "title": "t", "headRefName": branch, "state": "MERGED", "headRefOid": head,
        });
        PullRequest {
            created_at: Some(at),
            ..pull_request(&row).unwrap()
        }
    }

    /// A workspace's pull requests: the one for the branch it is on, one opened from another
    /// branch it checked out, and one pushed from a commit made there under a name it never
    /// checked out. Not an older workspace's on a reused name, nor one opened before this
    /// workspace checked its branch out.
    #[test]
    fn a_workspace_owns_what_was_opened_from_it() {
        let minute = 60_000;
        let history = HeadHistory {
            since: Some(100 * minute),
            branches: vec![("ys/part-two".into(), 200 * minute)],
            commits: ["c-made-here".to_owned()].into(),
        };
        let requests = [
            opened(1, "ys/fix", 50 * minute, "old"),
            opened(2, "ys/fix", 150 * minute, "x"),
            opened(3, "ys/part-two", 250 * minute, "y"),
            opened(4, "ys/part-two", 20 * minute, "z"),
            opened(5, "elsewhere", 300 * minute, "c-made-here"),
            opened(6, "someone-else", 300 * minute, "w"),
            // A moment before the checkout, by the forge's clock.
            opened(7, "ys/part-two", 199 * minute, "v"),
        ];
        assert_eq!(
            pull_requests_from(&requests, Some("ys/fix"), Some("ys/fix"), &history),
            [7, 5, 3, 2]
        );
        // The pull request the current branch shows is always there, however old.
        let only_old = [opened(1, "ys/fix", 50 * minute, "old")];
        assert_eq!(
            pull_requests_from(&only_old, Some("ys/fix"), Some("ys/fix"), &history),
            [1]
        );
        // A branch checked out later counts from that checkout, not from the worktree's birth,
        // even while it is the one checked out: #8 was opened on it before then, by someone
        // else. The branch's own pull request (#9, the newest) still shows, as it always has.
        let later = [
            opened(8, "ys/part-two", 150 * minute, "u"),
            opened(9, "ys/part-two", 160 * minute, "t"),
        ];
        assert_eq!(
            pull_requests_from(&later, Some("ys/part-two"), Some("ys/fix"), &history),
            [9]
        );
        // Nothing checked out and no history: nothing but what a commit proves.
        assert!(pull_requests_from(&requests, None, None, &HeadHistory::default()).is_empty());
    }

    /// Against the real `gh`, the real network and this repository — so the field names above
    /// are checked against what GitHub actually returns rather than against a fixture that was
    /// right once. Ignored by default: CI has no network and no login.
    ///
    /// `cargo test -p yardsort-core -- --ignored --nocapture reads_this_repositorys`
    #[test]
    #[ignore = "needs gh, a login and the network"]
    fn reads_this_repositorys_own_pull_requests() {
        let env = crate::env::ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        };
        let gh = Gh::find(&env).expect("gh on PATH");
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let found = gh.pull_requests(here, 5).expect("gh answered");

        assert!(!found.is_empty(), "this repository has pull requests");
        for pr in &found {
            println!(
                "#{} {:?} {:?} {}",
                pr.number, pr.state, pr.checks, pr.branch
            );
            assert!(pr.number > 0);
            assert!(pr.url.starts_with("https://"));
            assert!(!pr.branch.is_empty());
        }
        assert!(
            found.iter().any(|pr| pr.checks != Checks::None),
            "at least one of them ran checks, so the rollup really was read"
        );
    }

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/gh/2.102.0")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// What `gh api graphql` really answered: see the README beside the fixtures.
    #[test]
    fn reads_a_recorded_page_of_open_pull_requests() {
        let page = open_page(&fixture("open-page-1.json")).unwrap();
        assert_eq!(page.total, Some(7));
        assert_eq!(page.viewer.as_deref(), Some("ada"));
        assert!(page.next.is_some(), "there is a page after it");
        let numbers: Vec<u32> = page.pull_requests.iter().map(|pr| pr.number).collect();
        assert_eq!(numbers, [14578, 14572, 14044, 13665, 13506]);

        let find = |number: u32| {
            let pr = page.pull_requests.iter().find(|pr| pr.number == number);
            pr.cloned().unwrap()
        };
        let draft = find(14578);
        assert!(draft.draft);
        assert_eq!(draft.state, PullRequestState::Open);
        assert_eq!(draft.author.as_deref(), Some("grace"));
        assert_eq!(draft.url, "https://github.com/o/r/pull/14578");
        assert!(draft.created_at.is_some());
        assert_eq!(draft.checks, Checks::Passing);
        let details = draft.details.unwrap();
        // 20 skipped and 12 succeeded: a skipped check passes.
        assert_eq!(
            details.check_counts,
            CheckCounts {
                passed: 32,
                failed: 0,
                running: 0
            }
        );
        assert!(details.checks.is_empty(), "counted, not listed");
        assert_eq!(details.base, "trunk");
        assert_eq!(details.review, "REVIEW_REQUIRED");
        assert!(!details.cross_repository);
        assert_eq!(
            details.reviews,
            [PullRequestReview {
                login: "linus".into(),
                state: ReviewState::Commented
            }]
        );

        assert_eq!(
            find(14572).details.unwrap().review_requests,
            [ReviewRequest {
                name: "grace".into(),
                team: false
            }]
        );
        // A team the asking account may not see arrives as null, and is not a request we can
        // name.
        let fork = find(14044).details.unwrap();
        assert!(fork.cross_repository);
        assert_eq!(fork.review_requests.len(), 1);
        assert_eq!(fork.review_requests[0].name, "barbara");

        assert_eq!(find(13665).details.unwrap().review, "APPROVED");
        // No check has ever reported on this one: there is no rollup at all.
        let unchecked = find(13506);
        assert_eq!(unchecked.checks, Checks::None);
        let details = unchecked.details.unwrap();
        assert_eq!(details.check_counts.total(), 0);
        assert_eq!(details.reviews[1].state, ReviewState::ChangesRequested);

        let last = open_page(&fixture("open-page-2.json")).unwrap();
        assert_eq!(last.pull_requests.len(), 2);
        assert_eq!(last.next, None, "the last page has nothing after it");
    }

    #[test]
    fn reads_a_recorded_list_with_authors_and_reviews() {
        let found = pull_requests(&fixture("pr-list.json")).unwrap();
        let states: Vec<PullRequestState> = found.iter().map(|pr| pr.state).collect();
        assert_eq!(
            states,
            [
                PullRequestState::Open,
                PullRequestState::Merged,
                PullRequestState::Closed
            ]
        );
        for pr in &found {
            assert!(pr.author.is_some(), "#{}", pr.number);
            let details = pr.details.as_ref().unwrap();
            assert_eq!(details.checks.len(), 3, "listed by name");
            assert_eq!(details.check_counts.total(), 3, "and counted");
            assert_eq!(pr.checks, details.check_counts.verdict());
        }
        let merged = found[1].details.as_ref().unwrap();
        assert_eq!(
            merged.reviews.iter().map(|r| r.state).collect::<Vec<_>>(),
            [ReviewState::Commented, ReviewState::Approved]
        );
        let closed = found[2].details.as_ref().unwrap();
        assert_eq!(closed.review_requests.len(), 1);
        assert!(!closed.review_requests[0].team);
    }

    /// One pull request in full, as `gh pr view` really printed it.
    #[test]
    fn reads_a_recorded_pull_request_in_full() {
        let summary = pull_request_summary(&fixture("pr-view.json")).unwrap();
        let pr = &summary.pull_request;
        assert_eq!(pr.number, 13665);
        assert_eq!(pr.author.as_deref(), Some("dennis"));
        assert!(summary.body.starts_with("## What\n"));
        assert_eq!(summary.changed_files, 2);

        // Every check, by name, with the workflow it belongs to and where to read its run.
        let details = pr.details.as_ref().unwrap();
        assert!(
            crate::git::is_commit_id(&details.base_oid),
            "{}",
            details.base_oid
        );
        assert_eq!(details.checks.len(), 3);
        assert_eq!(
            details.checks[0],
            PullRequestCheck {
                name: "CodeQL-Build (go)".into(),
                state: Checks::Passing,
                workflow: Some("Code Scanning".into()),
                url: Some("https://github.com/o/r/actions/runs/1/job/1".into()),
            }
        );

        // The two reviews came first and the comment a month later: oldest first, whatever
        // order `gh` lists the two kinds in.
        let said: Vec<_> = summary
            .posts
            .iter()
            .map(|post| (post.kind, post.author.as_deref(), post.review))
            .collect();
        assert_eq!(
            said,
            [
                (
                    PullRequestPostKind::Review,
                    Some("margaret"),
                    Some(ReviewState::Commented)
                ),
                (
                    PullRequestPostKind::Review,
                    Some("grace"),
                    Some(ReviewState::Approved)
                ),
                (PullRequestPostKind::Comment, Some("linus"), None),
            ]
        );
        assert!(summary
            .posts
            .windows(2)
            .all(|pair| pair[0].at <= pair[1].at));
        assert_eq!(summary.posts[0].body, "Looks right to me.");
        assert_eq!(summary.posts[1].body, "", "an approval with nothing to add");
        assert_eq!(
            summary.posts[1].url.as_deref(),
            Some("https://github.com/o/r/pull/13665"),
            "a review has no address of its own"
        );
        assert_eq!(
            summary.posts[2].url.as_deref(),
            Some("https://github.com/o/r/pull/13665#issuecomment-1")
        );
        assert!(summary.posts.iter().all(|post| post.hidden.is_none()));
    }

    #[test]
    fn a_hidden_comment_keeps_its_place_and_loses_its_words() {
        let json = serde_json::json!({
            "number": 7, "url": "https://github.com/o/r/pull/7", "headRefName": "b",
            "body": "",
            "statusCheckRollup": [
                {"__typename": "StatusContext", "context": "ci/legacy", "state": "FAILURE",
                 "targetUrl": "https://ci.example.com/7"}
            ],
            "comments": [
                {"author": {"login": "spammer"}, "body": "Buy things", "isMinimized": true,
                 "minimizedReason": "SPAM", "createdAt": "2026-09-28T16:10:00Z",
                 "url": "https://github.com/o/r/pull/7#issuecomment-1"},
                {"author": null, "body": "From an account that has gone",
                 "isMinimized": false, "minimizedReason": "",
                 "createdAt": "2026-09-28T16:20:00Z", "url": "u"}
            ],
            "reviews": [
                {"author": {"login": "ada"}, "state": "PENDING", "body": "half written",
                 "submittedAt": null},
                {"author": {"login": "ada"}, "state": "CHANGES_REQUESTED", "body": "",
                 "submittedAt": "2026-09-28T16:15:00Z"}
            ],
        });
        let summary = pull_request_summary(&json.to_string()).unwrap();
        assert_eq!(
            summary.posts.len(),
            3,
            "a review still being written is not given yet"
        );
        let spam = &summary.posts[0];
        assert_eq!(spam.hidden.as_deref(), Some("spam"));
        assert_eq!(
            spam.body, "",
            "what the forge hid is not shown by another door"
        );
        assert_eq!(summary.posts[1].review, Some(ReviewState::ChangesRequested));
        assert_eq!(summary.posts[2].author, None);
        assert_eq!(summary.posts[2].body, "From an account that has gone");

        // A commit status has a context for a name and a target for a link, and no workflow.
        let check = &summary.pull_request.details.as_ref().unwrap().checks[0];
        assert_eq!(check.name, "ci/legacy");
        assert_eq!(check.state, Checks::Failing);
        assert_eq!(check.workflow, None);
        assert_eq!(check.url.as_deref(), Some("https://ci.example.com/7"));

        assert!(pull_request_summary("{}").is_err(), "not a pull request");
        assert!(pull_request_summary("gh: not found").is_err());
    }

    /// The question asked of `gh` is one argument array: the list's fields and the words.
    #[cfg(unix)]
    #[test]
    fn a_pull_request_in_full_is_one_question() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().display().to_string();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/gh/2.102.0/pr-view.json"),
            dir.path().join("1.json"),
        )
        .unwrap();
        let gh = paged_gh(dir.path(), "true", "true");
        let summary = gh
            .pull_request_summary(dir.path(), 13665, Duration::from_secs(10))
            .unwrap();
        assert_eq!(summary.posts.len(), 3);
        let asked = std::fs::read_to_string(format!("{here}/asked")).unwrap();
        assert!(
            asked.starts_with("pr view 13665 --json number,url,title,"),
            "{asked}"
        );
        assert!(
            asked
                .trim_end()
                .ends_with(",body,comments,reviews,changedFiles"),
            "{asked}"
        );
        assert_eq!(asked.lines().count(), 1);
    }

    #[test]
    fn reads_recorded_comments_on_lines() {
        let comments = line_comments(&fixture("pr-line-comments.json")).unwrap();
        assert_eq!(comments.len(), 3);
        let first = &comments[0];
        assert_eq!(first.path, "script/build.ps1");
        assert_eq!(first.line, Some(36));
        assert_eq!(first.start_line, None);
        assert_eq!(first.side, DiffSide::Right);
        assert_eq!(first.author.as_deref(), Some("linus"));
        assert_eq!(first.body, "Is this the right encoding?");
        assert_eq!(first.in_reply_to, None);
        assert!(!first.outdated && !first.whole_file);
        assert!(first.url.as_deref().unwrap().contains("#discussion_r"));

        let range = &comments[1];
        assert_eq!((range.start_line, range.line), (Some(43), Some(49)));

        let reply = &comments[2];
        assert_eq!(reply.in_reply_to.as_deref(), Some(first.id.as_str()));
        assert!(reply.at > first.at);
    }

    #[test]
    fn a_comment_the_diff_moved_on_from_and_one_on_the_whole_file_are_told_apart() {
        let json = serde_json::json!([[
            {"id": 1, "path": "a.rs", "line": null, "original_line": 12, "side": "LEFT",
             "position": null, "subject_type": "line", "body": "gone", "created_at": "2026-09-28T10:00:00Z",
             "user": {"login": "ada"}},
            {"id": 2, "path": "a.rs", "line": null, "original_line": null, "side": "RIGHT",
             "position": null, "subject_type": "file", "body": "the whole file",
             "created_at": "2026-09-28T09:00:00Z", "user": null},
            {"id": 3, "path": "a.rs", "line": 4, "side": "RIGHT", "position": 2,
             "body": "", "created_at": "not a time"}
        ]]);
        let comments = line_comments(&json.to_string()).unwrap();
        assert_eq!(comments.len(), 2, "one without a time is nothing to place");
        let (whole, gone) = (&comments[0], &comments[1]);
        assert!(whole.whole_file && !whole.outdated);
        assert_eq!(whole.author, None);
        assert!(gone.outdated && !gone.whole_file);
        assert_eq!(gone.side, DiffSide::Left);
        assert_eq!(gone.original_line, Some(12));
        assert!(line_comments("[]").unwrap().is_empty());
        assert!(line_comments("{}").is_err());
    }

    /// The words go on standard input, never in the argument list; the place goes as fields.
    #[cfg(unix)]
    #[test]
    fn comments_are_posted_with_their_words_on_stdin() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().display().to_string();
        // A `gh` that writes down its arguments and what it read.
        let gh = paged_gh(dir.path(), "true", "true");
        std::fs::write(
            dir.path().join("gh"),
            format!("#!/bin/sh\necho \"$*\" >> '{here}/asked'\ncat >> '{here}/read'\n"),
        )
        .unwrap();
        gh.comment(dir.path(), 7, "Thanks — one question.\n")
            .unwrap();
        gh.line_comment(
            dir.path(),
            7,
            &LinePlace {
                commit: "c".repeat(40),
                path: "src/a.rs".into(),
                side: DiffSide::Right,
                line: 12,
                start_line: Some(10),
            },
            "Could this be one loop?",
        )
        .unwrap();
        let asked = std::fs::read_to_string(format!("{here}/asked")).unwrap();
        let asked: Vec<&str> = asked.lines().collect();
        assert_eq!(asked[0], "pr comment 7 --body-file -");
        assert_eq!(
            asked[1],
            format!(
                "api -X POST repos/{{owner}}/{{repo}}/pulls/7/comments -F body=@- -f commit_id={} \
                 -f path=src/a.rs -f side=RIGHT -F line=12 -F start_line=10 -f start_side=RIGHT",
                "c".repeat(40)
            )
        );
        assert_eq!(
            std::fs::read_to_string(format!("{here}/read")).unwrap(),
            "Thanks — one question.\nCould this be one loop?"
        );
    }

    /// A team in `gh pr list`'s own shape, which has a name and a slug and no login.
    #[test]
    fn a_requested_team_is_named_by_its_slug() {
        let row = serde_json::json!({
            "number": 1, "url": "https://github.com/o/r/pull/1", "headRefName": "b",
            "reviewRequests": [
                {"__typename": "Team", "name": "Code Reviewers", "slug": "code-reviewers"},
                {"__typename": "User", "login": "ada"}
            ],
            "latestReviews": [
                {"author": {"login": "ada"}, "state": "PENDING"},
                {"author": {"login": "grace"}, "state": "DISMISSED"}
            ],
        });
        let details = pull_request(&row).unwrap().details.unwrap();
        assert_eq!(
            details.review_requests,
            [
                ReviewRequest {
                    name: "code-reviewers".into(),
                    team: true
                },
                ReviewRequest {
                    name: "ada".into(),
                    team: false
                }
            ]
        );
        assert_eq!(
            details.reviews,
            [PullRequestReview {
                login: "grace".into(),
                state: ReviewState::Dismissed
            }],
            "a review still being written has not been given"
        );
    }

    /// The same checks, listed one by one as `gh pr list` has them and counted by state as our
    /// own query has them, come to the same numbers and the same verdict.
    #[test]
    fn listed_and_counted_checks_agree() {
        let listed = serde_json::json!([
            {"status": "COMPLETED", "conclusion": "SUCCESS"},
            {"status": "COMPLETED", "conclusion": "SKIPPED"},
            {"status": "COMPLETED", "conclusion": "NEUTRAL"},
            {"status": "COMPLETED", "conclusion": "CANCELLED"},
            {"status": "COMPLETED", "conclusion": "TIMED_OUT"},
            {"status": "IN_PROGRESS", "conclusion": null},
            {"status": "QUEUED", "conclusion": null},
            {"context": "ci/legacy", "state": "PENDING"},
            {"context": "ci/other", "state": "SUCCESS"},
        ]);
        let by_hand = count(Some(&listed));
        assert_eq!(
            by_hand,
            CheckCounts {
                passed: 4,
                failed: 2,
                running: 3
            }
        );

        // The shape recorded from a repository with checks still going.
        let counted = serde_json::json!({"data": {"repository": {"pullRequests": {
            "totalCount": 1, "pageInfo": {"hasNextPage": false, "endCursor": null},
            "nodes": [{
                "number": 1, "url": "https://github.com/o/r/pull/1", "headRefName": "b",
                "reviewRequests": {"nodes": []}, "latestReviews": {"nodes": []},
                "commits": {"nodes": [{"commit": {"statusCheckRollup": {"contexts": {
                    "checkRunCountsByState": [
                        {"state": "ACTION_REQUIRED", "count": 0},
                        {"state": "CANCELLED", "count": 1},
                        {"state": "IN_PROGRESS", "count": 1},
                        {"state": "NEUTRAL", "count": 1},
                        {"state": "QUEUED", "count": 1},
                        {"state": "SKIPPED", "count": 1},
                        {"state": "SUCCESS", "count": 1},
                        {"state": "TIMED_OUT", "count": 1}
                    ],
                    "statusContextCountsByState": [
                        {"state": "PENDING", "count": 1},
                        {"state": "SUCCESS", "count": 1}
                    ]
                }}}}]}
            }]
        }}, "viewer": {"login": "ada"}}});
        let page = open_page(&counted.to_string()).unwrap();
        let pr = &page.pull_requests[0];
        assert_eq!(pr.details.as_ref().unwrap().check_counts, by_hand);
        assert_eq!(pr.checks, roll_up(Some(&listed)));
        assert_eq!(pr.checks, Checks::Failing, "one failure outranks the rest");
    }

    #[test]
    fn an_answer_without_a_repository_is_not_an_empty_list() {
        assert!(open_page(r#"{"data":{"repository":null}}"#).is_err());
        assert!(open_page(r#"{"errors":[{"message":"nope"}]}"#).is_err());
        assert!(open_page("<html>502</html>").is_err());
    }

    /// A page of our own query's answer with these pull requests on it.
    #[cfg(unix)]
    fn page_of(numbers: &[u32], next: Option<&str>, total: u32) -> String {
        let nodes: Vec<serde_json::Value> = numbers
            .iter()
            .map(|number| {
                serde_json::json!({
                    "number": number, "url": format!("https://github.com/o/r/pull/{number}"),
                    "headRefName": format!("b{number}"), "state": "OPEN",
                    "reviewRequests": {"nodes": []}, "latestReviews": {"nodes": []},
                    "commits": {"nodes": []},
                })
            })
            .collect();
        serde_json::json!({"data": {"repository": {"pullRequests": {
            "totalCount": total,
            "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next},
            "nodes": nodes,
        }}, "viewer": {"login": "ada"}}})
        .to_string()
    }

    /// A `gh` that answers the first page, then whatever `second` and `third` say to do for
    /// the pages after the cursors `c1` and `c2`, and writes down what it was asked.
    #[cfg(unix)]
    fn paged_gh(dir: &Path, second: &str, third: &str) -> Gh {
        use std::os::unix::fs::PermissionsExt;
        let here = dir.display();
        let script = dir.join("gh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{here}/asked'\ncase \"$*\" in\n\
                 *after=c2*) {third} ;;\n*after=c1*) {second} ;;\n*) cat '{here}/1.json' ;;\nesac\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = crate::env::ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        };
        Gh::at(script, &env)
    }

    #[cfg(unix)]
    #[test]
    fn open_pull_requests_are_read_a_page_at_a_time_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().display().to_string();
        std::fs::write(dir.path().join("1.json"), page_of(&[9, 8], Some("c1"), 5)).unwrap();
        // #8 changed while the pages were being read, and so is on the second one too.
        std::fs::write(dir.path().join("2.json"), page_of(&[8, 7], Some("c2"), 5)).unwrap();
        std::fs::write(dir.path().join("3.json"), page_of(&[6, 5], None, 5)).unwrap();
        let gh = paged_gh(
            dir.path(),
            &format!("cat '{here}/2.json'"),
            &format!("cat '{here}/3.json'"),
        );
        let found = gh.open_pull_requests(
            dir.path(),
            Some("github.example.com"),
            200,
            Duration::from_secs(10),
        );
        let numbers: Vec<u32> = found.pull_requests.iter().map(|pr| pr.number).collect();
        assert_eq!(numbers, [9, 8, 7, 6, 5], "in order, each once");
        assert_eq!(found.total, Some(5));
        assert_eq!(found.viewer.as_deref(), Some("ada"));
        assert!(found.answered);
        assert_eq!(found.problem, None);

        let asked = std::fs::read_to_string(dir.path().join("asked")).unwrap();
        let asked: Vec<&str> = asked.lines().collect();
        assert_eq!(asked.len(), 3, "three pages, three questions");
        assert!(asked[0].starts_with("api graphql --hostname github.example.com "));
        assert!(
            asked[0].contains("headRefOid baseRefName baseRefOid "),
            "{}",
            asked[0]
        );
        assert!(asked[0].contains("-F owner={owner} -F name={repo} -f query=query("));
        assert!(!asked[0].contains("after="), "the first page has no cursor");
        assert!(asked[1].ends_with("-f after=c1"));
        assert!(asked[2].ends_with("-f after=c2"));
    }

    #[cfg(unix)]
    #[test]
    fn a_page_that_fails_keeps_the_pages_before_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("1.json"), page_of(&[9, 8], Some("c1"), 120)).unwrap();
        let gh = paged_gh(
            dir.path(),
            "echo 'HTTP 504: no answer' >&2; exit 1",
            "exit 1",
        );
        let found = gh.open_pull_requests(dir.path(), None, 200, Duration::from_secs(10));
        assert_eq!(found.pull_requests.len(), 2, "what arrived is kept");
        assert_eq!(found.total, Some(120));
        assert!(found.answered);
        let problem = found.problem.unwrap();
        assert!(problem.contains("HTTP 504"), "{problem}");
        assert!(
            !problem.contains("query("),
            "the reason, not the question: {problem}"
        );
        assert!(!found.logged_out);
    }

    #[cfg(unix)]
    #[test]
    fn a_page_that_never_answers_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("1.json"), page_of(&[9], Some("c1"), 2)).unwrap();
        let gh = paged_gh(dir.path(), "sleep 30", "exit 1");
        let started = std::time::Instant::now();
        let found = gh.open_pull_requests(dir.path(), None, 200, Duration::from_millis(400));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "waited {:?}",
            started.elapsed()
        );
        assert_eq!(found.pull_requests.len(), 1);
        assert!(found.problem.unwrap().contains("no answer within"));
    }

    #[cfg(unix)]
    #[test]
    fn reading_stops_at_the_cap_and_still_says_how_many_there_are() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().display().to_string();
        std::fs::write(
            dir.path().join("1.json"),
            page_of(&[9, 8], Some("c1"), 1394),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("2.json"),
            page_of(&[7, 6], Some("c2"), 1394),
        )
        .unwrap();
        let gh = paged_gh(dir.path(), &format!("cat '{here}/2.json'"), "exit 1");
        let found = gh.open_pull_requests(dir.path(), None, 3, Duration::from_secs(10));
        let numbers: Vec<u32> = found.pull_requests.iter().map(|pr| pr.number).collect();
        assert_eq!(numbers, [9, 8, 7]);
        assert_eq!(found.total, Some(1394));
        assert_eq!(found.problem, None, "the third page was never asked for");
    }

    #[cfg(unix)]
    #[test]
    fn nobody_logged_in_is_said_once_and_nothing_is_claimed() {
        let dir = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        let script = dir.path().join("gh");
        std::fs::write(
            &script,
            "#!/bin/sh\necho 'To get started with GitHub CLI, please run: gh auth login' >&2\nexit 4\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = crate::env::ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        };
        let found =
            Gh::at(script, &env).open_pull_requests(dir.path(), None, 200, Duration::from_secs(10));
        assert!(found.logged_out);
        assert!(!found.answered);
        assert!(found.pull_requests.is_empty());
    }

    /// Closing and reopening are one argument array each: no comment, no branch deleted.
    #[cfg(unix)]
    #[test]
    fn closing_and_reopening_ask_for_nothing_more() {
        let dir = tempfile::tempdir().unwrap();
        let gh = paged_gh(dir.path(), "true", "true");
        std::fs::write(dir.path().join("1.json"), "").unwrap();
        gh.close_pull_request(dir.path(), 42).unwrap();
        gh.reopen_pull_request(dir.path(), 42).unwrap();
        let asked = std::fs::read_to_string(dir.path().join("asked")).unwrap();
        assert_eq!(asked, "pr close 42\npr reopen 42\n");
    }

    /// The open tier against the real `gh`, the real network and this repository: that `gh api
    /// graphql` takes the query as written, fills `{owner}` and `{repo}` in from the folder, and
    /// answers in the shape [`open_page`] reads. Ignored by default, like the one above.
    ///
    /// `cargo test -p yardsort-core -- --ignored --nocapture reads_this_repositorys`
    #[test]
    #[ignore = "needs gh, a login and the network"]
    fn reads_this_repositorys_open_pull_requests_a_page_at_a_time() {
        let env = crate::env::ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        };
        let gh = Gh::find(&env).expect("gh on PATH");
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let found = gh.open_pull_requests(here, None, 200, Duration::from_secs(20));
        println!(
            "{} open listed of {:?}, as {:?}; problem: {:?}",
            found.pull_requests.len(),
            found.total,
            found.viewer,
            found.problem
        );
        assert!(found.answered, "{:?}", found.problem);
        assert_eq!(found.problem, None);
        assert!(found.viewer.is_some(), "gh said who is logged in");
        let total = found.total.expect("the forge counted them");
        assert_eq!(found.pull_requests.len(), total.min(200) as usize);
        for pr in &found.pull_requests {
            assert_eq!(pr.state, PullRequestState::Open);
            assert!(pr.url.starts_with("https://"));
            assert!(pr.details.is_some());
        }
    }

    /// One pull request in full against the real `gh`: the newest one this repository has.
    /// Ignored by default, like the two above.
    #[test]
    #[ignore = "needs gh, a login and the network"]
    fn reads_this_repositorys_newest_pull_request_in_full() {
        let env = crate::env::ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        };
        let gh = Gh::find(&env).expect("gh on PATH");
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let newest = gh.pull_requests(here, 1).expect("gh answered")[0].number;
        let summary = gh
            .pull_request_summary(here, newest, Duration::from_secs(20))
            .expect("gh answered");
        let details = summary.pull_request.details.as_ref().unwrap();
        println!(
            "#{}: {} chars of description, {} files, {} checks ({} with a link), {} posts",
            summary.pull_request.number,
            summary.body.len(),
            summary.changed_files,
            details.checks.len(),
            details.checks.iter().filter(|c| c.url.is_some()).count(),
            summary.posts.len()
        );
        assert_eq!(summary.pull_request.number, newest);
        assert!(summary.changed_files > 0);
        assert_eq!(details.checks.len() as u32, details.check_counts.total());
        assert!(summary
            .posts
            .windows(2)
            .all(|pair| pair[0].at <= pair[1].at));
    }

    #[test]
    fn logged_out_is_told_apart_from_broken() {
        let logged_out = ForgeError::Failed {
            command: "pr list".into(),
            stderr: "To get started with GitHub CLI, please run: gh auth login".into(),
        };
        assert!(logged_out.is_logged_out());
        let broken = ForgeError::Failed {
            command: "pr list".into(),
            stderr: "could not resolve host".into(),
        };
        assert!(!broken.is_logged_out());
    }
}
