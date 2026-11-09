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
    HarnessSummary, ProjectId, SessionRecordId, SessionResumeParams, SessionStartParams, Started,
    WorkspaceId, WorkspaceStartParams,
};

use super::{lock, ComputerHome};
use crate::account::Failure;

/// What the host answers that can start work: one from before minor 5 cannot.
pub const START: &str = "workspace.start";
/// What the host answers that can resume a conversation: one from before minor 6 cannot.
pub const RESUME: &str = "session.resume";

/// The agents `host` can start, its default first.
pub async fn harnesses<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
) -> Result<Vec<HarnessSummary>, Failure> {
    let channel = open(home.as_ref(), host, START)?;
    channel.harnesses().await.map_err(|e| refused(&e, false))
}

/// A new workspace in `project` on `host`, named after `prompt`, with an agent given it.
pub async fn workspace<H: ComputerHome>(
    home: &Arc<H>,
    host: &str,
    project: &str,
    prompt: &str,
    harness: Option<&str>,
) -> Result<Started, Failure> {
    let project = ProjectId::new(project.to_owned()).map_err(|_| no_such())?;
    let channel = open(home.as_ref(), host, START)?;
    let started = channel
        .start_workspace(WorkspaceStartParams {
            project,
            prompt: prompt.to_owned(),
            harness: harness.map(str::to_owned),
        })
        .await;
    done(home.as_ref(), started)
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
fn open(home: &impl ComputerHome, host: &str, capability: &str) -> Result<Arc<Channel>, Failure> {
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

fn refused(error: &ChannelError, starting: bool) -> Failure {
    match error {
        ChannelError::Closed(_) if starting => Failure::new(
            "remote_lost",
            "That computer's answer did not arrive, so it may have started anyway. Look at its \
             list before starting again.",
        ),
        ChannelError::Remote(remote) => Failure::new("remote_refused", remote.message.clone()),
        other => Failure::new("remote_failed", other.to_string()),
    }
}

fn no_such() -> Failure {
    Failure::new("remote_unknown", "That is not one of that computer's.")
}
