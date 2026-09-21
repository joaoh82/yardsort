//! A bare PTY daemon: `pty-daemon <socket>`.
//!
//! The app does not use this — it runs the daemon out of its own binary, so the two can never be
//! different builds (see `src-tauri/src/daemon.rs`). This exists so the daemon can be exercised
//! by the tests as a real, separate process, and started by hand when something needs debugging.

use std::time::Duration;

use pty_ipc::{serve, Endpoint, IDLE_GRACE};

fn main() -> std::io::Result<()> {
    let Some(socket) = std::env::args_os().nth(1) else {
        eprintln!("usage: pty-daemon <socket path or pipe name>");
        std::process::exit(2);
    };
    // The tests want to watch the daemon give up without waiting the real grace period out.
    let grace = std::env::var("PTY_DAEMON_IDLE_GRACE_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map_or(IDLE_GRACE, Duration::from_millis);
    serve(&Endpoint::parse(&socket), env!("CARGO_PKG_VERSION"), grace)
}
