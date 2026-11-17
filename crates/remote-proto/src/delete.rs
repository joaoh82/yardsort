//! Deleting an account: everything the server keeps of it, gone, and every device of it signed
//! out.
//!
//! ```text
//! device   GET  /v1/login?…&purpose=confirm      browser → server → provider (sign in again)
//! provider GET  /v1/login/callback?code&state    browser → server → device redirect
//! device   POST /v1/account/delete {code, verifier}   → AccountDeleted
//! ```
//!
//! **Who may.** The person, having just signed in again at the identity provider: a sign-in
//! started with [`crate::account::LoginPurpose::Confirm`], which asks the provider to sign them in
//! whatever session it already has, and which they finished within [`FRESH_SIGN_IN_MS`]. Its code
//! is redeemed by the device that started it (PKCE, as for signing in), with that device's own
//! access, and only for the account that device is signed in to. A code from a sign-in as anyone
//! else deletes nothing. Clients ask for a typed confirmation before they start that sign-in.
//!
//! The same is on a page of the server's own, `/account/delete`, for a person with no device to
//! hand: they sign in again there, type the confirmation, and the account they signed in as goes.
//!
//! **What goes**: the account's row, and with it every device and its credentials, every host,
//! grant and pairing offer, push routes, notification settings, notices on their way, and every
//! synced record, tombstone, marker and change. Every relay link of the account closes at once, on
//! every instance. **What stays**: everything on each device. A device the account had is told
//! the next time it asks, with [`crate::ErrorReason::AccountDeleted`]: the server keeps the hashes
//! of the deleted devices' credentials, and nothing else, until those would have lapsed.
//!
//! A server that cannot delete answers `not_found`, and does not list `account.delete` in its
//! capabilities.

use serde::{Deserialize, Serialize};

use crate::account::Secret;
use crate::{ErrorCode, RemoteError};

/// The capability a server lists when it answers `POST /v1/account/delete`.
pub const DELETE_CAPABILITY: &str = "account.delete";
/// How recently the person must have signed in again for their deletion to go ahead.
pub const FRESH_SIGN_IN_MS: i64 = 5 * 60_000;
/// What a person types to confirm, in a client and on the server's page.
pub const CONFIRMATION: &str = "delete";

/// `POST /v1/account/delete`: the code a confirming sign-in brought back, and the verifier whose
/// SHA-256 started it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRequest {
    pub code: Secret,
    pub verifier: Secret,
}

impl DeleteRequest {
    pub fn validate(&self) -> Result<(), RemoteError> {
        let code = self.code.expose();
        let verifier = self.verifier.expose();
        if code.is_empty() || code.len() > crate::account::MAX_SECRET_LEN {
            return Err(invalid("The confirmation code is missing or too long."));
        }
        if !(crate::account::MIN_VERIFIER_LEN..=crate::account::MAX_VERIFIER_LEN)
            .contains(&verifier.len())
        {
            return Err(invalid("The verifier is not a PKCE verifier."));
        }
        Ok(())
    }
}

/// What went, as counts, for the client to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDeleted {
    pub devices: u32,
    pub hosts: u32,
    pub records: u32,
}

fn invalid(message: &'static str) -> RemoteError {
    RemoteError::new(ErrorCode::InvalidRequest, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_never_prints_its_code_and_is_bounded() {
        let request = DeleteRequest {
            code: Secret::new("the-code"),
            verifier: Secret::new("v".repeat(43)),
        };
        assert!(!format!("{request:?}").contains("the-code"));
        assert!(request.validate().is_ok());
        let short = DeleteRequest {
            verifier: Secret::new("v"),
            ..request.clone()
        };
        assert!(short.validate().is_err());
        let empty = DeleteRequest {
            code: Secret::new(""),
            ..request
        };
        assert!(empty.validate().is_err());
    }

    #[test]
    fn an_unknown_reason_is_read_as_unknown() {
        let error: RemoteError = serde_json::from_str(
            r#"{"code":"unauthenticated","message":"m","retryable":false,"reason":"later"}"#,
        )
        .unwrap();
        assert_eq!(error.reason, Some(crate::ErrorReason::Unknown));
        let deleted = RemoteError::new(ErrorCode::Unauthenticated, "m")
            .because(crate::ErrorReason::AccountDeleted);
        assert!(serde_json::to_string(&deleted)
            .unwrap()
            .contains(r#""reason":"accountDeleted""#));
    }
}
