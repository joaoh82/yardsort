//! What the Pull requests view does to a pull request: merge it, close it, reopen it, and get
//! its branch ready for a workspace.
//!
//! These act on *any* pull request of a project, which is the point of that view and the
//! opposite of the toolbar's rule (`ensure_own`): there, a pull request the workspace did not
//! open is someone else's work reached by accident. Here the confirmation names whose it is.
//! What the two share is that nothing rests on an answer half a minute old — the forge is asked
//! about the one pull request again, now, before anything is done to it.

use std::path::{Path, PathBuf};

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use super::commands::{failed, project_root, pull_request_changed, still_mergeable};
use crate::error::{IpcError, IpcResult};
use crate::forge::{
    parse_remote, ForgeError, Gh, MergeMethod, PullRequest, PullRequestState, PullRequestSummary,
};
use crate::git::Git;
use crate::state::{blocking, AppState};

/// Pull request `number` as the forge has it this moment, with what is needed to act on it.
fn fresh(state: &AppState, project_id: &str, number: u32) -> IpcResult<(Gh, PathBuf, PullRequest)> {
    let root = project_root(state, project_id)?;
    let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
    let pr = gh.pull_request(&root, number).map_err(failed)?;
    Ok((gh, root, pr))
}

/// How long `gh` may take over one pull request. It answers in about a second; this is for a
/// network that went away.
const SUMMARY_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

/// One pull request in full: its description, every check with its link, its reviewers and
/// the conversation. What the Pull requests view's Summary shows, read when a row is opened.
#[tauri::command]
#[specta::specta]
pub async fn pull_request_summary(
    app: AppHandle,
    project_id: String,
    number: u32,
    refresh: bool,
) -> IpcResult<PullRequestSummary> {
    blocking(app, move |state| {
        let root = project_root(state, &project_id)?;
        let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
        state
            .forge
            .summary(&project_id, number, refresh, || {
                gh.pull_request_summary(&root, number, SUMMARY_LIMIT)
            })
            .map_err(failed)
    })
    .await
}

fn no_longer(number: u32, what: &str) -> IpcError {
    IpcError::new(
        "pull_request_changed",
        format!("Pull request #{number} is {what}. Refresh and look again."),
    )
}

fn closable(pr: &PullRequest) -> IpcResult<()> {
    match pr.state {
        PullRequestState::Open => Ok(()),
        PullRequestState::Merged => Err(no_longer(pr.number, "already merged")),
        PullRequestState::Closed => Err(no_longer(pr.number, "already closed")),
    }
}

fn reopenable(pr: &PullRequest) -> IpcResult<()> {
    match pr.state {
        PullRequestState::Closed => Ok(()),
        PullRequestState::Merged => Err(no_longer(pr.number, "merged, and cannot be reopened")),
        PullRequestState::Open => Err(no_longer(pr.number, "already open")),
    }
}

/// Merge a pull request of this project, if it is still open, still not a draft and still at
/// the head commit the user confirmed. Never deletes a branch, never bypasses a protection.
#[tauri::command]
#[specta::specta]
pub async fn pull_request_merge(
    app: AppHandle,
    project_id: String,
    number: u32,
    head_oid: String,
    method: MergeMethod,
) -> IpcResult<()> {
    blocking(app, move |state| {
        let (gh, root, pr) = fresh(state, &project_id, number)?;
        if !still_mergeable(&pr, &head_oid) {
            return Err(pull_request_changed());
        }
        let result = gh.merge_pull_request(&root, number, &head_oid, method);
        state.forge.forget(&project_id);
        result.map_err(failed)
    })
    .await
}

/// Close a pull request without merging it. Its branch stays, and nothing is posted on it.
#[tauri::command]
#[specta::specta]
pub async fn pull_request_close(app: AppHandle, project_id: String, number: u32) -> IpcResult<()> {
    blocking(app, move |state| {
        let (gh, root, pr) = fresh(state, &project_id, number)?;
        closable(&pr)?;
        let result = gh.close_pull_request(&root, number);
        state.forge.forget(&project_id);
        result.map_err(failed)
    })
    .await
}

/// Open a closed pull request again.
#[tauri::command]
#[specta::specta]
pub async fn pull_request_reopen(app: AppHandle, project_id: String, number: u32) -> IpcResult<()> {
    blocking(app, move |state| {
        let (gh, root, pr) = fresh(state, &project_id, number)?;
        reopenable(&pr)?;
        let result = gh.reopen_pull_request(&root, number);
        state.forge.forget(&project_id);
        result.map_err(failed)
    })
    .await
}

/// A local branch a workspace can be opened on for a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PreparedBranch {
    pub branch: String,
    /// Commits the pull request has that this branch does not. Only a branch that was already
    /// here can be behind: it is opened as it is and never moved.
    pub behind: u32,
    /// The pull request comes from a fork, so the branch is `pr/<number>` and follows the pull
    /// request rather than a branch of this project's remote. Yardsort will not push it.
    pub fork: bool,
}

/// Get pull request `number`'s branch into the project's repository, ready for the composer's
/// "open existing branch". Nothing is checked out and no worktree is made here: that is the
/// composer's, with the agent and the message the user gives it.
#[tauri::command]
#[specta::specta]
pub async fn pull_request_prepare_branch(
    app: AppHandle,
    project_id: String,
    number: u32,
) -> IpcResult<PreparedBranch> {
    blocking(app, move |state| {
        let (_, root, pr) = fresh(state, &project_id, number)?;
        closable(&pr)?;
        let git = Git::new(&state.env())?;
        prepare_branch(&git, &root, &pr)
    })
    .await
}

/// See [`pull_request_prepare_branch`]. Four cases, by where the branch lives:
///
/// 1. Checked out somewhere already — refused, saying where. Git allows a branch one place.
/// 2. Here and not checked out — left exactly as it is, however far behind.
/// 3. In this project's remote and not here — fetched, and a local branch made that tracks it.
/// 4. In a fork — `pr/<number>`, made at the pull request's head and set to follow it.
pub fn prepare_branch(git: &Git, root: &Path, pr: &PullRequest) -> IpcResult<PreparedBranch> {
    let fork = pr
        .details
        .as_ref()
        .is_some_and(|details| details.cross_repository);
    let remote = remote_of(git, root, &pr.url)?.ok_or_else(|| {
        IpcError::new(
            "no_remote",
            "This project has no remote to fetch the pull request from.",
        )
    })?;

    if fork {
        let branch = format!("pr/{}", pr.number);
        not_checked_out(git, root, &branch)?;
        let head = git.fetch_pull_request(root, &remote, pr.number)?;
        let behind = if git.branch_exists(root, &branch)? {
            git.commits_between(root, &branch, &head)?
        } else {
            git.branch_create(root, &branch, &head, false)?;
            git.follow_pull_request(root, &branch, &remote, pr.number)?;
            0
        };
        return Ok(PreparedBranch {
            branch,
            behind,
            fork: true,
        });
    }

    let branch = pr.branch.clone();
    // The name is the forge's, which is to say whoever opened the pull request chose it.
    if !git.valid_branch_name(root, &branch)? {
        return Err(IpcError::new(
            "invalid_branch",
            format!("\"{branch}\" is not a branch name git will take."),
        ));
    }
    not_checked_out(git, root, &branch)?;
    git.fetch_branch(root, &remote, &branch)?;
    let tracking = format!("refs/remotes/{remote}/{branch}");
    let behind = if git.branch_exists(root, &branch)? {
        git.commits_between(root, &branch, &tracking)?
    } else {
        git.branch_create(root, &branch, &tracking, true)?;
        0
    };
    Ok(PreparedBranch {
        branch,
        behind,
        fork: false,
    })
}

fn not_checked_out(git: &Git, root: &Path, branch: &str) -> IpcResult<()> {
    let holder = git
        .worktrees(root)?
        .into_iter()
        .find(|entry| !entry.prunable && entry.branch.as_deref() == Some(branch));
    match holder {
        Some(entry) => Err(IpcError::new(
            "branch_checked_out",
            format!(
                "\"{branch}\" is already checked out at {}.",
                entry.path.display()
            ),
        )),
        None => Ok(()),
    }
}

/// The remote the pull request's repository is: the one whose URL names the same owner and
/// repository as the pull request's own. In a clone of a fork that is `upstream`, not `origin`
/// — `gh` lists the parent's pull requests there, and only the parent has their branches. When
/// no remote can be matched (a URL this cannot read), the one a push would go to.
fn remote_of(git: &Git, root: &Path, pull_request_url: &str) -> IpcResult<Option<String>> {
    let wanted = pull_request_url
        .split_once("/pull/")
        .and_then(|(repository, _)| parse_remote(repository));
    if let Some(wanted) = wanted {
        for remote in git.remotes(root)? {
            let here = git
                .remote_url(root, &remote)?
                .as_deref()
                .and_then(parse_remote);
            if here.is_some_and(|here| {
                here.owner.eq_ignore_ascii_case(&wanted.owner)
                    && here.name.eq_ignore_ascii_case(&wanted.name)
            }) {
                return Ok(Some(remote));
            }
        }
    }
    Ok(git.push_remote(root)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::{CheckCounts, Checks, Mergeable, PullRequestDetails};
    use crate::git::testing;
    use crate::publish::{pull_requests_of, state, ProjectPullRequests};
    use crate::store::WorkspaceRow;

    fn pr(number: u32, branch: &str, fork: bool) -> PullRequest {
        PullRequest {
            number,
            url: format!("https://github.com/o/r/pull/{number}"),
            title: "Their work".into(),
            branch: branch.into(),
            state: PullRequestState::Open,
            draft: false,
            checks: Checks::None,
            author: Some("grace".into()),
            created_at: None,
            details: Some(PullRequestDetails {
                base: "trunk".into(),
                head_oid: "abc".into(),
                additions: 1,
                deletions: 0,
                review: String::new(),
                updated_at: String::new(),
                checks: vec![],
                check_counts: CheckCounts::default(),
                mergeable: Mergeable::Mergeable,
                review_requests: vec![],
                reviews: vec![],
                cross_repository: fork,
            }),
        }
    }

    /// A repository on `trunk` whose bare remote also has someone else's branch `theirs`, one
    /// commit ahead, published as pull request 7 the way a forge does: `refs/pull/7/head`.
    /// Nothing of it is in the local repository yet. Returns the commit the pull request is at.
    fn fixture() -> (Git, tempfile::TempDir, tempfile::TempDir, String) {
        let git = testing::git();
        let repo = tempfile::tempdir().unwrap();
        git.init(repo.path()).unwrap();
        git.run(repo.path(), &["checkout", "-b", "trunk"]).unwrap();
        git.initial_commit(repo.path()).unwrap();
        let remote = tempfile::tempdir().unwrap();
        git.run(remote.path(), &["init", "--bare"]).unwrap();
        let url = remote.path().to_string_lossy().into_owned();
        git.run(repo.path(), &["remote", "add", "origin", &url])
            .unwrap();
        git.push(repo.path(), "origin", "trunk").unwrap();

        // Someone else's clone does the work and pushes it.
        let theirs = tempfile::tempdir().unwrap();
        git.run(theirs.path(), &["clone", "--quiet", &url, "."])
            .unwrap();
        git.run(theirs.path(), &["checkout", "-b", "theirs"])
            .unwrap();
        std::fs::write(theirs.path().join("work.txt"), "theirs").unwrap();
        git.commit_all(theirs.path(), "Their work").unwrap();
        git.run(theirs.path(), &["push", "--quiet", "origin", "theirs"])
            .unwrap();
        git.run(
            theirs.path(),
            &["push", "--quiet", "origin", "theirs:refs/pull/7/head"],
        )
        .unwrap();
        let head = git.run(theirs.path(), &["rev-parse", "HEAD"]).unwrap();
        (git, repo, remote, head)
    }

    fn rev(git: &Git, root: &Path, name: &str) -> Option<String> {
        git.run(root, &["rev-parse", "--verify", "--quiet", name])
            .ok()
    }

    #[test]
    fn a_branch_of_the_projects_remote_is_fetched_and_tracked() {
        let (git, repo, _remote, head) = fixture();
        assert!(!git.branch_exists(repo.path(), "theirs").unwrap());

        let prepared = prepare_branch(&git, repo.path(), &pr(7, "theirs", false)).unwrap();
        assert_eq!(
            prepared,
            PreparedBranch {
                branch: "theirs".into(),
                behind: 0,
                fork: false
            }
        );
        assert_eq!(
            rev(&git, repo.path(), "theirs").as_deref(),
            Some(head.as_str())
        );
        assert_eq!(
            git.run(repo.path(), &["rev-parse", "--abbrev-ref", "theirs@{u}"])
                .unwrap(),
            "origin/theirs",
            "so a push from the workspace goes back to the pull request"
        );
        assert_eq!(
            git.head(repo.path()).unwrap(),
            crate::git::Head::Branch("trunk".into()),
            "nothing was checked out"
        );
        assert_eq!(
            git.followed_pull_request(repo.path(), "theirs").unwrap(),
            None
        );
    }

    /// The rule that matters most here: a branch the user already has is theirs. It is opened
    /// as it is, and the composer is told how far behind the pull request it is.
    #[test]
    fn a_branch_that_is_already_here_is_never_moved() {
        let (git, repo, _remote, head) = fixture();
        git.run(repo.path(), &["branch", "theirs", "trunk"])
            .unwrap();
        let before = rev(&git, repo.path(), "theirs").unwrap();
        assert_ne!(before, head);

        let prepared = prepare_branch(&git, repo.path(), &pr(7, "theirs", false)).unwrap();
        assert_eq!(prepared.behind, 1, "the pull request's one commit");
        assert_eq!(
            rev(&git, repo.path(), "theirs").unwrap(),
            before,
            "not moved"
        );

        // And again, now that it is up to date by the user's own hand.
        git.run(repo.path(), &["branch", "-f", "theirs", &head])
            .unwrap();
        let prepared = prepare_branch(&git, repo.path(), &pr(7, "theirs", false)).unwrap();
        assert_eq!(prepared.behind, 0);
    }

    #[test]
    fn a_forks_pull_request_becomes_a_branch_that_follows_it() {
        let (git, repo, _remote, head) = fixture();
        // On the forge its branch is `patch-1`, in a repository this one has no remote for.
        let prepared = prepare_branch(&git, repo.path(), &pr(7, "patch-1", true)).unwrap();
        assert_eq!(
            prepared,
            PreparedBranch {
                branch: "pr/7".into(),
                behind: 0,
                fork: true
            }
        );
        assert_eq!(
            rev(&git, repo.path(), "pr/7").as_deref(),
            Some(head.as_str())
        );
        assert_eq!(
            git.followed_pull_request(repo.path(), "pr/7").unwrap(),
            Some(7)
        );
        assert!(!git.branch_exists(repo.path(), "patch-1").unwrap());
        assert_eq!(
            rev(&git, repo.path(), "refs/remotes/origin/theirs"),
            None,
            "no remote-tracking ref was touched"
        );

        // A second time finds the branch and leaves it.
        let again = prepare_branch(&git, repo.path(), &pr(7, "patch-1", true)).unwrap();
        assert_eq!(again, prepared);
    }

    /// A workspace on a fork's pull request: tied to it by git's config, so it shows the pull
    /// request, offers no new one, and cannot be pushed from Yardsort.
    #[test]
    fn a_workspace_on_a_forks_pull_request_shows_it_and_is_not_pushed() {
        let (git, repo, remote, _head) = fixture();
        prepare_branch(&git, repo.path(), &pr(7, "patch-1", true)).unwrap();
        let trees = tempfile::tempdir().unwrap();
        let tree = trees.path().join("pr-7");
        git.worktree_add_existing(repo.path(), &tree, "pr/7")
            .unwrap();
        // Something of the user's own on top, which is exactly when Push would be offered.
        std::fs::write(tree.join("mine.txt"), "mine").unwrap();
        git.commit_all(&tree, "My fix").unwrap();

        let row = WorkspaceRow {
            id: "w1".into(),
            project_id: "p1".into(),
            kind: "worktree".into(),
            name: "pr-7".into(),
            path: tree.to_string_lossy().into_owned(),
            branch: Some("pr/7".into()),
            base_branch: None,
            archived: false,
            forgotten: false,
        };
        let found = ProjectPullRequests {
            gh: true,
            pull_requests: vec![pr(8, "pr/7", false), pr(7, "patch-1", true)],
            ..Default::default()
        };
        let state = state(&git, &tree, &row, &found).unwrap();
        assert_eq!(state.follows_pull_request, Some(7));
        assert_eq!(
            state.pull_request.as_ref().map(|pr| pr.number),
            Some(7),
            "found by number, not by a branch that happens to share its local name"
        );
        assert!(!state.can_open);
        assert_eq!(
            pull_requests_of(&git, &tree, &row, &found.pull_requests).unwrap()[0],
            7
        );

        let refused = super::super::commands::push(&git, &tree, &state).unwrap_err();
        assert_eq!(refused.code, "follows_pull_request");
        assert_eq!(
            rev(&git, remote.path(), "refs/heads/pr/7"),
            None,
            "nothing reached the remote"
        );
    }

    #[test]
    fn a_branch_checked_out_somewhere_is_refused_saying_where() {
        let (git, repo, _remote, _head) = fixture();
        prepare_branch(&git, repo.path(), &pr(7, "theirs", false)).unwrap();
        let trees = tempfile::tempdir().unwrap();
        let tree = trees.path().join("theirs");
        git.worktree_add_existing(repo.path(), &tree, "theirs")
            .unwrap();

        let refused = prepare_branch(&git, repo.path(), &pr(7, "theirs", false)).unwrap_err();
        assert_eq!(refused.code, "branch_checked_out");
        assert!(refused.message.contains("theirs"), "{}", refused.message);
    }

    #[test]
    fn a_name_git_would_read_as_an_option_goes_nowhere() {
        let (git, repo, _remote, _head) = fixture();
        for name in ["--upload-pack=touch /tmp/x", "-f", "a..b", "with space"] {
            let refused = prepare_branch(&git, repo.path(), &pr(7, name, false)).unwrap_err();
            assert_eq!(refused.code, "invalid_branch", "{name}");
        }
    }

    #[test]
    fn a_pull_request_whose_branch_is_gone_says_what_git_said() {
        let (git, repo, _remote, _head) = fixture();
        let refused = prepare_branch(&git, repo.path(), &pr(9, "deleted", false)).unwrap_err();
        assert!(refused.message.contains("deleted"), "{}", refused.message);
        assert!(!git.branch_exists(repo.path(), "deleted").unwrap());
    }

    /// In a clone of a fork, the pull requests are the parent's and so are their branches.
    #[test]
    fn the_remote_is_the_one_the_pull_request_lives_on() {
        let git = testing::git();
        let repo = tempfile::tempdir().unwrap();
        git.init(repo.path()).unwrap();
        for (name, url) in [
            ("origin", "git@github.com:me/r.git"),
            ("upstream", "https://github.com/O/R.git"),
        ] {
            git.run(repo.path(), &["remote", "add", name, url]).unwrap();
        }
        let of = |url: &str| remote_of(&git, repo.path(), url).unwrap();
        assert_eq!(
            of("https://github.com/o/r/pull/7").as_deref(),
            Some("upstream")
        );
        assert_eq!(
            of("https://github.com/me/r/pull/7").as_deref(),
            Some("origin")
        );
        assert_eq!(
            of("https://github.com/someone/else/pull/7").as_deref(),
            Some("origin"),
            "the push remote, for want of a match"
        );
    }

    #[test]
    fn only_an_open_one_closes_and_only_a_closed_one_reopens() {
        let mut one = pr(7, "theirs", false);
        assert!(closable(&one).is_ok());
        assert_eq!(reopenable(&one).unwrap_err().code, "pull_request_changed");
        one.state = PullRequestState::Closed;
        assert!(reopenable(&one).is_ok());
        assert!(closable(&one).is_err());
        one.state = PullRequestState::Merged;
        assert!(closable(&one).is_err());
        let merged = reopenable(&one).unwrap_err();
        assert!(
            merged.message.contains("cannot be reopened"),
            "{}",
            merged.message
        );
    }

    /// What the confirmation showed is what is merged, or nothing is.
    #[test]
    fn a_merge_needs_the_head_that_was_confirmed() {
        let mut one = pr(7, "theirs", false);
        assert!(still_mergeable(&one, "abc"));
        assert!(!still_mergeable(&one, "def"), "someone pushed since");
        assert!(!still_mergeable(&one, ""));
        one.draft = true;
        assert!(!still_mergeable(&one, "abc"), "a draft is not ready");
        one.draft = false;
        one.state = PullRequestState::Merged;
        assert!(!still_mergeable(&one, "abc"));
    }
}
