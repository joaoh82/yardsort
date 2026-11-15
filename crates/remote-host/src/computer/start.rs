//! Starting work on another computer from this one: a new workspace in one of its projects,
//! another agent in one of its workspaces, or one of its ended conversations again. See
//! docs/design/35-remote-r4-r5.md §16.
//!
//! **By that host's ids, never a path or a program.** This computer names a project, a
//! workspace or a conversation as that host listed it, and an agent from that host's own list;
//! the host starts it with its own settings, as its window would. It needs the control a grant
//! gives (`terminal:control`), which the host checks again.
//!
//! **Nothing is started twice.** A start whose answer is lost with the link is not tried again:
//! the person is told it may have started, and to look before starting again.

use std::sync::Arc;

use remote_client::channel::ChannelError;
use remote_client::Channel;
use remote_proto::{
    ErrorCode, HarnessSummary, ProjectBranches, ProjectCloneParams, ProjectCloned, ProjectId,
    ProjectRef, SessionRecordId, SessionResumeParams, SessionStartParams, ShellStartParams,
    Started, WorkspaceId, WorkspaceRemoveParams, WorkspaceRenameParams, WorkspaceStartParams,
};

use super::{lock, ComputerHome};
use crate::account::Failure;

/// What the host answers that can start work: one from before minor 5 cannot.
pub const START: &str = "workspace.start";
/// What the host answers that can resume a conversation: one from before minor 6 cannot.
pub const RESUME: &str = "session.resume";
/// What the host answers that reads a start's name, model, effort and branch, lists a project's
/// branches, clones, starts a shell, and renames, archives and deletes a workspace: one from
/// before minor 10 does none of them.
pub const MANAGE: &str = "project.branches";

/// What a new workspace on another computer may say beyond its project and its first message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Said {
    pub harness: Option<String>,
    pub name: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub base_branch: Option<String>,
    pub existing_branch: Option<String>,
}

/// The agents `host` can start, its default first.
pub async fn harnesses<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
) -> Result<Vec<HarnessSummary>, Failure> {
    let channel = open(home.as_ref(), host, START)?;
    channel.harnesses().await.map_err(|e| refused(&e, false))
}

/// A new workspace in `project` on `host`, with an agent given `prompt`. What else is `said`
/// (a name, a model, an effort, a branch) is sent only to a host that reads it: an older one is
/// not asked, since it would start something other than what was chosen.
pub async fn workspace<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    project: &str,
    prompt: &str,
    said: Said,
) -> Result<Started, Failure> {
    let project = ProjectId::new(project.to_owned()).map_err(|_| no_such())?;
    let text = |part: Option<String>| part.filter(|p| !p.trim().is_empty());
    let params = WorkspaceStartParams {
        project,
        prompt: prompt.to_owned(),
        harness: text(said.harness),
        name: text(said.name),
        model: text(said.model),
        effort: text(said.effort),
        base_branch: text(said.base_branch),
        existing_branch: text(said.existing_branch),
    };
    let needs = if params.says_more() { MANAGE } else { START };
    let channel = open(home.as_ref(), host, needs)?;
    done(home.as_ref(), channel.start_workspace(params).await)
}

/// The branches a new workspace in `project` on `host` can start from or open.
pub async fn branches<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    project: &str,
) -> Result<ProjectBranches, Failure> {
    let project = ProjectId::new(project.to_owned()).map_err(|_| no_such())?;
    let channel = open(home.as_ref(), host, MANAGE)?;
    channel
        .branches(ProjectRef { project })
        .await
        .map_err(|e| refused(&e, false))
}

/// Have `host` clone the repository at `remote` into its own projects folder, with its own Git
/// credentials, and add it as a project. `remote` is an address with no credentials in it; where
/// it goes is that computer's setting.
pub async fn clone<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    remote: &str,
) -> Result<ProjectCloned, Failure> {
    let channel = open(home.as_ref(), host, MANAGE)?;
    let cloned = channel
        .clone_project(ProjectCloneParams {
            remote: remote.trim().to_owned(),
            name: None,
        })
        .await
        .map_err(|e| refused(&e, true))?;
    home.computer().reread.notify_waiters();
    Ok(cloned)
}

/// That computer's shell in `workspace` on `host`.
pub async fn shell<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    workspace: &str,
) -> Result<Started, Failure> {
    let workspace = WorkspaceId::new(workspace.to_owned()).map_err(|_| no_such())?;
    let channel = open(home.as_ref(), host, MANAGE)?;
    done(
        home.as_ref(),
        channel.start_shell(ShellStartParams { workspace }).await,
    )
}

/// Change what `workspace` on `host` is called.
pub async fn rename<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    workspace: &str,
    name: &str,
) -> Result<(), Failure> {
    let workspace = WorkspaceId::new(workspace.to_owned()).map_err(|_| no_such())?;
    let channel = open(home.as_ref(), host, MANAGE)?;
    changed(
        home.as_ref(),
        channel
            .rename_workspace(WorkspaceRenameParams {
                workspace,
                name: name.trim().to_owned(),
            })
            .await,
    )
}

/// Put `workspace` on `host` away, or with `delete` remove it. Both keep its branch. Without
/// `force`, one with uncommitted work is refused as `remote_uncommitted`, for the person to be
/// asked a second time.
pub async fn remove<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    workspace: &str,
    delete: bool,
    force: bool,
) -> Result<(), Failure> {
    let workspace = WorkspaceId::new(workspace.to_owned()).map_err(|_| no_such())?;
    let channel = open(home.as_ref(), host, MANAGE)?;
    let params = WorkspaceRemoveParams { workspace, force };
    changed(
        home.as_ref(),
        if delete {
            channel.delete_workspace(params).await
        } else {
            channel.archive_workspace(params).await
        },
    )
}

/// Another agent in `workspace` on `host`, with a first message or none.
pub async fn session<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    workspace: &str,
    harness: Option<&str>,
    prompt: Option<&str>,
) -> Result<Started, Failure> {
    let workspace = WorkspaceId::new(workspace.to_owned()).map_err(|_| no_such())?;
    let channel = open(home.as_ref(), host, START)?;
    let started = channel
        .start_session(SessionStartParams {
            workspace,
            harness: harness.map(str::to_owned),
            prompt: given(prompt),
        })
        .await;
    done(home.as_ref(), started)
}

/// The conversation recorded as `session` on `host` again, with a message to go on with or none.
pub async fn resume<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    session: &str,
    prompt: Option<&str>,
) -> Result<Started, Failure> {
    let session = SessionRecordId::new(session.to_owned()).map_err(|_| no_such())?;
    let channel = open(home.as_ref(), host, RESUME)?;
    let started = channel
        .resume_session(SessionResumeParams {
            session,
            prompt: given(prompt),
        })
        .await;
    done(home.as_ref(), started)
}

fn given(prompt: Option<&str>) -> Option<String> {
    prompt.filter(|p| !p.trim().is_empty()).map(str::to_owned)
}

/// The channel open to `host` now, if it answers `capability`.
pub(super) fn open(
    home: &impl ComputerHome,
    host: &str,
    capability: &str,
) -> Result<Arc<Channel>, Failure> {
    let channel = {
        let known = lock(home.computer());
        let link = known
            .links
            .iter()
            .find(|(id, _)| id.as_str() == host)
            .map(|(_, link)| link.channel.borrow().clone());
        match link {
            None => {
                return Err(Failure::new(
                    "remote_not_paired",
                    "This computer is not let in to that one.",
                ))
            }
            Some(channel) => channel,
        }
    };
    let channel = channel.ok_or_else(|| {
        Failure::new(
            "remote_offline",
            "That computer cannot be reached just now. Try again once it is online.",
        )
    })?;
    if !channel.host().capabilities.iter().any(|c| c == capability) {
        return Err(Failure::new(
            "remote_too_old",
            if capability == RESUME {
                "That computer's Yardsort is too old to resume a conversation from here. Update it, \
                 then try again."
            } else if capability == MANAGE || capability == super::look::LOOK {
                "That computer's Yardsort is too old for this from here. Update it, then try again."
            } else {
                "That computer's Yardsort is too old to start work from here. Update it, then try \
                 again."
            },
        ));
    }
    Ok(channel)
}

/// What a start came to. Started, the host's view is read again, so the new agent shows.
fn done(
    home: &impl ComputerHome,
    started: Result<Started, ChannelError>,
) -> Result<Started, Failure> {
    let started = started.map_err(|e| refused(&e, true))?;
    home.computer().reread.notify_waiters();
    Ok(started)
}

/// What a change to a workspace came to. Made, the host's view is read again, so it shows.
fn changed(home: &impl ComputerHome, made: Result<(), ChannelError>) -> Result<(), Failure> {
    made.map_err(|e| refused(&e, true))?;
    home.computer().reread.notify_waiters();
    Ok(())
}

pub(super) fn refused(error: &ChannelError, starting: bool) -> Failure {
    match error {
        ChannelError::Remote(remote) if remote.code == ErrorCode::UncommittedWork => {
            Failure::new("remote_uncommitted", remote.message.clone())
        }
        ChannelError::Closed(_) if starting => Failure::new(
            "remote_lost",
            "That computer's answer did not arrive, so it may have started anyway. Look at its \
             list before starting again.",
        ),
        ChannelError::Remote(remote) => Failure::new("remote_refused", remote.message.clone()),
        other => Failure::new("remote_failed", other.to_string()),
    }
}

pub(super) fn no_such() -> Failure {
    Failure::new("remote_unknown", "That is not one of that computer's.")
}
