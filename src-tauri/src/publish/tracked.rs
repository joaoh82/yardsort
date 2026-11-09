//! Pull requests a workspace follows in other repositories, read and kept beside the project's
//! own. What is tracked is the store's (`yardsort_core::tracked`); this reads each one by its
//! link and keeps the answer. See `docs/design/36-pull-requests-elsewhere.md`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::PoisonError;
use std::time::{Duration, Instant};

use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use super::commands::{failed, project_root};
use super::Forge;
use crate::changes::commands::workspace;
use crate::error::{IpcError, IpcResult};
use crate::forge::{
    ForgeError, ForgeResult, Gh, PullRequest, PullRequestState, PullRequestSummary, Repo,
};
use crate::git::Git;
use crate::state::{blocking, AppState};
use crate::store::TrackedPullRequestRow;
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
    state
        .store
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
        Ok(())
    })
    .await
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
        let row = row_of(state, &workspace_id, &id)?;
        let (workspace_row, _) = workspace(state, &workspace_id)?;
        let gh = Gh::find(&state.env()).ok_or_else(|| failed(ForgeError::NotInstalled))?;
        let root = project_root(state, &workspace_row.project_id)?;
        let cwd = if Path::new(&root).is_dir() {
            root
        } else {
            std::env::temp_dir()
        };
        // Kept apart from the project's own summaries, whose numbers this one's would collide
        // with: the key is the link.
        state
            .forge
            .summary(&format!("tracked:{}", row.url), row.number, refresh, || {
                gh.pull_request_summary_at(&cwd, &row.url, super::pull_requests::SUMMARY_LIMIT)
            })
            .map_err(failed)
    })
    .await
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
}
