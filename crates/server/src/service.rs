//! Signing in, refreshing, and an account's devices, over any [`Records`].

use std::sync::Arc;

use remote_proto::account::{
    Credentials, DeviceKind, DevicePage, DeviceSummary, LoginParams, LoginPurpose, Secret,
    ServerDescription, TokenRequest,
};
use remote_proto::delete::{AccountDeleted, DeleteRequest, FRESH_SIGN_IN_MS};
use remote_proto::{DeviceId, ErrorCode, ErrorReason, ProtocolVersion, RemoteError, ServerId};
use url::Url;

use crate::entitlements::{Action, Counts, Entitlements};
use crate::journal::Journal;
use crate::oidc::{OidcError, Provider, ProviderConfig, CLOCK_LEEWAY_MS};
use crate::push::{DisabledPush, PushProvider};
use crate::records::{
    AccountId, Device, Handoff, Issued, PendingLogin, Records, RevokeReason, Revoked, Rotation,
    Signal,
};
use crate::relay::Relay;
use crate::tokens::{self, Kind};
use crate::{ACCESS_TTL_MS, HANDOFF_TTL_MS, LOGIN_TTL_MS, MAX_DEVICES_PER_ACCOUNT, REFRESH_TTL_MS};

/// What a server's operator decides.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub server: ServerId,
    /// What the operator calls the server, shown at sign-in.
    pub name: String,
    /// The server's own public address, which the provider sends browsers back to:
    /// `https://yardsort.example.com/`. Its `v1/login/callback` is what is registered at the
    /// provider as the redirect URI, and the only one.
    pub public_url: Url,
    pub provider: ProviderConfig,
    /// Where a sign-in may send the browser back to a device, compared exactly: the app's own
    /// scheme (`yardsort://signed-in`), say. A loopback address (`http://127.0.0.1/signed-in`)
    /// matches on any port, since a computer's client listens on whichever port is free
    /// ([RFC 8252 §7.3](https://www.rfc-editor.org/rfc/rfc8252#section-7.3)).
    pub device_redirects: Vec<String>,
}

/// A device that presented a working access credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub account: AccountId,
    pub device: DeviceId,
    pub kind: DeviceKind,
}

/// Where the provider's answer leads.
#[derive(Debug)]
pub enum LoginFinish {
    /// Signed in: send the browser here, to the device's redirect with its one-time `code`.
    SignedIn(Url),
    /// Not signed in: send the browser here, to the device's redirect with an `error` code.
    /// `why` is for the server's log, never the browser.
    Refused { to: Url, why: String },
    /// No sign-in this answer belongs to, so nowhere to send it back to: show the error.
    Unknown(RemoteError),
}

pub struct ControlService<R> {
    config: ServerConfig,
    /// The public URL, ending in `/`.
    base: Url,
    callback: Url,
    redirects: Vec<Url>,
    provider: Provider,
    pub(crate) records: R,
    pub(crate) relay: Relay,
    pub(crate) push: Arc<dyn PushProvider>,
    pub(crate) entitlements: Entitlements,
    pub(crate) journal: Arc<Journal>,
}

impl<R: Records> ControlService<R> {
    pub fn new(config: ServerConfig, records: R, http: reqwest::Client) -> Result<Self, OidcError> {
        let mut base = config.public_url.clone();
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let callback = base
            .join("v1/login/callback")
            .map_err(|e| OidcError::Misconfigured(format!("public URL: {e}")))?;
        let redirects = config
            .device_redirects
            .iter()
            .map(|text| {
                Url::parse(text)
                    .map_err(|e| OidcError::Misconfigured(format!("device redirect {text:?}: {e}")))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            provider: Provider::new(config.provider.clone(), http)?,
            config,
            base,
            callback,
            redirects,
            records,
            relay: Relay::default(),
            push: Arc::new(DisabledPush),
            entitlements: Entitlements::none(),
            journal: Arc::new(Journal::none()),
        })
    }

    /// Replace the push provider. Tests pass a recording one. A server passes APNs and FCM
    /// when they are configured, and a provider that queues when they are not.
    pub fn with_push(mut self, push: Arc<dyn PushProvider>) -> Self {
        self.push = push;
        self
    }

    /// Ask `entitlements` before what costs the operator. Without, everything is allowed.
    pub fn with_entitlements(mut self, entitlements: Entitlements) -> Self {
        self.entitlements = entitlements;
        self
    }

    /// Write each revocation and deletion out to `journal`, and refuse every request while the
    /// records are found behind it. Without, nothing is written out and nothing refused.
    pub fn with_journal(mut self, journal: Journal) -> Self {
        self.journal = Arc::new(journal);
        self
    }

    /// The journal, to check it before serving.
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn records(&self) -> &R {
        &self.records
    }

    /// Whether an entitlements service is asked.
    pub fn entitlements_configured(&self) -> bool {
        self.entitlements.configured()
    }

    /// Add what this instance counted to the records. What fails to go is kept for the next.
    pub async fn flush_usage(&self, now: i64) {
        let deltas = self.relay.meter().take();
        if deltas.is_empty() {
            return;
        }
        if let Err(error) = self.records.add_usage(&deltas, now).await {
            tracing::error!("usage: records: {error}");
            self.relay.meter().restore(deltas);
        }
    }

    /// The relay's sockets, to close them all when the server stops.
    pub fn relay(&self) -> &Relay {
        &self.relay
    }

    /// Something of `account` was revoked or replaced: every instance checks its sockets now,
    /// this one without waiting for the signal to come back. A signal that fails to go is caught
    /// up with by the other instances' next round of checks.
    pub(crate) async fn revoked(&self, account: &AccountId) {
        self.relay.nudge(account);
        if let Err(error) = self.records.signal(Signal::Recheck(account.clone())).await {
            tracing::error!("relay: records: {error}");
        }
    }

    /// With `relay` in place of one instance named at random: for a server that runs as several.
    pub fn with_relay(mut self, relay: Relay) -> Self {
        self.relay = relay;
        self
    }

    /// The server's public URL, ending in `/`.
    pub(crate) fn base_url(&self) -> &Url {
        &self.base
    }

    /// The redirect URI to register at the identity provider.
    pub fn callback_url(&self) -> &Url {
        &self.callback
    }

    pub fn describe(&self) -> ServerDescription {
        ServerDescription {
            server: self.config.server.clone(),
            name: self.config.name.clone(),
            protocol: ProtocolVersion::CURRENT,
            issuer: self.provider.issuer().to_owned(),
            capabilities: vec![
                "device.list".into(),
                "device.revoke".into(),
                "host.list".into(),
                "host.pair".into(),
                remote_proto::export::EXPORT_CAPABILITY.into(),
                remote_proto::delete::DELETE_CAPABILITY.into(),
            ],
        }
    }

    /// Start a sign-in: where to send the browser.
    ///
    /// A redirect the server does not allow is refused here, before anything is kept, and never
    /// sent anywhere: otherwise the server would be an open redirector that sign-in codes go
    /// through.
    pub async fn begin_login(&self, params: &LoginParams, now: i64) -> Result<Url, RemoteError> {
        params.validate()?;
        let redirect = self.allowed_redirect(&params.redirect).ok_or_else(|| {
            RemoteError::new(
                ErrorCode::InvalidRequest,
                "This server does not send sign-ins back to that address.",
            )
        })?;
        let confirm = params.purpose == Some(LoginPurpose::Confirm);
        self.begin(&params.challenge, &redirect, confirm, now).await
    }

    /// Start a sign-in for `challenge`, sent back to `redirect`, already allowed.
    pub(crate) async fn begin(
        &self,
        challenge: &str,
        redirect: &Url,
        confirm: bool,
        now: i64,
    ) -> Result<Url, RemoteError> {
        let state = tokens::random();
        let nonce = tokens::random();
        let verifier = Secret::new(tokens::random());
        let url = self
            .provider
            .authorize_url(
                &self.callback,
                &state,
                &nonce,
                &tokens::s256(verifier.expose()),
                confirm,
                now,
            )
            .await
            .map_err(|_| provider_unavailable())?;
        self.records
            .put_login(
                PendingLogin {
                    state: tokens::hash(&state),
                    nonce: tokens::hash(&nonce),
                    provider_verifier: verifier,
                    device_challenge: challenge.to_owned(),
                    device_redirect: redirect.to_string(),
                    confirm,
                    expires_at: now + LOGIN_TTL_MS,
                },
                now,
            )
            .await
            .map_err(unreadable)?;
        Ok(url)
    }

    /// The provider sent the browser back, with a `code` or an `error`, and the `state` this
    /// server gave it.
    pub async fn finish_login(
        &self,
        code: Option<&str>,
        state: Option<&str>,
        error: Option<&str>,
        now: i64,
    ) -> LoginFinish {
        let unknown = || {
            LoginFinish::Unknown(RemoteError::new(
                ErrorCode::Unauthenticated,
                "This sign-in has expired or was already finished. Start again from the app.",
            ))
        };
        let Some(state) = state else {
            return unknown();
        };
        let login = match self.records.take_login(&tokens::hash(state)).await {
            Ok(Some(login)) => login,
            Ok(None) => return unknown(),
            Err(error) => return LoginFinish::Unknown(unreadable(error)),
        };
        let back = match Url::parse(&login.device_redirect) {
            Ok(back) => back,
            Err(error) => return LoginFinish::Unknown(unreadable(error)),
        };
        let refuse = |code: ErrorCode, why: String| LoginFinish::Refused {
            to: with_query(&back, "error", error_code(code)),
            why,
        };
        if login.expires_at <= now {
            return refuse(
                ErrorCode::Unauthenticated,
                "the sign-in took too long".into(),
            );
        }
        if let Some(error) = error {
            // The person cancelled, or the provider would not let them in. Its code is a short
            // token by specification, but it came through the browser: keep only a bounded copy.
            let error: String = error.chars().take(64).collect();
            return refuse(
                ErrorCode::Unauthenticated,
                format!("the provider answered {error:?}"),
            );
        }
        let Some(code) = code else {
            return refuse(ErrorCode::InvalidRequest, "no code and no error".into());
        };
        let identity = match self
            .provider
            .sign_in(
                code,
                &self.callback,
                &login.provider_verifier,
                &login.nonce,
                now,
            )
            .await
        {
            Ok(identity) => identity,
            Err(error) => return refuse(ErrorCode::Unauthenticated, error.to_string()),
        };
        let (account, expires_at) = if login.confirm {
            // The provider was asked to sign the person in again: its token has to say it did,
            // just now. A provider that kept its session and said nothing confirms nothing.
            let Some(at) = identity
                .auth_time
                .filter(|at| *at > now - FRESH_SIGN_IN_MS && *at <= now + CLOCK_LEEWAY_MS)
            else {
                return refuse(
                    ErrorCode::Unauthenticated,
                    "the provider did not say the person signed in again just now".into(),
                );
            };
            // Signing in again makes no account: one that has none has nothing to delete.
            match self
                .records
                .find_account(&identity.issuer, &identity.subject)
                .await
            {
                Ok(Some(account)) => (account, (at + FRESH_SIGN_IN_MS).min(now + FRESH_SIGN_IN_MS)),
                Ok(None) => {
                    return refuse(
                        ErrorCode::Unauthenticated,
                        "signed in again as someone with no account here".into(),
                    )
                }
                Err(error) => return refuse(ErrorCode::Internal, error.to_string()),
            }
        } else {
            match self
                .records
                .account(&identity.issuer, &identity.subject, now)
                .await
            {
                Ok(account) => (account, now + HANDOFF_TTL_MS),
                Err(error) => return refuse(ErrorCode::Internal, error.to_string()),
            }
        };
        let handoff = tokens::mint(Kind::Handoff);
        let kept = self
            .records
            .put_handoff(
                Handoff {
                    code: handoff.hash,
                    account,
                    device_challenge: login.device_challenge,
                    confirm: login.confirm,
                    expires_at,
                    // A confirming sign-in enrolls nothing, so has no device to tell.
                    email: identity.email.filter(|_| !login.confirm),
                },
                now,
            )
            .await;
        if let Err(error) = kept {
            return refuse(ErrorCode::Internal, error.to_string());
        }
        LoginFinish::SignedIn(with_query(&back, "code", handoff.secret.expose()))
    }

    /// `POST /v1/token`.
    pub async fn token(
        &self,
        request: &TokenRequest,
        now: i64,
    ) -> Result<Credentials, RemoteError> {
        request.validate()?;
        match request {
            TokenRequest::SignIn {
                code,
                verifier,
                device,
            } => {
                let refused = || {
                    unauthenticated(
                        "That sign-in code has expired or was already used. Sign in again.",
                    )
                };
                let hash = tokens::presented(Kind::Handoff, code.expose()).ok_or_else(refused)?;
                // Taken first, so a wrong verifier spends the code too: no second guess.
                let handoff = self
                    .records
                    .take_handoff(&hash)
                    .await
                    .map_err(unreadable)?
                    .ok_or_else(refused)?;
                // A confirming sign-in's code confirms, and enrolls nothing.
                if handoff.confirm
                    || handoff.expires_at <= now
                    || !tokens::verifies(&handoff.device_challenge, verifier.expose())
                {
                    return Err(refused());
                }
                // Counted before adding, so two sign-ins at once may both get in: a bound on
                // what one account can make the records hold, not an exact quota.
                let signed_in = self
                    .records
                    .signed_in_devices(&handoff.account, now)
                    .await
                    .map_err(unreadable)?;
                if signed_in >= MAX_DEVICES_PER_ACCOUNT {
                    return Err(RemoteError::new(
                        ErrorCode::Forbidden,
                        format!(
                            "This account already has {MAX_DEVICES_PER_ACCOUNT} devices signed in. \
                             Sign one out from another device, then sign in again."
                        ),
                    ));
                }
                self.entitlements
                    .may(
                        &handoff.account,
                        Action::Device,
                        Counts {
                            devices: Some(signed_in as u64),
                            ..Counts::default()
                        },
                        now,
                    )
                    .await?;
                let id = DeviceId::new(uuid::Uuid::new_v4().to_string()).map_err(unreadable)?;
                let (issued, pair) = issue(now);
                let credentials = Credentials {
                    email: handoff.email,
                    ..pair.for_device(id.clone())
                };
                self.records
                    .add_device(
                        Device {
                            id,
                            account: handoff.account,
                            name: device.name.trim().to_owned(),
                            kind: device.kind,
                            platform: device.platform,
                            created_at: now,
                            last_seen_at: None,
                            revoked: None,
                        },
                        issued,
                    )
                    .await
                    .map_err(unreadable)?;
                Ok(credentials)
            }
            TokenRequest::Refresh { refresh } => {
                let signed_out = || unauthenticated("This device is signed out. Sign in again.");
                let hash =
                    tokens::presented(Kind::Refresh, refresh.expose()).ok_or_else(signed_out)?;
                // Minted before the old one is looked at, and kept only if it is current: checking
                // and replacing it is one step in the records.
                let (issued, pair) = issue(now);
                match self
                    .records
                    .rotate(&hash, issued, now)
                    .await
                    .map_err(unreadable)?
                {
                    // A retry gets what the lost answer carried, freshly minted.
                    Rotation::Rotated(device) | Rotation::Retried(device) => {
                        Ok(pair.for_device(device.id))
                    }
                    Rotation::Reused(account) => {
                        // A copy's holder may have a socket open as that device: it closes now.
                        self.revoked(&account).await;
                        Err(unauthenticated(
                        "This device's sign-in was used from somewhere else, so the device was \
                         signed out. Sign in again.",
                        ))
                    }
                    Rotation::Unknown if self.deleted(&hash, now).await? => Err(account_deleted()),
                    Rotation::Expired | Rotation::Revoked | Rotation::Unknown => Err(signed_out()),
                }
            }
        }
    }

    /// The device an `Authorization: Bearer` access credential belongs to, if it may act.
    pub async fn authenticate(&self, access: &str, now: i64) -> Result<Caller, RemoteError> {
        let signed_out = || unauthenticated("Sign in again.");
        let hash = tokens::presented(Kind::Access, access).ok_or_else(signed_out)?;
        let Some((device, expires_at)) = self.records.access(&hash).await.map_err(unreadable)?
        else {
            return Err(if self.deleted(&hash, now).await? {
                account_deleted()
            } else {
                signed_out()
            });
        };
        if expires_at <= now || device.revoked.is_some() {
            return Err(signed_out());
        }
        self.records
            .seen(&device.id, now)
            .await
            .map_err(unreadable)?;
        self.relay.meter().active(&device.account, &device.id, now);
        Ok(Caller {
            account: device.account,
            device: device.id,
            kind: device.kind,
        })
    }

    /// `GET /v1/devices`: the caller's account's devices, newest first.
    pub async fn devices(&self, caller: &Caller) -> Result<DevicePage, RemoteError> {
        let mut devices = self
            .records
            .devices(&caller.account)
            .await
            .map_err(unreadable)?;
        devices.sort_by_key(|d| std::cmp::Reverse(d.created_at));
        Ok(DevicePage {
            devices: devices
                .into_iter()
                .map(|d| DeviceSummary {
                    current: d.id == caller.device,
                    id: d.id,
                    name: d.name,
                    kind: d.kind,
                    platform: d.platform,
                    created_at: d.created_at,
                    last_seen_at: d.last_seen_at,
                    revoked_at: d.revoked.map(|r| r.at),
                })
                .collect(),
        })
    }

    /// `POST /v1/devices/{id}/revoke`. Its credentials stop working at once. Another account's
    /// device is `not_found`, exactly as one that does not exist.
    pub async fn revoke(
        &self,
        caller: &Caller,
        device: &DeviceId,
        now: i64,
    ) -> Result<(), RemoteError> {
        let revoked = self
            .records
            .revoke(
                &caller.account,
                device,
                Revoked {
                    at: now,
                    reason: RevokeReason::ByOwner,
                },
            )
            .await
            .map_err(unreadable)?;
        if revoked {
            self.revoked(&caller.account).await;
            Ok(())
        } else {
            Err(RemoteError::not_found())
        }
    }

    /// `POST /v1/account/delete`: delete the caller's account, confirmed by the person having
    /// just signed in again as it, from this device. See [`remote_proto::delete`].
    pub async fn delete_account(
        &self,
        caller: &Caller,
        request: &DeleteRequest,
        now: i64,
    ) -> Result<AccountDeleted, RemoteError> {
        request.validate()?;
        let confirmed = self
            .confirmed(&request.code, &request.verifier, now)
            .await?;
        if confirmed != caller.account {
            return Err(RemoteError::new(
                ErrorCode::Forbidden,
                "You signed in again as someone else, not as this device's account. Nothing was \
                 deleted.",
            ));
        }
        self.delete(&confirmed, now).await
    }

    /// The account a confirming sign-in's code was for, the code spent in the same step. Only
    /// one from such a sign-in, unexpired, answered with the verifier that started it.
    pub(crate) async fn confirmed(
        &self,
        code: &Secret,
        verifier: &Secret,
        now: i64,
    ) -> Result<AccountId, RemoteError> {
        let refused = || {
            unauthenticated(
                "That confirmation has expired or was already used. Sign in again to confirm.",
            )
        };
        let hash = tokens::presented(Kind::Handoff, code.expose()).ok_or_else(refused)?;
        // Taken first, so a wrong verifier spends it too.
        let handoff = self
            .records
            .take_handoff(&hash)
            .await
            .map_err(unreadable)?
            .ok_or_else(refused)?;
        if !handoff.confirm
            || handoff.expires_at <= now
            || !tokens::verifies(&handoff.device_challenge, verifier.expose())
        {
            return Err(refused());
        }
        Ok(handoff.account)
    }

    /// Delete `account` and everything of it, then close its links on every instance.
    pub(crate) async fn delete(
        &self,
        account: &AccountId,
        now: i64,
    ) -> Result<AccountDeleted, RemoteError> {
        let deleted = self
            .records
            .delete_account(account, now)
            .await
            .map_err(unreadable)?
            // Deleted meanwhile, by another of its devices.
            .ok_or_else(account_deleted)?;
        self.revoked(account).await;
        tracing::info!(
            devices = deleted.devices,
            hosts = deleted.hosts,
            records = deleted.records,
            "an account was deleted by its person"
        );
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Ok(AccountDeleted {
            devices: count(deleted.devices),
            hosts: count(deleted.hosts),
            records: count(deleted.records),
        })
    }

    /// Whether `credential` was one of a deleted account's devices'.
    async fn deleted(&self, credential: &tokens::TokenHash, now: i64) -> Result<bool, RemoteError> {
        self.records
            .deleted_credential(credential, now)
            .await
            .map_err(unreadable)
    }

    fn allowed_redirect(&self, asked: &str) -> Option<Url> {
        let asked = Url::parse(asked).ok()?;
        if asked.fragment().is_some() || !asked.username().is_empty() || asked.password().is_some()
        {
            return None;
        }
        self.redirects
            .iter()
            .any(|rule| redirect_matches(rule, &asked))
            .then_some(asked)
    }
}

/// Exactly the same address, except that a loopback rule matches any port.
fn redirect_matches(rule: &Url, asked: &Url) -> bool {
    let loopback = |url: &Url| {
        let host = match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        url.scheme() == "http" && host
    };
    if loopback(rule) && loopback(asked) {
        rule.host() == asked.host() && rule.path() == asked.path() && rule.query() == asked.query()
    } else {
        rule.as_str() == asked.as_str()
    }
}

/// New credentials: the hashes to keep, and the pair to hand out.
fn issue(now: i64) -> (Issued, Pair) {
    let access = tokens::mint(Kind::Access);
    let refresh = tokens::mint(Kind::Refresh);
    (
        Issued {
            access: access.hash,
            access_expires_at: now + ACCESS_TTL_MS,
            refresh: refresh.hash,
            refresh_expires_at: now + REFRESH_TTL_MS,
        },
        Pair {
            access: access.secret,
            refresh: refresh.secret,
        },
    )
}

struct Pair {
    access: Secret,
    refresh: Secret,
}

impl Pair {
    fn for_device(self, device: DeviceId) -> Credentials {
        Credentials {
            device,
            access: self.access,
            access_expires_in_ms: ACCESS_TTL_MS as u64,
            refresh: self.refresh,
            email: None,
        }
    }
}

fn with_query(url: &Url, key: &str, value: &str) -> Url {
    let mut url = url.clone();
    url.query_pairs_mut().append_pair(key, value);
    url
}

fn error_code(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::Unauthenticated => "unauthenticated",
        ErrorCode::InvalidRequest => "invalid_request",
        _ => "internal",
    }
}

fn unauthenticated(message: &'static str) -> RemoteError {
    RemoteError::new(ErrorCode::Unauthenticated, message)
}

/// A deleted account's device, asking.
pub(crate) fn account_deleted() -> RemoteError {
    unauthenticated(
        "This account was deleted from the server, so this device is signed out. Nothing on the \
         device was deleted.",
    )
    .because(ErrorReason::AccountDeleted)
}

fn provider_unavailable() -> RemoteError {
    RemoteError::new(
        ErrorCode::Internal,
        "The server could not reach its identity provider. Try again in a moment.",
    )
}

/// The records would not answer. Their error is logged here and stays on the server.
pub(crate) fn unreadable(error: impl std::fmt::Display) -> RemoteError {
    tracing::error!("records: {error}");
    RemoteError::new(
        ErrorCode::Internal,
        "The server could not read its own records. Try again in a moment.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirects_match_exactly_except_a_loopback_port() {
        let url = |text: &str| Url::parse(text).unwrap();
        let app = url("yardsort://signed-in");
        assert!(redirect_matches(&app, &url("yardsort://signed-in")));
        assert!(!redirect_matches(
            &app,
            &url("yardsort://signed-in/elsewhere")
        ));
        assert!(!redirect_matches(&app, &url("yardsort://signed-in?next=x")));
        let computer = url("http://127.0.0.1/signed-in");
        assert!(redirect_matches(
            &computer,
            &url("http://127.0.0.1:49152/signed-in")
        ));
        assert!(!redirect_matches(
            &computer,
            &url("http://127.0.0.1:49152/other")
        ));
        assert!(!redirect_matches(
            &computer,
            &url("http://[::1]:49152/signed-in")
        ));
        assert!(!redirect_matches(
            &computer,
            &url("https://127.0.0.1:49152/signed-in")
        ));
        let web = url("https://app.example.invalid/signed-in");
        assert!(!redirect_matches(
            &web,
            &url("https://app.example.invalid:8443/signed-in")
        ));
    }
}
