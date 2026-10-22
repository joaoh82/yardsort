//! What the control service keeps: accounts, devices, their credentials' hashes, and sign-ins in
//! progress.
//!
//! [`Records`] is what the service asks of a store. Each method is one step that must happen
//! whole: taking a one-time value removes it in the same step that reads it, and rotating a
//! refresh credential checks and replaces it in one step, so two requests racing with the same
//! value cannot both succeed. [`MemoryRecords`] keeps everything in one process; the PostgreSQL
//! store comes next and passes the same tests.
//!
//! Nothing here is a secret: credentials and one-time values are kept as [`TokenHash`]es. The one
//! exception is the identity provider's PKCE verifier for a sign-in in progress, which has to be
//! sent to the provider as it is; it lives for at most [`crate::LOGIN_TTL_MS`].

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

use remote_proto::account::{DeviceKind, Secret};
use remote_proto::{DeviceId, Platform};

use crate::tokens::{self, TokenHash};

/// An account on this server: one identity provider subject. Never an email address, and never
/// linked to another account by one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountId(pub String);

/// A sign-in between the device starting it and the identity provider answering.
#[derive(Debug, Clone)]
pub struct PendingLogin {
    /// Hash of the `state` sent to the provider, which comes back with its answer.
    pub state: TokenHash,
    /// Hash of the `nonce` the provider must put in its ID token.
    pub nonce: TokenHash,
    /// The PKCE verifier for the provider's code, sent with the code exchange.
    pub provider_verifier: Secret,
    /// The device's own PKCE challenge, which its redemption must answer.
    pub device_challenge: String,
    /// Where the device asked to be sent back, already checked against the allowed redirects.
    pub device_redirect: String,
    pub expires_at: i64,
}

/// A signed-in person's one-time code, sent to the device's redirect, before the device redeems it.
#[derive(Debug, Clone)]
pub struct Handoff {
    pub code: TokenHash,
    pub account: AccountId,
    pub device_challenge: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: DeviceId,
    pub account: AccountId,
    pub name: String,
    pub kind: DeviceKind,
    pub platform: Platform,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    pub revoked: Option<Revoked>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revoked {
    pub at: i64,
    pub reason: RevokeReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokeReason {
    /// Its person revoked it, from it or another of their devices.
    ByOwner,
    /// A refresh credential it had already traded in was presented again: a copy of it exists.
    RefreshReused,
}

/// A device's credentials as kept: hashes and when they lapse.
#[derive(Debug, Clone, Copy)]
pub struct Issued {
    pub access: TokenHash,
    pub access_expires_at: i64,
    pub refresh: TokenHash,
    pub refresh_expires_at: i64,
}

/// What became of a refresh credential presented for rotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rotation {
    /// It was current: it is now spent, and the new credentials are in its place.
    Rotated(Device),
    /// It had already been traded in. The device is revoked in the same step.
    Reused,
    /// It lapsed unused.
    Expired,
    /// Its device was revoked.
    Revoked,
    /// No such credential.
    Unknown,
}

#[derive(Debug, thiserror::Error)]
#[error("the records could not be read or written: {0}")]
pub struct RecordsError(pub String);

pub type Result<T> = std::result::Result<T, RecordsError>;

/// What the control service needs of a store. See the module documentation for what "one step"
/// means for each method.
pub trait Records: Send + Sync {
    /// The account for this provider subject, created the first time it signs in.
    fn account(
        &self,
        issuer: &str,
        subject: &str,
        now: i64,
    ) -> impl Future<Output = Result<AccountId>> + Send;

    /// Keep a sign-in in progress. Ones that lapsed by `now` may be dropped.
    fn put_login(&self, login: PendingLogin, now: i64) -> impl Future<Output = Result<()>> + Send;
    /// The sign-in this `state` belongs to, removed in the same step, expired or not.
    fn take_login(
        &self,
        state: &TokenHash,
    ) -> impl Future<Output = Result<Option<PendingLogin>>> + Send;

    /// Keep a handoff. Ones that lapsed by `now` may be dropped.
    fn put_handoff(&self, handoff: Handoff, now: i64) -> impl Future<Output = Result<()>> + Send;
    /// The handoff this code belongs to, removed in the same step, expired or not.
    fn take_handoff(
        &self,
        code: &TokenHash,
    ) -> impl Future<Output = Result<Option<Handoff>>> + Send;

    /// Enroll a device with its first credentials.
    fn add_device(&self, device: Device, issued: Issued)
        -> impl Future<Output = Result<()>> + Send;

    /// The device an access credential belongs to, and when that credential lapses. Revoked
    /// devices are returned as they are: the caller decides.
    fn access(
        &self,
        access: &TokenHash,
    ) -> impl Future<Output = Result<Option<(Device, i64)>>> + Send;

    /// Trade `refresh` for `issued`, if it is current. See [`Rotation`].
    fn rotate(
        &self,
        refresh: &TokenHash,
        issued: Issued,
        now: i64,
    ) -> impl Future<Output = Result<Rotation>> + Send;

    /// Note that a device used its access.
    fn seen(&self, device: &DeviceId, now: i64) -> impl Future<Output = Result<()>> + Send;

    /// The account's devices, revoked ones included.
    fn devices(&self, account: &AccountId) -> impl Future<Output = Result<Vec<Device>>> + Send;

    /// Revoke one of the account's devices. `false` when the account has no such device, which
    /// includes another account's: the two are not told apart.
    fn revoke(
        &self,
        account: &AccountId,
        device: &DeviceId,
        revoked: Revoked,
    ) -> impl Future<Output = Result<bool>> + Send;
}

/// Records kept in this process. A restart forgets every account, device and credential: for
/// tests and trying the server out, not for running one.
#[derive(Default)]
pub struct MemoryRecords {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    accounts: HashMap<(String, String), AccountId>,
    logins: Vec<PendingLogin>,
    handoffs: Vec<Handoff>,
    devices: Vec<Device>,
    access: HashMap<TokenHash, (DeviceId, i64)>,
    refresh: HashMap<TokenHash, Refresh>,
}

#[derive(Debug, Clone)]
struct Refresh {
    device: DeviceId,
    expires_at: i64,
    /// Traded in already. Kept so that presenting it again is recognised as reuse.
    spent: bool,
}

impl MemoryRecords {
    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> Result<T> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RecordsError("a writer panicked".into()))?;
        Ok(f(&mut state))
    }

    /// How many accounts exist, for a test to check that a refused sign-in created none.
    pub fn account_count(&self) -> usize {
        self.with(|s| s.accounts.len()).unwrap_or_default()
    }
}

impl State {
    fn issue(&mut self, device: &DeviceId, issued: Issued) {
        self.access
            .insert(issued.access, (device.clone(), issued.access_expires_at));
        self.refresh.insert(
            issued.refresh,
            Refresh {
                device: device.clone(),
                expires_at: issued.refresh_expires_at,
                spent: false,
            },
        );
    }

    fn device_mut(&mut self, id: &DeviceId) -> Option<&mut Device> {
        self.devices.iter_mut().find(|d| d.id == *id)
    }
}

impl Records for MemoryRecords {
    async fn account(&self, issuer: &str, subject: &str, _now: i64) -> Result<AccountId> {
        self.with(|s| {
            s.accounts
                .entry((issuer.to_owned(), subject.to_owned()))
                .or_insert_with(|| AccountId(uuid::Uuid::new_v4().to_string()))
                .clone()
        })
    }

    async fn put_login(&self, login: PendingLogin, now: i64) -> Result<()> {
        self.with(|s| {
            s.logins.retain(|l| l.expires_at > now);
            s.logins.push(login);
        })
    }

    async fn take_login(&self, state: &TokenHash) -> Result<Option<PendingLogin>> {
        self.with(|s| {
            let at = s
                .logins
                .iter()
                .position(|l| tokens::same(&l.state, state))?;
            Some(s.logins.swap_remove(at))
        })
    }

    async fn put_handoff(&self, handoff: Handoff, now: i64) -> Result<()> {
        self.with(|s| {
            s.handoffs.retain(|h| h.expires_at > now);
            s.handoffs.push(handoff);
        })
    }

    async fn take_handoff(&self, code: &TokenHash) -> Result<Option<Handoff>> {
        self.with(|s| {
            let at = s
                .handoffs
                .iter()
                .position(|h| tokens::same(&h.code, code))?;
            Some(s.handoffs.swap_remove(at))
        })
    }

    async fn add_device(&self, device: Device, issued: Issued) -> Result<()> {
        self.with(|s| {
            s.issue(&device.id, issued);
            s.devices.push(device);
        })
    }

    async fn access(&self, access: &TokenHash) -> Result<Option<(Device, i64)>> {
        self.with(|s| {
            let (id, expires_at) = s.access.get(access)?.clone();
            let device = s.devices.iter().find(|d| d.id == id)?.clone();
            Some((device, expires_at))
        })
    }

    async fn rotate(&self, refresh: &TokenHash, issued: Issued, now: i64) -> Result<Rotation> {
        self.with(|s| {
            let Some(old) = s.refresh.get(refresh).cloned() else {
                return Rotation::Unknown;
            };
            let Some(device) = s.devices.iter().find(|d| d.id == old.device).cloned() else {
                return Rotation::Unknown;
            };
            if device.revoked.is_some() {
                return Rotation::Revoked;
            }
            if old.spent {
                if let Some(device) = s.device_mut(&old.device) {
                    device.revoked = Some(Revoked {
                        at: now,
                        reason: RevokeReason::RefreshReused,
                    });
                }
                return Rotation::Reused;
            }
            if old.expires_at <= now {
                return Rotation::Expired;
            }
            if let Some(spent) = s.refresh.get_mut(refresh) {
                spent.spent = true;
            }
            s.issue(&device.id, issued);
            Rotation::Rotated(device)
        })
    }

    async fn seen(&self, device: &DeviceId, now: i64) -> Result<()> {
        self.with(|s| {
            if let Some(device) = s.device_mut(device) {
                device.last_seen_at = Some(now);
            }
        })
    }

    async fn devices(&self, account: &AccountId) -> Result<Vec<Device>> {
        self.with(|s| {
            s.devices
                .iter()
                .filter(|d| d.account == *account)
                .cloned()
                .collect()
        })
    }

    async fn revoke(
        &self,
        account: &AccountId,
        device: &DeviceId,
        revoked: Revoked,
    ) -> Result<bool> {
        self.with(|s| match s.device_mut(device) {
            Some(d) if d.account == *account => {
                d.revoked.get_or_insert(revoked);
                true
            }
            _ => false,
        })
    }
}
