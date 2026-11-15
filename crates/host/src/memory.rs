//! `yardsort-host memory`, the running host's side: a project's notes, and the operator's own
//! decisions about them, which reach the account's other hosts when the project syncs. The rules
//! are `yardsort_core::memory`, the same as the desktop's Memory view. Every act is behind the
//! guard (`guard`), as `pair` is, so no agent approves its own proposal; only `list` is not.

use std::sync::Arc;

use remote_host::account::{self, Failure};
use yardsort_core::error::IpcError;
use yardsort_core::memory::{self, Decision, Settle};
use yardsort_core::store::MemoryRow;

use crate::control::{Note, Reply};
use crate::home::HostHome;

fn failed(error: IpcError) -> Failure {
    Failure::new("memory_refused", error.message)
}

fn store_failed(error: impl std::fmt::Display) -> Failure {
    Failure::new("remote_store", error.to_string())
}

/// A note here, by the start of its id, as `memory list` shows it.
fn entry(home: &HostHome, id: &str) -> account::Result<MemoryRow> {
    let wanted = id.to_lowercase();
    let mut found = Vec::new();
    for project in home.store.projects().map_err(store_failed)? {
        found.extend(
            home.store
                .memory_entries(&project.id)
                .map_err(store_failed)?
                .into_iter()
                .filter(|e| wanted.len() >= 4 && e.id.starts_with(&wanted)),
        );
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(Failure::new(
            "memory_refused",
            format!("There is no note {id:?} here. See `yardsort-host memory list <project>`."),
        )),
        _ => Err(Failure::new(
            "memory_refused",
            format!("More than one note starts with {id:?}. Give more of its id."),
        )),
    }
}

fn done(home: &HostHome, message: String) -> Reply {
    // The other hosts hear of it now, not at the next five minutes.
    remote_host::sync::wake(home);
    Reply::Done { message }
}

pub fn list(home: &Arc<HostHome>, project: &str) -> account::Result<Reply> {
    let project = crate::sync::project(home, project)?;
    let notes = home
        .store
        .memory_entries(&project.id)
        .map_err(store_failed)?
        .into_iter()
        .map(|e| Note {
            id: memory::short_id(&e.id).to_owned(),
            from: memory::citation(&e),
            conflict: e.conflict.map(|c| (c.text, c.from)),
            text: e.text,
            state: e.state,
        })
        .collect();
    Ok(Reply::Memory { notes })
}

pub fn write(home: &Arc<HostHome>, project: &str, text: &str) -> account::Result<Reply> {
    let project = crate::sync::project(home, project)?;
    let id = memory::write(&home.store, &project.id, text).map_err(failed)?;
    Ok(done(
        home,
        format!("Written and approved, as {}.", memory::short_id(&id)),
    ))
}

pub fn decide(home: &Arc<HostHome>, id: &str, decision: &str) -> account::Result<Reply> {
    let note = entry(home, id)?;
    let (decision, said) = match decision {
        "approve" => (Decision::Approve, "Approved"),
        "reject" => (Decision::Reject, "Rejected"),
        "revoke" => (Decision::Revoke, "Revoked"),
        "restore" => (Decision::Restore, "Restored"),
        _ => {
            return Err(Failure::new(
                "memory_refused",
                "Approve, reject, revoke or restore.",
            ))
        }
    };
    memory::decide(&home.store, &note.id, decision).map_err(failed)?;
    Ok(done(
        home,
        format!("{said} {}.", memory::short_id(&note.id)),
    ))
}

pub fn edit(home: &Arc<HostHome>, id: &str, text: &str) -> account::Result<Reply> {
    let note = entry(home, id)?;
    memory::edit(&home.store, &note.id, text).map_err(failed)?;
    Ok(done(
        home,
        format!("Rewrote {}.", memory::short_id(&note.id)),
    ))
}

pub fn settle(
    home: &Arc<HostHome>,
    id: &str,
    keep: &str,
    text: Option<String>,
) -> account::Result<Reply> {
    let note = entry(home, id)?;
    let how = match (keep, text) {
        ("standing", None) => Settle::Standing,
        ("other", None) => Settle::Other,
        ("text", Some(text)) => Settle::Text(text),
        _ => {
            return Err(Failure::new(
                "memory_refused",
                "Keep the text that stands, the other one, or write a third.",
            ))
        }
    };
    memory::settle(&home.store, &note.id, how).map_err(failed)?;
    Ok(done(
        home,
        format!(
            "Settled {}, here and on every computer it syncs with.",
            memory::short_id(&note.id)
        ),
    ))
}
