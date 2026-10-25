//! The records in PostgreSQL: what a server that is run for real keeps.
//!
//! Each [`Records`] method is one statement or one transaction, so "one step" (see
//! [`crate::records`]) holds across every instance of the server sharing the database: a one-time
//! value is taken with `DELETE … RETURNING`, and a refresh credential is checked and replaced with
//! its row and its device's locked.
//!
//! The schema is in `migrations/`, applied in order when the server connects, under an advisory
//! lock so that two instances starting together do not both apply it. Migrations are append-only:
//! a shipped file is never edited, and a change is a new file.

use std::sync::Arc;

use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod, Transaction};
use remote_proto::account::{DeviceKind, Secret};
use remote_proto::{DeviceId, Platform, ServerId};
use tokio_postgres::{Client, Row};

use crate::records::{
    AccountId, Device, Handoff, Issued, PendingLogin, Records, RecordsError, Result, RevokeReason,
    Revoked, Rotation, SEEN_RESOLUTION_MS,
};
use crate::tokens::TokenHash;

/// The schema, in the order it is applied. Append only.
const MIGRATIONS: &[(i32, &str, &str)] = &[(
    1,
    "accounts_and_devices",
    include_str!("../migrations/0001_accounts_and_devices.sql"),
)];

/// Held while migrating, so instances starting together take turns. Any constant will do, as
/// long as nothing else in the database uses it: "ysrv".
const MIGRATION_LOCK: i64 = 0x7973_7276;

/// Connections kept open per instance.
const POOL_SIZE: usize = 8;

/// A device row, as every query that returns one selects it.
const DEVICE: &str = "d.id, d.account, d.name, d.kind, d.platform, d.created_at, \
                      d.last_seen_at, d.revoked_at, d.revoked_reason";

pub struct PgRecords {
    pool: Pool,
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
            config,
            tls()?,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let pool = Pool::builder(manager)
            .max_size(POOL_SIZE)
            .build()
            .map_err(failed)?;
        let records = Self { pool };
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
            Rotation::Reused
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
