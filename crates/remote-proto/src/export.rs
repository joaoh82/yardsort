//! Exporting an account: everything the server keeps of it, as a file its person keeps.
//!
//! ```text
//! device GET /v1/account/export                → ExportPage { account, records, next }
//! device GET /v1/account/export?after={next}   → ExportPage { records, next }
//! ```
//!
//! Any device the account has signed in may ask, for its own account and no other. The first page
//! carries the account and what hangs off it; every page carries up to [`EXPORT_PAGE`] synced
//! records, by id, their bodies about [`EXPORT_PAGE_BYTES`] at most, and `next` until the last. A client puts the pages together as one
//! [`AccountExport`].
//!
//! **What it holds** is what a person put there or what names them: the account's own row (the
//! identity provider's issuer and subject, never an email address), its devices, hosts and
//! grants, which phones have a push route (the platform, never the token), the notification
//! settings, and every synced record that still has a body. **What it never holds**: a credential
//! or a hash of one, a push token, a pairing code, tombstones and pruned markers (which hold no
//! content), the change log, and notices on their way to a phone, which lapse within days.
//!
//! A server that cannot export answers `not_found`, and does not list `account.export` in its
//! capabilities.

use serde::{Deserialize, Serialize};

use crate::account::DeviceSummary;
use crate::ids::{DeviceId, ServerId};
use crate::notify::NotificationPreferences;
use crate::pairing::{GrantSummary, HostSummary};
use crate::sync::{RecordKind, Revision, SyncId};
use crate::HostId;

/// The capability a server lists when it answers `GET /v1/account/export`.
pub const EXPORT_CAPABILITY: &str = "account.export";
/// The most synced records on one page.
pub const EXPORT_PAGE: usize = 200;
/// About the most their bodies come to on one page, as JSON, so a page stays well inside what a
/// client reads in one answer. A page always holds at least one record.
pub const EXPORT_PAGE_BYTES: usize = 512 * 1024;
/// The file's format. Raised only when a reader of the old one would misread the new.
pub const EXPORT_FORMAT: u32 = 1;

/// `GET /v1/account/export`, as query parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportQuery {
    /// The last page's `next`. Absent for the first page.
    #[serde(default)]
    pub after: Option<String>,
}

/// One page of an export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPage {
    /// On the first page only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<ExportedAccount>,
    pub records: Vec<ExportedRecord>,
    /// Ask again with this as `after`. Absent from the last page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<SyncId>,
}

/// The account, and what hangs off it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedAccount {
    pub id: String,
    /// The identity provider it signs in with, and its subject there.
    pub issuer: String,
    pub subject: String,
    pub created_at: i64,
    /// Signed out ones included, while the server keeps them. `current` is the one that asked.
    pub devices: Vec<DeviceSummary>,
    /// Removed ones are not kept.
    pub hosts: Vec<HostSummary>,
    /// Every grant asked for, newest first, whatever became of it.
    pub grants: Vec<GrantSummary>,
    pub push_routes: Vec<PushRoute>,
    pub notifications: NotificationPreferences,
}

/// A phone the server can send notices to: which, and through whom. Never the token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRoute {
    pub device: DeviceId,
    pub platform: String,
    pub updated_at: i64,
}

/// A synced record with a body, at its latest revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedRecord {
    pub kind: RecordKind,
    pub id: SyncId,
    pub project: SyncId,
    pub revision: Revision,
    pub schema: u16,
    pub body: serde_json::Value,
    /// A revoked note, which keeps its body.
    #[serde(default)]
    pub revoked: bool,
    /// The host whose op made this revision.
    pub host: HostId,
    pub changed_at: i64,
}

/// The file: every page of an export put together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountExport {
    pub format: u32,
    /// The server it came from: its id, and what its operator calls it.
    pub server: ServerId,
    pub server_name: String,
    pub exported_at: i64,
    pub account: ExportedAccount,
    pub records: Vec<ExportedRecord>,
}
