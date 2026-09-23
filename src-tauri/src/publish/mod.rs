//! Getting a workspace's work out: commit, push, open the pull request, and say what became of
//! it afterwards.
//!
//! The git half always works. The forge half is [`crate::forge`]'s: `gh` when the user has it,
//! a link to the forge's own form when they do not. Nothing here ever holds a forge credential.
//!
//! Pull requests are fetched **per project, not per workspace**. A project with a dozen
//! workspaces would otherwise make a dozen network calls every time the window came back into
//! focus, so one `gh pr list` answers for all of them and the answer is cached for
//! [`FRESH_FOR`]; a workspace finds its own by branch name.

pub mod commands;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use specta::Type;

use crate::error::IpcResult;
use crate::forge::{parse_remote, Gh, PullRequest, PullRequestState, Repo};
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

    let pull_request = branch.as_ref().and_then(|branch| {
        found
            .pull_requests
            .iter()
            .find(|pr| &pr.branch == branch)
            .cloned()
    });

    // Somewhere to open it, and nothing open there already. A merged or closed one does not
    // stand in the way: the branch may well have moved on since.
    let can_open = branch.is_some()
        && base.is_some()
        && repo.is_some()
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
    })
}

/// The project-wide pull request cache. One per app.
#[derive(Default)]
pub struct Forge {
    cached: Mutex<HashMap<String, (Instant, ProjectPullRequests)>>,
}

impl Forge {
    /// This project's pull requests, from the cache when it is fresh enough.
    ///
    /// Blocking: it runs `gh`, which talks to the network. Never an `Err` — a forge that cannot
    /// be reached is a missing badge, not a failed command, and the reason travels in
    /// [`ProjectPullRequests::problem`] so the panel can show it where it belongs.
    pub fn pull_requests(
        &self,
        gh: Option<&Gh>,
        root: &Path,
        project_id: &str,
        refresh: bool,
    ) -> ProjectPullRequests {
        if !refresh {
            if let Some(fresh) = self.cached_fresh(project_id) {
                return fresh;
            }
        }
        let answer = ask(gh, root);
        self.cached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(project_id.to_owned(), (Instant::now(), answer.clone()));
        answer
    }

    fn cached_fresh(&self, project_id: &str) -> Option<ProjectPullRequests> {
        let cached = self.cached.lock().unwrap_or_else(PoisonError::into_inner);
        let (at, answer) = cached.get(project_id)?;
        (at.elapsed() < FRESH_FOR).then(|| answer.clone())
    }

    /// Drop what is remembered about a project, so the next look asks again. Called after
    /// anything that changes the answer — a push, a pull request being opened.
    pub fn forget(&self, project_id: &str) {
        self.cached
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(project_id);
    }
}

fn ask(gh: Option<&Gh>, root: &Path) -> ProjectPullRequests {
    let Some(gh) = gh else {
        return ProjectPullRequests::default();
    };
    match gh.pull_requests(root, LIMIT) {
        Ok(pull_requests) => ProjectPullRequests {
            gh: true,
            pull_requests,
            problem: None,
            logged_out: false,
        },
        Err(error) => ProjectPullRequests {
            gh: true,
            pull_requests: vec![],
            logged_out: error.is_logged_out(),
            problem: Some(error.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::{Checks, ForgeKind};
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
            problem: None,
            logged_out: false,
        };
        let state = state(&git, &tree, &row(&tree, Some("trunk")), &found).unwrap();
        assert_eq!(
            state.pull_request.map(|pr| pr.branch).as_deref(),
            Some("ys/feature")
        );
        assert!(!state.can_open, "it is already open");
    }

    #[test]
    fn a_closed_pull_request_can_be_opened_again() {
        let (git, repo, remote, trees) = fixture();
        pretend_origin_is_on_github(&git, repo.path(), remote.path());
        let tree = trees.path().join("feature");
        let found = ProjectPullRequests {
            gh: true,
            pull_requests: vec![pr("ys/feature", PullRequestState::Merged)],
            problem: None,
            logged_out: false,
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
        let answer = forge.pull_requests(None, dir.path(), "p1", false);
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
            problem: None,
            logged_out: false,
        };
        forge
            .cached
            .lock()
            .unwrap()
            .insert("p1".into(), (Instant::now(), answer.clone()));

        assert_eq!(forge.pull_requests(None, dir.path(), "p1", false), answer);
        assert_eq!(
            forge.pull_requests(None, dir.path(), "p1", true),
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
            forge.pull_requests(None, dir.path(), "p1", false),
            ProjectPullRequests::default(),
            "and so does a look after forgetting"
        );
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
                    problem: None,
                    logged_out: false,
                },
            ),
        );
        assert_eq!(
            forge.pull_requests(None, dir.path(), "p1", false),
            ProjectPullRequests::default()
        );
    }
}
