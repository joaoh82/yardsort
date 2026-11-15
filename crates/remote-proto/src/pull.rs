//! Pull requests, as a host last read them from its forge. Since minor 11.
//!
//! A host reads its projects' pull requests with the forge's own program (`gh`), with its
//! person's sign-in. A device asks the host, never the forge: nothing here holds or passes on a
//! forge credential, and nothing is asked of the forge on a device's behalf that the host would
//! not ask for itself.
//!
//! `pull.list` gives a page of them and `pull.describe` one in full, with each check by name.
//! Both are what a workspace list already says about a project, a little further on: a branch,
//! then what became of it. So both need `workspace:read`, and both keep to the projects the
//! grant reaches.
//!
//! What a host says is as old as its last reading. It never makes a device wait for the forge:
//! when what it has is old it starts reading and says so ([`PullPage::reading`]), and the device
//! asks again.

use serde::{Deserialize, Serialize};

use crate::ids::{ProjectId, WorkspaceId};
use crate::model::Cursor;

/// The most checks one pull request is described with, by name.
pub const MAX_CHECKS: usize = 40;
/// Longest name of a check, in characters.
pub const MAX_CHECK_NAME: usize = 100;
/// The most reviewers, and the most reviews still waited for, one is described with.
pub const MAX_REVIEWERS: usize = 12;
/// The most workspaces one pull request names.
pub const MAX_PULL_WORKSPACES: usize = 8;

/// Which pull requests `pull.list` gives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PullsOf {
    /// Those a workspace opened or follows, whatever their state: what a workspace list shows.
    /// Those workspaces opened come first, each workspace's in the host's order for it, the one
    /// it shows first leading; those they follow in other repositories come after.
    #[default]
    Workspaces,
    /// Every open one the host has read, most recently changed first.
    Open,
    /// The merged ones among the newest the host has read.
    Merged,
    /// The ones closed without merging among the newest the host has read.
    Closed,
}

/// `pull.list`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullListParams {
    #[serde(default)]
    pub of: PullsOf,
    #[serde(default)]
    pub cursor: Option<Cursor>,
    /// At most [`crate::MAX_PAGE`]; [`crate::DEFAULT_PAGE`] when absent.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// `pull.describe`: which pull request. A project's own by its number, or one a workspace
/// follows in another repository, by the host's id for that.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRef {
    pub project: ProjectId,
    pub number: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub followed: Option<String>,
}

impl PullRef {
    pub fn invalid(&self) -> Option<&'static str> {
        if self.number == 0 {
            return Some("Pull requests are numbered from 1.");
        }
        match &self.followed {
            Some(id) if crate::ids::check_id(id).is_err() => {
                Some("That is not a followed pull request's id.")
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PullState {
    Open,
    Merged,
    Closed,
    /// A state from a newer host.
    #[serde(other)]
    Other,
}

/// How a pull request's checks divide up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullChecks {
    pub passed: u32,
    pub failed: u32,
    /// Queued, in progress or waiting: not finished either way.
    pub running: u32,
}

/// What the forge says of a pull request's reviews, as one word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PullReview {
    Approved,
    ChangesRequested,
    /// A review is required and has not been given.
    Required,
    /// Nothing is required, and nothing was said.
    None,
    #[serde(other)]
    Other,
}

/// A pull request, as much of it as a row and a badge need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullSummary {
    pub project: ProjectId,
    pub number: u32,
    /// The host's id for it, when a workspace follows it in another repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub followed: Option<String>,
    /// The repository it is in, as `owner/name`, when that is not the project's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub title: String,
    /// Its page on the forge. Absent when the host's is not an `https` link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The branch it would merge.
    pub branch: String,
    /// The branch it would merge into, when the host read that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub state: PullState,
    pub draft: bool,
    pub checks: PullChecks,
    pub review: PullReview,
    /// The forge says it does not merge cleanly into its base.
    pub conflicts: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletions: Option<u32>,
    /// Who opened it, by login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// When it last changed, in epoch milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    /// The workspaces it came out of, or that follow it. At most [`MAX_PULL_WORKSPACES`].
    pub workspaces: Vec<WorkspaceId>,
}

/// The reply to `pull.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullPage {
    pub pulls: Vec<PullSummary>,
    /// `None` on the last page.
    pub next: Option<Cursor>,
    /// How many open ones the host has read, over every project the grant reaches.
    pub open: u32,
    /// The host is reading some of these from the forge now: what it has is old, or it has
    /// nothing yet. Asking again in a few seconds gives what it read.
    pub reading: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckState {
    Passed,
    Failed,
    Running,
    #[serde(other)]
    Other,
}

/// One check, by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullCheck {
    /// With its workflow before it, when it has one: `CI / test`.
    pub name: String,
    pub state: CheckState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewerState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    #[serde(other)]
    Other,
}

/// A reviewer's latest word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullReviewer {
    pub login: String,
    pub state: ReviewerState,
}

/// The reply to `pull.describe`: one pull request, with its checks and reviewers by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullDetail {
    pub pull: PullSummary,
    /// Each check: the failed first, then those running, then the rest. Empty when the host has
    /// only their counts. At most [`MAX_CHECKS`].
    pub checks: Vec<PullCheck>,
    /// How many checks are not in `checks`, when there were more than it holds.
    pub more_checks: u32,
    /// Each reviewer's latest review. At most [`MAX_REVIEWERS`].
    pub reviews: Vec<PullReviewer>,
    /// Who a review was asked of and has not answered: logins, and teams' names. At most
    /// [`MAX_REVIEWERS`].
    pub waiting_on: Vec<String>,
}
