//! Sending work on from a workspace on another computer: commit, push, and its pull request. See
//! docs/design/44-other-computers-like-local.md.
//!
//! **That computer does it, as itself.** It commits with its own Git identity, pushes with its
//! own credentials and asks the forge with its own sign-in. This computer names a workspace by
//! that host's id and sends the words: a commit message, a pull request's title and description.
//!
//! **Each is the person's press.** Nothing here is called for an agent, on a timer or again by
//! itself. A commit or a push whose answer is lost with the link is not tried again: the person
//! is told it may have happened, and reads how the workspace stands before pressing again.

use std::sync::Arc;

use remote_proto::publish::{
    MergeMethod, PublishState, PullRequestMergeParams, PullRequestOpenParams, PullRequestOpened,
    WorkspaceCommitParams, WorkspacePublishParams,
};
use remote_proto::WorkspaceId;

use super::start::{no_such, open, refused};
use super::ComputerHome;
use crate::account::Failure;

/// What the host answers that can commit, push and open a pull request: one from before minor
/// 14 cannot.
pub const PUBLISH: &str = "workspace.publish";

/// Why something that changes a repository there was not answered. When the link dropped
/// first, it may have been done all the same, and the person is told so.
fn unanswered(error: &remote_client::channel::ChannelError) -> Failure {
    match error {
        remote_client::channel::ChannelError::Closed(_) => Failure::new(
            "remote_lost",
            "That computer's answer did not arrive, so it may have been done anyway. Read how \
             the workspace stands before pressing again.",
        ),
        other => refused(other, false),
    }
}

fn workspace_id(workspace: &str) -> Result<WorkspaceId, Failure> {
    WorkspaceId::new(workspace.to_owned()).map_err(|_| no_such())
}

/// Where `workspace` on `host` stands with its remote. With `forge`, that computer asks the
/// forge about its pull request too.
pub async fn state<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    workspace: &str,
    forge: bool,
) -> Result<PublishState, Failure> {
    let workspace = workspace_id(workspace)?;
    let channel = open(home.as_ref(), host, PUBLISH)?;
    channel
        .publish_state(WorkspacePublishParams { workspace, forge })
        .await
        .map_err(|e| refused(&e, false))
}

/// Commit everything changed in `workspace` on `host`.
pub async fn commit<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    workspace: &str,
    message: &str,
) -> Result<PublishState, Failure> {
    let workspace = workspace_id(workspace)?;
    let channel = open(home.as_ref(), host, PUBLISH)?;
    channel
        .commit(WorkspaceCommitParams {
            workspace,
            message: message.trim().to_owned(),
        })
        .await
        .map_err(|e| unanswered(&e))
}

/// Push `workspace`'s branch on `host` to its project's remote.
pub async fn push<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    workspace: &str,
) -> Result<PublishState, Failure> {
    let workspace = workspace_id(workspace)?;
    let channel = open(home.as_ref(), host, PUBLISH)?;
    channel.push(workspace).await.map_err(|e| unanswered(&e))
}

/// Open a pull request for `workspace`'s branch on `host`.
pub async fn open_pull_request<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    workspace: &str,
    title: &str,
    body: &str,
    draft: bool,
) -> Result<PullRequestOpened, Failure> {
    let workspace = workspace_id(workspace)?;
    let channel = open(home.as_ref(), host, PUBLISH)?;
    channel
        .open_pull_request(PullRequestOpenParams {
            workspace,
            title: title.trim().to_owned(),
            body: body.to_owned(),
            draft,
        })
        .await
        .map_err(|e| unanswered(&e))
}

/// Merge `workspace`'s pull request on `host`, if it is still at `head_oid`.
pub async fn merge_pull_request<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    workspace: &str,
    number: u32,
    head_oid: &str,
    method: MergeMethod,
) -> Result<(), Failure> {
    let workspace = workspace_id(workspace)?;
    let channel = open(home.as_ref(), host, PUBLISH)?;
    channel
        .merge_pull_request(PullRequestMergeParams {
            workspace,
            number,
            head_oid: head_oid.to_owned(),
            method,
        })
        .await
        .map_err(|e| unanswered(&e))
}
