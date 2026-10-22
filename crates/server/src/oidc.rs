//! The server as an OpenID Connect client of its identity provider.
//!
//! The authorization-code flow with PKCE ([OIDC Core §3.1], [RFC 7636]), and nothing else: no
//! implicit flow, no refresh tokens from the provider (only `openid` is asked for, never
//! `offline_access`), and no user information beyond the ID token's issuer and subject. The
//! provider's access token is never kept or passed on.
//!
//! An ID token is accepted only when all of these hold:
//!
//! - It is signed with `RS256` or `ES256` by a key in the provider's published key set, and the
//!   algorithm is the one that key's own type allows. A token cannot pick a weaker check: `none`,
//!   an HMAC with the public key, or another family is refused before its signature is looked at.
//! - `iss` is exactly the configured issuer, which discovery also had to state exactly.
//! - `aud` includes this server's client id, and `azp`, when present, is that client id. With
//!   more than one audience, `azp` must be present.
//! - `exp` has not passed and `iat` is not in the future, each within [`CLOCK_LEEWAY_MS`].
//! - `nonce` is the one this sign-in sent.
//!
//! The provider's endpoints must use HTTPS. Plain HTTP is accepted only on a loopback address,
//! for a provider running beside the server in development; there is no switch to skip
//! certificate checks.
//!
//! [OIDC Core §3.1]: https://openid.net/specs/openid-connect-core-1_0.html#CodeFlowAuth
//! [RFC 7636]: https://www.rfc-editor.org/rfc/rfc7636

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jsonwebtoken::jwk::{
    AlgorithmParameters, EllipticCurve, Jwk, JwkSet, KeyAlgorithm, PublicKeyUse,
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use remote_proto::account::Secret;
use serde::Deserialize;
use url::Url;

use crate::tokens::{self, TokenHash};

/// How far the provider's clock may be from ours.
pub const CLOCK_LEEWAY_MS: i64 = 60_000;
/// The least time between two fetches of the provider's keys, so a stream of tokens naming an
/// unknown key cannot make the server hammer the provider.
pub const KEYS_REFETCH_MS: i64 = 60_000;
/// For every request to the provider.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The identity provider, as the server's operator configured it.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Exactly as the provider states it: `https://auth.example.com`. Compared byte for byte.
    pub issuer: String,
    pub client_id: String,
    /// For a confidential client, sent with HTTP Basic authentication. `None` for a public client,
    /// which PKCE alone protects.
    pub client_secret: Option<Secret>,
}

/// Why the provider could not be used, for the server's log. Never sent to a device as it is.
#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("the identity provider could not be reached: {0}")]
    Unreachable(String),
    #[error("the identity provider is not usable as configured: {0}")]
    Misconfigured(String),
    #[error("the identity provider refused the code: {0}")]
    Refused(String),
    #[error("the ID token was not accepted: {0}")]
    Rejected(&'static str),
}

/// Who the provider says signed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub issuer: String,
    pub subject: String,
}

/// An HTTP client for talking to providers: no redirects followed, a timeout on everything, and
/// rustls with the `ring` provider (the one the app installs too).
pub fn http_client() -> Result<reqwest::Client, OidcError> {
    http_client_builder()
        .build()
        .map_err(|e| OidcError::Unreachable(e.to_string()))
}

/// [`http_client`], before it is built, for a caller that needs to add to it.
pub fn http_client_builder() -> reqwest::ClientBuilder {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .user_agent(concat!("Yardsort-server/", env!("CARGO_PKG_VERSION")))
}

pub struct Provider {
    config: ProviderConfig,
    http: reqwest::Client,
    discovery: Mutex<Option<Arc<Discovery>>>,
    keys: Mutex<Keys>,
}

#[derive(Debug)]
struct Discovery {
    authorization_endpoint: Url,
    token_endpoint: Url,
    jwks_uri: Url,
}

#[derive(Default)]
struct Keys {
    set: Option<Arc<JwkSet>>,
    fetched_at: Option<i64>,
}

#[derive(Deserialize)]
struct DiscoveryDocument {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    #[serde(default)]
    code_challenge_methods_supported: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    aud: Audience,
    exp: i64,
    iat: i64,
    #[serde(default)]
    nbf: Option<i64>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    azp: Option<String>,
}

/// How many audiences there are. Which they are is checked by `jsonwebtoken`.
#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    Many(Vec<serde::de::IgnoredAny>),
    One(serde::de::IgnoredAny),
}

impl Audience {
    fn len(&self) -> usize {
        match self {
            Self::Many(all) => all.len(),
            Self::One(_) => 1,
        }
    }
}

impl Provider {
    pub fn new(config: ProviderConfig, http: reqwest::Client) -> Result<Self, OidcError> {
        let issuer = Url::parse(&config.issuer)
            .map_err(|e| OidcError::Misconfigured(format!("issuer: {e}")))?;
        secure(&issuer)?;
        Ok(Self {
            config,
            http,
            discovery: Mutex::new(None),
            keys: Mutex::new(Keys::default()),
        })
    }

    pub fn issuer(&self) -> &str {
        &self.config.issuer
    }

    /// Where to send the browser: the provider's authorization endpoint, asking for a code for
    /// `redirect_uri`, bound to `state`, `nonce` and the PKCE `challenge`.
    pub async fn authorize_url(
        &self,
        redirect_uri: &Url,
        state: &str,
        nonce: &str,
        challenge: &str,
    ) -> Result<Url, OidcError> {
        let discovery = self.discovery().await?;
        let mut url = discovery.authorization_endpoint.clone();
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", redirect_uri.as_str())
            .append_pair("scope", "openid")
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256");
        Ok(url)
    }

    /// Trade the provider's code for its ID token, and check the token. Only the identity in it
    /// is kept.
    pub async fn sign_in(
        &self,
        code: &str,
        redirect_uri: &Url,
        verifier: &Secret,
        nonce: &TokenHash,
        now: i64,
    ) -> Result<Identity, OidcError> {
        let discovery = self.discovery().await?;
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_verifier", verifier.expose()),
        ];
        let mut request = self.http.post(discovery.token_endpoint.clone());
        match &self.config.client_secret {
            // RFC 6749 §2.3.1: both parts are form-encoded before Basic encoding.
            Some(secret) => {
                request = request.basic_auth(
                    form_encode(&self.config.client_id),
                    Some(form_encode(secret.expose())),
                );
            }
            None => form.push(("client_id", &self.config.client_id)),
        }
        let response = request
            .form(&form)
            .send()
            .await
            .map_err(|e| OidcError::Unreachable(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            // The body is the provider's error; its code is enough, and safe to log.
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            let error = body["error"].as_str().unwrap_or("no error code").to_owned();
            return Err(OidcError::Refused(format!("{status}: {error}")));
        }
        let body: TokenResponse = response
            .json()
            .await
            .map_err(|_| OidcError::Refused("the answer was not a token response".into()))?;
        let id_token = body
            .id_token
            .ok_or(OidcError::Refused("the answer had no ID token".into()))?;
        self.verify(&id_token, nonce, now).await
    }

    async fn verify(
        &self,
        token: &str,
        nonce: &TokenHash,
        now: i64,
    ) -> Result<Identity, OidcError> {
        let header =
            jsonwebtoken::decode_header(token).map_err(|_| OidcError::Rejected("malformed"))?;
        let kid = header
            .kid
            .as_deref()
            .ok_or(OidcError::Rejected("no key id"))?;
        let jwk = match self.key(kid, now, false).await? {
            Some(jwk) => jwk,
            // The provider may have rotated its keys since they were fetched.
            None => self.key(kid, now, true).await?.ok_or(OidcError::Rejected(
                "signed by a key the provider does not publish",
            ))?,
        };
        let algorithm = algorithm_for(&jwk).ok_or(OidcError::Rejected("unsupported key"))?;
        if header.alg != algorithm {
            return Err(OidcError::Rejected("algorithm does not match the key"));
        }
        let key = DecodingKey::from_jwk(&jwk).map_err(|_| OidcError::Rejected("unusable key"))?;

        let mut validation = Validation::new(algorithm);
        validation.set_issuer(&[&self.config.issuer]);
        validation.set_audience(&[&self.config.client_id]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        // Times are checked below, against the caller's clock.
        validation.validate_exp = false;
        validation.validate_nbf = false;
        let claims = jsonwebtoken::decode::<Claims>(token, &key, &validation)
            .map_err(|_| OidcError::Rejected("signature, issuer or audience"))?
            .claims;

        if claims.iss != self.config.issuer {
            return Err(OidcError::Rejected("issuer"));
        }
        if claims.exp.saturating_mul(1000) + CLOCK_LEEWAY_MS <= now {
            return Err(OidcError::Rejected("expired"));
        }
        if claims.iat.saturating_mul(1000) - CLOCK_LEEWAY_MS > now {
            return Err(OidcError::Rejected("issued in the future"));
        }
        if claims
            .nbf
            .is_some_and(|nbf| nbf.saturating_mul(1000) - CLOCK_LEEWAY_MS > now)
        {
            return Err(OidcError::Rejected("not valid yet"));
        }
        match &claims.azp {
            Some(azp) if *azp != self.config.client_id => {
                return Err(OidcError::Rejected("issued to another client"))
            }
            None if claims.aud.len() > 1 => {
                return Err(OidcError::Rejected("several audiences and no azp"))
            }
            _ => {}
        }
        let sent = claims
            .nonce
            .as_deref()
            .ok_or(OidcError::Rejected("no nonce"))?;
        if !tokens::same(&tokens::hash(sent), nonce) {
            return Err(OidcError::Rejected("nonce"));
        }
        if claims.sub.is_empty() || claims.sub.len() > 255 {
            return Err(OidcError::Rejected("subject"));
        }
        Ok(Identity {
            issuer: claims.iss,
            subject: claims.sub,
        })
    }

    /// The provider's key `kid`. With `refetch`, the key set is fetched again first, unless it
    /// was fetched less than [`KEYS_REFETCH_MS`] ago.
    async fn key(&self, kid: &str, now: i64, refetch: bool) -> Result<Option<Jwk>, OidcError> {
        let (cached, fetched_at) = {
            let keys = self.keys.lock().expect("keys lock");
            (keys.set.clone(), keys.fetched_at)
        };
        let recent = fetched_at.is_some_and(|at| now - at < KEYS_REFETCH_MS);
        let set = match cached {
            Some(set) if !refetch || recent => set,
            _ => {
                let discovery = self.discovery().await?;
                let set: JwkSet = self.get_json(&discovery.jwks_uri).await?;
                let set = Arc::new(set);
                let mut keys = self.keys.lock().expect("keys lock");
                keys.set = Some(set.clone());
                keys.fetched_at = Some(now);
                set
            }
        };
        Ok(set.find(kid).cloned())
    }

    async fn discovery(&self) -> Result<Arc<Discovery>, OidcError> {
        if let Some(found) = self.discovery.lock().expect("discovery lock").clone() {
            return Ok(found);
        }
        let issuer = &self.config.issuer;
        let url = Url::parse(&format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        ))
        .map_err(|e| OidcError::Misconfigured(e.to_string()))?;
        let document: DiscoveryDocument = self.get_json(&url).await?;
        // OpenID Connect Discovery §4.3: exactly the issuer asked about.
        if document.issuer != *issuer {
            return Err(OidcError::Misconfigured(format!(
                "discovery names the issuer {:?}, not {issuer:?}",
                document.issuer
            )));
        }
        if let Some(methods) = &document.code_challenge_methods_supported {
            if !methods.iter().any(|m| m == "S256") {
                return Err(OidcError::Misconfigured(
                    "PKCE S256 is not supported".into(),
                ));
            }
        }
        let endpoint = |name: &str, text: &str| {
            let url =
                Url::parse(text).map_err(|e| OidcError::Misconfigured(format!("{name}: {e}")))?;
            secure(&url)?;
            Ok::<_, OidcError>(url)
        };
        let found = Arc::new(Discovery {
            authorization_endpoint: endpoint(
                "authorization_endpoint",
                &document.authorization_endpoint,
            )?,
            token_endpoint: endpoint("token_endpoint", &document.token_endpoint)?,
            jwks_uri: endpoint("jwks_uri", &document.jwks_uri)?,
        });
        *self.discovery.lock().expect("discovery lock") = Some(found.clone());
        Ok(found)
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &Url) -> Result<T, OidcError> {
        let response = self
            .http
            .get(url.clone())
            .send()
            .await
            .map_err(|e| OidcError::Unreachable(e.to_string()))?;
        if !response.status().is_success() {
            return Err(OidcError::Unreachable(format!(
                "{url} answered {}",
                response.status()
            )));
        }
        response
            .json()
            .await
            .map_err(|e| OidcError::Misconfigured(format!("{url}: {e}")))
    }
}

/// The one algorithm a key may be used with: `RS256` for an RSA key, `ES256` for a P-256 key.
/// Anything else — a shared secret, another curve, a key marked for encryption or for another
/// algorithm — is not used at all.
fn algorithm_for(jwk: &Jwk) -> Option<Algorithm> {
    if matches!(&jwk.common.public_key_use, Some(u) if *u != PublicKeyUse::Signature) {
        return None;
    }
    let algorithm = match &jwk.algorithm {
        AlgorithmParameters::RSA(_) => Algorithm::RS256,
        AlgorithmParameters::EllipticCurve(ec) if ec.curve == EllipticCurve::P256 => {
            Algorithm::ES256
        }
        _ => return None,
    };
    let stated = jwk.common.key_algorithm;
    let agrees = match stated {
        None => true,
        Some(KeyAlgorithm::RS256) => algorithm == Algorithm::RS256,
        Some(KeyAlgorithm::ES256) => algorithm == Algorithm::ES256,
        Some(_) => false,
    };
    agrees.then_some(algorithm)
}

/// HTTPS, or plain HTTP to a loopback address.
fn secure(url: &Url) -> Result<(), OidcError> {
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        _ => false,
    };
    match url.scheme() {
        "https" => Ok(()),
        "http" if loopback => Ok(()),
        _ => Err(OidcError::Misconfigured(format!(
            "{url} is not HTTPS (plain HTTP is accepted on a loopback address only)"
        ))),
    }
}

fn form_encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwk(json: serde_json::Value) -> Jwk {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn a_key_decides_its_own_algorithm() {
        let rsa = |extra: serde_json::Value| {
            let mut key = serde_json::json!({"kty": "RSA", "kid": "k", "n": "AQAB", "e": "AQAB"});
            key.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            jwk(key)
        };
        assert_eq!(
            algorithm_for(&rsa(serde_json::json!({}))),
            Some(Algorithm::RS256)
        );
        assert_eq!(
            algorithm_for(&rsa(serde_json::json!({"alg": "RS256", "use": "sig"}))),
            Some(Algorithm::RS256)
        );
        assert_eq!(
            algorithm_for(&rsa(serde_json::json!({"alg": "RS512"}))),
            None
        );
        assert_eq!(algorithm_for(&rsa(serde_json::json!({"use": "enc"}))), None);
        let secret = jwk(serde_json::json!({"kty": "oct", "kid": "k", "k": "c2VjcmV0"}));
        assert_eq!(
            algorithm_for(&secret),
            None,
            "a shared secret is never a provider key"
        );
    }

    #[test]
    fn only_https_or_loopback_http() {
        let ok = |text: &str| secure(&Url::parse(text).unwrap()).is_ok();
        assert!(ok("https://auth.example.invalid"));
        assert!(ok("http://127.0.0.1:8080"));
        assert!(ok("http://[::1]:8080"));
        assert!(!ok("http://auth.example.invalid"));
        assert!(!ok("http://localhost:8080"), "a name can resolve anywhere");
        assert!(!ok("http://10.0.0.5"));
    }
}
