//! A bare PTY daemon: `pty-daemon <socket> [<spool dir>]`.
//!
//! The app does not use this — it runs the daemon out of its own binary, so the two can never be
//! different builds (see `src-tauri/src/daemon.rs`). This exists so the daemon can be exercised
//! by the tests as a real, separate process, and started by hand when something needs debugging.

use std::time::Duration;

use pty_ipc::{serve_with, Endpoint, ServeOptions, IDLE_GRACE};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let Some(socket) = args.next() else {
        eprintln!("usage: pty-daemon <socket path or pipe name> [<exit spool directory>]");
        std::process::exit(2);
    };
    let options = ServeOptions {
        spool_dir: args.next().map(std::path::PathBuf::from),
    };
    // The tests want to watch the daemon give up without waiting the real grace period out.
    let grace = std::env::var("PTY_DAEMON_IDLE_GRACE_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
        .map_or(IDLE_GRACE, Duration::from_millis);
    serve_with(
        &Endpoint::parse(&socket),
        env!("CARGO_PKG_VERSION"),
        grace,
        options,
    )
}
