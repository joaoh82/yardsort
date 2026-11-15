//! Sending a workspace's work on for a device: the host's side of [`remote_proto::publish`].
//!
//! Each is what the window's own commit box and publish bar do, with this host's Git identity,
//! its Git credentials and its forge sign-in. A device names a workspace by its id and nothing
//! else: the branch pushed is the workspace's own, to the project's own remote.
//!
//! What Git and the forge's program print when they refuse can name a folder here, or a remote's
//! address with whatever is in it. So a device is never given their words. A refusal is sorted
//! into the few kinds a person can act on, and said in this host's own.

use std::path::Path;

use remote_proto::publish::{
    Checks, PublishState, PullRequest, PullRequestOpened, PullRequestState, MAX_UNPUSHED, MAX_URL,
};
use remote_proto::{clip, MAX_TEXT};

use super::start::Starter;
use crate::error::{IpcError, IpcResult};
use crate::forge::{self, parse_remote, pull_request_for, Gh, MergeMethod, Mergeable};
use crate::git::{GitError, Head};

/// How many of a project's pull requests are asked for to find a workspace's own.
const RECENT: u32 = 50;

/// What a workspace's branch has to do with its remote, as Git alone knows it.
struct Standing {
    branch: Option<String>,
    base: Option<String>,
    remote: Option<String>,
    repo: Option<forge::Repo>,
    upstream: Option<String>,
    /// The pull request the branch is a checkout of, from a fork: it is not pushed from here.
    follows: Option<u32>,
}

impl Starter<'_> {
    fn standing(&self, root: &Path, base_branch: Option<&str>) -> IpcResult<Standing> {
        let branch = match self.git.head(root)? {
            Head::Branch(name) => Some(name),
            Head::Unborn(_) | Head::Detached(_) => None,
        };
        let base = match base_branch {
            Some(base) => Some(base.to_owned()),
            None => self.git.default_branch(root)?,
        }
        .filter(|base| Some(base) != branch.as_ref());
        let remote = self.git.push_remote(root)?;
        let repo = match &remote {
            // As the repository's configuration names it: which forge it is on does not change
            // with this computer's own rewriting of addresses.
            Some(name) => self
                .git
                .remote_url_as_set(root, name)
                .as_deref()
                .and_then(parse_remote),
            None => None,
        };
        let follows = match &branch {
            Some(branch) => self.git.followed_pull_request(root, branch)?,
            None => None,
        };
        Ok(Standing {
            upstream: self.git.upstream(root)?,
            branch,
            base,
            remote,
            repo,
            follows,
        })
    }

    /// Where `workspace` stands with its remote. With `forge`, the forge is asked about its pull
    /// request, which is a network round trip; without, only what Git knows is said.
    pub fn publish(&self, workspace: &str, forge: bool) -> IpcResult<PublishState> {
        let (row, root) = self.folder(workspace)?;
        let standing = self.standing(&root, row.base_branch.as_deref())?;
        // Before the first push there is no upstream to count against: the base stands in.
        let reference = standing.upstream.clone().or_else(|| standing.base.clone());
        let (ahead, behind) = match &reference {
            Some(reference) => self.git.ahead_behind(&root, reference)?.unwrap_or((0, 0)),
            None => (0, 0),
        };
        let unpushed = match &reference {
            Some(reference) => self
                .git
                .commits_since(&root, reference)?
                .into_iter()
                .take(MAX_UNPUSHED)
                .map(|commit| clip(&commit.subject, MAX_TEXT))
                .collect(),
            None => vec![],
        };
        let gh = Gh::find(self.env);
        let (pull_request, problem) = match (&gh, forge) {
            (Some(gh), true) => match gh.pull_requests(&root, RECENT) {
                Ok(found) => (own(&found, &standing).map(told), None),
                Err(error) if error.is_logged_out() => (
                    None,
                    Some("The forge's program is not signed in on that computer.".to_owned()),
                ),
                Err(_) => (
                    None,
                    Some("The forge did not answer that computer just now.".to_owned()),
                ),
            },
            _ => (None, None),
        };
        let open = matches!(&pull_request, Some(pr) if pr.state == PullRequestState::Open);
        Ok(PublishState {
            identity: self.git.identity(&root)?.map(|who| clip(&who, MAX_TEXT)),
            can_open: standing.branch.is_some()
                && standing.base.is_some()
                && standing.repo.is_some()
                && standing.follows.is_none()
                && !open,
            branch: standing.branch,
            base: standing.base,
            remote: standing.remote.is_some(),
            pushed: standing.upstream.is_some(),
            ahead,
            behind,
            unpushed,
            pull_request,
            gh: gh.is_some(),
            problem,
        })
    }

    /// Commit everything changed in `workspace`, as the window's commit box does.
    pub fn commit(&self, workspace: &str, message: &str) -> IpcResult<PublishState> {
        let (_, root) = self.folder(workspace)?;
        let message = message.trim();
        if message.is_empty() {
            return Err(refusal(
                "empty_commit_message",
                "Write a commit message first.",
            ));
        }
        if self.git.identity(&root)?.is_none() {
            return Err(refusal(
                "no_git_identity",
                "Git has no name and email to commit with on that computer. Set them there \
                 with `git config --global user.name` and `user.email`.",
            ));
        }
        // Git says "nothing to commit" on its standard output and fails with nothing else, so it
        // is looked for before, not read out of the failure.
        if self.git.run(&root, &["status", "--porcelain"])?.is_empty() {
            return Err(refusal(
                "nothing_to_commit",
                "There is nothing to commit in that workspace.",
            ));
        }
        self.git.commit_all(&root, message).map_err(|_| {
            refusal(
                "commit_failed",
                "The commit failed on that computer. A commit hook there may have refused it: \
                 open a shell in the workspace to see why.",
            )
        })?;
        self.publish(workspace, false)
    }

    fn push_branch(&self, root: &Path, standing: &Standing) -> IpcResult<()> {
        if let Some(number) = standing.follows {
            return Err(refusal(
                "follows_pull_request",
                &format!(
                    "This branch is a checkout of pull request #{number}, which comes from a \
                     fork. Yardsort does not push to forks."
                ),
            ));
        }
        let (Some(remote), Some(branch)) = (&standing.remote, &standing.branch) else {
            return Err(refusal(
                "nothing_to_push",
                match standing.branch {
                    Some(_) => "That project has no remote to push to.",
                    None => "That workspace is not on a branch.",
                },
            ));
        };
        self.git.push(root, remote, branch).map_err(|error| {
            let said = said(&error);
            let has = |words: &[&str]| words.iter().any(|word| said.contains(word));
            if has(&["non-fast-forward", "fetch first", "[rejected]"]) {
                refusal(
                    "push_rejected",
                    "The remote has commits this branch does not have, so it refused the push. \
                     Bring them in on that computer first.",
                )
            } else if has(&["protected branch", "hook declined", "[remote rejected]"]) {
                refusal(
                    "push_refused",
                    "The remote refused the push: a rule there does not allow it for this branch.",
                )
            } else if has(&[
                "authentication failed",
                "could not read username",
                "permission denied",
                "terminal prompts disabled",
                "403",
            ]) {
                refusal(
                    "push_not_allowed",
                    "That computer's Git sign-in was not accepted by the remote. Sign in there, \
                     then push again.",
                )
            } else {
                refusal(
                    "push_failed",
                    "The push failed on that computer. Open a shell in the workspace there to \
                     see why.",
                )
            }
        })
    }

    /// Push `workspace`'s branch to the project's remote, setting its upstream the first time.
    pub fn push(&self, workspace: &str) -> IpcResult<PublishState> {
        let (row, root) = self.folder(workspace)?;
        let standing = self.standing(&root, row.base_branch.as_deref())?;
        self.push_branch(&root, &standing)?;
        self.publish(workspace, false)
    }

    /// Open a pull request for `workspace`'s branch, pushing first when the remote lacks any of
    /// it. Without the forge's program, or signed out of it, the answer is the forge's own form.
    pub fn open_pull_request(
        &self,
        workspace: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> IpcResult<PullRequestOpened> {
        let (row, root) = self.folder(workspace)?;
        let standing = self.standing(&root, row.base_branch.as_deref())?;
        let Some(base) = standing.base.clone() else {
            return Err(refusal(
                "no_base_branch",
                "That workspace is on the branch it would merge into.",
            ));
        };
        let ahead = match &standing.upstream {
            Some(upstream) => self
                .git
                .ahead_behind(&root, upstream)?
                .map_or(0, |(a, _)| a),
            None => 1,
        };
        if ahead > 0 {
            self.push_branch(&root, &standing)?;
        }
        let form = || -> IpcResult<PullRequestOpened> {
            let url = match (&standing.repo, &standing.branch) {
                (Some(repo), Some(branch)) => Some(repo.compare_url(&base, branch)),
                _ => None,
            }
            .filter(|url| url.len() <= MAX_URL)
            .ok_or_else(|| {
                refusal(
                    "no_forge",
                    "That project's remote is not a forge Yardsort can open a pull request on.",
                )
            })?;
            Ok(PullRequestOpened {
                url,
                created: false,
            })
        };
        let Some(gh) = Gh::find(self.env) else {
            return form();
        };
        match gh.create_pull_request(&root, &base, title.trim(), body, draft) {
            Ok(url) if url.len() <= MAX_URL => Ok(PullRequestOpened { url, created: true }),
            Ok(_) => Err(refusal(
                "pull_request_failed",
                "The pull request was opened, and its address could not be read. Look on the forge.",
            )),
            Err(error) if error.is_logged_out() => form(),
            Err(error) if said_by_gh(&error).contains("already exists") => Err(refusal(
                "pull_request_exists",
                "A pull request for this branch is already open.",
            )),
            Err(_) => Err(refusal(
                "pull_request_failed",
                "The forge did not open the pull request for that computer. Open a shell in the \
                 workspace there to see why.",
            )),
        }
    }

    /// Merge the pull request of `workspace`'s branch: only one that is still its own, open,
    /// not a draft, and at the commit the person confirmed.
    pub fn merge_pull_request(
        &self,
        workspace: &str,
        number: u32,
        head_oid: &str,
        method: remote_proto::publish::MergeMethod,
    ) -> IpcResult<()> {
        let (row, root) = self.folder(workspace)?;
        let standing = self.standing(&root, row.base_branch.as_deref())?;
        let gh = Gh::find(self.env).ok_or_else(|| {
            refusal(
                "gh_not_installed",
                "That computer has no program to merge a pull request with. Merge it on the forge.",
            )
        })?;
        let changed = || {
            refusal(
                "pull_request_changed",
                "The pull request changed or is no longer ready to merge. Read it again and \
                 look before merging.",
            )
        };
        let found = gh.pull_requests(&root, RECENT).map_err(|_| {
            refusal(
                "forge_unreachable",
                "The forge did not answer that computer just now, so nothing was merged.",
            )
        })?;
        let pr = own(&found, &standing)
            .filter(|pr| pr.number == number)
            .filter(|pr| pr.state == forge::PullRequestState::Open && !pr.draft)
            .filter(|pr| {
                pr.details
                    .as_ref()
                    .is_some_and(|details| details.head_oid == head_oid)
            })
            .ok_or_else(changed)?;
        let method = match method {
            remote_proto::publish::MergeMethod::Squash => MergeMethod::Squash,
            remote_proto::publish::MergeMethod::Merge => MergeMethod::Merge,
            remote_proto::publish::MergeMethod::Rebase => MergeMethod::Rebase,
        };
        gh.merge_pull_request(&root, pr.number, head_oid, method)
            .map_err(|_| {
                refusal(
                    "merge_failed",
                    "The forge did not merge the pull request. Its checks or its rules may not \
                     allow it yet: look at it on the forge.",
                )
            })
    }
}

/// The pull request that is this workspace's: the one its branch follows, from a fork, or else
/// the one whose head is its branch.
fn own<'a>(found: &'a [forge::PullRequest], standing: &Standing) -> Option<&'a forge::PullRequest> {
    match standing.follows {
        Some(number) => found.iter().find(|pr| pr.number == number),
        None => standing
            .branch
            .as_ref()
            .and_then(|branch| pull_request_for(found, branch)),
    }
}

/// A pull request as a device is told it.
fn told(pr: &forge::PullRequest) -> PullRequest {
    PullRequest {
        number: pr.number,
        url: if pr.url.len() <= MAX_URL {
            pr.url.clone()
        } else {
            String::new()
        },
        title: clip(&pr.title, remote_proto::publish::MAX_PULL_REQUEST_TITLE),
        state: match pr.state {
            forge::PullRequestState::Open => PullRequestState::Open,
            forge::PullRequestState::Merged => PullRequestState::Merged,
            forge::PullRequestState::Closed => PullRequestState::Closed,
        },
        draft: pr.draft,
        checks: match pr.checks {
            forge::Checks::None => Checks::None,
            forge::Checks::Running => Checks::Running,
            forge::Checks::Passing => Checks::Passing,
            forge::Checks::Failing => Checks::Failing,
        },
        head_oid: pr
            .details
            .as_ref()
            .map(|details| details.head_oid.clone())
            // Only one a merge could name back.
            .filter(|oid| {
                (7..=remote_proto::publish::MAX_OID).contains(&oid.len())
                    && oid.bytes().all(|b| b.is_ascii_hexdigit())
            }),
        conflicting: pr
            .details
            .as_ref()
            .is_some_and(|details| details.mergeable == Mergeable::Conflicting),
    }
}

/// A refusal in this host's own words, which a device may be shown as they are.
fn refusal(code: &str, message: &str) -> IpcError {
    IpcError::new(code, message)
}

/// What Git printed when it failed, in lower case, to sort the failure by. Never passed on.
fn said(error: &GitError) -> String {
    match error {
        GitError::Failed { stderr, .. } => stderr.to_ascii_lowercase(),
        other => other.to_string().to_ascii_lowercase(),
    }
}

fn said_by_gh(error: &forge::ForgeError) -> String {
    match error {
        forge::ForgeError::Failed { stderr, .. } => stderr.to_ascii_lowercase(),
        other => other.to_string().to_ascii_lowercase(),
    }
}

/// The codes of [`refusal`]s, whose messages are this host's own and safe to pass to a device.
pub const REFUSALS: &[&str] = &[
    "empty_commit_message",
    "no_git_identity",
    "nothing_to_commit",
    "commit_failed",
    "follows_pull_request",
    "nothing_to_push",
    "push_rejected",
    "push_refused",
    "push_not_allowed",
    "push_failed",
    "no_forge",
    "pull_request_failed",
    "pull_request_exists",
    "pull_request_changed",
    "gh_not_installed",
    "forge_unreachable",
    "merge_failed",
];
