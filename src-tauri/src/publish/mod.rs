//! Getting a workspace's work out: commit, push, open the pull request, and say what became of
//! it afterwards.
//!
//! The git half always works. The forge half is [`crate::forge`]'s: `gh` when the user has it,
//! a link to the forge's own form when they do not. Nothing here ever holds a forge credential.
//!
//! Pull requests are fetched **per project, not per workspace**. A project with a dozen
//! workspaces would otherwise make a dozen network calls every time the window came back into
//! focus, so one `gh pr list` answers for all of them and the answer is cached for
//! [`FRESH_FOR`]; a workspace finds its own by branch name and by its worktree's own history
//! (see [`pull_requests_of`]).
//!
//! That list is the *recent* tier: the newest fifty, whatever their state, with every check by
//! name. The Pull requests view adds the *open* tier — every open pull request up to
//! [`OPEN_LIMIT`], read a page at a time by [`Gh::open_pull_requests`] — which is only asked for
//! while that view is showing. [`Forge`] keeps both and hands out one list.

pub mod commands;
pub mod conflicts;
pub mod pull_requests;
pub mod tasks;

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use specta::Type;

use crate::error::IpcResult;
use crate::forge::{
    parse_remote, pull_request_for, pull_requests_from, ForgeKind, ForgeResult, Gh,
    OpenPullRequests, PullRequest, PullRequestState, PullRequestSummary, Repo,
};
use crate::git::{Commit, Git, Head};
use crate::store::WorkspaceRow;

/// How long a project's pull requests are reused before `gh` is asked again. Long enough that
/// switching between workspaces costs nothing, short enough that a check finishing is noticed
/// without pressing anything.
const FRESH_FOR: Duration = Duration::from_secs(30);

/// How many pull requests to ask for. Enough to cover every branch anyone still has a workspace
/// for; asking for every pull request a busy repository ever had would be a slow query for
/// answers nobody can use.
const LIMIT: u32 = 50;

/// How many open pull requests the Pull requests view reads, most recently updated first. Past
/// this the view says how many more there are and sends you to the forge for them.
const OPEN_LIMIT: usize = 200;

/// How long one page of open pull requests may take. GitHub gives up on a query after ten
/// seconds itself; this is for a network that went away instead.
const PAGE_LIMIT: Duration = Duration::from_secs(20);

/// Commits offered as a starting point for a pull request's title and body.
const MAX_COMMITS: usize = 20;

/// What `gh` says about one project's pull requests.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPullRequests {
    /// `gh` is installed. Without it there are no pull request numbers and no check results
    /// anywhere in the app — which is a supported state, not a fault.
    pub gh: bool,
    pub pull_requests: Vec<PullRequest>,
    /// Why the list is empty when it should not have been.
    pub problem: Option<String>,
    /// The problem is that nobody is logged in, which has its own one-line fix.
    pub logged_out: bool,
    /// Each workspace's own pull requests, by number, newest first: see
    /// [`pull_requests_from`]. A workspace can have several — a branch reused after its pull
    /// request merged, or a second branch its agent opened one from. Absent for a workspace
    /// git could not be asked about.
    pub workspaces: BTreeMap<String, Vec<u32>>,
    /// The project's remote read as a repository on a forge: what the Pull requests view calls
    /// it, and how it knows a project is not on GitHub. `None` for a remote that is a path on
    /// disk, and for no remote at all.
    pub repo: Option<Repo>,
    /// Who `gh` is logged in as, once the open pull requests have been read: what "by you"
    /// means in the view's filters.
    pub viewer: Option<String>,
    /// How many open pull requests the forge says the repository has. More than the list holds
    /// when there are more than [`OPEN_LIMIT`]. `None` until the open ones have been read.
    pub open_total: Option<u32>,
    /// Why the open pull requests could not all be read, when the recent ones could.
    pub open_problem: Option<String>,
}

/// Where a workspace stands: what it can commit, push and open, and what happened to it.
///
/// Deliberately not the *count* of uncommitted files — the changes panel already has that list
/// and asking git twice for the same thing is how the two come to disagree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PublishState {
    /// `None` when HEAD is detached or unborn: there is nothing to push or open.
    pub branch: Option<String>,
    /// What a pull request would merge into.
    pub base: Option<String>,
    /// Who commits would be by, or `None` when git has no identity configured.
    pub identity: Option<String>,
    /// The remote to push to, `origin` for choice.
    pub remote: Option<String>,
    /// The remote read as a repository on a forge; `None` for a remote that is a local path.
    pub repo: Option<Repo>,
    /// What the branch tracks, once it has been pushed.
    pub upstream: Option<String>,
    /// Commits the remote does not have: measured against the upstream once there is one, and
    /// against the base branch before that.
    pub ahead: u32,
    pub behind: u32,
    /// Those commits, newest first — what a pull request would be about. Their bodies come
    /// too: a well-written commit is a well-written pull request, and the dialog uses it.
    pub unpushed: Vec<Commit>,
    pub pull_request: Option<PullRequest>,
    /// Opening one makes sense: a branch that is not its own base, on a forge, without a pull
    /// request already open. Decided here rather than in the webview, which holds no truth.
    pub can_open: bool,
    /// `gh` is installed, so a pull request can be opened without leaving Yardsort.
    pub gh: bool,
    /// Why there is no forge answer, when there should have been one. Not a failure — it is why
    /// the row shows no number, which is worth saying somewhere the user is already looking.
    pub problem: Option<String>,
    /// That reason is "log in first", which has a one-line fix worth printing.
    pub logged_out: bool,
    /// The branch is a checkout of this pull request from a fork (see
    /// [`Git::follow_pull_request`]). Yardsort does not push it: a push would make a new branch
    /// on the project's own remote, not update the fork the pull request comes from.
    pub follows_pull_request: Option<u32>,
}

impl PublishState {
    /// The pull request this workspace would open — the forge's own form, filled in.
    pub fn compare_url(&self) -> Option<String> {
        let repo = self.repo.as_ref()?;
        Some(repo.compare_url(self.base.as_deref()?, self.branch.as_deref()?))
    }
}

/// Everything known about a workspace's relationship with its remote.
///
/// `found` is what `gh` said about the whole project, already fetched: this picks out the one
/// pull request whose head is this workspace's branch.
pub fn state(
    git: &Git,
    root: &Path,
    row: &WorkspaceRow,
    found: &ProjectPullRequests,
) -> IpcResult<PublishState> {
    let branch = match git.head(root)? {
        Head::Branch(name) => Some(name),
        Head::Unborn(_) | Head::Detached(_) => None,
    };

    // What Yardsort started the branch from, or the repository's default when it did not start
    // it. A workspace sitting *on* the default branch has nothing to merge into.
    let base = match row.base_branch.clone() {
        Some(base) => Some(base),
        None => git.default_branch(root)?,
    }
    .filter(|base| Some(base) != branch.as_ref());

    let remote = git.push_remote(root)?;
    let repo = match &remote {
        Some(name) => git
            .remote_url(root, name)?
            .as_deref()
            .and_then(parse_remote),
        None => None,
    };

    let upstream = git.upstream(root)?;
    // Before the first push there is no upstream to count against, so the base branch stands in:
    // "what this workspace has done that nobody else has seen" is the same question.
    let reference = upstream.clone().or_else(|| base.clone());
    let (ahead, behind) = match &reference {
        Some(reference) => git.ahead_behind(root, reference)?.unwrap_or((0, 0)),
        None => (0, 0),
    };
    let unpushed = match &reference {
        Some(reference) => {
            let mut commits = git.commits_since(root, reference)?;
            commits.truncate(MAX_COMMITS);
            commits
        }
        None => vec![],
    };

    let follows_pull_request = match &branch {
        Some(branch) => git.followed_pull_request(root, branch)?,
        None => None,
    };
    let pull_request = match follows_pull_request {
        // Its name here is not its name on the forge, so the number is what finds it.
        Some(number) => found.pull_requests.iter().find(|pr| pr.number == number),
        None => branch
            .as_ref()
            .and_then(|branch| pull_request_for(&found.pull_requests, branch)),
    }
    .cloned();

    // Somewhere to open it, and nothing open there already. A merged or closed one does not
    // stand in the way: the branch may well have moved on since. A branch that *is* a pull
    // request has nothing to open.
    let can_open = branch.is_some()
        && base.is_some()
        && repo.is_some()
        && follows_pull_request.is_none()
        && !matches!(&pull_request, Some(pr) if pr.state == PullRequestState::Open);

    Ok(PublishState {
        identity: git.identity(root)?,
        branch,
        base,
        remote,
        repo,
        upstream,
        ahead,
        behind,
        unpushed,
        pull_request,
        can_open,
        gh: found.gh,
        problem: found.problem.clone(),
        logged_out: found.logged_out,
        follows_pull_request,
    })
}

/// The numbers of the pull requests that came out of the workspace at `root`, newest first.
pub fn pull_requests_of(
    git: &Git,
    root: &Path,
    row: &WorkspaceRow,
    requests: &[PullRequest],
) -> IpcResult<Vec<u32>> {
    let branch = match git.head(root)? {
        Head::Branch(name) => Some(name),
        Head::Unborn(_) | Head::Detached(_) => None,
    };
    let history = git.head_history(root)?;
    let mut found =
        pull_requests_from(requests, branch.as_deref(), row.branch.as_deref(), &history);
    // A workspace started from a fork's pull request: its branch says which one in git's own
    // config, since neither its name nor a commit made here can.
    let followed = match &branch {
        Some(branch) => git.followed_pull_request(root, branch)?,
        None => None,
    };
    if let Some(number) = followed {
        if requests.iter().any(|pr| pr.number == number) && !found.contains(&number) {
            found.insert(0, number);
        }
    }
    Ok(found)
}

/// The project-wide pull request cache. One per app.
#[derive(Default)]
pub struct Forge {
    cached: Mutex<HashMap<String, (Instant, ProjectPullRequests)>>,
    requests: Mutex<HashMap<String, u64>>,
    next_request: AtomicU64,
    /// Each project's workspace-to-pull-request map, with the list it was worked out from.
    owned: Mutex<HashMap<String, OwnedAnswer>>,
    /// Each project's open pull requests, as last read: the open tier.
    open: Mutex<HashMap<String, OpenTier>>,
    open_requests: Mutex<HashMap<String, u64>>,
    /// Pull requests read in full, for the Pull requests view's Summary, by project and number.
    summaries: Mutex<HashMap<(String, u32), (Instant, PullRequestSummary)>>,
    /// Counts every [`forget`](Self::forget), so a summary read before one is not kept after.
    forgotten: AtomicU64,
}

/// A project's open pull requests and when they were read. `at` is `None` once something has
/// happened that the answer does not know about — it is still shown, and asked for again the
/// next time anyone looks at the whole list.
struct OpenTier {
    at: Option<Instant>,
    found: OpenPullRequests,
}

type Owned = BTreeMap<String, Vec<u32>>;
type OwnedAnswer = (Instant, Vec<PullRequest>, Owned);

impl Forge {
    /// This project's pull requests, from the cache when it is fresh enough.
    ///
    /// Blocking: it runs `gh`, which talks to the network. Never an `Err` — a forge that cannot
    /// be reached is a missing badge, not a failed command, and the reason travels in
    /// [`ProjectPullRequests::problem`] so the panel can show it where it belongs.
    ///
    /// The recent tier is always what is asked for. With `full` the open tier is too, when it
    /// is stale; without it, whatever the open tier last said is still part of the answer.
    /// `repo` is only called when `gh` is: it costs two git processes.
    pub fn pull_requests(
        &self,
        gh: Option<&Gh>,
        root: &Path,
        project_id: &str,
        refresh: bool,
        full: bool,
        repo: &dyn Fn() -> Option<Repo>,
    ) -> ProjectPullRequests {
        let recent = self.load(project_id, refresh, || ask(gh, root, repo()));
        if let (true, Some(gh), Some(host)) = (full, gh, open_host(&recent)) {
            self.load_open(project_id, refresh, || {
                gh.open_pull_requests(root, host, OPEN_LIMIT, PAGE_LIMIT)
            });
        }
        let open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        compose(recent, open.get(project_id).map(|tier| &tier.found))
    }

    /// Read the open tier unless it was read less than [`FRESH_FOR`] ago.
    fn load_open(&self, project_id: &str, refresh: bool, fetch: impl FnOnce() -> OpenPullRequests) {
        if !refresh {
            let open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            let fresh = open
                .get(project_id)
                .and_then(|tier| tier.at)
                .is_some_and(|at| at.elapsed() < FRESH_FOR);
            if fresh {
                return;
            }
        }
        let request = {
            let mut requests = self
                .open_requests
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let request = self.next_request.fetch_add(1, Ordering::Relaxed);
            requests.insert(project_id.to_owned(), request);
            request
        };
        let found = fetch();
        let requests = self
            .open_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if requests.get(project_id) != Some(&request) {
            return;
        }
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        let found = match open.remove(project_id) {
            // Nothing arrived at all: what was known is still the best there is, with the
            // reason it could not be brought up to date.
            Some(before) if !found.answered => OpenPullRequests {
                problem: found.problem,
                logged_out: found.logged_out,
                ..before.found
            },
            _ => found,
        };
        open.insert(
            project_id.to_owned(),
            OpenTier {
                at: Some(Instant::now()),
                found,
            },
        );
    }

    fn load(
        &self,
        project_id: &str,
        refresh: bool,
        fetch: impl FnOnce() -> ProjectPullRequests,
    ) -> ProjectPullRequests {
        if !refresh {
            if let Some(fresh) = self.cached_fresh(project_id) {
                return fresh;
            }
        }
        let request = {
            let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
            let request = self.next_request.fetch_add(1, Ordering::Relaxed);
            requests.insert(project_id.to_owned(), request);
            request
        };
        let answer = fetch();
        let requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        if requests.get(project_id) == Some(&request) {
            self.cached
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(project_id.to_owned(), (Instant::now(), answer.clone()));
        }
        answer
    }

    /// Pull request `number` in full, from the cache when it was read less than [`FRESH_FOR`]
    /// ago. Unlike the lists this *is* an `Err` when `gh` cannot answer: the Summary has
    /// nothing else to show, and says why in its place.
    pub fn summary(
        &self,
        project_id: &str,
        number: u32,
        refresh: bool,
        fetch: impl FnOnce() -> ForgeResult<PullRequestSummary>,
    ) -> ForgeResult<PullRequestSummary> {
        let key = (project_id.to_owned(), number);
        if !refresh {
            let summaries = self
                .summaries
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some((at, summary)) = summaries.get(&key) {
                if at.elapsed() < FRESH_FOR {
                    return Ok(summary.clone());
                }
            }
        }
        let before = self.forgotten.load(Ordering::Relaxed);
        let summary = fetch()?;
        // Something was done to the project while this was being read — a merge, a close — and
        // what was read is from before it. It is still the answer to this question; it is just
        // not kept for the next one.
        if self.forgotten.load(Ordering::Relaxed) == before {
            self.summaries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(key, (Instant::now(), summary.clone()));
        }
        Ok(summary)
    }

    /// Pull request `number` as the project's list has it now — the recent tier's copy first,
    /// then the open tier's — or `None` when neither has it. Never a fetch.
    pub fn find(&self, project_id: &str, number: u32) -> Option<PullRequest> {
        let in_recent = {
            let cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
            cached.get(project_id).and_then(|(_, answer)| {
                answer
                    .pull_requests
                    .iter()
                    .find(|pr| pr.number == number)
                    .cloned()
            })
        };
        in_recent.or_else(|| {
            let open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            open.get(project_id).and_then(|tier| {
                tier.found
                    .pull_requests
                    .iter()
                    .find(|pr| pr.number == number)
                    .cloned()
            })
        })
    }

    /// Whatever is cached for a project, however old, and never a fetch: for readers that must
    /// not wait on the network, like the Outcomes view. The publish panel keeps it current.
    pub fn cached(&self, project_id: &str) -> Option<ProjectPullRequests> {
        let cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        cached.get(project_id).map(|(_, answer)| answer.clone())
    }

    fn cached_fresh(&self, project_id: &str) -> Option<ProjectPullRequests> {
        let cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        let (at, answer) = cached.get(project_id)?;
        (at.elapsed() < FRESH_FOR).then(|| answer.clone())
    }

    /// Drop what is remembered about a project, so the next look asks again. Called after
    /// anything that changes the answer — a push, a pull request being opened.
    pub fn forget(&self, project_id: &str) {
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        requests.remove(project_id);
        self.cached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(project_id);
        self.owned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(project_id);
        self.forgotten.fetch_add(1, Ordering::Relaxed);
        self.summaries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(project, _), _| project != project_id);
        // The open tier is kept, marked out of date: it is only read while the Pull requests
        // view is showing, and dropping it would take the older open pull requests off every
        // workspace row until then. An answer still on its way is from before, and is dropped.
        self.open_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(project_id);
        if let Some(tier) = self
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(project_id)
        {
            tier.at = None;
        }
    }

    /// Which of `requests` each of a project's workspaces opened, as `compute` works it out.
    ///
    /// Working it out costs a few `git` processes per workspace, and the sidebar asks for every
    /// project each time the window comes back into focus. So an answer worked out from the same
    /// list less than [`FRESH_FOR`] ago is reused, like the list itself; `refresh`, or anything
    /// that makes the project be [forgotten](Self::forget), works it out again.
    pub fn owned(
        &self,
        project_id: &str,
        requests: &[PullRequest],
        refresh: bool,
        compute: impl FnOnce() -> Owned,
    ) -> Owned {
        if !refresh {
            let owned = self.owned.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((at, from, answer)) = owned.get(project_id) {
                if at.elapsed() < FRESH_FOR && from.as_slice() == requests {
                    return answer.clone();
                }
            }
        }
        let answer = compute();
        self.owned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                project_id.to_owned(),
                (Instant::now(), requests.to_vec(), answer.clone()),
            );
        answer
    }
}

/// Refuse pull request `number` unless it is one of the workspace's own (see
/// [`pull_requests_of`]): an action on it is an action on someone else's work otherwise.
pub fn ensure_own(
    git: &Git,
    root: &Path,
    row: &WorkspaceRow,
    requests: &[PullRequest],
    number: u32,
) -> IpcResult<()> {
    if pull_requests_of(git, root, row, requests)?.contains(&number) {
        Ok(())
    } else {
        Err(crate::error::IpcError::new(
            "pull_request_changed",
            format!(
                "Pull request #{number} is not one this workspace opened. Refresh and choose \
                 again."
            ),
        ))
    }
}

fn ask(gh: Option<&Gh>, root: &Path, repo: Option<Repo>) -> ProjectPullRequests {
    let Some(gh) = gh else {
        return ProjectPullRequests {
            repo,
            ..Default::default()
        };
    };
    match gh.pull_requests(root, LIMIT) {
        Ok(pull_requests) => ProjectPullRequests {
            gh: true,
            pull_requests,
            repo,
            ..Default::default()
        },
        Err(error) => ProjectPullRequests {
            gh: true,
            logged_out: error.is_logged_out(),
            problem: Some(error.to_string()),
            repo,
            ..Default::default()
        },
    }
}

/// Whether the open pull requests are worth asking for, and of which host: `Some(None)` is
/// "yes, of `gh`'s default host".
///
/// Not when nobody is logged in — the recent tier already said so, once. Not for a forge `gh`
/// does not speak to. A host this cannot place (an ssh alias, a GitHub Enterprise under a name
/// of its own) is asked: `gh` knows more about it than a URL does, and says so if not.
fn open_host(recent: &ProjectPullRequests) -> Option<Option<&str>> {
    if !recent.gh || recent.logged_out {
        return None;
    }
    let repo = recent.repo.as_ref()?;
    match repo.kind {
        ForgeKind::GitLab | ForgeKind::Bitbucket => None,
        ForgeKind::GitHub if repo.host != "github.com" => Some(Some(&repo.host)),
        ForgeKind::GitHub | ForgeKind::Unknown => Some(None),
    }
}

/// One list out of the two tiers.
///
/// A pull request both have is the recent tier's: that tier is asked at least as often, knows
/// when one was merged or closed, and has its checks by name. The open tier adds the open pull
/// requests the newest fifty did not reach.
fn compose(
    mut recent: ProjectPullRequests,
    open: Option<&OpenPullRequests>,
) -> ProjectPullRequests {
    let Some(open) = open else {
        return recent;
    };
    let older: Vec<PullRequest> = open
        .pull_requests
        .iter()
        .filter(|pr| !recent.pull_requests.iter().any(|r| r.number == pr.number))
        .cloned()
        .collect();
    recent.pull_requests.extend(older);
    recent.viewer = open.viewer.clone();
    recent.open_total = open.total;
    // Logged out, or `gh` failing for both: said once, by the recent tier.
    if recent.problem.is_none() && !open.logged_out {
        recent.open_problem = open.problem.clone();
    }
    recent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::Checks;
    use crate::git::testing;

    fn row(path: &Path, base: Option<&str>) -> WorkspaceRow {
        WorkspaceRow {
            id: "w1".into(),
            project_id: "p1".into(),
            kind: "worktree".into(),
            name: "feature".into(),
            path: path.to_string_lossy().into_owned(),
            branch: Some("ys/feature".into()),
            base_branch: base.map(str::to_owned),
            archived: false,
            forgotten: false,
        }
    }

    /// A repository on `trunk` with a bare remote next door, and a worktree on `ys/feature`.
    fn fixture() -> (Git, tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
        let git = testing::git();
        let repo = tempfile::tempdir().unwrap();
        git.init(repo.path()).unwrap();
        git.run(repo.path(), &["checkout", "-b", "trunk"]).unwrap();
        git.initial_commit(repo.path()).unwrap();

        let remote = tempfile::tempdir().unwrap();
        git.run(remote.path(), &["init", "--bare"]).unwrap();
        git.run(
            repo.path(),
            &["remote", "add", "origin", &remote.path().to_string_lossy()],
        )
        .unwrap();
        git.push(repo.path(), "origin", "trunk").unwrap();

        let trees = tempfile::tempdir().unwrap();
        let tree = trees.path().join("feature");
        git.worktree_add(repo.path(), &tree, "ys/feature", "trunk")
            .unwrap();
        (git, repo, remote, trees)
    }

    #[test]
    fn a_fresh_branch_is_ahead_of_its_base_before_it_has_an_upstream() {
        let (git, _repo, _remote, trees) = fixture();
        let tree = trees.path().join("feature");
        std::fs::write(tree.join("a.txt"), "one").unwrap();
        git.commit_all(&tree, "Add a").unwrap();

        let state = state(&git, &tree, &row(&tree, Some("trunk")), &Default::default()).unwrap();
        assert_eq!(state.branch.as_deref(), Some("ys/feature"));
        assert_eq!(state.base.as_deref(), Some("trunk"));
        assert_eq!(state.upstream, None, "nothing has been pushed yet");
        assert_eq!(state.ahead, 1, "counted against the base instead");
        assert_eq!(
            state
                .unpushed
                .iter()
                .map(|c| c.subject.as_str())
                .collect::<Vec<_>>(),
            ["Add a"]
        );
        assert_eq!(state.remote.as_deref(), Some("origin"));
        assert!(state.identity.is_some());
    }

    /// Point `origin` at a forge while still pushing to the bare repository next door: a test
    /// that wants both a real push and a URL a pull request could be opened from.
    fn pretend_origin_is_on_github(git: &Git, repo: &Path, bare: &Path) {
        git.run(
            repo,
            &["remote", "set-url", "origin", "git@github.com:o/r.git"],
        )
        .unwrap();
        git.run(
            repo,
            &[
                "remote",
                "set-url",
                "--push",
                "origin",
                &bare.to_string_lossy(),
            ],
        )
        .unwrap();
    }

    #[test]
    fn pushing_moves_the_count_onto_the_upstream() {
        let (git, repo, remote, trees) = fixture();
        pretend_origin_is_on_github(&git, repo.path(), remote.path());
        let tree = trees.path().join("feature");
        std::fs::write(tree.join("a.txt"), "one").unwrap();
        git.commit_all(&tree, "Add a").unwrap();
        git.push(&tree, "origin", "ys/feature").unwrap();

        let state = state(&git, &tree, &row(&tree, Some("trunk")), &Default::default()).unwrap();
        assert_eq!(state.upstream.as_deref(), Some("origin/ys/feature"));
        assert_eq!((state.ahead, state.behind), (0, 0));
        assert!(state.unpushed.is_empty());
        assert!(
            state.can_open,
            "pushed, on a branch, with a forge to open on"
        );
        assert_eq!(
            state.compare_url().as_deref(),
            Some("https://github.com/o/r/compare/trunk...ys/feature?expand=1")
        );
    }

    #[test]
    fn a_local_remote_is_not_somewhere_a_pull_request_can_be_opened() {
        let (git, _repo, _remote, trees) = fixture();
        let tree = trees.path().join("feature");
        let state = state(&git, &tree, &row(&tree, Some("trunk")), &Default::default()).unwrap();
        assert_eq!(state.repo, None, "the remote is a path on disk");
        assert_eq!(state.compare_url(), None);
        assert!(!state.can_open);
    }

    #[test]
    fn a_workspace_sitting_on_the_base_branch_has_nothing_to_merge() {
        let (git, repo, _remote, _trees) = fixture();
        let mut row = row(repo.path(), None);
        row.kind = "local".into();
        let state = state(&git, repo.path(), &row, &Default::default()).unwrap();
        assert_eq!(state.branch.as_deref(), Some("trunk"));
        assert_eq!(state.base, None, "trunk into trunk is not a pull request");
        assert!(!state.can_open);
    }

    #[test]
    fn a_detached_head_can_do_none_of_it() {
        let (git, _repo, _remote, trees) = fixture();
        let tree = trees.path().join("feature");
        git.run(&tree, &["checkout", "--detach"]).unwrap();
        let state = state(&git, &tree, &row(&tree, Some("trunk")), &Default::default()).unwrap();
        assert_eq!(state.branch, None);
        assert!(!state.can_open);
        assert_eq!(state.compare_url(), None);
    }

    fn pr(branch: &str, state: PullRequestState) -> PullRequest {
        PullRequest {
            number: 7,
            url: "https://example.com/pull/7".into(),
            title: "Add a".into(),
            branch: branch.into(),
            state,
            draft: false,
            checks: Checks::Passing,
            created_at: None,
            details: None,
            author: None,
        }
    }

    #[test]
    fn a_workspace_picks_its_own_pull_request_out_of_the_projects() {
        let (git, _repo, _remote, trees) = fixture();
        let tree = trees.path().join("feature");
        let found = ProjectPullRequests {
            gh: true,
            pull_requests: vec![
                pr("ys/something-else", PullRequestState::Open),
                pr("ys/feature", PullRequestState::Open),
            ],
            ..Default::default()
        };
        let state = state(&git, &tree, &row(&tree, Some("trunk")), &found).unwrap();
        assert_eq!(
            state.pull_request.map(|pr| pr.branch).as_deref(),
            Some("ys/feature")
        );
        assert!(!state.can_open, "it is already open");
    }

    /// A real worktree that opened two pull requests — its own branch, then a second branch it
    /// checked out — finds both, and not one another workspace opened on a branch it never had.
    #[test]
    fn a_workspace_finds_every_pull_request_it_opened() {
        let (git, _repo, _remote, trees) = fixture();
        let tree = trees.path().join("feature");
        std::fs::write(tree.join("a.txt"), "one").unwrap();
        git.commit_all(&tree, "Add a").unwrap();
        git.run(&tree, &["checkout", "-b", "ys/part-two"]).unwrap();
        std::fs::write(tree.join("b.txt"), "two").unwrap();
        git.commit_all(&tree, "Add b").unwrap();
        git.run(&tree, &["checkout", "ys/feature"]).unwrap();

        let now = now_ms();
        let opened = |number: u32, branch: &str| PullRequest {
            number,
            created_at: Some(now),
            ..pr(branch, PullRequestState::Open)
        };
        let requests = [
            opened(7, "ys/feature"),
            opened(8, "ys/part-two"),
            opened(9, "ys/someone-else"),
        ];
        let found = pull_requests_of(&git, &tree, &row(&tree, Some("trunk")), &requests).unwrap();
        assert_eq!(found, [8, 7]);
    }

    fn now_ms() -> i64 {
        i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap()
    }

    /// Another workspace's pull request is refused, however it was asked for.
    #[test]
    fn another_workspaces_pull_request_is_not_this_ones() {
        let (git, _repo, _remote, trees) = fixture();
        let tree = trees.path().join("feature");
        let requests = [
            PullRequest {
                number: 7,
                created_at: Some(now_ms()),
                ..pr("ys/feature", PullRequestState::Open)
            },
            PullRequest {
                number: 8,
                created_at: Some(now_ms()),
                ..pr("ys/someone-else", PullRequestState::Open)
            },
        ];
        let row = row(&tree, Some("trunk"));
        assert!(ensure_own(&git, &tree, &row, &requests, 7).is_ok());
        let refused = ensure_own(&git, &tree, &row, &requests, 8).unwrap_err();
        assert_eq!(refused.code, "pull_request_changed");
        assert!(ensure_own(&git, &tree, &row, &requests, 99).is_err());
    }

    /// The workspace map is worked out once per list for as long as the list is fresh, and again
    /// on a refresh, for a different list, or after the project is forgotten.
    #[test]
    fn the_workspace_map_is_reused_while_its_list_is() {
        let forge = Forge::default();
        let one = [pr("ys/feature", PullRequestState::Open)];
        let two = [pr("ys/feature", PullRequestState::Merged)];
        let computed = std::cell::Cell::new(0);
        let ask = |requests: &[PullRequest], refresh: bool| {
            forge.owned("p1", requests, refresh, || {
                computed.set(computed.get() + 1);
                BTreeMap::from([("w1".to_owned(), vec![7])])
            })
        };
        assert_eq!(ask(&one, false)["w1"], [7]);
        ask(&one, false);
        assert_eq!(computed.get(), 1, "reused");
        ask(&one, true);
        assert_eq!(computed.get(), 2, "a refresh works it out again");
        ask(&two, false);
        assert_eq!(computed.get(), 3, "so does a different list");
        forge.forget("p1");
        ask(&two, false);
        assert_eq!(computed.get(), 4, "and a forgotten project");
    }

    #[test]
    fn late_fetches_cannot_replace_a_newer_answer_or_undo_invalidation() {
        use std::sync::{mpsc, Arc};
        for invalidate in [false, true] {
            let forge = Arc::new(Forge::default());
            let worker_forge = Arc::clone(&forge);
            let (started, started_rx) = mpsc::channel();
            let (finish, finish_rx) = mpsc::channel();
            let worker = std::thread::spawn(move || {
                worker_forge.load("p1", true, || {
                    started.send(()).unwrap();
                    finish_rx.recv().unwrap();
                    ProjectPullRequests {
                        pull_requests: vec![pr("ys/feature", PullRequestState::Merged)],
                        ..Default::default()
                    }
                })
            });
            started_rx.recv().unwrap();
            if invalidate {
                forge.forget("p1");
            } else {
                forge.load("p1", true, || ProjectPullRequests {
                    pull_requests: vec![pr("ys/feature", PullRequestState::Open)],
                    ..Default::default()
                });
            }
            finish.send(()).unwrap();
            worker.join().unwrap();
            if invalidate {
                assert!(forge.cached("p1").is_none());
            } else {
                assert_eq!(
                    forge.cached("p1").unwrap().pull_requests[0].state,
                    PullRequestState::Open
                );
            }
        }
    }

    #[test]
    fn a_reused_branch_prefers_open_then_newest_regardless_of_response_order() {
        let (git, repo, remote, trees) = fixture();
        pretend_origin_is_on_github(&git, repo.path(), remote.path());
        let tree = trees.path().join("feature");
        let mut old = pr("ys/feature", PullRequestState::Merged);
        old.number = 9;
        let mut open = pr("ys/feature", PullRequestState::Open);
        open.number = 8;
        let mut found = ProjectPullRequests {
            gh: true,
            pull_requests: vec![old, open],
            ..Default::default()
        };
        for _ in 0..2 {
            let state = state(&git, &tree, &row(&tree, Some("trunk")), &found).unwrap();
            assert_eq!(state.pull_request.unwrap().number, 8);
            assert!(!state.can_open);
            found.pull_requests.reverse();
        }
        found.pull_requests[1].state = PullRequestState::Closed;
        let state = state(&git, &tree, &row(&tree, Some("trunk")), &found).unwrap();
        assert_eq!(state.pull_request.unwrap().number, 9);
        assert!(state.can_open);
    }

    #[test]
    fn a_closed_pull_request_can_be_opened_again() {
        let (git, repo, remote, trees) = fixture();
        pretend_origin_is_on_github(&git, repo.path(), remote.path());
        let tree = trees.path().join("feature");
        let found = ProjectPullRequests {
            gh: true,
            pull_requests: vec![pr("ys/feature", PullRequestState::Merged)],
            ..Default::default()
        };
        let state = state(&git, &tree, &row(&tree, Some("trunk")), &found).unwrap();
        assert!(state.pull_request.is_some());
        assert!(state.can_open);
    }

    #[test]
    fn the_compare_url_names_both_branches() {
        let state = PublishState {
            branch: Some("ys/feature".into()),
            base: Some("main".into()),
            identity: None,
            remote: Some("origin".into()),
            repo: Some(Repo {
                host: "github.com".into(),
                owner: "o".into(),
                name: "r".into(),
                kind: ForgeKind::GitHub,
            }),
            upstream: None,
            ahead: 1,
            behind: 0,
            unpushed: vec![],
            pull_request: None,
            can_open: true,
            gh: false,
            problem: None,
            logged_out: false,
            follows_pull_request: None,
        };
        assert_eq!(
            state.compare_url().as_deref(),
            Some("https://github.com/o/r/compare/main...ys/feature?expand=1")
        );
    }

    #[test]
    fn without_gh_there_is_nothing_to_cache_and_no_complaint_about_it() {
        let forge = Forge::default();
        let dir = tempfile::tempdir().unwrap();
        let answer = forge.pull_requests(None, dir.path(), "p1", false, false, &|| None);
        assert_eq!(answer, ProjectPullRequests::default());
        assert!(!answer.gh);
        assert_eq!(
            answer.problem, None,
            "not having gh is not a problem to report"
        );
    }

    #[test]
    fn a_cached_answer_is_reused_until_it_is_forgotten() {
        let forge = Forge::default();
        let dir = tempfile::tempdir().unwrap();
        let answer = ProjectPullRequests {
            gh: true,
            pull_requests: vec![pr("ys/feature", PullRequestState::Open)],
            ..Default::default()
        };
        forge
            .cached
            .lock()
            .unwrap()
            .insert("p1".into(), (Instant::now(), answer.clone()));

        assert_eq!(
            forge.pull_requests(None, dir.path(), "p1", false, false, &|| None),
            answer
        );
        assert_eq!(
            forge.pull_requests(None, dir.path(), "p1", true, false, &|| None),
            ProjectPullRequests::default(),
            "a refresh goes back to gh, which is not there"
        );

        forge
            .cached
            .lock()
            .unwrap()
            .insert("p1".into(), (Instant::now(), answer.clone()));
        forge.forget("p1");
        assert_eq!(
            forge.pull_requests(None, dir.path(), "p1", false, false, &|| None),
            ProjectPullRequests::default(),
            "and so does a look after forgetting"
        );
    }

    fn numbered(number: u32, state: PullRequestState) -> PullRequest {
        PullRequest {
            number,
            ..pr(&format!("b{number}"), state)
        }
    }

    fn open_tier(numbers: &[u32]) -> OpenPullRequests {
        OpenPullRequests {
            pull_requests: numbers
                .iter()
                .map(|number| numbered(*number, PullRequestState::Open))
                .collect(),
            total: Some(u32::try_from(numbers.len()).unwrap()),
            viewer: Some("ada".into()),
            answered: true,
            problem: None,
            logged_out: false,
        }
    }

    fn on(host: &str, kind: ForgeKind) -> Option<Repo> {
        Some(Repo {
            host: host.into(),
            owner: "o".into(),
            name: "r".into(),
            kind,
        })
    }

    /// The two tiers as one list: the recent tier's word on a pull request both have, and the
    /// open tier's older ones after it.
    #[test]
    fn the_open_tier_adds_what_the_newest_fifty_did_not_reach() {
        let recent = ProjectPullRequests {
            gh: true,
            // #9 was merged a moment ago; the open tier, read earlier, still has it open.
            pull_requests: vec![
                numbered(10, PullRequestState::Open),
                numbered(9, PullRequestState::Merged),
            ],
            ..Default::default()
        };
        let all = compose(recent.clone(), Some(&open_tier(&[10, 9, 3, 2])));
        let rows: Vec<(u32, PullRequestState)> = all
            .pull_requests
            .iter()
            .map(|pr| (pr.number, pr.state))
            .collect();
        assert_eq!(
            rows,
            [
                (10, PullRequestState::Open),
                (9, PullRequestState::Merged),
                (3, PullRequestState::Open),
                (2, PullRequestState::Open)
            ]
        );
        assert_eq!(all.viewer.as_deref(), Some("ada"));
        assert_eq!(all.open_total, Some(4));
        assert_eq!(all.open_problem, None);

        assert_eq!(compose(recent.clone(), None), recent, "no open tier yet");
    }

    #[test]
    fn why_the_open_ones_are_incomplete_is_said_once() {
        let recent = ProjectPullRequests {
            gh: true,
            ..Default::default()
        };
        let mut partial = open_tier(&[3]);
        partial.problem = Some("HTTP 504".into());
        assert_eq!(
            compose(recent.clone(), Some(&partial))
                .open_problem
                .as_deref(),
            Some("HTTP 504")
        );
        // The recent tier already has a reason of its own: one line, not two.
        let failing = ProjectPullRequests {
            problem: Some("gh broke".into()),
            ..recent.clone()
        };
        assert_eq!(compose(failing, Some(&partial)).open_problem, None);
        partial.logged_out = true;
        assert_eq!(compose(recent, Some(&partial)).open_problem, None);
    }

    #[test]
    fn the_open_tier_is_only_asked_of_a_forge_gh_speaks_to() {
        let answering = |repo: Option<Repo>| ProjectPullRequests {
            gh: true,
            repo,
            ..Default::default()
        };
        assert_eq!(
            open_host(&answering(on("github.com", ForgeKind::GitHub))),
            Some(None),
            "gh's default host"
        );
        assert_eq!(
            open_host(&answering(on("github.example.com", ForgeKind::GitHub))),
            Some(Some("github.example.com"))
        );
        assert_eq!(
            open_host(&answering(on("work", ForgeKind::Unknown))),
            Some(None),
            "an ssh alias: gh knows what it stands for"
        );
        assert_eq!(
            open_host(&answering(on("gitlab.com", ForgeKind::GitLab))),
            None
        );
        assert_eq!(
            open_host(&answering(on("bitbucket.org", ForgeKind::Bitbucket))),
            None
        );
        assert_eq!(open_host(&answering(None)), None, "no remote to ask about");
        let mut logged_out = answering(on("github.com", ForgeKind::GitHub));
        logged_out.logged_out = true;
        assert_eq!(open_host(&logged_out), None);
        let mut without_gh = answering(on("github.com", ForgeKind::GitHub));
        without_gh.gh = false;
        assert_eq!(open_host(&without_gh), None);
    }

    /// The open tier is reused while it is fresh, read again on a refresh, kept but marked out
    /// of date when the project is forgotten, and never emptied by a read that got nothing.
    #[test]
    fn the_open_tier_is_kept_until_something_better_arrives() {
        let forge = Forge::default();
        let asked = std::cell::Cell::new(0);
        let read = |refresh: bool, answer: OpenPullRequests| {
            forge.load_open("p1", refresh, || {
                asked.set(asked.get() + 1);
                answer
            });
        };
        let held = || {
            let open = forge.open.lock().unwrap();
            let tier = open.get("p1").unwrap();
            let numbers: Vec<u32> = tier
                .found
                .pull_requests
                .iter()
                .map(|pr| pr.number)
                .collect();
            (numbers, tier.found.problem.clone(), tier.at.is_some())
        };

        read(false, open_tier(&[3, 2]));
        read(false, open_tier(&[1]));
        assert_eq!(asked.get(), 1, "fresh: not asked again");
        assert_eq!(held(), (vec![3, 2], None, true));

        read(true, open_tier(&[4, 3]));
        assert_eq!(asked.get(), 2, "a refresh asks");
        assert_eq!(held().0, [4, 3]);

        forge.forget("p1");
        assert_eq!(
            held(),
            (vec![4, 3], None, false),
            "still shown, known to be old"
        );
        let nothing = OpenPullRequests {
            problem: Some("could not resolve host".into()),
            ..Default::default()
        };
        read(false, nothing);
        assert_eq!(asked.get(), 3, "out of date: asked without a refresh");
        assert_eq!(
            held(),
            (vec![4, 3], Some("could not resolve host".into()), true),
            "nothing arrived, so what was known stays, with the reason"
        );

        // Half a list is an answer: it replaces what was there.
        let mut partial = open_tier(&[5]);
        partial.problem = Some("HTTP 504".into());
        read(true, partial);
        assert_eq!(held(), (vec![5], Some("HTTP 504".into()), true));
    }

    #[test]
    fn an_open_tier_read_before_the_project_changed_is_dropped() {
        use std::sync::{mpsc, Arc};
        let forge = Arc::new(Forge::default());
        forge.load_open("p1", true, || open_tier(&[2]));
        let worker_forge = Arc::clone(&forge);
        let (started, started_rx) = mpsc::channel();
        let (finish, finish_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            worker_forge.load_open("p1", true, || {
                started.send(()).unwrap();
                finish_rx.recv().unwrap();
                open_tier(&[9])
            });
        });
        started_rx.recv().unwrap();
        // A merge, say: what is on its way was read before it.
        forge.forget("p1");
        finish.send(()).unwrap();
        worker.join().unwrap();
        let open = forge.open.lock().unwrap();
        let tier = open.get("p1").unwrap();
        assert_eq!(tier.found.pull_requests[0].number, 2);
        assert!(tier.at.is_none(), "and it is still due to be read again");
    }

    /// Without the view asking, the open tier is not read — and what it last said is still in
    /// the answer.
    #[test]
    fn only_a_full_look_reads_the_open_tier() {
        let forge = Forge::default();
        let dir = tempfile::tempdir().unwrap();
        forge.load_open("p1", true, || open_tier(&[3]));
        let answer = forge.pull_requests(None, dir.path(), "p1", false, false, &|| None);
        assert_eq!(
            answer.pull_requests.len(),
            1,
            "from the open tier, as last read"
        );
        assert_eq!(answer.open_total, Some(1));
        // A full look without gh has nobody to ask, and says nothing about it.
        let full = forge.pull_requests(None, dir.path(), "p1", true, true, &|| None);
        assert_eq!(full.pull_requests.len(), 1);
        assert_eq!(full.open_problem, None);
    }

    fn in_full(number: u32, body: &str) -> PullRequestSummary {
        PullRequestSummary {
            pull_request: numbered(number, PullRequestState::Open),
            body: body.into(),
            changed_files: 1,
            posts: vec![],
        }
    }

    /// A pull request read in full is reused while it is fresh, read again on a refresh and
    /// after the project is forgotten, and a failure is passed on rather than remembered.
    #[test]
    fn a_pull_request_in_full_is_reused_until_something_changes() {
        let forge = Forge::default();
        let asked = std::cell::Cell::new(0);
        let read = |number: u32, refresh: bool, body: &str| {
            forge.summary("p1", number, refresh, || {
                asked.set(asked.get() + 1);
                Ok(in_full(number, body))
            })
        };
        assert_eq!(read(7, false, "first").unwrap().body, "first");
        assert_eq!(
            read(7, false, "second").unwrap().body,
            "first",
            "fresh: reused"
        );
        assert_eq!(asked.get(), 1);
        assert_eq!(
            read(8, false, "other").unwrap().body,
            "other",
            "another number"
        );
        assert_eq!(
            read(7, true, "third").unwrap().body,
            "third",
            "a refresh asks"
        );

        forge.forget("p2");
        assert_eq!(
            read(7, false, "x").unwrap().body,
            "third",
            "another project's news"
        );
        forge.forget("p1");
        assert_eq!(read(7, false, "fourth").unwrap().body, "fourth");
        assert_eq!(read(8, false, "fifth").unwrap().body, "fifth");

        let failed = forge.summary("p1", 7, true, || {
            Err(crate::forge::ForgeError::Unreadable("nope".into()))
        });
        assert!(failed.is_err());
        assert_eq!(
            read(7, false, "y").unwrap().body,
            "fourth",
            "a failure does not replace what was known"
        );
    }

    /// A merge lands while a summary is being read: what was read is from before it.
    #[test]
    fn a_summary_read_before_the_project_changed_is_not_kept() {
        let forge = Forge::default();
        let stale = forge
            .summary("p1", 7, false, || {
                forge.forget("p1");
                Ok(in_full(7, "before the merge"))
            })
            .unwrap();
        assert_eq!(
            stale.body, "before the merge",
            "still the answer to that question"
        );
        let fresh = forge
            .summary("p1", 7, false, || Ok(in_full(7, "after")))
            .unwrap();
        assert_eq!(fresh.body, "after", "and asked again by the next one");
    }

    /// What `find` answers is what the list shows: the recent tier's copy, which is the one
    /// refreshed every minute, before the open tier's older one.
    #[test]
    fn a_pull_request_is_found_in_the_lists_as_the_window_sees_it() {
        let forge = Forge::default();
        assert_eq!(forge.find("p1", 7), None);

        forge.load_open("p1", true, || open_tier(&[7, 3]));
        assert_eq!(forge.find("p1", 3).map(|pr| pr.number), Some(3));
        assert_eq!(forge.find("p1", 9), None);

        let mut merged = numbered(7, PullRequestState::Merged);
        merged.details = None;
        forge.load("p1", true, || ProjectPullRequests {
            gh: true,
            pull_requests: vec![merged],
            ..Default::default()
        });
        assert_eq!(
            forge.find("p1", 7).map(|pr| pr.state),
            Some(PullRequestState::Merged),
            "the recent tier's word, not the open tier's older one"
        );
        assert_eq!(forge.find("p2", 7), None);
    }

    #[test]
    fn a_stale_answer_is_asked_again() {
        let forge = Forge::default();
        let dir = tempfile::tempdir().unwrap();
        let stale = Instant::now() - FRESH_FOR - Duration::from_secs(1);
        forge.cached.lock().unwrap().insert(
            "p1".into(),
            (
                stale,
                ProjectPullRequests {
                    gh: true,
                    pull_requests: vec![pr("ys/feature", PullRequestState::Open)],
                    ..Default::default()
                },
            ),
        );
        assert_eq!(
            forge.pull_requests(None, dir.path(), "p1", false, false, &|| None),
            ProjectPullRequests::default()
        );
    }
}
