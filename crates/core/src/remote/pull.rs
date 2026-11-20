//! `pull.list` and `pull.describe`: a host's pull requests, as it last read them from its forge.
//!
//! The host reads them with the forge's own program and its person's sign-in ([`crate::forge`]),
//! for its own window. A device is told what was read. It never reaches the forge, and nothing
//! is read for it that the host would not read for itself: no credential is held here or passed
//! on.
//!
//! Who reads, and when, is the host's to say, so the reading is lent ([`PullRequests`]): the
//! desktop app lends what its window read, and a host with no window reads for itself
//! ([`polled`]). A host that lends nothing does not answer these operations, and a device then
//! shows nothing of pull requests, as for a host from before them.
//!
//! What is said keeps to the grant, as a workspace list does: only the projects it reaches, and
//! of a project's workspaces only those on show.

pub mod polled;

use std::collections::{BTreeMap, HashSet};

use remote_proto::pull::{
    CheckState, PullCheck, PullChecks, PullDetail, PullListParams, PullPage, PullRef, PullReview,
    PullReviewer, PullState, PullSummary, PullsOf, ReviewerState, MAX_CHECKS, MAX_CHECK_NAME,
    MAX_PULL_WORKSPACES, MAX_REVIEWERS,
};
use remote_proto::{
    clip, ErrorCode, Grant, ProjectId, RemoteError, WorkspaceId, MAX_LABEL, MAX_LINK, MAX_TEXT,
};

use super::{encoded_len, local_id, page, unreadable, Answer, HostFacade, PAGE_BUDGET};
use crate::forge::{Checks, Mergeable, PullRequest, PullRequestState, ReviewState};

/// What a host that reads pull requests lends, for a device to be told of them.
pub trait PullRequests: Send + Sync {
    /// What was last read of a project's pull requests. It never waits for the forge: when what
    /// it has is old, or it has nothing, it starts reading and says so
    /// ([`ProjectPulls::reading`]). `every_open` asks that the next reading take in every open
    /// one, not only the newest.
    fn of_project(&self, project_id: &str, every_open: bool) -> ProjectPulls;

    /// Something a device did changed a project's pull requests (a merge, a push, one opened):
    /// what was read is kept to show, but the next question reads them again at once.
    fn changed(&self, _project_id: &str) {}
}

/// A project's pull requests as its host last read them.
#[derive(Debug, Clone, Default)]
pub struct ProjectPulls {
    pub pull_requests: Vec<PullRequest>,
    /// Each workspace's own, by number, the one it shows first leading.
    pub workspaces: BTreeMap<String, Vec<u32>>,
    /// Those its workspaces follow in other repositories.
    pub followed: Vec<FollowedPull>,
    /// A reading is on its way.
    pub reading: bool,
}

/// A pull request in another repository that a workspace follows.
#[derive(Debug, Clone)]
pub struct FollowedPull {
    /// The host's id for the following.
    pub id: String,
    pub workspace_id: String,
    /// `owner/name`.
    pub repo: String,
    pub pull_request: PullRequest,
}

impl HostFacade<'_> {
    pub(super) fn pulls(&self, grant: &Grant, params: &PullListParams) -> Answer<PullPage> {
        let lender = self.pulls.ok_or_else(not_here)?;
        let shown = self.workspaces_on_show()?;
        let mut found: Vec<(String, PullSummary)> = Vec::new();
        let mut open = 0u32;
        let mut reading = false;
        for project in self.store.projects().map_err(|_| unreadable())? {
            let id: ProjectId = local_id(&project.id)?;
            if !grant.reaches(&id) {
                continue;
            }
            let read = lender.of_project(&project.id, params.of == PullsOf::Open);
            reading |= read.reading;
            let of_project = summaries(&id, &read, shown.get(&project.id))?;
            open += count(
                of_project
                    .iter()
                    .filter(|(_, pull)| pull.state == PullState::Open),
            );
            found.extend(of_project.into_iter().filter(|(_, pull)| match params.of {
                PullsOf::Workspaces => !pull.workspaces.is_empty(),
                PullsOf::Open => pull.state == PullState::Open,
                PullsOf::Merged => pull.state == PullState::Merged,
                PullsOf::Closed => pull.state == PullState::Closed,
            }));
        }
        if params.of != PullsOf::Workspaces {
            // Most recently changed first; one whose time was not read goes last.
            found.sort_by(|(_, a), (_, b)| {
                b.updated_at
                    .cmp(&a.updated_at)
                    .then(b.number.cmp(&a.number))
            });
        }
        let (pulls, next) = page(
            found,
            |(key, _)| key.as_str(),
            params.cursor.as_ref(),
            params.limit,
            |pulls| {
                encoded_len(&PullPage {
                    pulls: pulls.iter().map(|(_, pull)| pull.clone()).collect(),
                    next: None,
                    open,
                    reading,
                })
            },
        )?;
        Ok(PullPage {
            pulls: pulls.into_iter().map(|(_, pull)| pull).collect(),
            next,
            open,
            reading,
        })
    }

    pub(super) fn pull(&self, grant: &Grant, params: &PullRef) -> Answer<PullDetail> {
        let lender = self.pulls.ok_or_else(not_here)?;
        let project = self.reachable_project(grant, &params.project)?;
        let read = lender.of_project(&project.id, false);
        let followed = match &params.followed {
            Some(id) => Some(
                read.followed
                    .iter()
                    .find(|f| f.id == *id && f.pull_request.number == params.number)
                    .ok_or_else(RemoteError::not_found)?,
            ),
            None => None,
        };
        let request = match followed {
            Some(followed) => &followed.pull_request,
            None => read
                .pull_requests
                .iter()
                .find(|pr| pr.number == params.number)
                .ok_or_else(RemoteError::not_found)?,
        };
        // As the list tells of it, so one followed from a workspace not on show is not found.
        let shown = self.workspaces_on_show()?;
        let pull = summaries(&params.project, &read, shown.get(&project.id))?
            .into_iter()
            .map(|(_, pull)| pull)
            .find(|pull| match followed {
                Some(followed) => {
                    pull.followed.is_some()
                        && pull.number == params.number
                        && pull.repo.as_deref() == Some(clip(&followed.repo, MAX_TEXT).as_str())
                }
                None => pull.followed.is_none() && pull.number == params.number,
            })
            .ok_or_else(RemoteError::not_found)?;
        let details = request.details.as_ref();

        let mut checks: Vec<PullCheck> = details
            .map(|details| {
                details
                    .checks
                    .iter()
                    .map(|check| PullCheck {
                        name: clip(
                            &match &check.workflow {
                                Some(workflow) if !workflow.is_empty() => {
                                    format!("{workflow} / {}", check.name)
                                }
                                _ => check.name.clone(),
                            },
                            MAX_CHECK_NAME,
                        ),
                        state: match check.state {
                            Checks::Failing => CheckState::Failed,
                            Checks::Running => CheckState::Running,
                            Checks::Passing => CheckState::Passed,
                            Checks::None => CheckState::Other,
                        },
                    })
                    .collect()
            })
            .unwrap_or_default();
        // What needs someone first. The order within each is the forge's.
        checks.sort_by_key(|check| match check.state {
            CheckState::Failed => 0,
            CheckState::Running => 1,
            CheckState::Passed | CheckState::Other => 2,
        });
        let more_checks = count(checks.iter().skip(MAX_CHECKS));
        checks.truncate(MAX_CHECKS);

        let mut detail = PullDetail {
            pull,
            checks,
            more_checks,
            reviews: details
                .map(|details| {
                    details
                        .reviews
                        .iter()
                        .take(MAX_REVIEWERS)
                        .map(|review| PullReviewer {
                            login: clip(&review.login, MAX_LABEL),
                            state: match review.state {
                                ReviewState::Approved => ReviewerState::Approved,
                                ReviewState::ChangesRequested => ReviewerState::ChangesRequested,
                                ReviewState::Commented => ReviewerState::Commented,
                                ReviewState::Dismissed => ReviewerState::Dismissed,
                            },
                        })
                        .collect()
                })
                .unwrap_or_default(),
            waiting_on: details
                .map(|details| {
                    details
                        .review_requests
                        .iter()
                        .take(MAX_REVIEWERS)
                        .map(|asked| clip(&asked.name, MAX_LABEL))
                        .collect()
                })
                .unwrap_or_default(),
        };
        // Every field has its limit, and together they fit a message with room to spare. Should
        // they ever not, the checks at the end go, counted, before a device is sent too much.
        while encoded_len(&detail) > PAGE_BUDGET && !detail.checks.is_empty() {
            let keep = detail.checks.len() / 2;
            detail.more_checks += count(detail.checks.iter().skip(keep));
            detail.checks.truncate(keep);
        }
        Ok(detail)
    }

    /// The workspaces on show, by project: not forgotten, and not put away.
    fn workspaces_on_show(&self) -> Answer<BTreeMap<String, Vec<String>>> {
        let mut shown: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for row in self.store.workspaces().map_err(|_| unreadable())? {
            if !row.forgotten && !row.archived {
                shown.entry(row.project_id).or_default().push(row.id);
            }
        }
        Ok(shown)
    }
}

/// A project's pull requests as a device is told of them, each with the key a page is cut at:
/// those its workspaces opened first, a workspace at a time and in the host's order for it, then
/// those they follow elsewhere, then the rest. So a workspace's own always come before what it
/// follows, and the first of its own is the one its host shows first.
fn summaries(
    project: &ProjectId,
    read: &ProjectPulls,
    shown: Option<&Vec<String>>,
) -> Answer<Vec<(String, PullSummary)>> {
    let shown: &[String] = shown.map_or(&[], Vec::as_slice);
    let mut found: Vec<(String, PullSummary)> = Vec::new();
    let mut own = HashSet::new();
    let mut elsewhere: HashSet<(&str, u32)> = HashSet::new();
    let add_workspace = |pull: &mut PullSummary, workspace: &str| -> Answer<()> {
        let id: WorkspaceId = local_id(workspace)?;
        if pull.workspaces.len() < MAX_PULL_WORKSPACES && !pull.workspaces.contains(&id) {
            pull.workspaces.push(id);
        }
        Ok(())
    };
    for workspace in shown {
        for number in read.workspaces.get(workspace).into_iter().flatten() {
            let Some(pr) = read.pull_requests.iter().find(|pr| pr.number == *number) else {
                continue;
            };
            if own.insert(*number) {
                found.push((
                    format!("{}.n.{number}", project.as_str()),
                    summary(project, pr),
                ));
            }
            if let Some((_, pull)) = found
                .iter_mut()
                .find(|(_, pull)| pull.followed.is_none() && pull.number == *number)
            {
                add_workspace(pull, workspace)?;
            }
        }
    }
    for workspace in shown {
        for followed in read
            .followed
            .iter()
            .filter(|f| f.workspace_id == *workspace)
        {
            let number = followed.pull_request.number;
            // Followed by two workspaces, it is one pull request, told of once.
            if elsewhere.insert((followed.repo.as_str(), number)) {
                // Not a local id: the store's own, which a protocol id is checked the same way.
                if remote_proto::ids::check_id(&followed.id).is_err() {
                    elsewhere.remove(&(followed.repo.as_str(), number));
                    continue;
                }
                let mut pull = summary(project, &followed.pull_request);
                pull.followed = Some(followed.id.clone());
                pull.repo = Some(clip(&followed.repo, MAX_TEXT));
                found.push((format!("{}.f.{}", project.as_str(), followed.id), pull));
            }
            if let Some((_, pull)) = found.iter_mut().find(|(_, pull)| {
                pull.followed.is_some()
                    && pull.number == number
                    && pull.repo.as_deref() == Some(clip(&followed.repo, MAX_TEXT).as_str())
            }) {
                add_workspace(pull, workspace)?;
            }
        }
    }
    for pr in &read.pull_requests {
        if own.insert(pr.number) {
            found.push((
                format!("{}.n.{}", project.as_str(), pr.number),
                summary(project, pr),
            ));
        }
    }
    Ok(found)
}

fn summary(project: &ProjectId, pr: &PullRequest) -> PullSummary {
    let details = pr.details.as_ref();
    PullSummary {
        project: project.clone(),
        number: pr.number,
        followed: None,
        repo: None,
        title: clip(&pr.title, MAX_TEXT),
        // A device opens this in a browser: only a link to a web page is passed on.
        url: Some(pr.url.as_str())
            .filter(|url| url.starts_with("https://") && url.len() <= MAX_LINK)
            .map(str::to_owned),
        branch: clip(&pr.branch, MAX_TEXT),
        base: details.map(|details| clip(&details.base, MAX_TEXT)),
        state: match pr.state {
            PullRequestState::Open => PullState::Open,
            PullRequestState::Merged => PullState::Merged,
            PullRequestState::Closed => PullState::Closed,
        },
        draft: pr.draft,
        checks: details.map_or_else(PullChecks::default, |details| PullChecks {
            passed: details.check_counts.passed,
            failed: details.check_counts.failed,
            running: details.check_counts.running,
        }),
        review: match details.map(|details| details.review.as_str()) {
            Some("APPROVED") => PullReview::Approved,
            Some("CHANGES_REQUESTED") => PullReview::ChangesRequested,
            Some("REVIEW_REQUIRED") => PullReview::Required,
            _ => PullReview::None,
        },
        conflicts: details.is_some_and(|details| details.mergeable == Mergeable::Conflicting),
        additions: details.map(|details| details.additions),
        deletions: details.map(|details| details.deletions),
        author: pr.author.as_deref().map(|login| clip(login, MAX_LABEL)),
        updated_at: details
            .and_then(|details| crate::activity::iso_to_ms(&details.updated_at))
            .or(pr.created_at),
        workspaces: Vec::new(),
    }
}

fn count<T>(items: impl Iterator<Item = T>) -> u32 {
    u32::try_from(items.count()).unwrap_or(u32::MAX)
}

fn not_here() -> RemoteError {
    RemoteError::new(
        ErrorCode::Unsupported,
        "Pull requests are not read from this computer.",
    )
}
