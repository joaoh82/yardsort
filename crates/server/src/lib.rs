//! The Yardsort server's control service: who may sign in, which devices they have, and the
//! credentials those devices hold. See [`remote_proto::account`] for the messages, and
//! [the R2 record](../../../docs/design/27-remote-r2.md) for why it is built this way.
//!
//! **Signing in is the only thing the identity provider is asked.** A person signs in at the
//! provider once per device; the server checks the provider's ID token, finds or creates the
//! account for its issuer and subject, and from then on the device holds the server's own
//! credentials: a short-lived access credential and a refresh credential that works once. The
//! provider issues no refresh token and hears nothing more from that device. Revocation is
//! therefore the server's alone, and immediate, and a provider that bills by active user counts
//! sign-ins, not every phone's every refresh.
//!
//! [`http::router`] serves the service over HTTP, [`postgres::PgRecords`] keeps its records, and
//! the `yardsort-server` binary (`src/main.rs`) puts the two together from its environment.
//!
//! **Hosts and grants** (`pairing.rs`, and [`remote_proto::pairing`] for the messages): a host
//! registers on its account, offers a code, and its person approves the device that redeemed it.
//!
//! **The relay** ([`relay`]) carries a device's traffic to its host and back. Both connect out to
//! the server with a single-use ticket; neither listens.

pub mod config;
pub mod http;
pub mod limit;
pub mod oidc;
mod pairing;
pub mod postgres;
pub mod records;
pub mod relay;
mod service;
pub mod tokens;

pub use service::{Caller, ControlService, LoginFinish, ServerConfig};

/// How long a sign-in may take at the provider, from the device starting it to the provider
/// sending the browser back.
pub const LOGIN_TTL_MS: i64 = 10 * 60_000;
/// How long the device has to redeem the code its redirect received.
pub const HANDOFF_TTL_MS: i64 = 60_000;
/// How long an access credential works.
pub const ACCESS_TTL_MS: i64 = 15 * 60_000;
/// How long a refresh credential works unused. Each refresh starts this again, so a device in
/// use stays signed in, and one left unused for this long must sign in again.
pub const REFRESH_TTL_MS: i64 = 90 * 24 * 60 * 60_000;
/// How long after a refresh credential is traded in it may be presented once more, by a device
/// whose answer was lost on the way, as long as nothing issued in its place has been used.
pub const REFRESH_RETRY_MS: i64 = 30_000;
/// How many devices one account may have signed in. A device counts while it is not revoked and
/// holds a refresh credential that is neither spent nor lapsed.
pub const MAX_DEVICES_PER_ACCOUNT: usize = 50;
