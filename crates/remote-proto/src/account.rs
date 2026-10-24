//! Signing in to a server, and the credentials it gives each device.
//!
//! A person signs in at the identity provider their server is set up with, in the system browser,
//! once per device. The server is that provider's client, not the app: it sends the browser to the
//! provider, receives the provider's answer and checks its ID token. The device never sees a token
//! the provider issued. What it gets is the server's own credential, which it refreshes with the
//! server alone, so the provider hears from a device only when its person signs in. Revoking a
//! device is then the server's decision, taken at once, rather than one waiting on the provider.
//!
//! ```text
//! device   GET  /v1/login?challenge&challengeMethod&redirect    browser → server → provider
//! provider GET  /v1/login/callback?code&state                   browser → server → device redirect
//! device   POST /v1/token  {grant: signIn, code, verifier, device}   → Credentials
//! device   POST /v1/token  {grant: refresh, refresh}                 → Credentials
//! device   GET  /v1/devices                                          → DevicePage
//! device   POST /v1/devices/{id}/revoke                              → (nothing)
//! ```
//!
//! The device proves it is the one that started a sign-in with PKCE ([RFC 7636]): it keeps a
//! random verifier, sends only its SHA-256 (`challenge`) when it starts, and the verifier when it
//! redeems the one-time code the server sent to its redirect. A code read from a log, a history or
//! another app's redirect is worth nothing without the verifier.
//!
//! [RFC 7636]: https://www.rfc-editor.org/rfc/rfc7636

use serde::{Deserialize, Serialize};

use crate::ids::{DeviceId, ServerId};
use crate::model::{Platform, MAX_TEXT};
use crate::{ErrorCode, ProtocolVersion, RemoteError};

/// The only challenge method accepted: the SHA-256 of the verifier. `plain` would send the
/// verifier itself through the browser.
pub const CHALLENGE_METHOD: &str = "S256";
/// Length of an S256 challenge: 32 bytes of SHA-256, base64url without padding.
pub const CHALLENGE_LEN: usize = 43;
/// A verifier's length bounds ([RFC 7636 §4.1](https://www.rfc-editor.org/rfc/rfc7636#section-4.1)).
pub const MIN_VERIFIER_LEN: usize = 43;
pub const MAX_VERIFIER_LEN: usize = 128;
/// Longest redirect a device may name.
pub const MAX_REDIRECT_LEN: usize = 512;
/// Longest one-time code or refresh credential accepted. The server issues far shorter ones.
pub const MAX_SECRET_LEN: usize = 256;

/// `GET /v1/server`: who this server is and how to sign in to it. Public: asked before signing in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerDescription {
    pub server: ServerId,
    /// What its operator calls it.
    pub name: String,
    pub protocol: ProtocolVersion,
    /// The identity provider people sign in with, as its issuer URL. Shown at sign-in; an
    /// account is this issuer's subject on this server, never an email address.
    pub issuer: String,
    /// What this server offers beyond signing in, by operation name.
    pub capabilities: Vec<String>,
}

/// `GET /v1/login`, as query parameters: where the device starts signing in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginParams {
    /// The SHA-256 of the device's verifier, base64url without padding.
    pub challenge: String,
    /// Always [`CHALLENGE_METHOD`].
    pub challenge_method: String,
    /// Where the browser goes back to the device: one of the redirects the server allows, such
    /// as the app's own scheme, or a loopback address on any port for a computer.
    pub redirect: String,
}

impl LoginParams {
    pub fn validate(&self) -> Result<(), RemoteError> {
        if self.challenge_method != CHALLENGE_METHOD {
            return Err(invalid("Only S256 challenges are accepted."));
        }
        if self.challenge.len() != CHALLENGE_LEN || !unreserved(&self.challenge) {
            return Err(invalid("The challenge is not an S256 challenge."));
        }
        if self.redirect.is_empty() || self.redirect.len() > MAX_REDIRECT_LEN {
            return Err(invalid("The redirect is missing or too long."));
        }
        Ok(())
    }
}

/// `POST /v1/token`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "grant", rename_all = "camelCase")]
pub enum TokenRequest {
    /// Redeem the one-time code a sign-in sent to the device's redirect. Enrolls the device.
    #[serde(rename_all = "camelCase")]
    SignIn {
        code: Secret,
        /// The verifier whose SHA-256 started the sign-in.
        verifier: Secret,
        device: NewDevice,
    },
    /// Trade a refresh credential for new credentials. Each refresh credential works once: the
    /// answer carries its replacement, and presenting a replaced one revokes the device, since
    /// only a copy could still hold it.
    #[serde(rename_all = "camelCase")]
    Refresh { refresh: Secret },
}

impl TokenRequest {
    pub fn validate(&self) -> Result<(), RemoteError> {
        match self {
            Self::SignIn {
                code,
                verifier,
                device,
            } => {
                if code.0.is_empty() || code.0.len() > MAX_SECRET_LEN {
                    return Err(invalid("The sign-in code is missing or too long."));
                }
                let v = &verifier.0;
                if !(MIN_VERIFIER_LEN..=MAX_VERIFIER_LEN).contains(&v.len()) || !unreserved(v) {
                    return Err(invalid("The verifier is not a PKCE verifier."));
                }
                let name = device.name.trim();
                if name.is_empty()
                    || name.chars().count() > MAX_TEXT
                    || name.chars().any(char::is_control)
                {
                    return Err(invalid(
                        "A device name is one line of at most 256 characters.",
                    ));
                }
                Ok(())
            }
            Self::Refresh { refresh } => {
                if refresh.0.is_empty() || refresh.0.len() > MAX_SECRET_LEN {
                    return Err(invalid("The refresh credential is missing or too long."));
                }
                Ok(())
            }
        }
    }
}

/// The device being enrolled, as it describes itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewDevice {
    /// What its person will recognise it by in a list: "Ana's phone".
    pub name: String,
    pub kind: DeviceKind,
    pub platform: Platform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    /// The mobile app.
    Phone,
    /// Another computer, as a client of hosts.
    Computer,
    /// A machine that runs agents and connects out to the server for others to reach.
    Host,
    #[serde(other)]
    Other,
}

/// What a device holds after signing in or refreshing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    pub device: DeviceId,
    /// Sent as `Authorization: Bearer …` with each request.
    pub access: Secret,
    /// How long `access` works for, from when it was issued. A duration rather than a time, so a
    /// device's wrong clock does not matter.
    pub access_expires_in_ms: u64,
    /// Kept in the device's credential store; never in a log, a URL or a terminal's environment.
    pub refresh: Secret,
}

/// `GET /v1/devices`: the signed-in account's devices, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePage {
    pub devices: Vec<DeviceSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSummary {
    pub id: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
    pub platform: Platform,
    pub created_at: i64,
    /// When it last used its access, as far as the server noticed. Historical, not presence.
    pub last_seen_at: Option<i64>,
    /// When it was revoked; a revoked device is listed until it is forgotten.
    pub revoked_at: Option<i64>,
    /// The device asking.
    pub current: bool,
}

/// A credential, as it crosses the wire. `Debug` shows that there is one and nothing of it, so a
/// request or a reply can be logged whole without leaking it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The credential itself, for the one place that needs it: sending it, or hashing it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// The characters RFC 7636 allows in a verifier, which a base64url challenge also keeps to.
fn unreserved(text: &str) -> bool {
    text.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

fn invalid(message: &'static str) -> RemoteError {
    RemoteError::new(ErrorCode::InvalidRequest, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign_in(code: &str, verifier: &str, name: &str) -> TokenRequest {
        TokenRequest::SignIn {
            code: Secret::new(code),
            verifier: Secret::new(verifier),
            device: NewDevice {
                name: name.into(),
                kind: DeviceKind::Phone,
                platform: Platform::Ios,
            },
        }
    }

    #[test]
    fn a_secret_is_never_printed() {
        let request = sign_in("the-code", &"v".repeat(43), "Phone");
        let printed = format!("{request:?}");
        assert!(!printed.contains("the-code"), "{printed}");
        assert!(!printed.contains("vvvv"), "{printed}");
    }

    #[test]
    fn only_s256_challenges_of_the_right_shape_start_a_sign_in() {
        let params = |challenge: &str, method: &str| LoginParams {
            challenge: challenge.into(),
            challenge_method: method.into(),
            redirect: "yardsort://signed-in".into(),
        };
        let good = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(params(good, "S256").validate().is_ok());
        assert!(params(good, "plain").validate().is_err());
        assert!(params(&good[1..], "S256").validate().is_err());
        assert!(params(&format!("{}+", &good[1..]), "S256")
            .validate()
            .is_err());
    }

    #[test]
    fn a_verifier_and_a_device_name_are_bounded() {
        let verifier = "v".repeat(MIN_VERIFIER_LEN);
        assert!(sign_in("c", &verifier, "Phone").validate().is_ok());
        assert!(sign_in("c", &"v".repeat(42), "Phone").validate().is_err());
        assert!(sign_in("c", &"v".repeat(129), "Phone").validate().is_err());
        assert!(sign_in("c", &verifier, " ").validate().is_err());
        assert!(sign_in("c", &verifier, "two\nlines").validate().is_err());
        assert!(sign_in("", &verifier, "Phone").validate().is_err());
    }
}
