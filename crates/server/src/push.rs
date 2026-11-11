//! Handing a notice to APNs or FCM. The provider is a trait so a test can record what would
//! have been sent, and a server with no credentials queues notices instead of refusing them.
//!
//! A payload is the title, the body, the deep link and the category. It never carries a token,
//! an event key, or a one-time code. A failure from the provider is not logged with the request
//! address, because that address contains the device token.

use std::future::Future;
use std::pin::Pin;
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
        }
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
        let der = der_of(pem)?;
        let key = jsonwebtoken::EncodingKey::from_ec_der(&der);
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
            Ok(response) => classify(
                response.status().as_u16(),
                &response.text().await.unwrap_or_default(),
                &["BadDeviceToken", "Unregistered"],
            ),
            Err(_) => PushOutcome::Unreachable,
        }
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
        let der = der_of(pem)?;
        let key = jsonwebtoken::EncodingKey::from_rsa_der(&der);
        if project_id.is_empty() || client_email.is_empty() {
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
            Ok(response) => classify(
                response.status().as_u16(),
                &response.text().await.unwrap_or_default(),
                &["UNREGISTERED", "NOT_FOUND"],
            ),
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
        let response = http
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
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

fn classify(status: u16, body: &str, gone: &[&str]) -> PushOutcome {
    if (200..300).contains(&status) {
        return PushOutcome::Sent;
    }
    if status == 410 || gone.iter().any(|reason| body.contains(reason)) {
        tracing::info!(status, "a push token is no longer registered");
        return PushOutcome::Gone;
    }
    tracing::info!(status, "a push was not accepted");
    PushOutcome::Unreachable
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(0))
        .unwrap_or(0)
}

fn der_of(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('-'))
        .collect();
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body).ok()
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
}
