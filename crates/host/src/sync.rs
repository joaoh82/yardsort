//! `yardsort-host sync`, the running host's side: how sync stands, and the operator's own acts,
//! turning a project on, linking it, stopping and removing. The loop and the rules are
//! `remote_host::sync` and `yardsort_core::sync`, the same as the desktop's. Every act is behind
//! the guard (`guard`), as `pair` is; only `status` and `preview` are not, and change nothing.

use std::sync::Arc;

use remote_host::account::{self, Failure};
use remote_host::sync::{self, Loop};
use remote_proto::sync::SyncId;
use yardsort_core::store::ProjectRow;
use yardsort_core::sync::{
    approved_notes, project_hint, project_sync, same_remote, synced_workspace_count, ProjectSync,
    SyncPreview,
};

use crate::control::{Offered, Reply, SyncStatus, SyncedProject};
use crate::home::HostHome;

fn refused(message: impl Into<String>) -> Failure {
    Failure::new("sync_refused", message)
}

fn store_failed(error: impl std::fmt::Display) -> Failure {
    Failure::new("remote_store", error.to_string())
}

/// A project here, by its name (any case) or its id.
pub(crate) fn project(home: &HostHome, named: &str) -> account::Result<ProjectRow> {
    let projects = home.store.projects().map_err(store_failed)?;
    let wanted = named.to_lowercase();
    let found: Vec<&ProjectRow> = projects
        .iter()
        .filter(|p| p.id == named || p.name.to_lowercase() == wanted)
        .collect();
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(refused(format!(
            "There is no project {named:?} here. See `ys project list`."
        ))),
        _ => Err(refused(format!(
            "More than one project is called {named:?}. Use its id from `ys project list`."
        ))),
    }
}

/// Run blocking Git on this thread, letting the runtime's other work go on.
fn hint(home: &HostHome, project: &ProjectRow) -> Option<String> {
    let git = home.git()?;
    tokio::task::block_in_place(|| project_hint(git, std::path::Path::new(&project.root_path)))
}

/// What turning sync on for a project sends.
pub fn preview(home: &Arc<HostHome>, named: &str) -> account::Result<Reply> {
    account::enabled(home.as_ref())?;
    let project = project(home, named)?;
    if home
        .store
        .synced_project(&project.id)
        .map_err(store_failed)?
        .is_some()
    {
        return Err(refused(format!("{} syncs already.", project.name)));
    }
    let remote = hint(home, &project);
    let notes = approved_notes(&home.store, &project.id).map_err(store_failed)?;
    let workspaces = synced_workspace_count(&home.store, &project.id).map_err(store_failed)?;
    Ok(Reply::Preview(SyncPreview {
        name: project.name,
        remote,
        notes,
        workspaces,
    }))
}

pub fn turn_on(home: &Arc<HostHome>, named: &str) -> account::Result<Reply> {
    let project = project(home, named)?;
    let remote = hint(home, &project);
    sync::turn_on(home.as_ref(), &project.id, remote)?;
    Ok(Reply::Done {
        message: format!(
            "{} syncs now. Link it on another computer with `yardsort-host sync link`, or in \
             the app's Project settings.",
            project.name
        ),
    })
}

/// A synced project the account has, by the id `status` shows (or its start) or its name.
async fn offer(home: &HostHome, named: &str) -> account::Result<sync::Offer> {
    let offered = sync::offered(home).await?;
    let wanted = named.to_lowercase();
    let by_id: Vec<&sync::Offer> = offered
        .iter()
        .filter(|o| o.id.as_str().starts_with(&wanted))
        .collect();
    let found = if by_id.is_empty() {
        offered
            .iter()
            .filter(|o| o.name.to_lowercase() == wanted)
            .collect()
    } else {
        by_id
    };
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(refused(format!(
            "No synced project {named:?} is waiting to be linked here. See `yardsort-host sync \
             status`."
        ))),
        _ => Err(refused(format!(
            "More than one synced project is {named:?}. Use the id `sync status` shows."
        ))),
    }
}

pub async fn link(home: &Arc<HostHome>, named: &str, synced: &str) -> account::Result<Reply> {
    let project = project(home, named)?;
    let offer = offer(home, synced).await?;
    sync::link(home.as_ref(), &project.id, &offer.id).await?;
    Ok(Reply::Done {
        message: format!("{} is linked to the synced {}.", project.name, offer.name),
    })
}

pub async fn stop(home: &Arc<HostHome>, named: &str) -> account::Result<Reply> {
    let project = project(home, named)?;
    sync::stop(home.as_ref(), &project.id).await?;
    Ok(Reply::Done {
        message: format!(
            "{} no longer syncs here. Its records stay on the server for your other computers.",
            project.name
        ),
    })
}

pub fn remove(home: &Arc<HostHome>, named: &str) -> account::Result<Reply> {
    let project = project(home, named)?;
    sync::remove(home.as_ref(), &project.id)?;
    Ok(Reply::Done {
        message: format!(
            "{}'s records are being removed from the server. Every computer keeps its own copy \
             and stops syncing it.",
            project.name
        ),
    })
}

/// The loop, in words.
fn loop_words(state: Option<Loop>) -> String {
    match state {
        None => "not running".into(),
        Some(Loop::Idle) => "idle".into(),
        Some(Loop::Syncing) => "syncing".into(),
        Some(Loop::Waiting { why, retry_in }) => {
            format!("waiting ({why}); trying again in {}s", retry_in.as_secs())
        }
    }
}

/// A synced project's state, in words.
pub fn project_words(state: &ProjectSync) -> String {
    match state {
        ProjectSync::On {
            refused: Some(why), ..
        } => format!("refused by the server: {why}"),
        ProjectSync::On { removing: true, .. } => "being removed from the server".into(),
        ProjectSync::On { reached: false, .. } => "waiting to reach the server".into(),
        ProjectSync::On { .. } => "syncing".into(),
        ProjectSync::Off { .. } => "not syncing".into(),
        ProjectSync::RemoteOff => "Remote is off".into(),
    }
}

pub async fn status(home: &Arc<HostHome>) -> SyncStatus {
    let remote = home.store.remote_host().ok().flatten().is_some();
    let state = home.store.sync_state().unwrap_or_default();
    let projects = home.store.projects().unwrap_or_default();
    let synced = home.store.synced_projects().unwrap_or_default();
    let mut lines = Vec::new();
    for row in &synced {
        let Some(project) = projects.iter().find(|p| p.id == row.local_id) else {
            continue;
        };
        let state = project_sync(&home.store, &project.id, remote, || unreachable!())
            .unwrap_or(ProjectSync::RemoteOff);
        lines.push(SyncedProject {
            name: project.name.clone(),
            state: project_words(&state),
        });
    }
    let (offered, offered_problem) = if remote {
        match sync::offered(home.as_ref()).await {
            Ok(offers) => {
                let unsynced: Vec<(&ProjectRow, Option<String>)> = projects
                    .iter()
                    .filter(|p| !synced.iter().any(|row| row.local_id == p.id))
                    .map(|p| (p, hint(home, p)))
                    .collect();
                let offered = offers
                    .into_iter()
                    .map(|offer| Offered {
                        matches: offer.remote.as_deref().and_then(|theirs| {
                            unsynced
                                .iter()
                                .find(|(_, ours)| {
                                    ours.as_deref()
                                        .is_some_and(|ours| same_remote(ours, theirs))
                                })
                                .map(|(p, _)| p.name.clone())
                        }),
                        id: short(&offer.id),
                        name: offer.name,
                        remote: offer.remote,
                    })
                    .collect();
                (offered, None)
            }
            Err(failure) => (Vec::new(), Some(failure.message)),
        }
    } else {
        (Vec::new(), None)
    };
    SyncStatus {
        remote,
        state: loop_words(sync::state(home.as_ref())),
        synced_at: state.synced_at,
        problem: state.problem,
        waiting: home.store.sync_waiting().unwrap_or(0),
        projects: lines,
        offered,
        offered_problem,
    }
}

/// The start of a logical id, enough to tell the account's projects apart.
fn short(id: &SyncId) -> String {
    id.as_str()[..8].to_owned()
}
