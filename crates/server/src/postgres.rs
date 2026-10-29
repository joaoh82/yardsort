//! The records in PostgreSQL: what a server that is run for real keeps.
//!
//! Each [`Records`] method is one statement or one transaction, so "one step" (see
//! [`crate::records`]) holds across every instance of the server sharing the database: a one-time
//! value is taken with `DELETE … RETURNING`, and a refresh credential is checked and replaced with
//! its row and its device's locked.
//!
//! **Signals** between instances ([`Signal`]) are PostgreSQL notifications on one channel
//! ([`SIGNAL_CHANNEL`]): sent on any pooled connection, and heard on one connection of each
//! instance's own, opened the first time something listens and opened again when it breaks, with a
//! [`Signal::RecheckAll`] each time it starts listening, for whatever went by meanwhile.
//!
//! The schema is in `migrations/`, applied in order when the server connects, under an advisory
//! lock so that two instances starting together do not both apply it. Migrations are append-only:
//! a shipped file is never edited, and a change is a new file.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod, Transaction};
use std::collections::BTreeSet;

use remote_proto::account::{DeviceKind, Secret};
use remote_proto::pairing::MAX_OFFER_MISSES;
use remote_proto::{DeviceId, GrantId, HostId, Platform, ProjectGrant, Scope, ServerId};
use tokio::sync::{broadcast, mpsc};
use tokio_postgres::{AsyncMessage, Client, Row};

use crate::records::{
    AccountId, Decision, Device, GrantFilter, GrantRecord, GrantStatus, GrantView, Handoff, Host,
    HostOffer, Issued, LinkPlace, PendingLogin, Records, RecordsError, Redemption, Result,
    RevokeReason, Revoked, Rotation, Signal, SEEN_RESOLUTION_MS, SIGNALS_KEPT,
};
use crate::tokens::TokenHash;

/// The schema, in the order it is applied. Append only.
const MIGRATIONS: &[(i32, &str, &str)] = &[
    (
        1,
        "accounts_and_devices",
        include_str!("../migrations/0001_accounts_and_devices.sql"),
    ),
    (
        2,
        "hosts_and_grants",
        include_str!("../migrations/0002_hosts_and_grants.sql"),
    ),
    (
        3,
        "relay_links",
        include_str!("../migrations/0003_relay_links.sql"),
    ),
];

/// Held while migrating, so instances starting together take turns. Any constant will do, as
/// long as nothing else in the database uses it: "ysrv".
const MIGRATION_LOCK: i64 = 0x7973_7276;

/// Connections kept open per instance.
const POOL_SIZE: usize = 8;

/// A device row, as every query that returns one selects it.
const DEVICE: &str = "d.id, d.account, d.name, d.kind, d.platform, d.created_at, \
                      d.last_seen_at, d.revoked_at, d.revoked_reason";

/// A host row, joined to its device for when that was last seen.
const HOST: &str = "h.id, h.account, h.device, h.name, h.created_at, hd.last_seen_at";

/// A grant row and its names, as [`GrantView`] holds them.
const GRANT: &str = "g.id, g.account, g.host, g.device, g.requested, g.scopes, g.projects, \
                     g.status, g.created_at, g.expires_at, g.decided_at, \
                     h.name, d.name, d.revoked_at IS NOT NULL";

/// The notification channel signals travel on.
pub const SIGNAL_CHANNEL: &str = "yardsort_relay";

/// The longest wait before listening again after the connection broke.
const MAX_LISTEN_BACKOFF: Duration = Duration::from_secs(30);

pub struct PgRecords {
    pool: Pool,
    /// For the listening connection, which is not one of the pool's.
    config: tokio_postgres::Config,
    signals: OnceLock<broadcast::Sender<Signal>>,
}

impl PgRecords {
    /// Connect to `url` (`postgres://user:password@host/database?sslmode=require`) and bring the
    /// schema up to date. TLS is used as `sslmode` says, checked against the public web roots.
    pub async fn connect(url: &str) -> Result<Self> {
        let config: tokio_postgres::Config = url
            .parse()
            .map_err(|e| RecordsError(format!("the database URL: {e}")))?;
        Self::connect_with(config).await
    }

    pub async fn connect_with(config: tokio_postgres::Config) -> Result<Self> {
        let manager = Manager::from_config(
            config.clone(),
            tls()?,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let pool = Pool::builder(manager)
            .max_size(POOL_SIZE)
            .build()
            .map_err(failed)?;
        let records = Self {
            pool,
            config,
            signals: OnceLock::new(),
        };
        let mut client = records.client().await?;
        migrate(&mut client).await?;
        Ok(records)
    }

    /// This server's id: made up the first time any instance starts, the same ever after.
    pub async fn server_id(&self, now: i64) -> Result<ServerId> {
        let client = self.client().await?;
        client
            .execute(
                "INSERT INTO server_identity (id, created_at) VALUES ($1, $2)
                 ON CONFLICT (singleton) DO NOTHING",
                &[&uuid::Uuid::new_v4().to_string(), &now],
            )
            .await
            .map_err(failed)?;
        let row = client
            .query_one("SELECT id FROM server_identity", &[])
            .await
            .map_err(failed)?;
        ServerId::new(row.get::<_, String>(0)).map_err(failed)
    }

    async fn client(&self) -> Result<deadpool_postgres::Object> {
        self.pool.get().await.map_err(failed)
    }
}

/// Hear the signals every instance sends, on a connection of this instance's own, until nobody
/// listens any more. Each time it starts listening it says [`Signal::RecheckAll`]: whatever went
/// by while it was not is checked that way.
async fn listen(config: tokio_postgres::Config, signals: broadcast::Sender<Signal>) {
    let mut backoff = Duration::from_secs(1);
    while signals.receiver_count() > 0 {
        match listen_once(&config, &signals).await {
            Ok(()) => backoff = Duration::from_secs(1),
            Err(error) => tracing::warn!("records: not listening for signals: {error}"),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_LISTEN_BACKOFF);
    }
}

/// One connection's worth of listening: until it breaks, or nobody listens.
async fn listen_once(
    config: &tokio_postgres::Config,
    signals: &broadcast::Sender<Signal>,
) -> Result<()> {
    let (client, mut connection) = config.connect(tls()?).await.map_err(failed)?;
    let (heard, mut hearing) = mpsc::unbounded_channel();
    // The connection is driven here, which is also where its notifications come out.
    let driving = tokio::spawn(async move {
        let mut messages = futures_util::stream::poll_fn(move |cx| connection.poll_message(cx));
        while let Some(Ok(message)) = futures_util::StreamExt::next(&mut messages).await {
            if let AsyncMessage::Notification(notification) = message {
                if heard.send(notification.payload().to_owned()).is_err() {
                    return;
                }
            }
        }
    });
    let listening = client
        .batch_execute(&format!("LISTEN {SIGNAL_CHANNEL}"))
        .await
        .map_err(failed);
    if listening.is_ok() {
        let _ = signals.send(Signal::RecheckAll);
        while let Some(payload) = hearing.recv().await {
            if signals.receiver_count() == 0 {
                break;
            }
            if let Some(signal) = Signal::decode(&payload) {
                let _ = signals.send(signal);
            }
        }
    }
    driving.abort();
    listening
}

fn tls() -> Result<tokio_postgres_rustls::MakeRustlsConnect> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(failed)?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}

async fn migrate(client: &mut Client) -> Result<()> {
    let tx = client.transaction().await.map_err(failed)?;
    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK])
        .await
        .map_err(failed)?;
    tx.batch_execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    INTEGER PRIMARY KEY,
             name       TEXT NOT NULL,
             applied_at BIGINT NOT NULL
         )",
    )
    .await
    .map_err(failed)?;
    let applied: Vec<i32> = tx
        .query("SELECT version FROM schema_migrations", &[])
        .await
        .map_err(failed)?
        .iter()
        .map(|row| row.get(0))
        .collect();
    // A database a newer server already migrated is left as it is: an older instance still
    // running through a rolling deploy reads only what it knows.
    for (version, name, sql) in MIGRATIONS {
        if applied.contains(version) {
            continue;
        }
        tx.batch_execute(sql)
            .await
            .map_err(|e| RecordsError(format!("migration {version} ({name}): {e}")))?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at)
             VALUES ($1, $2, (extract(epoch FROM now()) * 1000)::BIGINT)",
            &[version, name],
        )
        .await
        .map_err(failed)?;
    }
    tx.commit().await.map_err(failed)
}

impl Records for PgRecords {
    async fn account(&self, issuer: &str, subject: &str, now: i64) -> Result<AccountId> {
        // The no-op update makes `RETURNING` answer for an account that already existed.
        let row = self
            .client()
            .await?
            .query_one(
                "INSERT INTO accounts (id, issuer, subject, created_at) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (issuer, subject) DO UPDATE SET issuer = EXCLUDED.issuer
                 RETURNING id",
                &[&uuid::Uuid::new_v4().to_string(), &issuer, &subject, &now],
            )
            .await
            .map_err(failed)?;
        Ok(AccountId(row.get(0)))
    }

    async fn put_login(&self, login: PendingLogin, now: i64) -> Result<()> {
        self.client()
            .await?
            .execute(
                "WITH lapsed AS (DELETE FROM pending_logins WHERE expires_at <= $7)
                 INSERT INTO pending_logins
                     (state, nonce, provider_verifier, device_challenge, device_redirect,
                      expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6)",
                &[
                    &bytes(&login.state),
                    &bytes(&login.nonce),
                    &login.provider_verifier.expose(),
                    &login.device_challenge,
                    &login.device_redirect,
                    &login.expires_at,
                    &now,
                ],
            )
            .await
            .map_err(failed)?;
        Ok(())
    }

    async fn take_login(&self, state: &TokenHash) -> Result<Option<PendingLogin>> {
        let row = self
            .client()
            .await?
            .query_opt(
                "DELETE FROM pending_logins WHERE state = $1
                 RETURNING nonce, provider_verifier, device_challenge, device_redirect,
                           expires_at",
                &[&bytes(state)],
            )
            .await
            .map_err(failed)?;
        row.map(|row| {
            Ok(PendingLogin {
                state: *state,
                nonce: hash(&row, 0)?,
                provider_verifier: Secret::new(row.get::<_, String>(1)),
                device_challenge: row.get(2),
                device_redirect: row.get(3),
                expires_at: row.get(4),
            })
        })
        .transpose()
    }

    async fn put_handoff(&self, handoff: Handoff, now: i64) -> Result<()> {
        self.client()
            .await?
            .execute(
                "WITH lapsed AS (DELETE FROM handoffs WHERE expires_at <= $5)
                 INSERT INTO handoffs (code, account, device_challenge, expires_at)
                 VALUES ($1, $2, $3, $4)",
                &[
                    &bytes(&handoff.code),
                    &handoff.account.0,
                    &handoff.device_challenge,
                    &handoff.expires_at,
                    &now,
                ],
            )
            .await
            .map_err(failed)?;
        Ok(())
    }

    async fn take_handoff(&self, code: &TokenHash) -> Result<Option<Handoff>> {
        let row = self
            .client()
            .await?
            .query_opt(
                "DELETE FROM handoffs WHERE code = $1
                 RETURNING account, device_challenge, expires_at",
                &[&bytes(code)],
            )
            .await
            .map_err(failed)?;
        Ok(row.map(|row| Handoff {
            code: *code,
            account: AccountId(row.get(0)),
            device_challenge: row.get(1),
            expires_at: row.get(2),
        }))
    }

    async fn add_device(&self, device: Device, issued: Issued) -> Result<()> {
        let mut client = self.client().await?;
        let tx = client.transaction().await.map_err(failed)?;
        tx.execute(
            "INSERT INTO devices (id, account, name, kind, platform, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &device.id.as_str(),
                &device.account.0,
                &device.name,
                &spelled(&device.kind)?,
                &spelled(&device.platform)?,
                &device.created_at,
            ],
        )
        .await
        .map_err(failed)?;
        issue(&tx, &device.id, &issued).await?;
        tx.commit().await.map_err(failed)
    }

    async fn access(&self, access: &TokenHash) -> Result<Option<(Device, i64)>> {
        let row = self
            .client()
            .await?
            .query_opt(
                &format!(
                    "SELECT {DEVICE}, a.expires_at FROM access_credentials a
                     JOIN devices d ON d.id = a.device WHERE a.hash = $1"
                ),
                &[&bytes(access)],
            )
            .await
            .map_err(failed)?;
        row.map(|row| Ok((device(&row)?, row.get(9)))).transpose()
    }

    async fn rotate(&self, refresh: &TokenHash, issued: Issued, now: i64) -> Result<Rotation> {
        let mut client = self.client().await?;
        let tx = client.transaction().await.map_err(failed)?;
        // Both rows locked: a second rotation with the same credential waits here, then finds
        // it spent; a revocation waits, or is waited for.
        let Some(row) = tx
            .query_opt(
                &format!(
                    "SELECT {DEVICE}, r.expires_at, r.spent_at FROM refresh_credentials r
                     JOIN devices d ON d.id = r.device WHERE r.hash = $1
                     FOR UPDATE OF r, d"
                ),
                &[&bytes(refresh)],
            )
            .await
            .map_err(failed)?
        else {
            return Ok(Rotation::Unknown);
        };
        let device = device(&row)?;
        let expires_at: i64 = row.get(9);
        let spent_at: Option<i64> = row.get(10);
        let outcome = if device.revoked.is_some() {
            Rotation::Revoked
        } else if spent_at.is_some() {
            tx.execute(
                "UPDATE devices SET revoked_at = $2, revoked_reason = $3 WHERE id = $1",
                &[
                    &device.id.as_str(),
                    &now,
                    &reason_spelled(RevokeReason::RefreshReused),
                ],
            )
            .await
            .map_err(failed)?;
            Rotation::Reused(device.account.clone())
        } else if expires_at <= now {
            Rotation::Expired
        } else {
            tx.execute(
                "UPDATE refresh_credentials SET spent_at = $2 WHERE hash = $1",
                &[&bytes(refresh), &now],
            )
            .await
            .map_err(failed)?;
            issue(&tx, &device.id, &issued).await?;
            // What can no longer matter: lapsed access, and spent refresh credentials past the
            // time they would have lapsed anyway, so presenting one is plain `Expired`.
            tx.execute(
                "DELETE FROM access_credentials WHERE device = $1 AND expires_at <= $2",
                &[&device.id.as_str(), &now],
            )
            .await
            .map_err(failed)?;
            tx.execute(
                "DELETE FROM refresh_credentials
                 WHERE device = $1 AND spent_at IS NOT NULL AND expires_at <= $2",
                &[&device.id.as_str(), &now],
            )
            .await
            .map_err(failed)?;
            Rotation::Rotated(device)
        };
        tx.commit().await.map_err(failed)?;
        Ok(outcome)
    }

    async fn seen(&self, device: &DeviceId, now: i64) -> Result<()> {
        self.client()
            .await?
            .execute(
                "UPDATE devices SET last_seen_at = $2
                 WHERE id = $1
                   AND (last_seen_at IS NULL OR last_seen_at <= $2::BIGINT - $3::BIGINT)",
                &[&device.as_str(), &now, &SEEN_RESOLUTION_MS],
            )
            .await
            .map_err(failed)?;
        Ok(())
    }

    async fn devices(&self, account: &AccountId) -> Result<Vec<Device>> {
        self.client()
            .await?
            .query(
                &format!("SELECT {DEVICE} FROM devices d WHERE d.account = $1"),
                &[&account.0],
            )
            .await
            .map_err(failed)?
            .iter()
            .map(device)
            .collect()
    }

    async fn signed_in_devices(&self, account: &AccountId, now: i64) -> Result<usize> {
        let row = self
            .client()
            .await?
            .query_one(
                "SELECT count(*) FROM devices d
                 WHERE d.account = $1 AND d.revoked_at IS NULL AND EXISTS (
                     SELECT 1 FROM refresh_credentials r
                     WHERE r.device = d.id AND r.spent_at IS NULL AND r.expires_at > $2
                 )",
                &[&account.0, &now],
            )
            .await
            .map_err(failed)?;
        let count: i64 = row.get(0);
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    async fn revoke(
        &self,
        account: &AccountId,
        device: &DeviceId,
        revoked: Revoked,
    ) -> Result<bool> {
        // An earlier revocation, and its reason, stand.
        let changed = self
            .client()
            .await?
            .execute(
                "UPDATE devices SET revoked_at = COALESCE(revoked_at, $3),
                                    revoked_reason = COALESCE(revoked_reason, $4)
                 WHERE id = $1 AND account = $2",
                &[
                    &device.as_str(),
                    &account.0,
                    &revoked.at,
                    &reason_spelled(revoked.reason),
                ],
            )
            .await
            .map_err(failed)?;
        Ok(changed > 0)
    }

    async fn revoke_account(
        &self,
        issuer: &str,
        subject: &str,
        revoked: Revoked,
    ) -> Result<Option<usize>> {
        let row = self
            .client()
            .await?
            .query_one(
                "WITH account AS (
                     SELECT id FROM accounts WHERE issuer = $1 AND subject = $2
                 ), revoked AS (
                     UPDATE devices SET revoked_at = $3, revoked_reason = $4
                     WHERE account IN (SELECT id FROM account) AND revoked_at IS NULL
                     RETURNING 1
                 )
                 SELECT (SELECT count(*) FROM account), (SELECT count(*) FROM revoked)",
                &[
                    &issuer,
                    &subject,
                    &revoked.at,
                    &reason_spelled(revoked.reason),
                ],
            )
            .await
            .map_err(failed)?;
        let (accounts, devices): (i64, i64) = (row.get(0), row.get(1));
        Ok((accounts > 0).then(|| usize::try_from(devices).unwrap_or(usize::MAX)))
    }

    async fn add_host(&self, host: Host) -> Result<()> {
        self.client()
            .await?
            .execute(
                "INSERT INTO hosts (id, account, device, name, created_at)
                 VALUES ($1, $2, $3, $4, $5)",
                &[
                    &host.id.as_str(),
                    &host.account.0,
                    &host.device.as_str(),
                    &host.name,
                    &host.created_at,
                ],
            )
            .await
            .map_err(failed)?;
        Ok(())
    }

    async fn rebind_host(
        &self,
        account: &AccountId,
        id: &HostId,
        device: &DeviceId,
        name: &str,
    ) -> Result<Option<Host>> {
        let row = self
            .client()
            .await?
            .query_opt(
                &format!(
                    "WITH h AS (
                         UPDATE hosts SET device = $3, name = $4
                         WHERE id = $1 AND account = $2 AND removed_at IS NULL
                         RETURNING *
                     )
                     SELECT {HOST} FROM h JOIN devices hd ON hd.id = h.device"
                ),
                &[&id.as_str(), &account.0, &device.as_str(), &name],
            )
            .await
            .map_err(failed)?;
        row.map(|row| host(&row)).transpose()
    }

    async fn host(&self, account: &AccountId, id: &HostId) -> Result<Option<Host>> {
        let row = self
            .client()
            .await?
            .query_opt(
                &format!(
                    "SELECT {HOST} FROM hosts h JOIN devices hd ON hd.id = h.device
                     WHERE h.id = $1 AND h.account = $2 AND h.removed_at IS NULL"
                ),
                &[&id.as_str(), &account.0],
            )
            .await
            .map_err(failed)?;
        row.map(|row| host(&row)).transpose()
    }

    async fn hosts(&self, account: &AccountId) -> Result<Vec<Host>> {
        self.client()
            .await?
            .query(
                &format!(
                    "SELECT {HOST} FROM hosts h JOIN devices hd ON hd.id = h.device
                     WHERE h.account = $1 AND h.removed_at IS NULL"
                ),
                &[&account.0],
            )
            .await
            .map_err(failed)?
            .iter()
            .map(host)
            .collect()
    }

    async fn put_offer(&self, offer: HostOffer, now: i64) -> Result<()> {
        let mut client = self.client().await?;
        let tx = client.transaction().await.map_err(failed)?;
        tx.execute(
            "DELETE FROM host_offers WHERE host = $1 OR expires_at <= $2",
            &[&offer.host.as_str(), &now],
        )
        .await
        .map_err(failed)?;
        tx.execute(
            "INSERT INTO host_offers (secret, code, host, account, expires_at)
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &bytes(&offer.secret),
                &bytes(&offer.code),
                &offer.host.as_str(),
                &offer.account.0,
                &offer.expires_at,
            ],
        )
        .await
        .map_err(failed)?;
        tx.commit().await.map_err(failed)
    }

    async fn redeem_offer(
        &self,
        account: &AccountId,
        key: &TokenHash,
        now: i64,
    ) -> Result<Redemption> {
        let mut client = self.client().await?;
        let tx = client.transaction().await.map_err(failed)?;
        let taken = tx
            .query_opt(
                "DELETE FROM host_offers
                 WHERE account = $1 AND (secret = $2 OR code = $2)
                   AND expires_at > $3 AND misses < $4
                 RETURNING secret, code, host, expires_at",
                &[&account.0, &bytes(key), &now, &MAX_OFFER_MISSES],
            )
            .await
            .map_err(failed)?;
        let redemption = match taken {
            Some(row) => Redemption::Taken(HostOffer {
                secret: hash(&row, 0)?,
                code: hash(&row, 1)?,
                host: HostId::new(row.get::<_, String>(2)).map_err(failed)?,
                account: account.clone(),
                expires_at: row.get(3),
            }),
            None => {
                tx.execute(
                    "UPDATE host_offers SET misses = misses + 1
                     WHERE account = $1 AND expires_at > $2 AND misses < $3",
                    &[&account.0, &now, &MAX_OFFER_MISSES],
                )
                .await
                .map_err(failed)?;
                Redemption::Missed
            }
        };
        tx.commit().await.map_err(failed)?;
        Ok(redemption)
    }

    async fn add_grant(&self, grant: GrantRecord) -> Result<()> {
        self.client()
            .await?
            .execute(
                "INSERT INTO grants (id, account, host, device, requested, scopes, projects,
                                     status, created_at, expires_at, decided_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
                &[
                    &grant.id.as_str(),
                    &grant.account.0,
                    &grant.host.as_str(),
                    &grant.device.as_str(),
                    &scopes_spelled(&grant.requested)?,
                    &scopes_spelled(&grant.scopes)?,
                    &projects_spelled(grant.projects.as_ref())?,
                    &status_spelled(grant.status),
                    &grant.created_at,
                    &grant.expires_at,
                    &grant.decided_at,
                ],
            )
            .await
            .map_err(failed)?;
        Ok(())
    }

    async fn grants(&self, account: &AccountId, filter: GrantFilter) -> Result<Vec<GrantView>> {
        let (column, id) = match &filter {
            GrantFilter::One(id) => ("g.id", id.as_str()),
            GrantFilter::OfHost(host) => ("g.host", host.as_str()),
            GrantFilter::ForDevice(device) => ("g.device", device.as_str()),
        };
        self.client()
            .await?
            .query(
                &format!(
                    "SELECT {GRANT} FROM grants g
                     JOIN hosts h ON h.id = g.host JOIN devices d ON d.id = g.device
                     WHERE g.account = $1 AND {column} = $2
                     ORDER BY g.created_at DESC"
                ),
                &[&account.0, &id],
            )
            .await
            .map_err(failed)?
            .iter()
            .map(grant_view)
            .collect()
    }

    async fn decide(
        &self,
        account: &AccountId,
        id: &GrantId,
        decision: Decision,
        now: i64,
    ) -> Result<Option<GrantRecord>> {
        let mut client = self.client().await?;
        let tx = client.transaction().await.map_err(failed)?;
        let (status, scopes, projects) = match &decision {
            Decision::Approve { scopes, projects } => {
                (GrantStatus::Approved, scopes.clone(), Some(projects))
            }
            Decision::Deny => (GrantStatus::Denied, BTreeSet::new(), None),
        };
        // Answers on one host take turns, so two approvals for the same device cannot both stand.
        // The grant's row is then locked by the update: a revocation or another answer to it
        // waits, and finds it no longer pending.
        tx.execute(
            "SELECT 1 FROM hosts WHERE id = (SELECT host FROM grants WHERE id = $1) FOR UPDATE",
            &[&id.as_str()],
        )
        .await
        .map_err(failed)?;
        let Some(row) = tx
            .query_opt(
                &format!(
                    "WITH g AS (
                         UPDATE grants SET status = $3, scopes = $4, projects = $5,
                                           decided_at = $6
                         WHERE id = $1 AND account = $2 AND status = 'pending'
                           AND expires_at > $6
                         RETURNING *
                     )
                     SELECT {GRANT} FROM g
                     JOIN hosts h ON h.id = g.host JOIN devices d ON d.id = g.device"
                ),
                &[
                    &id.as_str(),
                    &account.0,
                    &status_spelled(status),
                    &scopes_spelled(&scopes)?,
                    &projects_spelled(projects)?,
                    &now,
                ],
            )
            .await
            .map_err(failed)?
        else {
            return Ok(None);
        };
        let grant = grant_view(&row)?.grant;
        if status == GrantStatus::Approved {
            tx.execute(
                "UPDATE grants SET status = 'revoked', decided_at = $4
                 WHERE host = $1 AND device = $2 AND status = 'approved' AND id <> $3",
                &[
                    &grant.host.as_str(),
                    &grant.device.as_str(),
                    &id.as_str(),
                    &now,
                ],
            )
            .await
            .map_err(failed)?;
        }
        tx.commit().await.map_err(failed)?;
        Ok(Some(grant))
    }

    async fn revoke_grant(&self, account: &AccountId, id: &GrantId, now: i64) -> Result<bool> {
        let row = self
            .client()
            .await?
            .query_one(
                "WITH revoked AS (
                     UPDATE grants SET status = 'revoked', decided_at = $3
                     WHERE id = $1 AND account = $2 AND status IN ('pending', 'approved')
                 )
                 SELECT count(*) FROM grants WHERE id = $1 AND account = $2",
                &[&id.as_str(), &account.0, &now],
            )
            .await
            .map_err(failed)?;
        Ok(row.get::<_, i64>(0) > 0)
    }

    async fn remove_host(&self, account: &AccountId, id: &HostId, now: i64) -> Result<bool> {
        let mut client = self.client().await?;
        let tx = client.transaction().await.map_err(failed)?;
        let removed = tx
            .execute(
                "UPDATE hosts SET removed_at = $3
                 WHERE id = $1 AND account = $2 AND removed_at IS NULL",
                &[&id.as_str(), &account.0, &now],
            )
            .await
            .map_err(failed)?;
        if removed > 0 {
            tx.execute("DELETE FROM host_offers WHERE host = $1", &[&id.as_str()])
                .await
                .map_err(failed)?;
            tx.execute(
                "UPDATE grants SET status = 'revoked', decided_at = $2
                 WHERE host = $1 AND status IN ('pending', 'approved')",
                &[&id.as_str(), &now],
            )
            .await
            .map_err(failed)?;
        }
        tx.commit().await.map_err(failed)?;
        Ok(removed > 0)
    }

    async fn put_link(&self, place: LinkPlace) -> Result<()> {
        self.client()
            .await?
            .execute(
                "INSERT INTO relay_links (host, instance, link, seen_at) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (host) DO UPDATE
                 SET instance = EXCLUDED.instance, link = EXCLUDED.link,
                     seen_at = EXCLUDED.seen_at",
                &[
                    &place.host.as_str(),
                    &place.instance,
                    &place.link,
                    &place.seen_at,
                ],
            )
            .await
            .map_err(failed)?;
        Ok(())
    }

    async fn link_place(&self, host: &HostId) -> Result<Option<LinkPlace>> {
        let row = self
            .client()
            .await?
            .query_opt(
                "SELECT host, instance, link, seen_at FROM relay_links WHERE host = $1",
                &[&host.as_str()],
            )
            .await
            .map_err(failed)?;
        row.map(|row| link_place(&row)).transpose()
    }

    async fn keep_links(&self, instance: &str, now: i64) -> Result<Vec<LinkPlace>> {
        self.client()
            .await?
            .query(
                "UPDATE relay_links SET seen_at = $2 WHERE instance = $1
                 RETURNING host, instance, link, seen_at",
                &[&instance, &now],
            )
            .await
            .map_err(failed)?
            .iter()
            .map(link_place)
            .collect()
    }

    async fn signal(&self, signal: Signal) -> Result<()> {
        self.client()
            .await?
            .execute(
                "SELECT pg_notify($1, $2)",
                &[&SIGNAL_CHANNEL, &signal.encode()],
            )
            .await
            .map_err(failed)?;
        Ok(())
    }

    fn signals(&self) -> broadcast::Receiver<Signal> {
        // The first listener exists before the task that serves it starts, or the task could
        // find nobody listening and stop at once.
        let mut first = None;
        let sender = self.signals.get_or_init(|| {
            let (sender, receiver) = broadcast::channel(SIGNALS_KEPT);
            first = Some(receiver);
            tokio::spawn(listen(self.config.clone(), sender.clone()));
            sender
        });
        first.unwrap_or_else(|| sender.subscribe())
    }

    async fn pending_on_links(&self, instance: &str, now: i64) -> Result<Vec<(HostId, GrantId)>> {
        self.client()
            .await?
            .query(
                "SELECT g.host, g.id FROM grants g JOIN relay_links l ON l.host = g.host
                 WHERE l.instance = $1 AND g.status = 'pending' AND g.expires_at > $2",
                &[&instance, &now],
            )
            .await
            .map_err(failed)?
            .iter()
            .map(|row| {
                Ok((
                    HostId::new(row.get::<_, String>(0)).map_err(failed)?,
                    GrantId::new(row.get::<_, String>(1)).map_err(failed)?,
                ))
            })
            .collect()
    }

    async fn drop_link(&self, host: &HostId, link: &str) -> Result<()> {
        self.client()
            .await?
            .execute(
                "DELETE FROM relay_links WHERE host = $1 AND link = $2",
                &[&host.as_str(), &link],
            )
            .await
            .map_err(failed)?;
        Ok(())
    }
}

fn link_place(row: &Row) -> Result<LinkPlace> {
    Ok(LinkPlace {
        host: HostId::new(row.get::<_, String>(0)).map_err(failed)?,
        instance: row.get(1),
        link: row.get(2),
        seen_at: row.get(3),
    })
}

fn host(row: &Row) -> Result<Host> {
    Ok(Host {
        id: HostId::new(row.get::<_, String>(0)).map_err(failed)?,
        account: AccountId(row.get(1)),
        device: DeviceId::new(row.get::<_, String>(2)).map_err(failed)?,
        name: row.get(3),
        created_at: row.get(4),
        last_seen_at: row.get(5),
    })
}

fn grant_view(row: &Row) -> Result<GrantView> {
    let scopes = |at: usize| -> Result<BTreeSet<Scope>> {
        row.get::<_, Vec<String>>(at)
            .into_iter()
            .map(unspelled::<Scope>)
            .collect()
    };
    let projects: Option<String> = row.get(6);
    let status: String = row.get(7);
    Ok(GrantView {
        grant: GrantRecord {
            id: GrantId::new(row.get::<_, String>(0)).map_err(failed)?,
            account: AccountId(row.get(1)),
            host: HostId::new(row.get::<_, String>(2)).map_err(failed)?,
            device: DeviceId::new(row.get::<_, String>(3)).map_err(failed)?,
            requested: scopes(4)?,
            scopes: scopes(5)?,
            projects: projects
                .map(|text| serde_json::from_str::<ProjectGrant>(&text).map_err(failed))
                .transpose()?,
            status: match status.as_str() {
                "pending" => GrantStatus::Pending,
                "approved" => GrantStatus::Approved,
                "denied" => GrantStatus::Denied,
                "revoked" => GrantStatus::Revoked,
                other => return Err(RecordsError(format!("a grant is {other:?}"))),
            },
            created_at: row.get(8),
            expires_at: row.get(9),
            decided_at: row.get(10),
        },
        host_name: row.get(11),
        device_name: row.get(12),
        device_revoked: row.get(13),
    })
}

fn status_spelled(status: GrantStatus) -> &'static str {
    match status {
        GrantStatus::Pending => "pending",
        GrantStatus::Approved => "approved",
        GrantStatus::Denied => "denied",
        GrantStatus::Revoked => "revoked",
    }
}

fn scopes_spelled(scopes: &BTreeSet<Scope>) -> Result<Vec<String>> {
    scopes.iter().map(spelled).collect()
}

fn projects_spelled(projects: Option<&ProjectGrant>) -> Result<Option<String>> {
    projects
        .map(|p| serde_json::to_string(p).map_err(failed))
        .transpose()
}

async fn issue(tx: &Transaction<'_>, device: &DeviceId, issued: &Issued) -> Result<()> {
    tx.execute(
        "INSERT INTO access_credentials (hash, device, expires_at) VALUES ($1, $2, $3)",
        &[
            &bytes(&issued.access),
            &device.as_str(),
            &issued.access_expires_at,
        ],
    )
    .await
    .map_err(failed)?;
    tx.execute(
        "INSERT INTO refresh_credentials (hash, device, expires_at) VALUES ($1, $2, $3)",
        &[
            &bytes(&issued.refresh),
            &device.as_str(),
            &issued.refresh_expires_at,
        ],
    )
    .await
    .map_err(failed)?;
    Ok(())
}

fn device(row: &Row) -> Result<Device> {
    let revoked_at: Option<i64> = row.get(7);
    let reason: Option<String> = row.get(8);
    Ok(Device {
        id: DeviceId::new(row.get::<_, String>(0)).map_err(failed)?,
        account: AccountId(row.get(1)),
        name: row.get(2),
        kind: unspelled::<DeviceKind>(row.get(3))?,
        platform: unspelled::<Platform>(row.get(4))?,
        created_at: row.get(5),
        last_seen_at: row.get(6),
        revoked: revoked_at.map(|at| Revoked {
            at,
            reason: match reason.as_deref() {
                Some("refreshReused") => RevokeReason::RefreshReused,
                Some("byOperator") => RevokeReason::ByOperator,
                _ => RevokeReason::ByOwner,
            },
        }),
    })
}

fn reason_spelled(reason: RevokeReason) -> &'static str {
    match reason {
        RevokeReason::ByOwner => "byOwner",
        RevokeReason::RefreshReused => "refreshReused",
        RevokeReason::ByOperator => "byOperator",
    }
}

/// A wire enum as the protocol spells it, so the column reads the same as the API.
fn spelled(value: &impl serde::Serialize) -> Result<String> {
    match serde_json::to_value(value).map_err(failed)? {
        serde_json::Value::String(text) => Ok(text),
        other => Err(RecordsError(format!("{other} is not a name"))),
    }
}

/// Back from [`spelled`]. A name a newer server wrote reads as the enum's `Other`.
fn unspelled<T: serde::de::DeserializeOwned>(text: String) -> Result<T> {
    serde_json::from_value(serde_json::Value::String(text)).map_err(failed)
}

fn bytes(hash: &TokenHash) -> &[u8] {
    &hash.0
}

fn hash(row: &Row, at: usize) -> Result<TokenHash> {
    let bytes: &[u8] = row.get(at);
    bytes
        .try_into()
        .map(TokenHash)
        .map_err(|_| RecordsError(format!("a stored hash is {} bytes", bytes.len())))
}

/// The error and its causes: a database error's own `Display` is only "db error".
fn failed(error: impl std::error::Error) -> RecordsError {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        text.push_str(&format!(": {next}"));
        cause = next.source();
    }
    RecordsError(text)
}
