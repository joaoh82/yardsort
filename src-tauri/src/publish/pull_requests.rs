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
use super::ProjectPullRequests;
use crate::changes::{Between, FileChange, FileDiff};
use crate::error::{IpcError, IpcResult};
use crate::forge::{
    parse_remote, ForgeError, Gh, MergeMethod, PullRequest, PullRequestState, PullRequestSummary,
};
use crate::git::{is_commit_id, pull_request_ref, Git, PULL_REQUEST_REFS};
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

/// A pull request's diff: the files it changes, and the two commits they are read between.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestChanges {
    pub files: Vec<FileChange>,
    /// The pull request's head, as fetched. [`pull_request_diff`] is asked with it.
    pub head_oid: String,
    /// Where the pull request left its base branch: what the diff is measured from.
    pub base_oid: String,
}

/// The files a pull request changes. Fetches its commits into refs of Yardsort's own if they
/// are not here yet — nothing is checked out, and no branch or remote-tracking ref moves.
///
/// Which head and base: the ones the project's list names. The list is what the window shows,
/// it is what notices a push, and it is already here — so this asks the forge for nothing. A
/// pull request the list does not have (the list was emptied by an action a moment ago, say)
/// is asked about afresh, never out of a cache: a diff is only as good as its head.
#[tauri::command]
#[specta::specta]
pub async fn pull_request_changes(
    app: AppHandle,
    project_id: String,
    number: u32,
) -> IpcResult<PullRequestChanges> {
    blocking(app, move |state| {
        let root = project_root(state, &project_id)?;
        let pr = match state.forge.find(&project_id, number) {
            Some(pr) => pr,
            None => {
                let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
                gh.pull_request(&root, number).map_err(failed)?
            }
        };
        let git = Git::new(&state.env())?;
        changes_of(&git, &root, &pr)
    })
    .await
}

/// Both sides of one file of a pull request, between the two commits
/// [`pull_request_changes`] answered with.
#[tauri::command]
#[specta::specta]
pub async fn pull_request_diff(
    app: AppHandle,
    project_id: String,
    base_oid: String,
    head_oid: String,
    path: String,
    old_path: Option<String>,
) -> IpcResult<FileDiff> {
    blocking(app, move |state| {
        let root = project_root(state, &project_id)?;
        // They come back from the window; they go to git only as commit ids, written in full.
        commit_id(&base_oid)?;
        commit_id(&head_oid)?;
        let git = Git::new(&state.env())?;
        Between {
            git: &git,
            root: &root,
            old: &base_oid,
            new: &head_oid,
        }
        .diff(&path, old_path.as_deref())
    })
    .await
}

fn commit_id(text: &str) -> IpcResult<()> {
    if is_commit_id(text) {
        Ok(())
    } else {
        Err(IpcError::new(
            "not_a_commit",
            "That is not a commit id. Open the pull request's Code again.",
        ))
    }
}

/// See [`pull_request_changes`]. `pr` is the pull request as the list has it now.
pub fn changes_of(git: &Git, root: &Path, pr: &PullRequest) -> IpcResult<PullRequestChanges> {
    let details = pr.details.as_ref();
    let head = details.map(|d| d.head_oid.as_str()).unwrap_or_default();
    let base = details.map(|d| d.base_oid.as_str()).unwrap_or_default();
    let base_branch = details.map(|d| d.base.as_str()).unwrap_or_default();
    let remote = remote_of(git, root, &pr.url)?.ok_or_else(|| {
        IpcError::new(
            "no_remote",
            "This project has no remote to fetch the pull request from.",
        )
    })?;
    let (head_oid, base_oid) =
        fetch_for_diff(git, root, &remote, pr.number, head, base, base_branch)?;
    let files = Between {
        git,
        root,
        old: &base_oid,
        new: &head_oid,
    }
    .list()?;
    Ok(PullRequestChanges {
        files,
        head_oid,
        base_oid,
    })
}

/// Make sure pull request `number`'s commits are in the repository, and say which two the diff
/// is between: its head, and the commit where it left its base.
///
/// `head` and `base` are what the forge named, as the project's list has them. Both are kept
/// under refs of Yardsort's own ([`PULL_REQUEST_REFS`]), so git does not collect them and
/// nothing the user has is touched. When the ref already *is* `head` and the base is here,
/// nothing is fetched — which is why `head` must be the newest the list knows, not something
/// remembered from an earlier look. The head that is answered with is the one that was
/// fetched: if the pull request moved on between the list and the fetch, the newer one is the
/// truth.
///
/// The base is asked for by its commit id, which a forge answers. If the server will not, the
/// head is fetched alone and the base branch by name: its tip rather than the commit the forge
/// recorded, which is the same thing for an open pull request and close enough to show one.
pub fn fetch_for_diff(
    git: &Git,
    root: &Path,
    remote: &str,
    number: u32,
    head: &str,
    base: &str,
    base_branch: &str,
) -> IpcResult<(String, String)> {
    let head_ref = pull_request_ref(number, "head");
    let base_ref = pull_request_ref(number, "base");
    // Ids from the forge go to git as arguments, so they are checked to be ids and nothing else.
    let base = Some(base).filter(|id| is_commit_id(id));
    let here = is_commit_id(head)
        && git.commit_id(root, &head_ref)?.as_deref() == Some(head)
        && match base {
            Some(base) => git.commit_id(root, base)?.is_some(),
            None => false,
        };
    if !here && git.fetch_pull_request(root, remote, number, base).is_err() {
        // Without the base, then: this is the fetch whose failure is the user's to read.
        git.fetch_pull_request(root, remote, number, None)?;
    }
    let head_oid = git.commit_id(root, &head_ref)?.ok_or_else(|| {
        IpcError::new(
            "pull_request_not_fetched",
            format!("Pull request #{number} has no commits to show."),
        )
    })?;

    let base_oid = match base {
        Some(base) if git.commit_id(root, base)?.is_some() => base.to_owned(),
        _ => {
            if !git.valid_branch_name(root, base_branch)? {
                return Err(IpcError::new(
                    "invalid_branch",
                    format!("\"{base_branch}\" is not a branch name git will take."),
                ));
            }
            git.fetch_into(
                root,
                remote,
                &format!("refs/heads/{base_branch}"),
                &base_ref,
            )?;
            git.commit_id(root, &base_ref)?.ok_or_else(|| {
                IpcError::new(
                    "pull_request_not_fetched",
                    format!("The base branch \"{base_branch}\" could not be fetched."),
                )
            })?
        }
    };
    // Kept from being collected, like the head: a commit fetched by id is on no branch here.
    git.update_ref(root, &base_ref, &base_oid)?;

    let fork = git.merge_base(root, &base_oid, &head_oid)?.ok_or_else(|| {
        IpcError::new(
            "no_common_history",
            format!("Pull request #{number} shares no history with {base_branch}, so there is no diff to show."),
        )
    })?;
    Ok((head_oid, fork))
}

/// Delete the refs kept for pull requests the project's list no longer has.
///
/// Only when the list can be trusted to be whole — `gh` answered, for both tiers. A list that
/// is empty because the network is down is not a reason to throw anything away. What is deleted
/// is fetched again if anyone asks; these are refs of Yardsort's own and nothing else's.
pub fn prune_refs(git: &Git, root: &Path, found: &ProjectPullRequests) {
    if !found.gh || found.problem.is_some() || found.open_problem.is_some() {
        return;
    }
    let Ok(refs) = git.refs_under(root, PULL_REQUEST_REFS) else {
        return;
    };
    for name in refs {
        let number = name
            .strip_prefix(PULL_REQUEST_REFS)
            .and_then(|rest| rest.trim_start_matches('/').split('/').next())
            .and_then(|number| number.parse::<u32>().ok());
        let listed = number.is_some_and(|n| found.pull_requests.iter().any(|pr| pr.number == n));
        if !listed {
            let _ = git.delete_ref(root, &name);
        }
    }
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
        let head = git.fetch_pull_request(root, &remote, pr.number, None)?;
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
    use crate::git::PULL_REQUEST_REFS;
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
                base_oid: String::new(),
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
        // From trunk, by name: the bare remote's own HEAD points at a branch that is not there,
        // and a clone that checked nothing out would start `theirs` with no history at all.
        git.run(
            theirs.path(),
            &["clone", "--quiet", "--branch", "trunk", &url, "."],
        )
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

    /// The pull request of [`fixture`] as the list would have it, naming its two commits.
    fn in_full(pr: PullRequest, head: &str, base: &str) -> PullRequest {
        let mut pr = pr;
        let details = pr.details.as_mut().unwrap();
        details.head_oid = head.to_owned();
        details.base_oid = base.to_owned();
        pr
    }

    fn text_of(content: &crate::changes::Content) -> &str {
        match content {
            crate::changes::Content::Text { text } => text,
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_pull_requests_diff_is_read_without_checking_anything_out() {
        let (git, repo, _remote, head) = fixture();
        let trunk = rev(&git, repo.path(), "trunk").unwrap();
        let summary = in_full(pr(7, "theirs", false), &head, &trunk);

        let changes = changes_of(&git, repo.path(), &summary).unwrap();
        assert_eq!(changes.head_oid, head);
        assert_eq!(changes.base_oid, trunk, "where it left trunk");
        assert_eq!(changes.files.len(), 1);
        let file = &changes.files[0];
        assert_eq!(file.path, "work.txt");
        assert_eq!(file.kind, crate::changes::ChangeKind::Added);
        assert_eq!((file.additions, file.deletions), (Some(1), Some(0)));

        let diff = Between {
            git: &git,
            root: repo.path(),
            old: &changes.base_oid,
            new: &changes.head_oid,
        }
        .diff("work.txt", None)
        .unwrap();
        assert_eq!(diff.old, crate::changes::Content::Absent);
        assert_eq!(text_of(&diff.new), "theirs");

        // Kept under refs of Yardsort's own, and nothing of the user's moved.
        assert_eq!(
            rev(&git, repo.path(), "refs/yardsort/pull/7/head").as_deref(),
            Some(head.as_str())
        );
        assert_eq!(
            rev(&git, repo.path(), "refs/yardsort/pull/7/base").as_deref(),
            Some(trunk.as_str())
        );
        assert!(!git.branch_exists(repo.path(), "theirs").unwrap());
        assert_eq!(rev(&git, repo.path(), "refs/remotes/origin/theirs"), None);
        assert_eq!(
            git.head(repo.path()).unwrap(),
            crate::git::Head::Branch("trunk".into())
        );
        assert!(
            !repo.path().join("work.txt").exists(),
            "nothing was checked out"
        );
    }

    /// Once its commits are here, looking again asks the remote for nothing: it works with the
    /// remote gone.
    #[test]
    fn a_second_look_fetches_nothing() {
        let (git, repo, _remote, head) = fixture();
        let trunk = rev(&git, repo.path(), "trunk").unwrap();
        let summary = in_full(pr(7, "theirs", false), &head, &trunk);
        changes_of(&git, repo.path(), &summary).unwrap();

        git.run(
            repo.path(),
            &["remote", "set-url", "origin", "/nowhere/at/all.git"],
        )
        .unwrap();
        let again = changes_of(&git, repo.path(), &summary).unwrap();
        assert_eq!(again.files.len(), 1);

        // A head the repository has not got is a reason to fetch, and then the failure shows.
        let moved = in_full(pr(7, "theirs", false), &"1".repeat(40), &trunk);
        assert!(changes_of(&git, repo.path(), &moved).is_err());
    }

    /// Someone pushes after the diff was first read. The list names the new head, so the next
    /// look fetches again — the ref sitting on the old head is not "already here".
    #[test]
    fn a_push_the_list_has_noticed_is_fetched_however_fresh_the_last_look_was() {
        let (git, repo, remote, head) = fixture();
        let trunk = rev(&git, repo.path(), "trunk").unwrap();
        let first = changes_of(
            &git,
            repo.path(),
            &in_full(pr(7, "theirs", false), &head, &trunk),
        )
        .unwrap();
        assert_eq!(first.head_oid, head);

        let theirs = tempfile::tempdir().unwrap();
        let url = remote.path().to_string_lossy().into_owned();
        git.run(
            theirs.path(),
            &["clone", "--quiet", "--branch", "theirs", &url, "."],
        )
        .unwrap();
        std::fs::write(theirs.path().join("more.txt"), "more").unwrap();
        git.commit_all(theirs.path(), "More of their work").unwrap();
        git.run(
            theirs.path(),
            &["push", "--quiet", "origin", "theirs:refs/pull/7/head"],
        )
        .unwrap();
        let pushed = git.run(theirs.path(), &["rev-parse", "HEAD"]).unwrap();

        let again = changes_of(
            &git,
            repo.path(),
            &in_full(pr(7, "theirs", false), &pushed, &trunk),
        )
        .unwrap();
        assert_eq!(again.head_oid, pushed);
        let paths: Vec<&str> = again.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["more.txt", "work.txt"]);
        assert_eq!(
            rev(&git, repo.path(), "refs/yardsort/pull/7/head").as_deref(),
            Some(pushed.as_str())
        );
    }

    /// The forge was asked a moment before someone pushed: what is fetched is the newer head,
    /// and that is what is shown.
    #[test]
    fn a_pull_request_that_moved_on_is_shown_as_it_is_now() {
        let (git, repo, remote, head) = fixture();
        let trunk = rev(&git, repo.path(), "trunk").unwrap();
        let theirs = tempfile::tempdir().unwrap();
        let url = remote.path().to_string_lossy().into_owned();
        git.run(
            theirs.path(),
            &["clone", "--quiet", "--branch", "theirs", &url, "."],
        )
        .unwrap();
        std::fs::write(theirs.path().join("more.txt"), "more").unwrap();
        git.commit_all(theirs.path(), "More of their work").unwrap();
        git.run(
            theirs.path(),
            &["push", "--quiet", "origin", "theirs:refs/pull/7/head"],
        )
        .unwrap();
        let newer = git.run(theirs.path(), &["rev-parse", "HEAD"]).unwrap();

        let stale = in_full(pr(7, "theirs", false), &head, &trunk);
        let changes = changes_of(&git, repo.path(), &stale).unwrap();
        assert_eq!(changes.head_oid, newer);
        let paths: Vec<&str> = changes.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["more.txt", "work.txt"]);
    }

    /// A pull request merged with a merge commit: against the base branch as it is now there
    /// is nothing left to show. Against the base the forge recorded, there is.
    #[test]
    fn a_merged_pull_request_still_gives_its_diff() {
        let (git, repo, remote, head) = fixture();
        let base_then = rev(&git, repo.path(), "trunk").unwrap();
        // The merge happens on the forge; the local trunk hears of it by fetching.
        let merger = tempfile::tempdir().unwrap();
        let url = remote.path().to_string_lossy().into_owned();
        git.run(
            merger.path(),
            &["clone", "--quiet", "--branch", "trunk", &url, "."],
        )
        .unwrap();
        git.run(
            merger.path(),
            &[
                "merge",
                "--quiet",
                "--no-ff",
                "-m",
                "Merge #7",
                "origin/theirs",
            ],
        )
        .unwrap();
        git.run(merger.path(), &["push", "--quiet", "origin", "trunk"])
            .unwrap();
        git.run(repo.path(), &["pull", "--quiet", "origin", "trunk"])
            .unwrap();
        assert!(
            repo.path().join("work.txt").exists(),
            "trunk has the work now"
        );

        let mut merged = pr(7, "theirs", false);
        merged.state = PullRequestState::Merged;
        let changes = changes_of(&git, repo.path(), &in_full(merged, &head, &base_then)).unwrap();
        assert_eq!(changes.base_oid, base_then);
        assert_eq!(changes.files.len(), 1, "its one file, not nothing");
        assert_eq!(changes.files[0].path, "work.txt");
    }

    /// A base the server will not hand over by id — here, one it has never had — falls back
    /// to the base branch by name.
    #[test]
    fn without_the_recorded_base_the_base_branch_stands_in() {
        let (git, repo, _remote, head) = fixture();
        let trunk = rev(&git, repo.path(), "trunk").unwrap();
        let summary = in_full(pr(7, "theirs", false), &head, &"2".repeat(40));
        let changes = changes_of(&git, repo.path(), &summary).unwrap();
        assert_eq!(changes.head_oid, head);
        assert_eq!(changes.base_oid, trunk);
        assert_eq!(changes.files.len(), 1);

        // And an id that is not an id never reaches git at all.
        let hostile = in_full(pr(7, "theirs", false), &head, "--upload-pack=touch /tmp/x");
        let changes = changes_of(&git, repo.path(), &hostile).unwrap();
        assert_eq!(changes.base_oid, trunk);
    }

    /// Against the real GitHub: a pull request merged years ago, from a fork, in a small public
    /// repository, fetched into an empty one. That the forge hands over `refs/pull/<n>/head`
    /// and a commit asked for by id is the one thing no local remote can show. Ignored by
    /// default: CI has no business depending on the network.
    ///
    /// `cargo test -p yardsort -- --ignored --nocapture fetches_a_real_pull_request`
    #[test]
    #[ignore = "needs the network"]
    fn fetches_a_real_pull_request_from_github() {
        let git = testing::git();
        let repo = tempfile::tempdir().unwrap();
        git.init(repo.path()).unwrap();
        git.run(
            repo.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/octocat/Hello-World.git",
            ],
        )
        .unwrap();
        let mut merged = pr(6, "patch-1", true);
        merged.url = "https://github.com/octocat/Hello-World/pull/6".into();
        merged.state = PullRequestState::Merged;
        let summary = in_full(
            merged,
            "762941318ee16e59dabbacb1b4049eec22f0d303",
            "553c2077f0edc3d5dc5d17262f6aa498e69d6f8e",
        );
        let changes = changes_of(&git, repo.path(), &summary).expect("fetched");
        println!(
            "{} file(s) between {} and {}",
            changes.files.len(),
            &changes.base_oid[..8],
            &changes.head_oid[..8]
        );
        assert_eq!(changes.head_oid, "762941318ee16e59dabbacb1b4049eec22f0d303");
        let paths: Vec<&str> = changes.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["README"]);
        assert!(
            git.branches(repo.path()).unwrap().is_empty(),
            "no branch was made"
        );
    }

    #[test]
    fn only_a_commit_id_written_in_full_is_taken_from_the_window() {
        assert!(commit_id(&"a".repeat(40)).is_ok());
        assert!(commit_id(&"A1".repeat(32)).is_ok(), "sha-256 repositories");
        for not in [
            "",
            "HEAD",
            "abc123",
            "--output=/tmp/x",
            &"g".repeat(40),
            &"a".repeat(41),
        ] {
            assert_eq!(commit_id(not).unwrap_err().code, "not_a_commit", "{not}");
        }
    }

    /// Refs kept for pull requests the list no longer has are let go — but only on the word of
    /// a list that is whole.
    #[test]
    fn refs_are_dropped_with_the_pull_requests_they_were_for() {
        let (git, repo, _remote, head) = fixture();
        let trunk = rev(&git, repo.path(), "trunk").unwrap();
        changes_of(
            &git,
            repo.path(),
            &in_full(pr(7, "theirs", false), &head, &trunk),
        )
        .unwrap();
        for name in ["refs/yardsort/pull/9/head", "refs/yardsort/pull/9/base"] {
            git.update_ref(repo.path(), name, &trunk).unwrap();
        }
        let kept = || git.refs_under(repo.path(), PULL_REQUEST_REFS).unwrap();
        assert_eq!(kept().len(), 4);

        let listing = |numbers: &[u32]| ProjectPullRequests {
            gh: true,
            pull_requests: numbers.iter().map(|n| pr(*n, "b", false)).collect(),
            ..Default::default()
        };
        // gh failed, or only half the open ones arrived: not a list to throw things away on.
        let mut broken = listing(&[]);
        broken.problem = Some("could not resolve host".into());
        prune_refs(&git, repo.path(), &broken);
        let mut partial = listing(&[]);
        partial.open_problem = Some("HTTP 504".into());
        prune_refs(&git, repo.path(), &partial);
        prune_refs(&git, repo.path(), &ProjectPullRequests::default());
        assert_eq!(
            kept().len(),
            4,
            "nothing dropped on a list that cannot be trusted"
        );

        prune_refs(&git, repo.path(), &listing(&[7, 12]));
        assert_eq!(
            kept(),
            ["refs/yardsort/pull/7/base", "refs/yardsort/pull/7/head"]
        );
        assert!(
            git.branch_exists(repo.path(), "trunk").unwrap(),
            "branches are not its to touch"
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
