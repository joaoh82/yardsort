//! Sync between an account's hosts, this installation's rules: what a project's record carries,
//! the remote hint that goes with it, and how a project stands. The tables are `store::sync`; the
//! loop that pushes and pulls is `remote_host::sync`; the design is
//! [42](../../../docs/design/42-remote-r6.md).
//!
//! A project syncs only where the person turned it on or linked it, at that machine. What goes
//! up for it is its name and its fetch remote's address without any user or password, as a hint
//! for linking it elsewhere; never its path, and no hint at all for a remote on this disk.

use std::path::Path;

use remote_proto::sync::{Action, RecordKind, SyncRecord};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::git::Git;
use crate::store::{Store, StoreResult};

/// What a project's record says. Schema 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBody {
    pub name: String,
    /// The fetch remote's address, without credentials, as a hint for linking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
}

impl ProjectBody {
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("a project body is JSON")
    }

    /// What a synced project's record says, when it says it in a way this version reads.
    pub fn of(record: &SyncRecord) -> Option<Self> {
        serde_json::from_value(record.body.clone()?).ok()
    }
}

/// The hint for a project at `root`: its push remote's address, without credentials, or `None`
/// when it has none, or one on this disk.
pub fn project_hint(git: &Git, root: &Path) -> Option<String> {
    let remote = git.push_remote(root).ok()??;
    remote_hint(&git.remote_url(root, &remote).ok()??)
}

/// A remote's address with any user and password removed, or `None` for an address on this disk,
/// whose path is nobody else's business.
pub fn remote_hint(url: &str) -> Option<String> {
    let url = url.trim();
    if let Some((scheme, rest)) = url.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if !matches!(scheme.as_str(), "https" | "http" | "ssh" | "git") {
            return None;
        }
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        if host.is_empty() {
            return None;
        }
        return Some(format!("{scheme}://{host}/{path}"));
    }
    // `user@host:path`, git's short form for ssh. A path with a slash before any colon, or none,
    // is a directory, and so is a single letter before it: a Windows drive, as git reads it.
    let (before, path) = url.split_once(':')?;
    let drive = before.len() == 1 && before.chars().all(|c| c.is_ascii_alphabetic());
    if before.contains(['/', '\\']) || path.is_empty() || drive {
        return None;
    }
    let host = before.rsplit_once('@').map_or(before, |(_, host)| host);
    if host.is_empty() {
        return None;
    }
    Some(format!("{host}:{path}"))
}

/// Whether two hints name the same repository: host and path, ignoring the scheme, a port, a
/// trailing `.git` and case.
pub fn same_remote(a: &str, b: &str) -> bool {
    fn key(hint: &str) -> String {
        let rest = hint
            .split_once("://")
            .map_or_else(|| hint.replacen(':', "/", 1), |(_, rest)| rest.to_owned());
        let (host, path) = rest.split_once('/').unwrap_or((&rest, ""));
        let host = host.split_once(':').map_or(host, |(host, _)| host);
        let path = path.trim_end_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        format!("{}/{}", host, path).to_ascii_lowercase()
    }
    key(a) == key(b)
}

/// How a project stands for sync on this installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum ProjectSync {
    /// Remote is off here, so nothing syncs.
    RemoteOff,
    /// It does not sync. Turning it on sends what `preview` shows.
    Off { preview: SyncPreview },
    /// It syncs, or is on its way to.
    On {
        /// The server has its record.
        reached: bool,
        /// Being removed from the server.
        removing: bool,
        /// Why the server refused something of it, when it did.
        refused: Option<String>,
    },
}

/// What turning sync on for a project sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SyncPreview {
    pub name: String,
    pub remote: Option<String>,
}

/// How a project stands. `remote_on` is whether Remote is on here; `preview` is only read when
/// it does not sync.
pub fn project_sync(
    store: &Store,
    project_id: &str,
    remote_on: bool,
    preview: impl FnOnce() -> SyncPreview,
) -> StoreResult<ProjectSync> {
    if !remote_on {
        return Ok(ProjectSync::RemoteOff);
    }
    let Some(row) = store.synced_project(project_id)? else {
        return Ok(ProjectSync::Off { preview: preview() });
    };
    let held: Vec<_> = store
        .sync_refused()?
        .into_iter()
        .filter(|op| op.project == row.project)
        .collect();
    let removing = store.sync_outbox(usize::MAX)?.iter().any(|op| {
        op.project == row.project && op.kind == RecordKind::Project && op.action == Action::Delete
    }) || held
        .iter()
        .any(|op| op.kind == RecordKind::Project && op.action == Action::Delete);
    Ok(ProjectSync::On {
        reached: row.revision.is_some(),
        removing,
        refused: held.last().map(|op| op.why.clone()),
    })
}

#[cfg(test)]
mod tests;
