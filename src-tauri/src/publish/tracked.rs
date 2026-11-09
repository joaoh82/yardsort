//! Pull requests a workspace follows in other repositories, read and kept beside the project's
//! own. What is tracked is the store's (`yardsort_core::tracked`); this reads each one by its
//! link and keeps the answer. See `docs/design/38-pull-requests-elsewhere.md`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::PoisonError;
use std::time::{Duration, Instant};

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use super::commands::{failed, project_root, pull_request_changed, still_mergeable};
use super::pull_requests::{changes_of, closable, commit_id, reopenable, PullRequestChanges};
use super::Forge;
use crate::changes::{Between, FileDiff};
use crate::error::{IpcError, IpcResult};
use crate::forge::{
    ForgeError, ForgeResult, Gh, LineComment, MergeMethod, PullRequest, PullRequestState,
    PullRequestSummary, Repo,
};
use crate::git::Git;
use crate::state::{blocking, AppState};
use crate::store::{Store, TrackedPullRequestRow};
use crate::tracked::TrackedBy;

/// How many of a project's tracked pull requests are read on each look, newest tracked first.
const TRACKED_LIMIT: usize = 20;

/// How many are read at once. Each is one `gh pr view`, about half a second.
const AT_ONCE: usize = 4;

/// How long a merged or closed one is reused: final, or nearly — a reopening shows within this.
const FINAL_FOR: Duration = Duration::from_secs(10 * 60);

/// How long one read may take.
const READ_LIMIT: Duration = Duration::from_secs(20);

/// A pull request in another repository that a workspace follows, as the window sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrackedPullRequest {
    pub id: String,
    pub workspace_id: String,
    /// The conversation that opened it, when that is known.
    pub session_id: Option<String>,
    pub repo: Repo,
    pub number: u32,
    pub url: String,
    /// `app`, `cli` or `hook`.
    pub tracked_by: String,
    /// What `gh` last said about it. `None` until it has been read, or when it never could be.
    pub pull_request: Option<PullRequest>,
    /// Why the last read failed. What was read before stays in `pull_request`.
    pub problem: Option<String>,
}

/// One tracked pull request as last read.
pub(super) struct TrackedRead {
    at: Instant,
    pull_request: Option<PullRequest>,
    problem: Option<String>,
}

pub(super) type TrackedReads = HashMap<String, TrackedRead>;

impl TrackedRead {
    /// Worth reading again: an open one when asked to refresh or after the list's own lifetime,
    /// a merged or closed one after [`FINAL_FOR`].
    fn stale(&self, refresh: bool) -> bool {
        let open = self
            .pull_request
            .as_ref()
            .is_none_or(|pr| pr.state == PullRequestState::Open);
        if open {
            refresh || self.at.elapsed() >= super::FRESH_FOR
        } else {
            self.at.elapsed() >= FINAL_FOR
        }
    }
}

impl Forge {
    /// The pull requests `rows` track, each with what `fetch` last said about it, reading the
    /// ones that are due. Blocking: `fetch` is `gh` over the network. Never an `Err` — one that
    /// cannot be read says why in its place, and keeps what was read of it before.
    pub fn tracked(
        &self,
        rows: &[TrackedPullRequestRow],
        refresh: bool,
        fetch: impl Fn(&str) -> ForgeResult<PullRequest> + Sync,
    ) -> Vec<TrackedPullRequest> {
        let due: Vec<String> = {
            let reads = self.tracked.lock().unwrap_or_else(PoisonError::into_inner);
            let mut seen = std::collections::HashSet::new();
            rows.iter()
                .take(TRACKED_LIMIT)
                .map(|row| row.url.clone())
                // Two workspaces may follow the same one: it is read once.
                .filter(|url| seen.insert(url.clone()))
                .filter(|url| reads.get(url).is_none_or(|read| read.stale(refresh)))
                .collect()
        };
        let read: Vec<(String, ForgeResult<PullRequest>)> = due
            .chunks(AT_ONCE)
            .flat_map(|chunk| {
                std::thread::scope(|scope| {
                    let asked: Vec<_> = chunk
                        .iter()
                        .map(|url| (url, scope.spawn(|| fetch(url))))
                        .collect();
                    asked
                        .into_iter()
                        .map(|(url, handle)| {
                            let answer = handle.join().unwrap_or_else(|_| {
                                Err(ForgeError::Unreadable("the read stopped".to_owned()))
                            });
                            (url.clone(), answer)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut reads = self.tracked.lock().unwrap_or_else(PoisonError::into_inner);
        for (url, answer) in read {
            let before = reads.remove(&url).and_then(|read| read.pull_request);
            let (pull_request, problem) = match answer {
                Ok(pr) => (Some(pr), None),
                Err(error) => (before, Some(error.to_string())),
            };
            reads.insert(
                url,
                TrackedRead {
                    at: Instant::now(),
                    pull_request,
                    problem,
                },
            );
        }
        rows.iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let repo = crate::tracked::repo_from_key(&row.repo)?;
                let read = reads.get(&row.url);
                let problem = match read {
                    Some(read) => read.problem.clone(),
                    None if index >= TRACKED_LIMIT => Some(format!(
                        "Not read: a project's workspaces follow at most {TRACKED_LIMIT} pull \
                         requests elsewhere, the newest tracked."
                    )),
                    None => None,
                };
                Some(TrackedPullRequest {
                    id: row.id.clone(),
                    workspace_id: row.workspace_id.clone(),
                    session_id: row.session_id.clone(),
                    repo,
                    number: row.number,
                    url: row.url.clone(),
                    tracked_by: row.tracked_by.clone(),
                    pull_request: read.and_then(|read| read.pull_request.clone()),
                    problem,
                })
            })
            .collect()
    }

    /// The pull request at `url` as last read, however long ago: what the window was shown.
    pub fn last_tracked(&self, url: &str) -> Option<PullRequest> {
        self.tracked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(url)
            .and_then(|read| read.pull_request.clone())
    }

    /// Read `url` afresh on the next look: something was just done to it.
    pub fn forget_tracked(&self, url: &str) {
        self.tracked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(url);
    }
}

/// The pull requests a project's workspaces follow elsewhere, read as they are due. Nothing is
/// read without `gh`; the rows are still listed, with nothing to show for them.
pub(super) fn of_project(
    state: &AppState,
    project_id: &str,
    refresh: bool,
) -> Vec<TrackedPullRequest> {
    let Ok(rows) = state.store.tracked_pull_requests_in_project(project_id) else {
        return Vec::new();
    };
    if rows.is_empty() {
        return Vec::new();
    }
    let (Some(gh), Ok(root)) = (Gh::find(&state.env()), project_root(state, project_id)) else {
        return state
            .forge
            .tracked(&rows, false, |_| Err(ForgeError::NotInstalled))
            .into_iter()
            .map(|tracked| TrackedPullRequest {
                problem: None,
                ..tracked
            })
            .collect();
    };
    // `gh pr view <url>` needs no repository around it; the project's checkout is only where it
    // runs, and one that has gone is no reason not to ask.
    let cwd = if root.is_dir() {
        root
    } else {
        std::env::temp_dir()
    };
    state.forge.tracked(&rows, refresh, |url| {
        gh.pull_request_at(&cwd, url, READ_LIMIT)
    })
}

/// The tracked row `id` of the workspace, or the error that says it is gone.
fn row_of(state: &AppState, workspace_id: &str, id: &str) -> IpcResult<TrackedPullRequestRow> {
    row_in(&state.store, workspace_id, id)
}

fn row_in(store: &Store, workspace_id: &str, id: &str) -> IpcResult<TrackedPullRequestRow> {
    store
        .tracked_pull_requests(workspace_id)?
        .into_iter()
        .find(|row| row.id == id)
        .ok_or_else(|| {
            IpcError::new(
                "not_tracked",
                "That pull request is no longer tracked with this workspace.",
            )
        })
}

/// Follow the pull request at `url` with the workspace. Resolves to whether it is new: tracking
/// one twice changes nothing.
#[tauri::command]
#[specta::specta]
pub async fn workspace_track_pull_request(
    app: AppHandle,
    workspace_id: String,
    url: String,
) -> IpcResult<bool> {
    blocking(app, move |state| {
        let git = Git::new(&state.env())?;
        let (_, added) = crate::tracked::track(
            &state.store,
            &git,
            &workspace_id,
            &url,
            None,
            TrackedBy::App,
        )?;
        Ok(added)
    })
    .await
}

/// Stop following one. The pull request is left as it is; only Yardsort's note of it goes.
#[tauri::command]
#[specta::specta]
pub async fn workspace_untrack_pull_request(
    app: AppHandle,
    workspace_id: String,
    id: String,
) -> IpcResult<()> {
    blocking(app, move |state| {
        let row = row_of(state, &workspace_id, &id)?;
        state.store.untrack_pull_request(&workspace_id, &row.id)?;
        // Another workspace may follow it too; that one reads it afresh, which costs one call.
        state.forge.forget_tracked(&row.url);
        forget_commits(state, &workspace_id, &row);
        Ok(())
    })
    .await
}

/// The refs **Code** fetched a pull request's commits into, in the nested clone: Yardsort's own,
/// and nobody else's, so they go when no workspace of the project follows it any more. Best
/// effort — they are fetched again if anyone asks.
fn forget_commits(state: &AppState, workspace_id: &str, row: &TrackedPullRequestRow) {
    let Ok(Some(workspace)) = state.store.workspace(workspace_id) else {
        return;
    };
    let still = state
        .store
        .tracked_pull_requests_in_project(&workspace.project_id)
        .is_ok_and(|rows| rows.iter().any(|other| other.url == row.url));
    if still {
        return;
    }
    state
        .forge
        .tracked_clones
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&row.url);
    let (Ok(git), Ok(root)) = (
        Git::new(&state.env()),
        project_root(state, &workspace.project_id),
    ) else {
        return;
    };
    if let Ok(clone) = clone_in(&git, &root, row) {
        for which in ["head", "base"] {
            let _ = git.delete_ref(&clone, &crate::git::pull_request_ref(row.number, which));
        }
    }
}

/// A tracked pull request in full, as [`super::pull_requests::pull_request_summary`] reads a
/// project's own: by its link.
#[tauri::command]
#[specta::specta]
pub async fn tracked_pull_request_summary(
    app: AppHandle,
    workspace_id: String,
    id: String,
    refresh: bool,
) -> IpcResult<PullRequestSummary> {
    blocking(app, move |state| {
        let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
        summary_of(&state.store, &state.forge, &gh, &workspace_id, &id, refresh)
    })
    .await
}

/// [`tracked_pull_request_summary`] without the app around it. The workspace's folder is not
/// needed — archiving removes it, and the link is all `gh` asks for — so only its row is read.
fn summary_of(
    store: &Store,
    forge: &Forge,
    gh: &Gh,
    workspace_id: &str,
    id: &str,
    refresh: bool,
) -> IpcResult<PullRequestSummary> {
    let row = row_in(store, workspace_id, id)?;
    let root = cwd_of(store, workspace_id)?;
    // Kept apart from the project's own summaries, whose numbers this one's would collide with:
    // the key is the link.
    forge
        .summary(&summary_key(&row.url), row.number, refresh, || {
            gh.pull_request_summary_at(&root, &row.url, super::pull_requests::SUMMARY_LIMIT)
        })
        .map_err(failed)
}

/// Where `gh` runs for a workspace's tracked pull requests: its project's checkout, or, when
/// that has gone, anywhere — a link needs no repository around it.
fn cwd_of(store: &Store, workspace_id: &str) -> IpcResult<PathBuf> {
    let workspace = store
        .workspace(workspace_id)?
        .ok_or_else(|| IpcError::new("unknown_workspace", "That workspace no longer exists."))?;
    Ok(store
        .project(&workspace.project_id)?
        .map(|project| PathBuf::from(project.root_path))
        .filter(|root| root.is_dir())
        .unwrap_or_else(std::env::temp_dir))
}

/// A tracked pull request as the forge has it this moment, with what is needed to act on it:
/// nothing done to one rests on an answer half a minute old.
fn fresh(
    state: &AppState,
    workspace_id: &str,
    id: &str,
) -> IpcResult<(Gh, PathBuf, TrackedPullRequestRow, PullRequest)> {
    let row = row_of(state, workspace_id, id)?;
    let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
    let cwd = cwd_of(&state.store, workspace_id)?;
    let pr = gh
        .pull_request_at(&cwd, &row.url, READ_LIMIT)
        .map_err(failed)?;
    Ok((gh, cwd, row, pr))
}

/// Something was done to the pull request at `url`: read it, and its summary, afresh.
fn changed(state: &AppState, url: &str) {
    state.forge.forget_tracked(url);
    state.forge.forget(&summary_key(url));
}

/// Where a tracked pull request's summary is kept, apart from the project's own, whose numbers
/// its number would collide with.
fn summary_key(url: &str) -> String {
    format!("tracked:{url}")
}

/// Merge a tracked pull request, as [`super::pull_requests::pull_request_merge`] merges a
/// project's own: only if it is still open, not a draft, and at the head the user confirmed.
/// Never deletes a branch, never bypasses a protection.
#[tauri::command]
#[specta::specta]
pub async fn tracked_pull_request_merge(
    app: AppHandle,
    workspace_id: String,
    id: String,
    head_oid: String,
    method: MergeMethod,
) -> IpcResult<()> {
    blocking(app, move |state| {
        let (gh, cwd, row, pr) = fresh(state, &workspace_id, &id)?;
        if !still_mergeable(&pr, &head_oid) {
            return Err(pull_request_changed());
        }
        let result = gh.merge_pull_request_at(&cwd, &row.url, &head_oid, method);
        changed(state, &row.url);
        result.map_err(failed)
    })
    .await
}

/// Close a tracked pull request without merging it. Its branch stays, and nothing is posted.
#[tauri::command]
#[specta::specta]
pub async fn tracked_pull_request_close(
    app: AppHandle,
    workspace_id: String,
    id: String,
) -> IpcResult<()> {
    blocking(app, move |state| {
        let (gh, cwd, row, pr) = fresh(state, &workspace_id, &id)?;
        closable(&pr)?;
        let result = gh.close_pull_request_at(&cwd, &row.url);
        changed(state, &row.url);
        result.map_err(failed)
    })
    .await
}

/// Open a closed tracked pull request again.
#[tauri::command]
#[specta::specta]
pub async fn tracked_pull_request_reopen(
    app: AppHandle,
    workspace_id: String,
    id: String,
) -> IpcResult<()> {
    blocking(app, move |state| {
        let (gh, cwd, row, pr) = fresh(state, &workspace_id, &id)?;
        reopenable(&pr)?;
        let result = gh.reopen_pull_request_at(&cwd, &row.url);
        changed(state, &row.url);
        result.map_err(failed)
    })
    .await
}

/// Post a comment on a tracked pull request's conversation. The words are the user's, sent as
/// they are; pressing Send was the confirmation.
#[tauri::command]
#[specta::specta]
pub async fn tracked_pull_request_comment(
    app: AppHandle,
    workspace_id: String,
    id: String,
    body: String,
) -> IpcResult<()> {
    let body = body.trim().to_owned();
    if body.is_empty() {
        return Err(IpcError::new("empty_comment", "Write something first."));
    }
    blocking(app, move |state| {
        let row = row_of(state, &workspace_id, &id)?;
        let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
        let cwd = cwd_of(&state.store, &workspace_id)?;
        let result = gh.comment_at(&cwd, &row.url, &body);
        changed(state, &row.url);
        result.map_err(failed)
    })
    .await
}

/// The nested clone a tracked pull request's diff is read in.
fn clone_for(
    state: &AppState,
    git: &Git,
    workspace_id: &str,
    row: &TrackedPullRequestRow,
) -> IpcResult<PathBuf> {
    let mut known = state
        .forge
        .tracked_clones
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(clone) = known.get(&row.url).filter(|clone| clone.is_dir()) {
        return Ok(clone.clone());
    }
    let workspace = state
        .store
        .workspace(workspace_id)?
        .ok_or_else(|| IpcError::new("unknown_workspace", "That workspace no longer exists."))?;
    let clone = clone_in(git, &project_root(state, &workspace.project_id)?, row)?;
    known.insert(row.url.clone(), clone.clone());
    Ok(clone)
}

/// [`clone_for`] in the project's checkout at `root`.
fn clone_in(git: &Git, root: &std::path::Path, row: &TrackedPullRequestRow) -> IpcResult<PathBuf> {
    let repo = crate::tracked::repo_from_key(&row.repo).ok_or_else(|| {
        IpcError::new("not_tracked", "That pull request's repository is unknown.")
    })?;
    crate::tracked::clone_of(git, root, &repo).ok_or_else(|| {
        IpcError::new(
            "no_clone",
            format!(
                "This project's folder holds no clone of {}/{} to read the diff in. Its diff is on \
                 GitHub.",
                repo.owner, repo.name
            ),
        )
    })
}

/// The files a tracked pull request changes, read in the project's nested clone of its
/// repository: its commits are fetched there, into refs of Yardsort's own, as a project's own
/// are fetched into the project's checkout. Nothing is checked out, and no branch moves. The
/// head and base are the ones last read, which the window shows; one never read is read now.
#[tauri::command]
#[specta::specta]
pub async fn tracked_pull_request_changes(
    app: AppHandle,
    workspace_id: String,
    id: String,
) -> IpcResult<PullRequestChanges> {
    blocking(app, move |state| {
        let row = row_of(state, &workspace_id, &id)?;
        let git = Git::new(&state.env())?;
        let clone = clone_for(state, &git, &workspace_id, &row)?;
        let pr = match state.forge.last_tracked(&row.url) {
            Some(pr) if pr.details.is_some() => pr,
            _ => fresh(state, &workspace_id, &id)?.3,
        };
        changes_of(&git, &clone, &pr)
    })
    .await
}

/// Both sides of one file of a tracked pull request, between the two commits
/// [`tracked_pull_request_changes`] answered with, in the same nested clone.
#[tauri::command]
#[specta::specta]
pub async fn tracked_pull_request_diff(
    app: AppHandle,
    workspace_id: String,
    id: String,
    base_oid: String,
    head_oid: String,
    path: String,
    old_path: Option<String>,
) -> IpcResult<FileDiff> {
    blocking(app, move |state| {
        // They come back from the window; they go to git only as commit ids, written in full.
        commit_id(&base_oid)?;
        commit_id(&head_oid)?;
        let row = row_of(state, &workspace_id, &id)?;
        let git = Git::new(&state.env())?;
        let clone = clone_for(state, &git, &workspace_id, &row)?;
        Between {
            git: &git,
            root: &clone,
            old: &base_oid,
            new: &head_oid,
        }
        .diff(&path, old_path.as_deref())
    })
    .await
}

/// The comments on lines of a tracked pull request's diff, asked of its own repository by name.
#[tauri::command]
#[specta::specta]
pub async fn tracked_pull_request_line_comments(
    app: AppHandle,
    workspace_id: String,
    id: String,
) -> IpcResult<Vec<LineComment>> {
    blocking(app, move |state| {
        let row = row_of(state, &workspace_id, &id)?;
        let repo = crate::tracked::repo_from_key(&row.repo).ok_or_else(|| {
            IpcError::new("not_tracked", "That pull request's repository is unknown.")
        })?;
        let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
        let cwd = cwd_of(&state.store, &workspace_id)?;
        gh.line_comments_in(&cwd, &repo, row.number).map_err(failed)
    })
    .await
}

/// A tracked pull request as a workflow run is about it (`context: tracked_pull_request`): read
/// now by its link, and open. With where it is — its repository, the project's nested clone of
/// it when there is one (a workflow that uses `pr.clone` is refused without, by the core) — and
/// the conversation that opened it, which `session: origin` prefers.
pub(crate) fn for_a_run(
    state: &AppState,
    workspace_id: &str,
    id: &str,
) -> IpcResult<yardsort_core::workflow::runs::PrFacts> {
    let (_, _, row, pr) = fresh(state, workspace_id, id)?;
    if pr.state != PullRequestState::Open {
        return Err(IpcError::new(
            "pull_request_missing",
            format!("{} is not open.", name_of(&row)),
        ));
    }
    let git = Git::new(&state.env())?;
    let clone = match clone_for(state, &git, workspace_id, &row) {
        Ok(clone) => Some(clone.to_string_lossy().into_owned()),
        Err(error) if error.code == "no_clone" => None,
        Err(error) => return Err(error),
    };
    Ok(yardsort_core::workflow::runs::PrFacts {
        number: pr.number,
        url: pr.url,
        title: pr.title,
        elsewhere: Some(yardsort_core::workflow::runs::Elsewhere {
            repo: repo_name(&row),
            clone,
            opened_by: row.session_id,
        }),
    })
}

/// `owner/name` of a tracked row's repository.
fn repo_name(row: &TrackedPullRequestRow) -> String {
    crate::tracked::repo_from_key(&row.repo)
        .map(|repo| format!("{}/{}", repo.owner, repo.name))
        .unwrap_or_else(|| row.repo.clone())
}

/// `owner/name#7`.
fn name_of(row: &TrackedPullRequestRow) -> String {
    format!("{}#{}", repo_name(row), row.number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::Checks;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn row(id: &str, number: u32) -> TrackedPullRequestRow {
        TrackedPullRequestRow {
            id: id.into(),
            workspace_id: "ws".into(),
            session_id: None,
            repo: "github.com/o/lib".into(),
            number,
            url: format!("https://github.com/o/lib/pull/{number}"),
            tracked_by: "cli".into(),
            created_at: 0,
        }
    }

    fn pr(number: u32, state: PullRequestState) -> PullRequest {
        PullRequest {
            number,
            url: format!("https://github.com/o/lib/pull/{number}"),
            title: format!("PR {number}"),
            branch: "fix".into(),
            state,
            draft: false,
            checks: Checks::Passing,
            details: None,
            author: None,
            created_at: None,
        }
    }

    /// **Code** on a tracked pull request reads its diff in the project's nested clone of its
    /// repository, which has its history — never in the project's own checkout, which has not.
    /// The commits are where a fetch would have put them, so no network is needed.
    #[test]
    fn a_tracked_pull_requests_diff_is_read_in_the_nested_clone() {
        use crate::forge::{CheckCounts, Mergeable, PullRequestDetails};
        let git = crate::git::testing::git();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git.init(root).unwrap();
        let clone = root.join("lib");
        std::fs::create_dir_all(&clone).unwrap();
        git.init(&clone).unwrap();
        git.run(
            &clone,
            &["remote", "add", "origin", "https://github.com/o/lib.git"],
        )
        .unwrap();
        std::fs::write(clone.join("lib.rs"), "one\n").unwrap();
        git.run(&clone, &["add", "."]).unwrap();
        git.run(&clone, &["commit", "-qm", "base"]).unwrap();
        let base = git.commit_id(&clone, "HEAD").unwrap().unwrap();
        std::fs::write(clone.join("lib.rs"), "one\ntwo\n").unwrap();
        git.run(&clone, &["commit", "-qam", "bump"]).unwrap();
        let head = git.commit_id(&clone, "HEAD").unwrap().unwrap();
        git.run(&clone, &["reset", "-q", "--hard", &base]).unwrap();
        git.update_ref(&clone, &crate::git::pull_request_ref(7, "head"), &head)
            .unwrap();

        let tracked = row("a", 7);
        let found = clone_in(&git, root, &tracked).unwrap();
        assert_eq!(found, clone);
        let mut pull_request = pr(7, PullRequestState::Open);
        pull_request.details = Some(PullRequestDetails {
            base: "main".into(),
            head_oid: head.clone(),
            base_oid: base.clone(),
            additions: 1,
            deletions: 0,
            review: String::new(),
            updated_at: String::new(),
            checks: vec![],
            check_counts: CheckCounts::default(),
            mergeable: Mergeable::Mergeable,
            review_requests: vec![],
            reviews: vec![],
            cross_repository: false,
        });
        let changes = changes_of(&git, &found, &pull_request).unwrap();
        assert_eq!(
            (changes.head_oid.as_str(), changes.base_oid.as_str()),
            (head.as_str(), base.as_str())
        );
        assert_eq!(changes.files.len(), 1);
        assert_eq!(changes.files[0].path, "lib.rs");
        assert_eq!(
            git.run(&clone, &["status", "--porcelain"]).unwrap().trim(),
            "",
            "nothing checked out, nothing changed in the clone"
        );

        // No clone of it: said so, with where the diff is instead.
        let mut elsewhere = row("b", 8);
        elsewhere.repo = "github.com/o/other".into();
        let refused = clone_in(&git, root, &elsewhere).unwrap_err();
        assert_eq!(refused.code, "no_clone");
        assert!(refused.message.contains("o/other"), "{}", refused.message);
    }

    fn number_of(url: &str) -> u32 {
        url.rsplit('/').next().unwrap().parse().unwrap()
    }

    #[test]
    fn an_open_one_is_read_on_refresh_and_a_merged_one_is_left_alone() {
        let forge = Forge::default();
        let rows = [row("a", 7), row("b", 8)];
        let asked = AtomicUsize::new(0);
        let read = |url: &str| {
            asked.fetch_add(1, Ordering::Relaxed);
            let number = number_of(url);
            Ok(pr(
                number,
                if number == 8 {
                    PullRequestState::Merged
                } else {
                    PullRequestState::Open
                },
            ))
        };
        let first = forge.tracked(&rows, false, read);
        assert_eq!(asked.load(Ordering::Relaxed), 2);
        assert_eq!(first[0].pull_request.as_ref().unwrap().number, 7);
        assert_eq!(first[0].repo.name, "lib");
        assert_eq!(
            first[1].pull_request.as_ref().unwrap().state,
            PullRequestState::Merged
        );

        forge.tracked(&rows, false, read);
        assert_eq!(asked.load(Ordering::Relaxed), 2, "fresh: nothing asked");
        forge.tracked(&rows, true, read);
        assert_eq!(
            asked.load(Ordering::Relaxed),
            3,
            "a refresh asks about the open one only"
        );

        forge.forget_tracked("https://github.com/o/lib/pull/8");
        forge.tracked(&rows, false, read);
        assert_eq!(asked.load(Ordering::Relaxed), 4, "forgotten: asked again");
    }

    #[test]
    fn a_failed_read_keeps_what_was_known_and_says_why() {
        let forge = Forge::default();
        let rows = [row("a", 7)];
        forge.tracked(&rows, false, |url| {
            Ok(pr(number_of(url), PullRequestState::Open))
        });
        let after = forge.tracked(&rows, true, |_| {
            Err(ForgeError::Failed {
                command: "pr view".into(),
                stderr: "HTTP 502".into(),
            })
        });
        assert_eq!(after[0].pull_request.as_ref().unwrap().number, 7);
        assert!(after[0].problem.as_deref().unwrap().contains("HTTP 502"));
        let recovered = forge.tracked(&rows, true, |url| {
            Ok(pr(number_of(url), PullRequestState::Open))
        });
        assert_eq!(recovered[0].problem, None);
    }

    #[test]
    fn only_the_newest_are_read_and_the_rest_say_so() {
        let forge = Forge::default();
        let rows: Vec<_> = (1..=TRACKED_LIMIT as u32 + 2)
            .map(|n| row(&n.to_string(), n))
            .collect();
        let asked = AtomicUsize::new(0);
        let listed = forge.tracked(&rows, false, |url| {
            asked.fetch_add(1, Ordering::Relaxed);
            Ok(pr(number_of(url), PullRequestState::Open))
        });
        assert_eq!(asked.load(Ordering::Relaxed), TRACKED_LIMIT);
        assert_eq!(listed.len(), rows.len(), "every row is listed");
        let last = listed.last().unwrap();
        assert!(last.pull_request.is_none());
        assert!(last.problem.as_deref().unwrap().contains("at most"));
    }

    /// Archiving removes a workspace's folder and keeps its row, and its tracked pull requests
    /// still show; reading one must not need the folder.
    #[cfg(unix)]
    #[test]
    fn an_archived_workspace_s_tracked_pull_request_can_still_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("yardsort.db")).unwrap();
        let checkout = dir.path().join("app");
        std::fs::create_dir(&checkout).unwrap();
        let project = store
            .add_project("app", checkout.to_str().unwrap())
            .unwrap();
        let folder = dir.path().join("wt-fix");
        std::fs::create_dir(&folder).unwrap();
        let workspace = store
            .add_worktree(
                &project.id,
                "fix",
                folder.to_str().unwrap(),
                Some("ys/fix"),
                Some("main"),
            )
            .unwrap();
        let url = "https://github.com/o/lib/pull/7";
        let (tracked, _) = store
            .track_pull_request(&crate::store::NewTrackedPullRequest {
                workspace_id: &workspace.id,
                session_id: None,
                repo: "github.com/o/lib",
                number: 7,
                url,
                tracked_by: "cli",
            })
            .unwrap();
        // What archiving leaves: the row, marked so, and no folder.
        std::fs::remove_dir(&folder).unwrap();
        assert!(store.set_workspace_archived(&workspace.id, true).unwrap());

        let script = dir.path().join("gh");
        let asked = dir.path().join("asked");
        let answer = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/core/fixtures/gh/2.102.0/pr-view.json");
        yardsort_core::program::testing::executable(
            &script,
            format!(
                "#!/bin/sh\necho \"$@\" >> '{}'\ncat '{}'\n",
                asked.display(),
                answer.display()
            ),
        );
        let env = crate::env::ShellEnv {
            vars: std::env::vars().collect(),
            source: crate::env::EnvSource::Process,
            warning: None,
        };
        let gh = Gh::at(script, &env);

        let summary = summary_of(
            &store,
            &Forge::default(),
            &gh,
            &workspace.id,
            &tracked.id,
            false,
        )
        .unwrap();
        assert!(summary.pull_request.number > 0);
        let asked = std::fs::read_to_string(&asked).unwrap();
        assert!(
            asked.starts_with(&format!("pr view {url} --json ")),
            "{asked}"
        );
    }
}
