//! Handing a notice to APNs or FCM. The provider is a trait so a test can record what would
//! have been sent, and a server with no credentials queues notices instead of refusing them.
//!
//! A payload is the title, the body, the deep link and the category. It never carries a token,
//! an event key, or a one-time code. A failure from the provider is not logged with the request
//! address, because that address contains the device token.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use remote_proto::notify::PushPlatform;
use serde::Serialize;

/// What sending one notice came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    Sent,
    /// The phone or the provider could not be reached. The notice stays queued.
    Unreachable,
    /// The token will never work again.
    Gone,
}

/// One notice, as the provider sends it. The token is not part of [`Debug`].
pub struct PushMessage {
    pub platform: PushPlatform,
    pub token: String,
    pub collapse: String,
    pub title: String,
    pub body: String,
    pub deep_link: String,
    pub category: String,
}

impl std::fmt::Debug for PushMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushMessage")
            .field("platform", &self.platform)
            .field("collapse", &self.collapse)
            .field("title", &self.title)
            .field("category", &self.category)
            .finish_non_exhaustive()
    }
}

/// The JSON a phone is shown. Fixed fields only.
pub fn alert_json(message: &PushMessage) -> String {
    serde_json::json!({
        "title": message.title,
        "body": message.body,
        "deepLink": message.deep_link,
        "category": message.category,
    })
    .to_string()
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait PushProvider: Send + Sync {
    fn send(&self, message: PushMessage) -> BoxFuture<'_, PushOutcome>;
}

/// Queues every notice. Used when APNs and FCM are not configured.
pub struct DisabledPush;

impl PushProvider for DisabledPush {
    fn send(&self, _message: PushMessage) -> BoxFuture<'_, PushOutcome> {
        Box::pin(async { PushOutcome::Unreachable })
    }
}

/// Records the alert JSON of each accepted send. `outcome` is what the next send reports.
pub struct RecordingProvider {
    outcome: Mutex<PushOutcome>,
    sent: Mutex<Vec<SentAlert>>,
    attempts: AtomicUsize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentAlert {
    pub json: String,
    pub collapse: String,
}

impl RecordingProvider {
    pub fn new(outcome: PushOutcome) -> Self {
        Self {
            outcome: Mutex::new(outcome),
            sent: Mutex::new(Vec::new()),
            attempts: AtomicUsize::new(0),
        }
    }

    /// How many sends have been attempted, whatever they came to.
    pub fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }

    pub fn set_outcome(&self, outcome: PushOutcome) {
        *self.outcome.lock().unwrap_or_else(|p| p.into_inner()) = outcome;
    }

    pub fn sent(&self) -> Vec<SentAlert> {
        self.sent.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl PushProvider for RecordingProvider {
    fn send(&self, message: PushMessage) -> BoxFuture<'_, PushOutcome> {
        let outcome = *self.outcome.lock().unwrap_or_else(|p| p.into_inner());
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if outcome == PushOutcome::Sent {
            self.sent
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(SentAlert {
                    json: alert_json(&message),
                    collapse: message.collapse,
                });
        }
        Box::pin(async move { outcome })
    }
}

/// APNs, FCM, or both. A platform that is not configured answers [`PushOutcome::Unreachable`].
pub struct ConfiguredPush {
    http: reqwest::Client,
    apns: Option<Apns>,
    fcm: Option<Fcm>,
}

pub struct Apns {
    key_id: String,
    team_id: String,
    bundle_id: String,
    sandbox: bool,
    key: jsonwebtoken::EncodingKey,
    jwt: Mutex<Option<(String, i64)>>,
}

pub struct Fcm {
    pub project_id: String,
    client_email: String,
    key: jsonwebtoken::EncodingKey,
    jwt: Mutex<Option<(String, i64)>>,
}

impl ConfiguredPush {
    pub fn new(http: reqwest::Client, apns: Option<Apns>, fcm: Option<Fcm>) -> Self {
        Self { http, apns, fcm }
    }
}

impl PushProvider for ConfiguredPush {
    fn send(&self, message: PushMessage) -> BoxFuture<'_, PushOutcome> {
        Box::pin(async move {
            match message.platform {
                PushPlatform::Apns => match &self.apns {
                    Some(apns) => apns.send(&self.http, &message).await,
                    None => PushOutcome::Unreachable,
                },
                PushPlatform::Fcm => match &self.fcm {
                    Some(fcm) => fcm.send(&self.http, &message).await,
                    None => PushOutcome::Unreachable,
                },
            }
        })
    }
}

impl Apns {
    pub fn parse(
        key_id: String,
        team_id: String,
        bundle_id: String,
        sandbox: bool,
        pem: &str,
    ) -> Option<Self> {
        let key = jsonwebtoken::EncodingKey::from_ec_pem(pem.as_bytes()).ok()?;
        if !signs(&key, jsonwebtoken::Algorithm::ES256) {
            return None;
        }
        Some(Self {
            key_id,
            team_id,
            bundle_id,
            sandbox,
            key,
            jwt: Mutex::new(None),
        })
    }

    async fn send(&self, http: &reqwest::Client, message: &PushMessage) -> PushOutcome {
        let Some(jwt) = self.bearer() else {
            return PushOutcome::Unreachable;
        };
        let host = if self.sandbox {
            "https://api.sandbox.push.apple.com"
        } else {
            "https://api.push.apple.com"
        };
        let url = format!("{host}/3/device/{}", message.token);
        let body = serde_json::json!({
            "aps": { "alert": { "title": message.title, "body": message.body } },
            "deepLink": message.deep_link,
            "category": message.category,
        });
        let response = http
            .post(url)
            .bearer_auth(jwt)
            .header("apns-topic", &self.bundle_id)
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .header("apns-collapse-id", &message.collapse)
            .json(&body)
            .send()
            .await;
        match response {
            Ok(response) => {
                let status = response.status().as_u16();
                let body = response.text().await.unwrap_or_default();
                if self.provider_token_rejected(status, &body) {
                    tracing::info!(status, "the APNs provider token was rejected");
                    return PushOutcome::Unreachable;
                }
                classify_apns(status, &body)
            }
            Err(_) => PushOutcome::Unreachable,
        }
    }

    /// A 403 whose reason is an expired or invalid provider token. The cached JWT is dropped so
    /// the next send signs a new one.
    fn provider_token_rejected(&self, status: u16, body: &str) -> bool {
        if status != 403 || !apns_reason_is(body, &["ExpiredProviderToken", "InvalidProviderToken"])
        {
            return false;
        }
        *self.jwt.lock().unwrap_or_else(|p| p.into_inner()) = None;
        true
    }

    fn bearer(&self) -> Option<String> {
        let now = unix_now();
        let mut slot = self.jwt.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((token, exp)) = slot.as_ref() {
            if *exp > now + 60 {
                return Some(token.clone());
            }
        }
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some(self.key_id.clone());
        let token = jsonwebtoken::encode(
            &header,
            &ApnsClaims {
                iss: self.team_id.clone(),
                iat: now,
            },
            &self.key,
        )
        .ok()?;
        *slot = Some((token.clone(), now + 50 * 60));
        Some(token)
    }
}

impl Fcm {
    pub fn parse(json: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        let project_id = value.get("project_id")?.as_str()?.to_owned();
        let client_email = value.get("client_email")?.as_str()?.to_owned();
        let pem = value.get("private_key")?.as_str()?;
        // A service-account key is PKCS#8. `from_rsa_pem` lifts the PKCS#1 key inside it, which
        // is what the signer accepts. A key that cannot sign is left unset.
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes()).ok()?;
        if project_id.is_empty()
            || client_email.is_empty()
            || !signs(&key, jsonwebtoken::Algorithm::RS256)
        {
            return None;
        }
        Some(Self {
            project_id,
            client_email,
            key,
            jwt: Mutex::new(None),
        })
    }

    async fn send(&self, http: &reqwest::Client, message: &PushMessage) -> PushOutcome {
        let Some(jwt) = self.bearer(http).await else {
            return PushOutcome::Unreachable;
        };
        let url = format!(
            "https://fcm.googleapis.com/v1/projects/{}/messages:send",
            self.project_id
        );
        let body = serde_json::json!({
            "message": {
                "token": message.token,
                "notification": { "title": message.title, "body": message.body },
                "data": { "deepLink": message.deep_link, "category": message.category },
                "android": { "collapse_key": message.collapse },
            }
        });
        let response = http.post(url).bearer_auth(jwt).json(&body).send().await;
        match response {
            Ok(response) => {
                let status = response.status().as_u16();
                let body = response.text().await.unwrap_or_default();
                classify_fcm(status, &body)
            }
            Err(_) => PushOutcome::Unreachable,
        }
    }

    async fn bearer(&self, http: &reqwest::Client) -> Option<String> {
        let now = unix_now();
        {
            let slot = self.jwt.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((token, exp)) = slot.as_ref() {
                if *exp > now + 60 {
                    return Some(token.clone());
                }
            }
        }
        let assertion = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &FcmClaims {
                iss: self.client_email.clone(),
                scope: "https://www.googleapis.com/auth/firebase.messaging".into(),
                aud: "https://oauth2.googleapis.com/token".into(),
                iat: now,
                exp: now + 3600,
            },
            &self.key,
        )
        .ok()?;
        let response = match http
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(error = %error, "the FCM token exchange could not be reached");
                return None;
            }
        };
        if !response.status().is_success() {
            tracing::warn!(
                status = %response.status(),
                "the FCM token exchange was refused"
            );
            return None;
        }
        let body: serde_json::Value = response.json().await.ok()?;
        let token = body.get("access_token")?.as_str()?.to_owned();
        *self.jwt.lock().unwrap_or_else(|p| p.into_inner()) = Some((token.clone(), now + 50 * 60));
        Some(token)
    }
}

#[derive(Serialize)]
struct ApnsClaims {
    iss: String,
    iat: i64,
}

#[derive(Serialize)]
struct FcmClaims {
    iss: String,
    scope: String,
    aud: String,
    iat: i64,
    exp: i64,
}

/// A key is usable when it can sign a token. A bad key is reported by the caller as unset.
fn signs(key: &jsonwebtoken::EncodingKey, algorithm: jsonwebtoken::Algorithm) -> bool {
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(algorithm),
        &serde_json::json!({ "sub": "yardsort" }),
        key,
    )
    .is_ok()
}

/// APNs: the token is gone only on 410 with a JSON reason of `BadDeviceToken` or `Unregistered`.
fn classify_apns(status: u16, body: &str) -> PushOutcome {
    if (200..300).contains(&status) {
        return PushOutcome::Sent;
    }
    if status == 410 && apns_reason_is(body, &["BadDeviceToken", "Unregistered"]) {
        tracing::info!(status, "a push token is no longer registered");
        return PushOutcome::Gone;
    }
    tracing::info!(status, "a push was not accepted");
    PushOutcome::Unreachable
}

/// FCM: the token is gone only on 404 whose `error.details[].errorCode` is `UNREGISTERED`.
/// A `NOT_FOUND` from the wrong project is not that, and must not wipe every token.
fn classify_fcm(status: u16, body: &str) -> PushOutcome {
    if (200..300).contains(&status) {
        return PushOutcome::Sent;
    }
    if status == 404 && fcm_unregistered(body) {
        tracing::info!(status, "a push token is no longer registered");
        return PushOutcome::Gone;
    }
    tracing::info!(status, "a push was not accepted");
    PushOutcome::Unreachable
}

fn apns_reason_is(body: &str, reasons: &[&str]) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    value
        .get("reason")
        .and_then(|reason| reason.as_str())
        .is_some_and(|reason| reasons.contains(&reason))
}

fn fcm_unregistered(body: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    value
        .pointer("/error/details")
        .and_then(|details| details.as_array())
        .is_some_and(|details| {
            details.iter().any(|detail| {
                detail.get("errorCode").and_then(|code| code.as_str()) == Some("UNREGISTERED")
            })
        })
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(0))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alert_holds_the_sentence_and_the_link_only() {
        let message = PushMessage {
            platform: PushPlatform::Apns,
            token: "device-token-secret".into(),
            collapse: "abc".into(),
            title: "A task moved forward".into(),
            body: "The work stays on the computer.".into(),
            deep_link: "yardsort://notice/task/11111111-1111-1111-1111-111111111111".into(),
            category: "task".into(),
        };
        let json = alert_json(&message);
        assert!(json.contains("A task moved forward"));
        assert!(json.contains("yardsort://notice/task/"));
        assert!(!json.contains("device-token-secret"));
        assert!(!json.contains("Q7K2M9NP"));
        assert!(!json.contains("eventKey"));
        assert!(!json.contains("code"));
    }

    fn rsa_pem() -> String {
        use rsa::pkcs8::EncodePrivateKey;
        let mut rng = rand::thread_rng();
        let key = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
            .unwrap()
            .to_string()
    }

    fn ec_pem() -> String {
        use p256::pkcs8::EncodePrivateKey;
        let key = p256::ecdsa::SigningKey::random(&mut rand::thread_rng());
        key.to_pkcs8_pem(p256::pkcs8::LineEnding::LF)
            .unwrap()
            .to_string()
    }

    #[test]
    fn a_pkcs8_service_account_key_signs() {
        let json = serde_json::json!({
            "project_id": "yardsort-test",
            "client_email": "push@yardsort-test.iam.gserviceaccount.com",
            "private_key": rsa_pem(),
        });
        let fcm = Fcm::parse(&json.to_string()).expect("a PKCS#8 RSA key signs");
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &serde_json::json!({ "sub": "yardsort" }),
            &fcm.key,
        )
        .unwrap();
        assert_eq!(token.split('.').count(), 3);
        assert!(Fcm::parse(
            r#"{"project_id":"p","client_email":"a@b.c","private_key":"not-a-key"}"#
        )
        .is_none());
    }

    #[test]
    fn an_ec_p8_key_signs() {
        let apns = Apns::parse(
            "KEYID".into(),
            "TEAMID".into(),
            "dev.yardsort.mobile".into(),
            true,
            &ec_pem(),
        )
        .expect("an EC PKCS#8 key signs");
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256),
            &serde_json::json!({ "iss": "TEAMID", "iat": 1 }),
            &apns.key,
        )
        .unwrap();
        assert_eq!(token.split('.').count(), 3);
        assert!(Apns::parse("K".into(), "T".into(), "b".into(), true, "not-a-key").is_none());
    }

    #[test]
    fn a_rejected_provider_token_is_forgotten() {
        let apns = Apns::parse(
            "KEYID".into(),
            "TEAMID".into(),
            "dev.yardsort.mobile".into(),
            true,
            &ec_pem(),
        )
        .unwrap();
        *apns.jwt.lock().unwrap() = Some(("stale".into(), 9_000_000_000));
        assert!(apns.provider_token_rejected(403, r#"{"reason":"ExpiredProviderToken"}"#));
        assert!(apns.jwt.lock().unwrap().is_none());
        *apns.jwt.lock().unwrap() = Some(("kept".into(), 9_000_000_000));
        assert!(!apns.provider_token_rejected(403, r#"{"reason":"BadDeviceToken"}"#));
        assert!(apns.jwt.lock().unwrap().is_some());
        assert!(apns.provider_token_rejected(403, r#"{"reason":"InvalidProviderToken"}"#));
        assert!(apns.jwt.lock().unwrap().is_none());
    }

    #[test]
    fn only_an_unregistered_token_is_gone() {
        let unregistered = r#"{"error":{"code":404,"status":"NOT_FOUND","details":[{"errorCode":"UNREGISTERED"}]}}"#;
        assert_eq!(classify_fcm(404, unregistered), PushOutcome::Gone);
        let wrong_project = r#"{"error":{"code":404,"status":"NOT_FOUND","message":"NOT_FOUND"}}"#;
        assert_eq!(classify_fcm(404, wrong_project), PushOutcome::Unreachable);
        assert_eq!(
            classify_fcm(400, "UNREGISTERED NOT_FOUND"),
            PushOutcome::Unreachable
        );
        assert_eq!(
            classify_apns(410, r#"{"reason":"Unregistered"}"#),
            PushOutcome::Gone
        );
        assert_eq!(
            classify_apns(410, r#"{"reason":"BadDeviceToken"}"#),
            PushOutcome::Gone
        );
        assert_eq!(classify_apns(410, "Unregistered"), PushOutcome::Unreachable);
        assert_eq!(
            classify_apns(400, r#"{"reason":"BadDeviceToken"}"#),
            PushOutcome::Unreachable
        );
        assert_eq!(classify_apns(200, ""), PushOutcome::Sent);
        assert_eq!(classify_fcm(200, ""), PushOutcome::Sent);
    }
}
