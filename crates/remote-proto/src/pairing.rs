//! Pairing a device with a host: the host offers, the device asks, the host's person approves.
//!
//! A host is a computer that runs agents. It signs in to the server like any device (as a
//! [`DeviceKind::Host`](crate::account::DeviceKind::Host)) and registers itself, which gives it a
//! [`HostId`] on that server. To let a phone in, it makes an [`Offer`]: a high-entropy secret for
//! a QR code and a short code for a person to type, both good once and for
//! [`OFFER_TTL_MS`]. A device signed in to the **same account** redeems one, which asks for a
//! grant; that asks, it does not give. Nothing reaches the host until its person approves the
//! request at the host, choosing the scopes and projects, and the grant is what they chose.
//!
//! ```text
//! host    POST /v1/hosts                          {host?, name}       → HostSummary
//! host    POST /v1/hosts/{host}/offers                                → Offer
//! device  POST /v1/pairings                       {offer, scopes}     → GrantSummary (pending)
//! host    GET  /v1/hosts/{host}/grants                                → GrantPage
//! host    POST /v1/grants/{grant}/approve         {scopes, projects}  → GrantSummary
//! host    POST /v1/grants/{grant}/deny                                → GrantSummary
//! device  GET  /v1/grants/{grant}                                     → GrantSummary
//! device  GET  /v1/grants                                             → GrantPage (its own)
//! either  POST /v1/grants/{grant}/revoke                              → (nothing)
//! any     GET  /v1/hosts                                              → HostPage
//! host    POST /v1/hosts/{host}/remove                                → (nothing)
//! ```
//!
//! The short code is only a way to find the offer: 40 bits, looked up within the caller's own
//! account, with every miss counted against that account's open offers ([`MAX_OFFER_MISSES`]).
//! What keeps a stranger out is that they would have to be signed in to the account, and that
//! the host's person still has to say yes to a device they can see by name.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::account::Secret;
use crate::grant::{ProjectGrant, Scope};
use crate::ids::{DeviceId, GrantId, HostId};
use crate::model::MAX_TEXT;
use crate::{ErrorCode, RemoteError};

/// How long an offer can be redeemed, and how long a request then waits for the host's person.
pub const OFFER_TTL_MS: i64 = 5 * 60_000;
/// Wrong codes an account may try while its offers are open. Past this, every open offer of the
/// account is spent and the host has to show a new one.
pub const MAX_OFFER_MISSES: i32 = 5;
/// Characters in a short code, not counting the dash it is shown with.
pub const CODE_LEN: usize = 8;
/// The most projects a grant names one by one.
pub const MAX_GRANT_PROJECTS: usize = 200;

/// Crockford's base32: no I, L, O or U, so nothing reads as something else.
const CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// `POST /v1/hosts`: a host signing in registers, or registers again with the id it was given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRegistration {
    /// The id this server gave the host before, when it signs in again. Absent the first time.
    #[serde(default)]
    pub host: Option<HostId>,
    /// What its person calls it: "Workstation".
    pub name: String,
}

impl HostRegistration {
    pub fn validate(&self) -> Result<(), RemoteError> {
        check_name(&self.name, "A host name")
    }
}

/// A host on the account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSummary {
    pub id: HostId,
    pub name: String,
    pub created_at: i64,
    /// When its device last used its access, as far as the server noticed. Historical.
    pub last_seen_at: Option<i64>,
    /// The device asking is this host.
    pub current: bool,
}

/// `GET /v1/hosts`: the account's hosts, newest first. Removed hosts are not listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostPage {
    pub hosts: Vec<HostSummary>,
}

/// What a host shows to let one device in: as a QR code, and as a code to type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    /// For the QR code, with the server's address: `ys1.po.…`. 256 random bits.
    pub secret: Secret,
    /// For a person to type: eight characters shown as `ABCD-EFGH`.
    pub code: Secret,
    /// From when it was made; a duration, so a wrong clock does not matter.
    pub expires_in_ms: u64,
}

/// `POST /v1/pairings`: a device redeems an offer and asks for access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingRequest {
    pub offer: OfferKey,
    /// What the device asks for. The host's person may give less, never more.
    pub scopes: BTreeSet<Scope>,
}

impl PairingRequest {
    pub fn validate(&self) -> Result<(), RemoteError> {
        match &self.offer {
            OfferKey::Secret(secret) => {
                if secret.expose().is_empty()
                    || secret.expose().len() > crate::account::MAX_SECRET_LEN
                {
                    return Err(invalid("The pairing secret is missing or too long."));
                }
            }
            OfferKey::Code(code) => {
                if normalize_code(code.expose()).is_none() {
                    return Err(invalid(
                        "A pairing code is eight letters and digits, like ABCD-EFGH.",
                    ));
                }
            }
        }
        if self.scopes.is_empty() {
            return Err(invalid("A pairing asks for at least one scope."));
        }
        Ok(())
    }
}

/// Which half of an offer a device presents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum OfferKey {
    /// Read from the QR code.
    Secret(Secret),
    /// Typed by a person.
    Code(Secret),
}

/// `POST /v1/grants/{grant}/approve`: what the host's person chose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantDecision {
    /// Some or all of what was asked for.
    pub scopes: BTreeSet<Scope>,
    pub projects: ProjectGrant,
}

impl GrantDecision {
    pub fn validate(&self, requested: &BTreeSet<Scope>) -> Result<(), RemoteError> {
        if self.scopes.is_empty() {
            return Err(invalid("An approval gives at least one scope."));
        }
        if !self.scopes.is_subset(requested) {
            return Err(invalid(
                "An approval cannot give more than the device asked for.",
            ));
        }
        if let ProjectGrant::Only(projects) = &self.projects {
            if projects.is_empty() || projects.len() > MAX_GRANT_PROJECTS {
                return Err(invalid(
                    "An approval names between one and 200 projects, or all of them.",
                ));
            }
        }
        Ok(())
    }
}

/// Where a grant stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GrantState {
    /// Asked for; the host's person has not answered yet.
    Pending,
    /// In force.
    Approved,
    /// The host's person said no.
    Denied,
    /// Nobody answered in time.
    Expired,
    /// Taken back: by the host, by the device, or because either was signed out or removed.
    Revoked,
    #[serde(other)]
    Unknown,
}

/// A device's grant on a host, or its request for one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantSummary {
    pub id: GrantId,
    pub host: HostId,
    pub host_name: String,
    pub device: DeviceId,
    pub device_name: String,
    pub state: GrantState,
    /// What the device asked for.
    pub requested: BTreeSet<Scope>,
    /// What it was given: empty unless approved.
    pub scopes: BTreeSet<Scope>,
    /// Which projects: absent unless approved.
    #[serde(default)]
    pub projects: Option<ProjectGrant>,
    pub created_at: i64,
    /// When the host's person approved or denied it, or it was revoked.
    #[serde(default)]
    pub decided_at: Option<i64>,
    /// While pending: how long it still waits.
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
}

/// A list of grants, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantPage {
    pub grants: Vec<GrantSummary>,
}

/// A short code from five random bytes: eight characters of Crockford's base32.
pub fn format_code(bytes: [u8; 5]) -> String {
    let bits = bytes.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b));
    (0..CODE_LEN)
        .rev()
        .map(|i| CODE_ALPHABET[((bits >> (i * 5)) & 31) as usize] as char)
        .collect()
}

/// How a short code is shown: `ABCD-EFGH`.
pub fn display_code(code: &str) -> String {
    match normalize_code(code) {
        Some(code) => format!("{}-{}", &code[..4], &code[4..]),
        None => code.to_owned(),
    }
}

/// A short code as typed, made canonical: dashes and spaces dropped, case ignored, and the
/// letters people mistake for digits read as those digits. `None` when it is not one.
pub fn normalize_code(typed: &str) -> Option<String> {
    let mut code = String::with_capacity(CODE_LEN);
    for c in typed.chars() {
        let c = match c.to_ascii_uppercase() {
            '-' | ' ' => continue,
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        if !c.is_ascii() || !CODE_ALPHABET.contains(&(c as u8)) || code.len() == CODE_LEN {
            return None;
        }
        code.push(c);
    }
    (code.len() == CODE_LEN).then_some(code)
}

fn check_name(name: &str, what: &str) -> Result<(), RemoteError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_TEXT || name.chars().any(char::is_control) {
        return Err(RemoteError::new(
            ErrorCode::InvalidRequest,
            format!("{what} is one line of at most 256 characters."),
        ));
    }
    Ok(())
}

fn invalid(message: &'static str) -> RemoteError {
    RemoteError::new(ErrorCode::InvalidRequest, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_reads_back_however_it_is_typed() {
        let code = format_code([0xde, 0xad, 0xbe, 0xef, 0x01]);
        assert_eq!(code.len(), CODE_LEN);
        assert_eq!(normalize_code(&code).as_deref(), Some(code.as_str()));
        let shown = display_code(&code);
        assert_eq!(shown.len(), CODE_LEN + 1);
        assert_eq!(normalize_code(&shown.to_lowercase()), Some(code.clone()));
        assert_eq!(normalize_code("o1il-0000").as_deref(), Some("01110000"));
        assert_eq!(format_code([0; 5]), "00000000");
        assert_eq!(format_code([0xff; 5]), "ZZZZZZZZ");
    }

    #[test]
    fn what_is_not_a_code_is_refused() {
        for bad in [
            "",
            "ABCD-EFG",
            "ABCD-EFGHJ",
            "ABCD-EFGU",
            "ABCD/EFGH",
            "ÄBCD-EFGH",
        ] {
            assert_eq!(normalize_code(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn an_approval_gives_no_more_than_was_asked() {
        let asked: BTreeSet<_> = [Scope::WorkspaceRead, Scope::TerminalRead].into();
        let decide = |scopes: &[Scope], projects| GrantDecision {
            scopes: scopes.iter().copied().collect(),
            projects,
        };
        assert!(decide(&[Scope::WorkspaceRead], ProjectGrant::All)
            .validate(&asked)
            .is_ok());
        assert!(decide(&[Scope::TerminalControl], ProjectGrant::All)
            .validate(&asked)
            .is_err());
        assert!(decide(&[], ProjectGrant::All).validate(&asked).is_err());
        assert!(
            decide(&[Scope::WorkspaceRead], ProjectGrant::Only(BTreeSet::new()))
                .validate(&asked)
                .is_err()
        );
    }

    #[test]
    fn a_pairing_names_an_offer_and_asks_for_something() {
        let request = |offer, scopes: &[Scope]| PairingRequest {
            offer,
            scopes: scopes.iter().copied().collect(),
        };
        let code = || OfferKey::Code(Secret::new("abcd efgh"));
        assert!(request(code(), &[Scope::WorkspaceRead]).validate().is_ok());
        assert!(request(code(), &[]).validate().is_err());
        assert!(
            request(OfferKey::Code(Secret::new("nope")), &[Scope::WorkspaceRead])
                .validate()
                .is_err()
        );
        assert!(
            request(OfferKey::Secret(Secret::new("")), &[Scope::WorkspaceRead])
                .validate()
                .is_err()
        );
        let printed = format!("{:?}", request(code(), &[Scope::WorkspaceRead]));
        assert!(!printed.contains("abcd"), "{printed}");
    }
}
