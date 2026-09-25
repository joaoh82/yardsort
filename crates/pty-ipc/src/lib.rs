//! The wire between Yardsort and the daemon that owns its terminals.
//!
//! [`pty_host`] was written from the start as a host reachable by messages: serialisable
//! requests in, byte streams and events out, sessions addressed by id, nothing borrowed across
//! the boundary. This crate is the boundary made real — a blocking server that wraps one
//! [`PtyHost`](pty_host::PtyHost), and a [`DaemonClient`] that implements the very same
//! [`TerminalHost`](pty_host::TerminalHost) trait over a local socket.
//!
//! Nothing above the trait knows which one it is holding, which is what lets agents keep working
//! when the window closes: the app is a client, and closing it is just a disconnect.
//!
//! ```text
//! app ──local socket──▶ yardsort --yardsort-daemon
//!                         └─ PtyHost ──▶ claude / codex / a shell …
//! ```

mod client;
mod endpoint;
mod proto;
mod server;
pub mod spool;

pub use client::DaemonClient;
pub use endpoint::Endpoint;
pub use proto::{DaemonInfo, PROTOCOL};
pub use server::{serve, serve_with, ServeOptions, IDLE_GRACE};
