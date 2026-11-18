//! Exporting an account: see [`remote_proto::export`] for what it holds and what it never does.
//!
//! The caller's account and no other: every read here is by the account the caller's credential
//! belongs to, so no id a request names can reach another's.

use remote_proto::export::{
    ExportPage, ExportedAccount, ExportedRecord, ExportedUsageDay, EXPORT_PAGE, EXPORT_PAGE_BYTES,
};
use remote_proto::sync::{SyncId, SyncRecord};
use remote_proto::RemoteError;

use crate::pairing::{grant_summary, host_summary};
use crate::records::{GrantFilter, Records};
use crate::service::unreadable;
use crate::{Caller, ControlService};

impl<R: Records> ControlService<R> {
    /// `GET /v1/account/export`: the account and its first records, or, `after` the last page's
    /// last record, the next records.
    pub async fn export(
        &self,
        caller: &Caller,
        after: Option<SyncId>,
        now: i64,
    ) -> Result<ExportPage, RemoteError> {
        let account = match after {
            None => Some(self.exported_account(caller, now).await?),
            Some(_) => None,
        };
        let (mut records, mut more) = self
            .records
            .exported_records(&caller.account, after.as_ref(), EXPORT_PAGE)
            .await
            .map_err(unreadable)?;
        let fit = fitting(&records);
        if fit < records.len() {
            records.truncate(fit);
            more = true;
        }
        let next = more.then(|| records.last().map(|r| r.id.clone())).flatten();
        Ok(ExportPage {
            account,
            records: records.into_iter().map(exported).collect(),
            next,
        })
    }

    async fn exported_account(
        &self,
        caller: &Caller,
        now: i64,
    ) -> Result<ExportedAccount, RemoteError> {
        let row = self
            .records
            .account_row(&caller.account)
            .await
            .map_err(unreadable)?
            .ok_or_else(|| unreadable("a signed-in device's account is missing"))?;
        let mut hosts = self
            .records
            .hosts(&caller.account)
            .await
            .map_err(unreadable)?;
        hosts.sort_by_key(|h| std::cmp::Reverse(h.created_at));
        let grants = self
            .records
            .grants(&caller.account, GrantFilter::All)
            .await
            .map_err(unreadable)?;
        Ok(ExportedAccount {
            id: caller.account.0.clone(),
            issuer: row.issuer,
            subject: row.subject,
            created_at: row.created_at,
            devices: self.devices(caller).await?.devices,
            hosts: hosts.into_iter().map(|h| host_summary(h, caller)).collect(),
            grants: grants.into_iter().map(|g| grant_summary(g, now)).collect(),
            push_routes: self
                .records
                .push_routes(&caller.account)
                .await
                .map_err(unreadable)?,
            notifications: self
                .records
                .notification_preferences(&caller.account)
                .await
                .map_err(unreadable)?,
            usage: self
                .records
                .account_usage(&caller.account)
                .await
                .map_err(unreadable)?
                .into_iter()
                .map(|day| ExportedUsageDay {
                    date: crate::usage::date(day.day),
                    active_devices: day.active_devices,
                    peak_connections: day.peak_streams,
                    relay_bytes: day.relay_bytes,
                    sync_bytes: day.sync_bytes,
                })
                .collect(),
        })
    }
}

/// How many of `records` a page holds: as many as their bodies fit in [`EXPORT_PAGE_BYTES`], and
/// never none.
fn fitting(records: &[SyncRecord]) -> usize {
    let mut bytes = 0;
    for (n, record) in records.iter().enumerate() {
        bytes += record.body.as_ref().map_or(0, |b| b.to_string().len());
        if bytes > EXPORT_PAGE_BYTES {
            return n.max(1);
        }
    }
    records.len()
}

fn exported(record: SyncRecord) -> ExportedRecord {
    ExportedRecord {
        kind: record.kind,
        id: record.id,
        project: record.project,
        revision: record.revision,
        schema: record.schema,
        body: record.body.unwrap_or_default(),
        revoked: record.revoked,
        host: record.host,
        changed_at: record.changed_at,
    }
}
