//! Sync between an account's hosts: pushing ops, pulling what changed, snapshots and links. See
//! [`remote_proto::sync`] for the messages and [42](../../../docs/design/42-remote-r6.md) for
//! the design.
//!
//! Only a host device with a host registered syncs. Each op is one step of the records
//! ([`Records::apply_sync_op`]): what it comes to is decided here, by [`decide`], the same for
//! every store. Every host linked to a project an op changed, other than the one that pushed it,
//! is told on its relay link ([`Signal::Synced`]).
//!
//! Nothing here logs a body, a name or a hint: a record's body is the host's own JSON, kept as
//! text and read by nothing on the server.

use std::collections::BTreeSet;
use std::sync::Weak;
use std::time::Duration;

use remote_proto::account::DeviceKind;
use remote_proto::sync::{
    Action, Cursor, OpResult, PushReply, PushRequest, RecordKind, Revision, SnapshotPage, SyncId,
    SyncOp, SyncPage, SyncProjects, SyncRecord, CHANGE_RETENTION_MS, MAX_PAGE, OP_RETENTION_MS,
};
use remote_proto::{ErrorCode, HostId, RemoteError};

use crate::http::now_ms;
use crate::records::{Records, Signal};
use crate::service::{unreadable, Caller, ControlService};

/// How often the pruner runs. Nothing depends on it running on time.
pub const PRUNE_EVERY: Duration = Duration::from_secs(60 * 60);
/// Rows one prune statement deletes at most, so no statement holds locks for long.
pub const PRUNE_BATCH: usize = 1000;

/// Where an op's project stands, for the host that sent it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectState {
    /// No project record by that id.
    Missing,
    /// Deleted, or not; and whether this host is linked to it.
    Present { deleted: bool, linked: bool },
}

/// What an op comes to, before anything is written.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Write the record at this new revision. The store gives it its cursor.
    Write(Write),
    /// Applied already: a revoke of a revoked note, a delete of a deleted record. The op is
    /// remembered all the same, so a retry after a later restore does not revoke again.
    Unchanged {
        revision: Revision,
        cursor: Cursor,
    },
    Conflict(SyncRecord),
    Refused(RemoteError),
}

/// A record's new revision, as an op makes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Write {
    pub revision: Revision,
    pub schema: u16,
    pub body: Option<serde_json::Value>,
    pub revoked: bool,
    pub deleted: bool,
    /// What the change log says it was: `created`, `updated`, `restored`, `revoked` or `deleted`.
    pub outcome: &'static str,
    /// A project deleted: every record of it is deleted too.
    pub cascade: bool,
}

/// What `op` comes to against the record as it is and its project's state for this host.
pub fn decide(op: &SyncOp, current: Option<&SyncRecord>, project: ProjectState) -> Verdict {
    if let Err(why) = op.check() {
        return Verdict::Refused(RemoteError::new(ErrorCode::InvalidRequest, why));
    }
    if let Some(current) = current {
        if current.kind != op.kind || current.project != op.project {
            return Verdict::Refused(RemoteError::new(
                ErrorCode::InvalidRequest,
                "That id is another record's.",
            ));
        }
    }
    let creates_project = op.kind == RecordKind::Project && current.is_none();
    if !creates_project {
        match project {
            ProjectState::Missing => return Verdict::Refused(RemoteError::not_found()),
            // A record of a deleted project is a tombstone already, or about to be one.
            ProjectState::Present { deleted: true, .. } if op.kind != RecordKind::Project => {
                return Verdict::Refused(RemoteError::new(
                    ErrorCode::NotFound,
                    "That project was removed from the server.",
                ))
            }
            // The project's own tombstone answers, below, even once its links are pruned.
            ProjectState::Present { deleted: true, .. } => {}
            ProjectState::Present { linked: false, .. } => {
                return Verdict::Refused(RemoteError::new(
                    ErrorCode::Forbidden,
                    "This computer is not linked to that project.",
                ))
            }
            ProjectState::Present { .. } => {}
        }
    }
    let Some(current) = current else {
        return match (op.action, op.base) {
            (Action::Put, None) => Verdict::Write(Write {
                revision: 1,
                schema: op.schema.unwrap_or_default(),
                body: op.body.clone(),
                revoked: false,
                deleted: false,
                outcome: "created",
                cascade: false,
            }),
            _ => Verdict::Refused(RemoteError::not_found()),
        };
    };
    let next = current.revision + 1;
    match op.action {
        Action::Put if current.deleted || op.base != Some(current.revision) => {
            Verdict::Conflict(current.clone())
        }
        Action::Put => Verdict::Write(Write {
            revision: next,
            schema: op.schema.unwrap_or_default(),
            body: op.body.clone(),
            revoked: false,
            deleted: false,
            outcome: if current.revoked {
                "restored"
            } else {
                "updated"
            },
            cascade: false,
        }),
        Action::Revoke if current.deleted => Verdict::Conflict(current.clone()),
        Action::Revoke if current.revoked => Verdict::Unchanged {
            revision: current.revision,
            cursor: current.cursor,
        },
        // Over any base: a stale copy never keeps a note the person revoked elsewhere.
        Action::Revoke => Verdict::Write(Write {
            revision: next,
            schema: current.schema,
            body: current.body.clone(),
            revoked: true,
            deleted: false,
            outcome: "revoked",
            cascade: false,
        }),
        Action::Delete if current.deleted => Verdict::Unchanged {
            revision: current.revision,
            cursor: current.cursor,
        },
        Action::Delete if op.base != Some(current.revision) => Verdict::Conflict(current.clone()),
        Action::Delete => Verdict::Write(Write {
            revision: next,
            schema: current.schema,
            body: None,
            revoked: false,
            deleted: true,
            outcome: "deleted",
            cascade: op.kind == RecordKind::Project,
        }),
        Action::Unknown => Verdict::Refused(RemoteError::new(
            ErrorCode::InvalidRequest,
            "That is not an op Yardsort syncs.",
        )),
    }
}

/// What one op came to in the records, and who to tell.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub result: OpResult,
    /// The hosts linked to the op's project when it was written. Empty when nothing was.
    pub linked: Vec<HostId>,
}

/// A page of a pull, or `Expired` when the cursor is below the floor or past the head.
#[derive(Debug, Clone, PartialEq)]
pub enum Pulled {
    Page {
        records: Vec<SyncRecord>,
        more: bool,
    },
    Expired,
}

/// A page of a snapshot: the records, the head when it was read, and whether more follow.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub records: Vec<SyncRecord>,
    pub head: Cursor,
    pub more: bool,
}

/// The kind as it is stored.
pub fn kind_spelled(kind: RecordKind) -> &'static str {
    match kind {
        RecordKind::Project => "project",
        RecordKind::Note => "note",
        RecordKind::Workspace => "workspace",
        RecordKind::Unknown => "unknown",
    }
}

pub fn kind_of(text: &str) -> RecordKind {
    match text {
        "project" => RecordKind::Project,
        "note" => RecordKind::Note,
        "workspace" => RecordKind::Workspace,
        _ => RecordKind::Unknown,
    }
}

impl<R: Records> ControlService<R> {
    /// The host the caller syncs as: a host device, with a host registered.
    async fn sync_host(&self, caller: &Caller) -> Result<HostId, RemoteError> {
        if caller.kind != DeviceKind::Host {
            return Err(RemoteError::new(
                ErrorCode::Forbidden,
                "Only a computer that runs agents syncs.",
            ));
        }
        self.records
            .hosts(&caller.account)
            .await
            .map_err(unreadable)?
            .into_iter()
            .find(|host| host.device == caller.device)
            .map(|host| host.id)
            .ok_or_else(|| {
                RemoteError::new(
                    ErrorCode::Forbidden,
                    "Turn on Remote on this computer before syncing.",
                )
            })
    }

    pub(crate) async fn sync_push(
        &self,
        caller: &Caller,
        request: PushRequest,
    ) -> Result<PushReply, RemoteError> {
        let host = self.sync_host(caller).await?;
        request
            .check()
            .map_err(|why| RemoteError::new(ErrorCode::InvalidRequest, why))?;
        let mut results = Vec::with_capacity(request.ops.len());
        let mut tell = BTreeSet::new();
        for op in &request.ops {
            let applied = self
                .records
                .apply_sync_op(&caller.account, &host, op, now_ms())
                .await
                .map_err(unreadable)?;
            tell.extend(applied.linked.into_iter().filter(|linked| *linked != host));
            results.push(applied.result);
        }
        for linked in tell {
            if let Err(error) = self.records.signal(Signal::Synced(linked)).await {
                tracing::error!("sync: records: {error}");
            }
        }
        Ok(PushReply { results })
    }

    pub(crate) async fn sync_pull(
        &self,
        caller: &Caller,
        after: Cursor,
    ) -> Result<SyncPage, RemoteError> {
        let host = self.sync_host(caller).await?;
        match self
            .records
            .sync_pull(&caller.account, &host, after, MAX_PAGE)
            .await
            .map_err(unreadable)?
        {
            Pulled::Expired => Err(RemoteError::new(
                ErrorCode::CursorExpired,
                "This computer has been away too long to catch up. Take a snapshot.",
            )),
            Pulled::Page { records, more } => Ok(SyncPage {
                cursor: records.last().map_or(after, |record| record.cursor),
                records,
                more,
            }),
        }
    }

    pub(crate) async fn sync_snapshot(
        &self,
        caller: &Caller,
        project: Option<SyncId>,
        after: Option<SyncId>,
    ) -> Result<SnapshotPage, RemoteError> {
        let host = self.sync_host(caller).await?;
        let Some(snapshot) = self
            .records
            .sync_snapshot(
                &caller.account,
                &host,
                project.as_ref(),
                after.as_ref(),
                MAX_PAGE,
            )
            .await
            .map_err(unreadable)?
        else {
            return Err(RemoteError::not_found());
        };
        Ok(SnapshotPage {
            next: snapshot
                .more
                .then(|| snapshot.records.last().map(|record| record.id.clone()))
                .flatten(),
            records: snapshot.records,
            cursor: snapshot.head,
        })
    }

    pub(crate) async fn sync_projects(&self, caller: &Caller) -> Result<SyncProjects, RemoteError> {
        let host = self.sync_host(caller).await?;
        Ok(SyncProjects {
            projects: self
                .records
                .sync_projects(&caller.account, &host)
                .await
                .map_err(unreadable)?,
        })
    }

    pub(crate) async fn sync_link(
        &self,
        caller: &Caller,
        project: &SyncId,
        linked: bool,
    ) -> Result<(), RemoteError> {
        let host = self.sync_host(caller).await?;
        let found = self
            .records
            .sync_link(&caller.account, &host, project, linked, now_ms())
            .await
            .map_err(unreadable)?;
        if found {
            Ok(())
        } else {
            Err(RemoteError::not_found())
        }
    }
}

/// Prune the change log, tombstones and op ids past their retention, every [`PRUNE_EVERY`],
/// for as long as the service lives. Safe on several instances: each statement deletes a
/// bounded batch, and two instances deleting the same rows delete them once.
pub(crate) fn start_pruning<R: Records + 'static>(service: Weak<ControlService<R>>) {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(PRUNE_EVERY);
        loop {
            every.tick().await;
            let Some(service) = service.upgrade() else {
                return;
            };
            prune(service.records()).await;
        }
    });
}

/// One round of pruning, until a batch comes back short.
pub async fn prune<R: Records>(records: &R) {
    let now = now_ms();
    loop {
        match records
            .prune_sync(
                now - CHANGE_RETENTION_MS,
                now - OP_RETENTION_MS,
                PRUNE_BATCH,
            )
            .await
        {
            Ok(deleted) if deleted < PRUNE_BATCH => return,
            Ok(_) => {}
            Err(error) => {
                tracing::error!("sync: pruning: {error}");
                return;
            }
        }
    }
}
