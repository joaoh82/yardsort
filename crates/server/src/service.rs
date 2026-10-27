//! Signing in, refreshing, and an account's devices, over any [`Records`].

use remote_proto::account::{
    Credentials, DeviceKind, DevicePage, DeviceSummary, LoginParams, Secret, ServerDescription,
    TokenRequest,
};
use remote_proto::{DeviceId, ErrorCode, ProtocolVersion, RemoteError, ServerId};
use url::Url;

use crate::oidc::{OidcError, Provider, ProviderConfig};
use crate::records::{
    AccountId, Device, Handoff, Issued, PendingLogin, Records, RevokeReason, Revoked, Rotation,
};
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
    callback: Url,
    redirects: Vec<Url>,
    provider: Provider,
    pub(crate) records: R,
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
            callback,
            redirects,
            records,
        })
    }

    pub fn records(&self) -> &R {
        &self.records
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
                    device_challenge: params.challenge.clone(),
                    device_redirect: redirect.to_string(),
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
        let account = match self
            .records
            .account(&identity.issuer, &identity.subject, now)
            .await
        {
            Ok(account) => account,
            Err(error) => return refuse(ErrorCode::Internal, error.to_string()),
        };
        let handoff = tokens::mint(Kind::Handoff);
        let kept = self
            .records
            .put_handoff(
                Handoff {
                    code: handoff.hash,
                    account,
                    device_challenge: login.device_challenge,
                    expires_at: now + HANDOFF_TTL_MS,
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
                if handoff.expires_at <= now
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
                let id = DeviceId::new(uuid::Uuid::new_v4().to_string()).map_err(unreadable)?;
                let (issued, pair) = issue(now);
                let credentials = pair.for_device(id.clone());
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
                    Rotation::Rotated(device) => Ok(pair.for_device(device.id)),
                    Rotation::Reused => Err(unauthenticated(
                        "This device's sign-in was used from somewhere else, so the device was \
                         signed out. Sign in again.",
                    )),
                    Rotation::Expired | Rotation::Revoked | Rotation::Unknown => Err(signed_out()),
                }
            }
        }
    }

    /// The device an `Authorization: Bearer` access credential belongs to, if it may act.
    pub async fn authenticate(&self, access: &str, now: i64) -> Result<Caller, RemoteError> {
        let signed_out = || unauthenticated("Sign in again.");
        let hash = tokens::presented(Kind::Access, access).ok_or_else(signed_out)?;
        let (device, expires_at) = self
            .records
            .access(&hash)
            .await
            .map_err(unreadable)?
            .ok_or_else(signed_out)?;
        if expires_at <= now || device.revoked.is_some() {
            return Err(signed_out());
        }
        self.records
            .seen(&device.id, now)
            .await
            .map_err(unreadable)?;
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
            Ok(())
        } else {
            Err(RemoteError::not_found())
        }
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
