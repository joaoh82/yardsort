//! Whether an account may do something that costs the operator: sign in another device, open a
//! connection to a host, store more synced data (`docs/design/30-yardsort-cloud.md` §4,
//! `docs/design/46-remote-r7.md` §11).
//!
//! **Unset, everything is allowed**, up to the server's own limits such as
//! [`crate::MAX_DEVICES_PER_ACCOUNT`]. That is how a self-hosted server runs.
//!
//! **Set** (`YARDSORT_ENTITLEMENTS_URL`), the server asks that service, with a `POST` of the
//! account's id (the server's own, never the provider's subject), the action and the counts the
//! decision needs. No email address, name or credential is sent. The service answers
//! `{"allowed": true}`, or `{"allowed": false, "message": "…"}` with words safe to show the
//! person. Each answer is kept for [`KEEP_ANSWER_MS`] for that account and action, so a burst of
//! requests asks once: a bound, not an exact quota, as the device limit is.
//!
//! **When the service cannot be reached**, what this account was allowed to do in the last
//! [`GRACE_MS`] stays allowed, and only something it was never allowed waits, refused with a
//! retryable error. A refusal the service gave is never overridden by the grace period. What was
//! allowed is remembered by each instance, in memory, so an instance started while the service
//! is down knows of nothing allowed yet.
//!
//! Nothing here is asked before reading what the server holds, revoking, removing, exporting or
//! deleting.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use remote_proto::account::Secret;
use remote_proto::{ErrorCode, ErrorReason, RemoteError};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::records::AccountId;

/// How long an answer is kept for an account and action before it is asked again.
pub const KEEP_ANSWER_MS: i64 = 60_000;
/// How long what was allowed stays allowed while the service cannot be reached.
pub const GRACE_MS: i64 = 24 * 60 * 60_000;
/// How long the server waits for an answer before treating the service as unreachable.
pub const ASK_TIMEOUT: Duration = Duration::from_secs(3);
/// The longest message from the service a person is shown; anything longer is cut.
pub const MAX_MESSAGE_CHARS: usize = 300;
/// Answers kept beyond this many are dropped, oldest first, past the grace period.
const MAX_KEPT: usize = 100_000;

/// What the server asks about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    /// Signing in another device.
    Device,
    /// Opening a connection from a device to a host through the relay.
    Stream,
    /// Storing synced records.
    Sync,
}

/// The counts the decision needs, as they are before the action: only the action's own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    /// Devices signed in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub devices: Option<u64>,
    /// Connections open to the account's hosts, through this instance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streams: Option<u64>,
    /// Bytes of synced bodies stored, with the ones this request would store.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_bytes: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Question<'a> {
    account: &'a str,
    action: Action,
    counts: Counts,
}

#[derive(Deserialize)]
struct Answer {
    allowed: bool,
    #[serde(default)]
    message: Option<String>,
}

/// The server's entitlements: none, or a service to ask.
#[derive(Default)]
pub struct Entitlements {
    service: Option<Service>,
}

struct Service {
    url: Url,
    token: Option<Secret>,
    http: reqwest::Client,
    kept: Mutex<HashMap<(AccountId, Action), Kept>>,
}

#[derive(Clone)]
struct Kept {
    /// What the last answer, or the grace period, decided.
    verdict: Verdict,
    /// When it was decided; asked again [`KEEP_ANSWER_MS`] later.
    at: i64,
    /// When the service last said yes.
    allowed_at: Option<i64>,
}

#[derive(Clone)]
enum Verdict {
    Allowed,
    Refused(Option<String>),
    Unchecked,
}

impl Entitlements {
    /// Everything allowed: no service is configured.
    pub fn none() -> Self {
        Self::default()
    }

    /// Ask the service at `url`, with `token` as a bearer credential when given.
    pub fn service(url: Url, token: Option<Secret>, http: reqwest::Client) -> Self {
        Self {
            service: Some(Service {
                url,
                token,
                http,
                kept: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Whether a service is configured.
    pub fn configured(&self) -> bool {
        self.service.is_some()
    }

    /// Whether `account` may do `action` now. A refusal is a `forbidden` error with
    /// [`ErrorReason::PlanLimit`]; a service that cannot be reached, for something not allowed in
    /// the last [`GRACE_MS`], an `internal` one, which may be tried again.
    pub async fn may(
        &self,
        account: &AccountId,
        action: Action,
        counts: Counts,
        now: i64,
    ) -> Result<(), RemoteError> {
        let Some(service) = &self.service else {
            return Ok(());
        };
        let key = (account.clone(), action);
        let before = lock(&service.kept).get(&key).cloned();
        let kept = match before {
            Some(kept) if now - kept.at < KEEP_ANSWER_MS && now >= kept.at => kept,
            before => {
                let allowed_at = before.and_then(|kept| kept.allowed_at);
                let kept = match service.ask(account, action, counts).await {
                    Ok(answer) if answer.allowed => Kept {
                        verdict: Verdict::Allowed,
                        at: now,
                        allowed_at: Some(now),
                    },
                    Ok(answer) => Kept {
                        verdict: Verdict::Refused(answer.message),
                        at: now,
                        allowed_at: None,
                    },
                    Err(why) => {
                        tracing::warn!(?action, "entitlements: cannot ask: {why}");
                        let within = allowed_at.is_some_and(|at| now - at < GRACE_MS);
                        Kept {
                            verdict: if within {
                                Verdict::Allowed
                            } else {
                                Verdict::Unchecked
                            },
                            at: now,
                            allowed_at,
                        }
                    }
                };
                service.keep(key, kept.clone(), now);
                kept
            }
        };
        match kept.verdict {
            Verdict::Allowed => Ok(()),
            Verdict::Refused(message) => Err(refused(action, message)),
            Verdict::Unchecked => Err(RemoteError::new(
                ErrorCode::Internal,
                "The server cannot check this account's plan right now. Try again in a few \
                 minutes.",
            )),
        }
    }
}

impl Service {
    async fn ask(
        &self,
        account: &AccountId,
        action: Action,
        counts: Counts,
    ) -> Result<Answer, String> {
        let mut request = self
            .http
            .post(self.url.clone())
            .timeout(ASK_TIMEOUT)
            .json(&Question {
                account: &account.0,
                action,
                counts,
            });
        if let Some(token) = &self.token {
            request = request.bearer_auth(token.expose());
        }
        let response = request
            .send()
            .await
            .map_err(|e| e.without_url().to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("it answered {status}"));
        }
        response
            .json::<Answer>()
            .await
            .map_err(|e| format!("its answer: {}", e.without_url()))
    }

    fn keep(&self, key: (AccountId, Action), kept: Kept, now: i64) {
        let mut all = lock(&self.kept);
        all.insert(key, kept);
        if all.len() > MAX_KEPT {
            all.retain(|_, kept| now - kept.at < GRACE_MS);
        }
    }
}

/// The service's refusal, in its words when it gave some that are safe to show.
fn refused(action: Action, message: Option<String>) -> RemoteError {
    let theirs = message
        .map(|m| {
            m.chars()
                .filter(|c| !c.is_control())
                .take(MAX_MESSAGE_CHARS)
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .filter(|m| !m.is_empty());
    let message = theirs.unwrap_or_else(|| {
        match action {
            Action::Device => {
                "This account's plan has no room for another device. Sign one out from another \
                 device, or change the plan."
            }
            Action::Stream => {
                "This account's plan has no room for another connection to a computer right now."
            }
            Action::Sync => "This account's plan has no room for more synced data.",
        }
        .to_owned()
    });
    RemoteError::new(ErrorCode::Forbidden, message).because(ErrorReason::PlanLimit)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_shows_the_services_words_cut_and_cleaned_or_its_own() {
        let theirs = refused(Action::Device, Some("No more\u{7} devices.\n".into()));
        assert_eq!(theirs.code, ErrorCode::Forbidden);
        assert_eq!(theirs.reason, Some(ErrorReason::PlanLimit));
        assert_eq!(theirs.message, "No more devices.");
        let long = refused(Action::Sync, Some("x".repeat(1000)));
        assert_eq!(long.message.chars().count(), MAX_MESSAGE_CHARS);
        let none = refused(Action::Stream, Some("  ".into()));
        assert!(none.message.contains("connection"), "{}", none.message);
    }

    #[test]
    fn a_question_carries_the_account_action_and_only_its_counts() {
        let question = Question {
            account: "acct-1",
            action: Action::Sync,
            counts: Counts {
                sync_bytes: Some(12),
                ..Counts::default()
            },
        };
        assert_eq!(
            serde_json::to_value(&question).unwrap(),
            serde_json::json!({"account": "acct-1", "action": "sync", "counts": {"syncBytes": 12}})
        );
    }

    #[tokio::test]
    async fn unset_allows_everything() {
        let account = AccountId("acct-1".into());
        for action in [Action::Device, Action::Stream, Action::Sync] {
            Entitlements::none()
                .may(&account, action, Counts::default(), 0)
                .await
                .unwrap();
        }
    }
}
