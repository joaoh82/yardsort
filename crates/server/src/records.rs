//! What the control service keeps: accounts, devices, their credentials' hashes, sign-ins in
//! progress, and the hosts of each account with the grants their people gave other devices.
//!
//! [`Records`] is what the service asks of a store. Each method is one step that must happen
//! whole: taking a one-time value removes it in the same step that reads it, and rotating a
//! refresh credential checks and replaces it in one step, so two requests racing with the same
//! value cannot both succeed. [`MemoryRecords`] keeps everything in one process, for tests;
//! [`crate::postgres::PgRecords`] is the store a server runs on. Both pass the same tests
//! (`tests/records.rs`).
//!
//! Nothing here is a secret: credentials and one-time values are kept as [`TokenHash`]es. The one
//! exception is the identity provider's PKCE verifier for a sign-in in progress, which has to be
//! sent to the provider as it is; it lives for at most [`crate::LOGIN_TTL_MS`].

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use remote_proto::account::{DeviceKind, Secret};
use remote_proto::pairing::MAX_OFFER_MISSES;
use remote_proto::{DeviceId, GrantId, HostId, Platform, ProjectGrant, Scope};
use tokio::sync::broadcast;

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
    /// The server's operator revoked every device of its account (`yardsort-server
    /// revoke-account`).
    ByOperator,
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
    /// It had already been traded in. The device, of this account, is revoked in the same step.
    Reused(AccountId),
    /// It lapsed unused.
    Expired,
    /// Its device was revoked.
    Revoked,
    /// No such credential.
    Unknown,
}

/// A machine that runs agents, registered on an account by the device it signs in as. Removed
/// hosts are never returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub id: HostId,
    pub account: AccountId,
    /// The device it is signed in as now. Signing in again moves it to the new one.
    pub device: DeviceId,
    pub name: String,
    pub created_at: i64,
    /// Its device's.
    pub last_seen_at: Option<i64>,
}

/// What a host showed to let one device in, as kept: two hashes, either of which redeems it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostOffer {
    pub host: HostId,
    pub account: AccountId,
    /// Of the QR code's secret.
    pub secret: TokenHash,
    /// Of the short code, normalized.
    pub code: TokenHash,
    pub expires_at: i64,
}

/// What became of a presented offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redemption {
    /// It was open: it is spent now.
    Taken(HostOffer),
    /// No open offer of the account matched. The miss is counted against all of them.
    Missed,
}

/// A grant's state as kept. "Expired" is not kept: it is a pending grant whose `expires_at` has
/// passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantStatus {
    Pending,
    Approved,
    Denied,
    Revoked,
}

/// One device's grant on one host, from when it was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRecord {
    pub id: GrantId,
    pub account: AccountId,
    pub host: HostId,
    /// The device it is for.
    pub device: DeviceId,
    pub requested: BTreeSet<Scope>,
    /// Empty unless approved.
    pub scopes: BTreeSet<Scope>,
    /// `None` unless approved.
    pub projects: Option<ProjectGrant>,
    pub status: GrantStatus,
    pub created_at: i64,
    /// When a pending grant stops waiting for its host's answer.
    pub expires_at: i64,
    pub decided_at: Option<i64>,
}

/// A grant as listed: with its host's and device's names, and whether that device is signed out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantView {
    pub grant: GrantRecord,
    pub host_name: String,
    pub device_name: String,
    pub device_revoked: bool,
}

/// Which grants to list. Always within one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantFilter {
    One(GrantId),
    OfHost(HostId),
    ForDevice(DeviceId),
}

/// The host's person's answer to a pending grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Approve {
        scopes: BTreeSet<Scope>,
        projects: ProjectGrant,
    },
    Deny,
}

#[derive(Debug, thiserror::Error)]
#[error("the records could not be read or written: {0}")]
pub struct RecordsError(pub String);

pub type Result<T> = std::result::Result<T, RecordsError>;

/// How stale a device's `last_seen_at` may be: a use within this long of the last one noted is
/// not written again, so a busy device does not write on every request.
pub const SEEN_RESOLUTION_MS: i64 = 60_000;

/// Where a host's relay link is held: which instance of the server, and which of its links, so
/// that a device reaching another instance can be sent there. Its instance notes it again every
/// few seconds while the link stays; a place not noted for a while is from an instance that went
/// away, and is not believed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPlace {
    pub host: HostId,
    pub instance: String,
    /// Tells this link from a later one of the same host, on any instance.
    pub link: String,
    /// When its instance last said it still holds it.
    pub seen_at: i64,
}

/// What is left of a rate limit for one address: taken, or how long until one is free.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Allowance {
    Taken,
    Wait(Duration),
}

/// A bucket of `per_minute`, holding `tokens` at `at`, refilled until `now`: `per_minute` at most,
/// and full again a minute after it was empty. A clock behind `at` refills nothing.
pub fn refilled(tokens: f64, at: i64, per_minute: f64, now: i64) -> f64 {
    let elapsed = (now - at).max(0) as f64;
    (tokens + elapsed * per_minute / 60_000.0).min(per_minute)
}

/// What a bucket of `per_minute` left with `tokens` says, one having been taken or not.
pub fn allowance(taken: bool, tokens: f64, per_minute: f64) -> Allowance {
    if taken {
        Allowance::Taken
    } else {
        Allowance::Wait(Duration::from_secs_f64(
            (1.0 - tokens).max(0.0) * 60.0 / per_minute,
        ))
    }
}

/// How long a bucket goes untouched before it is full again, and is swept.
pub const ALLOWANCE_REFILLS_MS: i64 = 60_000;

/// What one instance of the server tells every instance sharing the records, itself included, so
/// that each acts at once on what the others did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// Something of this account was revoked or replaced: check its live sockets now.
    Recheck(AccountId),
    /// Check every live socket: every account may have changed, or signals may have been missed.
    RecheckAll,
    /// A device asks this host to pair, through this grant.
    Pairing(HostId, GrantId),
    /// This host's link was just noted ([`LinkPlace`]): an instance holding an earlier one closes
    /// it, once the records say a later one took its place.
    Linked(HostId),
}

impl Signal {
    /// As it travels between instances: a few words, ids only. Ids never hold a space.
    pub fn encode(&self) -> String {
        match self {
            Self::Recheck(account) => format!("recheck {}", account.0),
            Self::RecheckAll => "recheck-all".into(),
            Self::Pairing(host, grant) => format!("pairing {host} {grant}"),
            Self::Linked(host) => format!("linked {host}"),
        }
    }

    /// `None` for anything else, such as a newer server's signal.
    pub fn decode(text: &str) -> Option<Self> {
        let words: Vec<&str> = text.split(' ').collect();
        match words.as_slice() {
            ["recheck", account] if !account.is_empty() => {
                Some(Self::Recheck(AccountId((*account).to_owned())))
            }
            ["recheck-all"] => Some(Self::RecheckAll),
            ["pairing", host, grant] => Some(Self::Pairing(
                HostId::new(*host).ok()?,
                GrantId::new(*grant).ok()?,
            )),
            ["linked", host] => Some(Self::Linked(HostId::new(*host).ok()?)),
            _ => None,
        }
    }
}

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
    ///
    /// A spent refresh credential must be kept until its own expiry: one presented again has to
    /// come back [`Rotation::Reused`], which revokes the device, and not [`Rotation::Unknown`],
    /// which leaves the copy's holder signed in. A store may drop it once it has lapsed.
    fn rotate(
        &self,
        refresh: &TokenHash,
        issued: Issued,
        now: i64,
    ) -> impl Future<Output = Result<Rotation>> + Send;

    /// Note that a device used its access, unless one was noted less than
    /// [`SEEN_RESOLUTION_MS`] ago.
    fn seen(&self, device: &DeviceId, now: i64) -> impl Future<Output = Result<()>> + Send;

    /// The account's devices, revoked ones included.
    fn devices(&self, account: &AccountId) -> impl Future<Output = Result<Vec<Device>>> + Send;

    /// How many of the account's devices are signed in at `now`: not revoked, and holding a
    /// refresh credential that is neither spent nor lapsed.
    fn signed_in_devices(
        &self,
        account: &AccountId,
        now: i64,
    ) -> impl Future<Output = Result<usize>> + Send;

    /// Revoke one of the account's devices. `false` when the account has no such device, which
    /// includes another account's: the two are not told apart.
    fn revoke(
        &self,
        account: &AccountId,
        device: &DeviceId,
        revoked: Revoked,
    ) -> impl Future<Output = Result<bool>> + Send;

    /// Revoke every device of the account for this provider subject that is not revoked yet, in
    /// one step, and say how many that was. `None` when there is no such account; none is made.
    fn revoke_account(
        &self,
        issuer: &str,
        subject: &str,
        revoked: Revoked,
    ) -> impl Future<Output = Result<Option<usize>>> + Send;

    /// Register a new host.
    fn add_host(&self, host: Host) -> impl Future<Output = Result<()>> + Send;

    /// The account's host `id`, now signed in as `device` and called `name`. `None` when the
    /// account has no such host (another account's included), in which case nothing changes.
    fn rebind_host(
        &self,
        account: &AccountId,
        id: &HostId,
        device: &DeviceId,
        name: &str,
    ) -> impl Future<Output = Result<Option<Host>>> + Send;

    /// The account's host `id`, unless it was removed.
    fn host(
        &self,
        account: &AccountId,
        id: &HostId,
    ) -> impl Future<Output = Result<Option<Host>>> + Send;

    /// The account's hosts, removed ones left out.
    fn hosts(&self, account: &AccountId) -> impl Future<Output = Result<Vec<Host>>> + Send;

    /// Keep an offer, in place of any other its host still had open. Lapsed offers may be
    /// dropped.
    fn put_offer(&self, offer: HostOffer, now: i64) -> impl Future<Output = Result<()>> + Send;

    /// The account's open offer whose secret or code hashes to `key`, removed in the same step.
    /// An offer is open until it lapses or the account has missed [`MAX_OFFER_MISSES`] times
    /// while it was. A miss is counted against every open offer of the account in the same step.
    fn redeem_offer(
        &self,
        account: &AccountId,
        key: &TokenHash,
        now: i64,
    ) -> impl Future<Output = Result<Redemption>> + Send;

    /// Keep a new grant.
    fn add_grant(&self, grant: GrantRecord) -> impl Future<Output = Result<()>> + Send;

    /// The account's grants that match, newest first, with their names.
    fn grants(
        &self,
        account: &AccountId,
        filter: GrantFilter,
    ) -> impl Future<Output = Result<Vec<GrantView>>> + Send;

    /// Answer a grant that is still pending at `now`, in one step. Approving revokes any other
    /// grant in force for the same device on the same host, so one device holds one grant per
    /// host. `None` when the account has no such grant or it is not pending any more.
    fn decide(
        &self,
        account: &AccountId,
        id: &GrantId,
        decision: Decision,
        now: i64,
    ) -> impl Future<Output = Result<Option<GrantRecord>>> + Send;

    /// Revoke a grant that is pending or in force. `false` when the account has no such grant;
    /// one already decided otherwise is left as it was, and still `true`.
    fn revoke_grant(
        &self,
        account: &AccountId,
        id: &GrantId,
        now: i64,
    ) -> impl Future<Output = Result<bool>> + Send;

    /// Remove a host in one step: its open offers go, and every grant on it that is pending or
    /// in force is revoked. `false` when the account has no such host.
    fn remove_host(
        &self,
        account: &AccountId,
        id: &HostId,
        now: i64,
    ) -> impl Future<Output = Result<bool>> + Send;

    /// Note where `place.host`'s relay link is now, in place of wherever it was.
    fn put_link(&self, place: LinkPlace) -> impl Future<Output = Result<()>> + Send;

    /// Where `host`'s relay link was last noted, believed or not.
    fn link_place(&self, host: &HostId) -> impl Future<Output = Result<Option<LinkPlace>>> + Send;

    /// `instance` still holds its links: note each of its places as seen at `now`, and return
    /// them. A link it holds that is not among them was taken by a later one, on another
    /// instance.
    fn keep_links(
        &self,
        instance: &str,
        now: i64,
    ) -> impl Future<Output = Result<Vec<LinkPlace>>> + Send;

    /// `link` of `host` closed: forget its place, unless a later link has taken it.
    fn drop_link(&self, host: &HostId, link: &str) -> impl Future<Output = Result<()>> + Send;

    /// Tell every instance sharing these records, this one included, of `signal`. Best effort:
    /// what a signal hastens, the relay's rounds of checks do anyway.
    fn signal(&self, signal: Signal) -> impl Future<Output = Result<()>> + Send;

    /// The signals every instance sends from here on, as they arrive. A [`Signal::RecheckAll`]
    /// comes once listening starts, and again after any break in which some may have been
    /// missed. Needs a Tokio runtime.
    fn signals(&self) -> broadcast::Receiver<Signal>;

    /// The grants still waiting for an answer at `now`, of the hosts whose links `instance`
    /// holds: what their hosts are to be told of.
    fn pending_on_links(
        &self,
        instance: &str,
        now: i64,
    ) -> impl Future<Output = Result<Vec<(HostId, GrantId)>>> + Send;

    /// Take one for `key` from `gate`'s rate limit, which every instance sharing these records
    /// shares: a bucket holding `per_minute` ([`refilled`]). One step, so racing requests never
    /// take more than there is.
    fn take_allowance(
        &self,
        gate: &str,
        key: &TokenHash,
        per_minute: u32,
        now: i64,
    ) -> impl Future<Output = Result<Allowance>> + Send;

    /// Give back one taken from `key`'s bucket at `gate`: the request it was taken for was sent on to
    /// another instance, which counts it there.
    fn return_allowance(
        &self,
        gate: &str,
        key: &TokenHash,
        per_minute: u32,
    ) -> impl Future<Output = Result<()>> + Send;

    /// Forget the buckets untouched since `before`, which are full again if `before` is at least
    /// [`ALLOWANCE_REFILLS_MS`] ago.
    fn sweep_allowances(&self, before: i64) -> impl Future<Output = Result<()>> + Send;
}

/// Records kept in this process. A restart forgets every account, device and credential: for
/// tests and trying the server out, not for running one. A clone shares them, and their signals,
/// as instances of a server share a database.
#[derive(Clone)]
pub struct MemoryRecords {
    state: Arc<Mutex<State>>,
    signals: broadcast::Sender<Signal>,
}

/// Signals waiting for a slow listener before it is told it missed some.
pub const SIGNALS_KEPT: usize = 256;

impl Default for MemoryRecords {
    fn default() -> Self {
        Self {
            state: Arc::default(),
            signals: broadcast::channel(SIGNALS_KEPT).0,
        }
    }
}

#[derive(Default)]
struct State {
    accounts: HashMap<(String, String), AccountId>,
    logins: Vec<PendingLogin>,
    handoffs: Vec<Handoff>,
    devices: Vec<Device>,
    access: HashMap<TokenHash, (DeviceId, i64)>,
    refresh: HashMap<TokenHash, Refresh>,
    /// With when each was removed.
    hosts: Vec<(Host, Option<i64>)>,
    /// With the misses counted against each.
    offers: Vec<(HostOffer, i32)>,
    grants: Vec<GrantRecord>,
    links: HashMap<HostId, LinkPlace>,
    /// Each gate's and key's bucket: what it held, and when.
    allowances: HashMap<(String, TokenHash), (f64, i64)>,
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

    fn host(&self, account: &AccountId, id: &HostId) -> Option<Host> {
        let (host, _) = self
            .hosts
            .iter()
            .find(|(h, removed)| h.id == *id && h.account == *account && removed.is_none())?;
        let mut host = host.clone();
        host.last_seen_at = self
            .devices
            .iter()
            .find(|d| d.id == host.device)
            .and_then(|d| d.last_seen_at);
        Some(host)
    }

    fn view(&self, grant: &GrantRecord) -> GrantView {
        let device = self.devices.iter().find(|d| d.id == grant.device);
        GrantView {
            host_name: self
                .hosts
                .iter()
                .find(|(h, _)| h.id == grant.host)
                .map(|(h, _)| h.name.clone())
                .unwrap_or_default(),
            device_name: device.map(|d| d.name.clone()).unwrap_or_default(),
            device_revoked: device.is_none_or(|d| d.revoked.is_some()),
            grant: grant.clone(),
        }
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
                return Rotation::Reused(device.account.clone());
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
                if device
                    .last_seen_at
                    .is_none_or(|at| at <= now - SEEN_RESOLUTION_MS)
                {
                    device.last_seen_at = Some(now);
                }
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

    async fn signed_in_devices(&self, account: &AccountId, now: i64) -> Result<usize> {
        self.with(|s| {
            s.devices
                .iter()
                .filter(|d| d.account == *account && d.revoked.is_none())
                .filter(|d| {
                    s.refresh
                        .values()
                        .any(|r| r.device == d.id && !r.spent && r.expires_at > now)
                })
                .count()
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
    async fn revoke_account(
        &self,
        issuer: &str,
        subject: &str,
        revoked: Revoked,
    ) -> Result<Option<usize>> {
        self.with(|s| {
            let account = s
                .accounts
                .get(&(issuer.to_owned(), subject.to_owned()))?
                .clone();
            let mut count = 0;
            for device in &mut s.devices {
                if device.account == account && device.revoked.is_none() {
                    device.revoked = Some(revoked);
                    count += 1;
                }
            }
            Some(count)
        })
    }

    async fn add_host(&self, host: Host) -> Result<()> {
        self.with(|s| s.hosts.push((host, None)))
    }

    async fn rebind_host(
        &self,
        account: &AccountId,
        id: &HostId,
        device: &DeviceId,
        name: &str,
    ) -> Result<Option<Host>> {
        self.with(|s| {
            let (host, _) = s
                .hosts
                .iter_mut()
                .find(|(h, removed)| h.id == *id && h.account == *account && removed.is_none())?;
            host.device = device.clone();
            host.name = name.to_owned();
            s.host(account, id)
        })
    }

    async fn host(&self, account: &AccountId, id: &HostId) -> Result<Option<Host>> {
        self.with(|s| s.host(account, id))
    }

    async fn hosts(&self, account: &AccountId) -> Result<Vec<Host>> {
        self.with(|s| {
            let ids: Vec<HostId> = s.hosts.iter().map(|(h, _)| h.id.clone()).collect();
            ids.iter().filter_map(|id| s.host(account, id)).collect()
        })
    }

    async fn put_offer(&self, offer: HostOffer, now: i64) -> Result<()> {
        self.with(|s| {
            s.offers
                .retain(|(o, _)| o.expires_at > now && o.host != offer.host);
            s.offers.push((offer, 0));
        })
    }

    async fn redeem_offer(
        &self,
        account: &AccountId,
        key: &TokenHash,
        now: i64,
    ) -> Result<Redemption> {
        self.with(|s| {
            let open = |o: &HostOffer, misses: i32| {
                o.account == *account && o.expires_at > now && misses < MAX_OFFER_MISSES
            };
            let found = s.offers.iter().position(|(o, misses)| {
                open(o, *misses) && (tokens::same(&o.secret, key) || tokens::same(&o.code, key))
            });
            match found {
                Some(at) => Redemption::Taken(s.offers.swap_remove(at).0),
                None => {
                    for (o, misses) in &mut s.offers {
                        if open(o, *misses) {
                            *misses += 1;
                        }
                    }
                    Redemption::Missed
                }
            }
        })
    }

    async fn add_grant(&self, grant: GrantRecord) -> Result<()> {
        self.with(|s| s.grants.push(grant))
    }

    async fn grants(&self, account: &AccountId, filter: GrantFilter) -> Result<Vec<GrantView>> {
        self.with(|s| {
            let mut views: Vec<GrantView> = s
                .grants
                .iter()
                .filter(|g| g.account == *account)
                .filter(|g| match &filter {
                    GrantFilter::One(id) => g.id == *id,
                    GrantFilter::OfHost(host) => g.host == *host,
                    GrantFilter::ForDevice(device) => g.device == *device,
                })
                .map(|g| s.view(g))
                .collect();
            views.sort_by_key(|v| std::cmp::Reverse(v.grant.created_at));
            views
        })
    }

    async fn decide(
        &self,
        account: &AccountId,
        id: &GrantId,
        decision: Decision,
        now: i64,
    ) -> Result<Option<GrantRecord>> {
        self.with(|s| {
            let at = s.grants.iter().position(|g| {
                g.id == *id
                    && g.account == *account
                    && g.status == GrantStatus::Pending
                    && g.expires_at > now
            })?;
            let (host, device) = (s.grants[at].host.clone(), s.grants[at].device.clone());
            if let Decision::Approve { .. } = decision {
                for other in &mut s.grants {
                    if other.host == host
                        && other.device == device
                        && other.status == GrantStatus::Approved
                    {
                        other.status = GrantStatus::Revoked;
                        other.decided_at = Some(now);
                    }
                }
            }
            let grant = &mut s.grants[at];
            match decision {
                Decision::Approve { scopes, projects } => {
                    grant.status = GrantStatus::Approved;
                    grant.scopes = scopes;
                    grant.projects = Some(projects);
                }
                Decision::Deny => grant.status = GrantStatus::Denied,
            }
            grant.decided_at = Some(now);
            Some(grant.clone())
        })
    }

    async fn revoke_grant(&self, account: &AccountId, id: &GrantId, now: i64) -> Result<bool> {
        self.with(|s| {
            let Some(grant) = s
                .grants
                .iter_mut()
                .find(|g| g.id == *id && g.account == *account)
            else {
                return false;
            };
            if matches!(grant.status, GrantStatus::Pending | GrantStatus::Approved) {
                grant.status = GrantStatus::Revoked;
                grant.decided_at = Some(now);
            }
            true
        })
    }

    async fn remove_host(&self, account: &AccountId, id: &HostId, now: i64) -> Result<bool> {
        self.with(|s| {
            let Some((_, removed)) = s
                .hosts
                .iter_mut()
                .find(|(h, removed)| h.id == *id && h.account == *account && removed.is_none())
            else {
                return false;
            };
            *removed = Some(now);
            s.offers.retain(|(o, _)| o.host != *id);
            for grant in &mut s.grants {
                if grant.host == *id
                    && matches!(grant.status, GrantStatus::Pending | GrantStatus::Approved)
                {
                    grant.status = GrantStatus::Revoked;
                    grant.decided_at = Some(now);
                }
            }
            true
        })
    }

    async fn put_link(&self, place: LinkPlace) -> Result<()> {
        self.with(|s| {
            s.links.insert(place.host.clone(), place);
        })
    }

    async fn link_place(&self, host: &HostId) -> Result<Option<LinkPlace>> {
        self.with(|s| s.links.get(host).cloned())
    }

    async fn keep_links(&self, instance: &str, now: i64) -> Result<Vec<LinkPlace>> {
        self.with(|s| {
            s.links
                .values_mut()
                .filter(|place| place.instance == instance)
                .map(|place| {
                    place.seen_at = now;
                    place.clone()
                })
                .collect()
        })
    }

    async fn drop_link(&self, host: &HostId, link: &str) -> Result<()> {
        self.with(|s| {
            if s.links.get(host).is_some_and(|place| place.link == link) {
                s.links.remove(host);
            }
        })
    }

    async fn signal(&self, signal: Signal) -> Result<()> {
        // Nobody listening is not a failure: there is nothing to hasten.
        let _ = self.signals.send(signal);
        Ok(())
    }

    fn signals(&self) -> broadcast::Receiver<Signal> {
        let listening = self.signals.subscribe();
        let _ = self.signals.send(Signal::RecheckAll);
        listening
    }

    async fn pending_on_links(&self, instance: &str, now: i64) -> Result<Vec<(HostId, GrantId)>> {
        self.with(|s| {
            s.grants
                .iter()
                .filter(|g| g.status == GrantStatus::Pending && g.expires_at > now)
                .filter(|g| {
                    s.links
                        .get(&g.host)
                        .is_some_and(|place| place.instance == instance)
                })
                .map(|g| (g.host.clone(), g.id.clone()))
                .collect()
        })
    }

    async fn take_allowance(
        &self,
        gate: &str,
        key: &TokenHash,
        per_minute: u32,
        now: i64,
    ) -> Result<Allowance> {
        let per_minute = f64::from(per_minute.max(1));
        self.with(|s| {
            let bucket = s
                .allowances
                .entry((gate.to_owned(), *key))
                .or_insert((per_minute, now));
            let tokens = refilled(bucket.0, bucket.1, per_minute, now);
            let taken = tokens >= 1.0;
            let left = if taken { tokens - 1.0 } else { tokens };
            *bucket = (left, bucket.1.max(now));
            allowance(taken, left, per_minute)
        })
    }

    async fn return_allowance(&self, gate: &str, key: &TokenHash, per_minute: u32) -> Result<()> {
        let per_minute = f64::from(per_minute.max(1));
        self.with(|s| {
            if let Some(bucket) = s.allowances.get_mut(&(gate.to_owned(), *key)) {
                bucket.0 = (bucket.0 + 1.0).min(per_minute);
            }
        })
    }

    async fn sweep_allowances(&self, before: i64) -> Result<()> {
        self.with(|s| s.allowances.retain(|_, (_, at)| *at >= before))
    }
}
