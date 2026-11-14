//! Sync between hosts: the records two installations of the same person keep alike, and how a
//! host pushes its changes to the server and pulls the others'. See
//! [42](../../../docs/design/42-remote-r6.md) for what syncs and why.
//!
//! Only a host syncs: the desktop with Remote on, and `yardsort-host`. A phone or a computer
//! acting as a client is refused. A host syncs only the projects it is **linked** to, and the
//! server refuses it any other.
//!
//! ```text
//! host  POST /v1/sync/push                  PushRequest             → PushReply
//! host  GET  /v1/sync/pull?after=CURSOR                             → SyncPage
//! host  GET  /v1/sync/snapshot?project=&after=                      → SnapshotPage
//! host  GET  /v1/sync/projects                                      → SyncProjects
//! host  POST /v1/sync/projects/{id}/link    (no body)               → nothing
//! host  POST /v1/sync/projects/{id}/unlink  (no body)               → nothing
//! ```
//!
//! **Ids.** Every record has a logical id ([`SyncId`]), a UUID v4 made by the installation that
//! first synced it, kept beside its local id. The server never sees a local id. Every op has an
//! id of its own ([`OpId`]), so a push retried after its answer was lost is applied once.
//!
//! **Revisions, not clocks.** Every record has a revision, 1 when created and one more with each
//! change. An op names the revision it was made against ([`SyncOp::base`]). The server applies
//! one whose base is current and answers [`OpResult::Conflict`] with the current record for one
//! whose base is not. Times are shown, never compared. Two exceptions, both so that a stale copy
//! cannot undo what a person did: a **revoke** applies over any base, and a **deleted** record
//! keeps a tombstone with its last revision, so a stale create or edit is a conflict rather than
//! bringing it back.
//!
//! **What a record says** is its body: the fields its kind syncs, as JSON, at most
//! [`MAX_RECORD_BODY`] bytes. The server keeps a body as it came and reads nothing in it; each
//! kind's fields are defined by the slice that syncs that kind. A body has its own schema
//! version ([`SyncOp::schema`]), apart from the protocol's. A host that does not know a record's
//! schema leaves it on the server untouched.
//!
//! **Cursors.** Each change an account's records take is numbered, one after another, and every
//! record carries the number of the change that made its revision. A pull returns the records of
//! linked projects changed after the host's cursor, oldest change first. A cursor older than the
//! oldest tombstone the server still keeps, or newer than any change it made, is answered with
//! `cursorExpired`: take a snapshot. Tombstones are pruned after [`CHANGE_RETENTION_MS`], so a host
//! away longer than that takes one.
//!
//! **Snapshots** page through a project's records, tombstones included, by id. The first page's
//! cursor is where pulling carries on: read before the records were, so a change made while
//! paging comes again in the next pull, and applying a record twice at one revision changes
//! nothing.
//!
//! **The server's hint.** When a linked project changes, every other host linked to it is told on
//! its relay link, [`crate::relay::HostNotice::SyncChanged`], with nothing about what changed. The
//! host pulls, as it would anyway every few minutes.

use serde::{Deserialize, Serialize};

use crate::ids::HostId;
use crate::RemoteError;

/// The schema of the records this build writes.
pub const SYNC_SCHEMA: u16 = 1;
/// The most ops in one push.
pub const MAX_PUSH_OPS: usize = 100;
/// The largest push body, in bytes: [`MAX_PUSH_OPS`] small records, or a few large ones.
pub const MAX_SYNC_BODY: usize = 256 * 1024;
/// The largest record body, as JSON, in bytes.
pub const MAX_RECORD_BODY: usize = 16 * 1024;
/// The most records in one pull or snapshot page.
pub const MAX_PAGE: usize = 200;
/// How long the server keeps the change log and tombstones: 90 days.
pub const CHANGE_RETENTION_MS: i64 = 90 * 24 * 60 * 60_000;
/// How long the server remembers an op it applied, so a retry is answered as the first was: 30
/// days, the longest a host may stay offline with a push waiting.
pub const OP_RETENTION_MS: i64 = 30 * 24 * 60 * 60_000;

/// A change's number in its account's records. 0 is before any.
pub type Cursor = u64;
/// A record's revision: 1 when created.
pub type Revision = u64;

/// Why a string is not a sync id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a sync id is a lowercase UUID v4")]
pub struct NotASyncId;

/// Whether `text` is a lowercase, hyphenated UUID v4: `xxxxxxxx-xxxx-4xxx-[89ab]xxx-xxxxxxxxxxxx`.
fn uuid_v4(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(at, byte)| match at {
            8 | 13 | 18 | 23 => *byte == b'-',
            14 => *byte == b'4',
            19 => matches!(byte, b'8' | b'9' | b'a' | b'b'),
            _ => matches!(byte, b'0'..=b'9' | b'a'..=b'f'),
        })
}

macro_rules! uuid_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(text: impl Into<String>) -> Result<Self, NotASyncId> {
                let text = text.into();
                if uuid_v4(&text) {
                    Ok(Self(text))
                } else {
                    Err(NotASyncId)
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = NotASyncId;
            fn try_from(text: String) -> Result<Self, NotASyncId> {
                Self::new(text)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

uuid_id!(
    /// A synced record's logical id, the same on every installation. Kept beside the local id,
    /// never in its place.
    SyncId
);
uuid_id!(
    /// One op, so that a retried push is applied once.
    OpId
);

/// What a record is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordKind {
    /// A project: its name, and the hint offered when linking it elsewhere.
    Project,
    /// An approved or revoked memory note of a project.
    Note,
    /// A workspace of a project, on the host it lives on.
    Workspace,
    /// A kind from a newer peer. Never accepted in an op.
    #[serde(other)]
    Unknown,
}

/// A record as the server keeps it, at its latest revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRecord {
    pub kind: RecordKind,
    pub id: SyncId,
    /// The project it belongs to. A project's is its own id.
    pub project: SyncId,
    pub revision: Revision,
    /// Its body's schema.
    pub schema: u16,
    /// Absent from a tombstone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
    /// A revoked note. It keeps its body.
    #[serde(default, skip_serializing_if = "is_false")]
    pub revoked: bool,
    /// A tombstone: the record was deleted at this revision.
    #[serde(default, skip_serializing_if = "is_false")]
    pub deleted: bool,
    /// The host whose op made this revision.
    pub host: HostId,
    /// When the server applied that op. Shown, never compared.
    pub changed_at: i64,
    /// The change that made this revision.
    pub cursor: Cursor,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// What an op does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    /// Create a record (no base), or change one at its current revision. Changing a revoked note
    /// at its current revision restores it: the person's own act, as it is locally.
    Put,
    /// Revoke a note, over any base.
    Revoke,
    /// Delete a record at its current revision, leaving a tombstone. Deleting a project
    /// deletes every record of it, and its hosts stay linked until its tombstone is pruned, so
    /// each of them hears of it.
    Delete,
    #[serde(other)]
    Unknown,
}

/// One change a host made, to be applied on the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOp {
    pub op: OpId,
    pub action: Action,
    pub kind: RecordKind,
    pub id: SyncId,
    /// The project it belongs to. A project's is its own id.
    pub project: SyncId,
    /// The revision it was made against. Absent when it creates the record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Revision>,
    /// A put's body schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<u16>,
    /// A put's body: a JSON object of at most [`MAX_RECORD_BODY`] bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

impl SyncOp {
    /// What is wrong with it on its own, before any record is looked at. The message is safe to
    /// show and names nothing in the op.
    pub fn check(&self) -> Result<(), &'static str> {
        if self.kind == RecordKind::Unknown {
            return Err("That is not a kind of record Yardsort syncs.");
        }
        if (self.kind == RecordKind::Project) != (self.id == self.project) {
            return Err("A project is its own project, and only a project is.");
        }
        match self.action {
            Action::Put => {
                let (Some(schema), Some(body)) = (self.schema, self.body.as_ref()) else {
                    return Err("A put has a schema and a body.");
                };
                if schema == 0 {
                    return Err("A put has a schema and a body.");
                }
                if !body.is_object() {
                    return Err("A record's body is a JSON object.");
                }
                if serde_json::to_vec(body).map_or(usize::MAX, |b| b.len()) > MAX_RECORD_BODY {
                    return Err("That record is too large to sync.");
                }
            }
            Action::Revoke => {
                if self.kind != RecordKind::Note {
                    return Err("Only a note is revoked.");
                }
                if self.schema.is_some() || self.body.is_some() {
                    return Err("A revoke carries no body.");
                }
            }
            Action::Delete => {
                if self.base.is_none() {
                    return Err("A delete names the revision it was made against.");
                }
                if self.schema.is_some() || self.body.is_some() {
                    return Err("A delete carries no body.");
                }
            }
            Action::Unknown => return Err("That is not an op Yardsort syncs."),
        }
        Ok(())
    }
}

/// `POST /v1/sync/push`. Its ops are applied in order, each on its own: one refused or in
/// conflict does not stop the next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRequest {
    pub ops: Vec<SyncOp>,
}

impl PushRequest {
    /// Between one and [`MAX_PUSH_OPS`] ops, each op id once.
    pub fn check(&self) -> Result<(), &'static str> {
        if self.ops.is_empty() || self.ops.len() > MAX_PUSH_OPS {
            return Err("A push has between 1 and 100 ops.");
        }
        let mut seen = std::collections::HashSet::new();
        if !self.ops.iter().all(|op| seen.insert(&op.op)) {
            return Err("An op id is used once in a push.");
        }
        Ok(())
    }
}

/// The answer to a push: one result for each op, in the same order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushReply {
    pub results: Vec<OpResult>,
}

/// What became of one op.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum OpResult {
    /// Applied, now or by an earlier push of the same op: the revision and change it made.
    Applied {
        op: OpId,
        revision: Revision,
        cursor: Cursor,
    },
    /// Not applied: its base is not the record's revision, it creates a record that exists, it
    /// changes a deleted one, or it restores a revoked note from a stale copy. The record as it
    /// is. Nothing was written.
    Conflict { op: OpId, current: SyncRecord },
    /// Not applied, and trying again will not help: no such record, a project this host is not
    /// linked to, or an op that is wrong on its own. The op is held, not retried.
    Refused { op: OpId, error: RemoteError },
    /// An outcome from a newer server.
    #[serde(other)]
    Unknown,
}

/// `GET /v1/sync/pull?after=`: the linked projects' records changed after `after`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPage {
    pub records: Vec<SyncRecord>,
    /// Where the next pull starts: the last record's change, or `after` when there are none.
    pub cursor: Cursor,
    /// More changed after `cursor`: pull again at once.
    pub more: bool,
}

/// `GET /v1/sync/snapshot`: a page of the linked projects' records, or of one of them, by id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotPage {
    pub records: Vec<SyncRecord>,
    /// The latest change when the page was read. Pull after the first page's once the last page
    /// is in.
    pub cursor: Cursor,
    /// The id to ask for the next page after. Absent on the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<SyncId>,
}

/// `GET /v1/sync/projects`: the account's synced projects, deleted ones left out, so that
/// another host can link one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProjects {
    pub projects: Vec<SyncProject>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProject {
    pub record: SyncRecord,
    /// The asking host is linked to it.
    pub linked: bool,
}
