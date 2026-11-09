//! `ys pr …` — pull requests a workspace follows in other repositories.
//!
//! A project's own pull requests need nothing from here: Yardsort finds them by their branch. One
//! opened in another repository — a nested clone inside a parent project's checkout — leaves no
//! trace Yardsort can see, so the agent that opened it says so with `ys pr track`, and the app
//! shows it with the workspace from then on. See `yardsort_core::tracked`.

use serde::Serialize;
use yardsort_core::store::{TrackedPullRequestRow, WorkspaceRow};
use yardsort_core::tracked::{self, TrackedBy};

use crate::{table, Failure, Output, Yardsort};

#[derive(clap::Subcommand)]
pub enum Command {
    /// Follow a pull request in another repository with this workspace: its checks and reviews
    /// show in Yardsort beside the workspace's own.
    Track {
        /// The pull request's link, like https://github.com/owner/repo/pull/12.
        url: String,
        /// The workspace, by name or id. Default: the one this is run in.
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
    },
    /// The pull requests this workspace follows in other repositories.
    List {
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
    },
    /// Stop following one. The pull request itself is left as it is.
    Untrack {
        /// Its link.
        url: String,
        #[arg(long, value_name = "WORKSPACE")]
        workspace: Option<String>,
    },
}

/// A tracked pull request as it is printed.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Tracked {
    repo: String,
    number: u32,
    url: String,
    tracked_by: String,
}

impl From<&TrackedPullRequestRow> for Tracked {
    fn from(row: &TrackedPullRequestRow) -> Self {
        Self {
            repo: row.repo.clone(),
            number: row.number,
            url: row.url.clone(),
            tracked_by: row.tracked_by.clone(),
        }
    }
}

pub fn run(ys: &Yardsort, command: Command, out: &Output) -> Result<(), Failure> {
    match command {
        Command::Track { url, workspace } => {
            let workspace = locate(ys, workspace.as_deref())?;
            // The conversation asking, when an agent Yardsort started is the one running this.
            let session = env(yardsort_core::activity::RECORD_ENV);
            let (row, added) = tracked::track(
                &ys.store,
                &ys.git()?,
                &workspace.id,
                &url,
                session.as_deref(),
                TrackedBy::Cli,
            )?;
            #[derive(Serialize)]
            #[serde(rename_all = "camelCase")]
            struct Result {
                #[serde(flatten)]
                tracked: Tracked,
                workspace: String,
                added: bool,
            }
            let result = Result {
                tracked: Tracked::from(&row),
                workspace: workspace.name.clone(),
                added,
            };
            out.emit(&result, || {
                if result.added {
                    sayln!(
                        "Tracking {} with {}. Yardsort shows its checks and reviews beside the \
                         workspace's own.",
                        result.tracked.url,
                        result.workspace
                    );
                } else {
                    sayln!(
                        "{} is tracked with {} already.",
                        result.tracked.url,
                        result.workspace
                    );
                }
            })
        }
        Command::List { workspace } => {
            let workspace = locate(ys, workspace.as_deref())?;
            let rows = ys.store.tracked_pull_requests(&workspace.id)?;
            let listed: Vec<Tracked> = rows.iter().map(Tracked::from).collect();
            out.emit(&listed, || {
                if listed.is_empty() {
                    sayln!("This workspace follows no pull requests in other repositories.");
                    return;
                }
                let mut lines = vec![vec!["PULL REQUEST".to_owned(), "TRACKED BY".to_owned()]];
                lines.extend(
                    listed
                        .iter()
                        .map(|t| vec![t.url.clone(), t.tracked_by.clone()]),
                );
                table(&lines);
            })
        }
        Command::Untrack { url, workspace } => {
            let workspace = locate(ys, workspace.as_deref())?;
            let link = tracked::parse(&url)?;
            if !ys.store.untrack_pull_request(&workspace.id, &link.url)? {
                return Err(Failure::new(format!(
                    "{} is not tracked with {}.",
                    link.url, workspace.name
                )));
            }
            #[derive(Serialize)]
            struct Result<'a> {
                url: &'a str,
                removed: bool,
            }
            out.emit(
                &Result {
                    url: &link.url,
                    removed: true,
                },
                || sayln!("No longer tracking {}.", link.url),
            )
        }
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// The workspace a command is about: `--workspace` when given; otherwise the launch environment
/// of the agent running it, then the folder it runs in. An agent that went into a nested clone
/// to open its pull request is outside its worktree, so the environment comes first.
fn locate(ys: &Yardsort, wanted: Option<&str>) -> Result<WorkspaceRow, Failure> {
    if let Some(wanted) = wanted {
        return super::workspace::find_workspace(ys, wanted);
    }
    let cwd = std::env::current_dir().map_err(|e| Failure::new(format!("cwd: {e}")))?;
    let located = yardsort_core::memory::locate(
        &ys.store,
        env(yardsort_core::activity::RUN_ENV).as_deref(),
        env(yardsort_core::activity::WORKSPACE_ENV).as_deref(),
        &cwd,
    )?;
    let id = located
        .and_then(|(_, source)| source.workspace_id)
        .ok_or_else(|| {
            Failure::new(
                "Not inside a Yardsort workspace. Run this in one, or name it with --workspace.",
            )
        })?;
    super::workspace::find_workspace(ys, &id)
}
