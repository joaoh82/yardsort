//! Take control of a terminal the app is showing, the way the remote side will: watch it
//! numbered, take a lease, type under it, and lose it to someone typing at the computer.
//!
//! ```sh
//! cargo run -p pty-ipc --example lease -- <data dir> [session id prefix]
//! ```
//!
//! For the smoke test of the daemon's control leases ([27 §12]). It talks to the daemon of the
//! profile in `<data dir>` — the one `YARDSORT_DATA_DIR` names — and never starts one.
//!
//! [27 §12]: ../../../docs/design/27-remote-r2.md

use std::io::BufRead;
use std::sync::{Arc, Mutex};

use pty_host::{HostError, Numbered, SessionState, TerminalHost};
use pty_ipc::{DaemonClient, Endpoint};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(data_dir) = args.next() else {
        fail("usage: lease <data dir> [session id prefix]");
    };
    let prefix = args.next().unwrap_or_default();
    let endpoint = Endpoint::for_data_dir(std::path::Path::new(&data_dir));
    let client = DaemonClient::connect(&endpoint, "lease example", Arc::new(|_| {}))
        .unwrap_or_else(|e| fail(&format!("no daemon at {endpoint}: {e}")));
    let info = client.daemon_info();
    println!(
        "Daemon {} (pid {}), protocol {}.",
        info.version, info.pid, info.protocol
    );
    if !client.shares_control() {
        fail("This daemon is from before control leases; it keeps running terminals, but cannot share them.");
    }

    let running: Vec<_> = client
        .list()
        .into_iter()
        .filter(|s| matches!(s.state, SessionState::Running) && s.id.0.starts_with(&prefix))
        .collect();
    let session = match running.as_slice() {
        [one] => one.id.clone(),
        [] => fail("No running terminal matches."),
        many => fail(&format!(
            "Which one? Give the start of its id: {}",
            many.iter()
                .map(|s| s.id.0[..8].to_owned())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    };
    println!("Terminal {}.", &session.0[..8]);

    // Numbered, from here until the end: the summary says whether anything was lost or doubled.
    let items: Arc<Mutex<Vec<(u64, char)>>> = Arc::default();
    let recording = Arc::clone(&items);
    let watching = client
        .attach_numbered(
            &session,
            Box::new(move |item| {
                let entry = match item {
                    Numbered::Snapshot { seq, .. } => (seq, 's'),
                    Numbered::Output { seq, .. } => (seq, 'o'),
                    Numbered::Resized { seq, .. } => (seq, 'r'),
                };
                recording.lock().unwrap().push(entry);
                true
            }),
        )
        .unwrap_or_else(|e| fail(&format!("cannot watch it: {e}")));

    let lease = match client.acquire(&session, "lease-example", false) {
        Ok(lease) => lease,
        Err(HostError::ControlHeld { by, .. }) => fail(&format!(
            "Held by {by:?}. Nothing typed in the app for 30 seconds frees it."
        )),
        Err(e) => fail(&format!("cannot take control: {e}")),
    };
    println!(
        "Leased: generation {}, for {} s, to renew every {} s.",
        lease.generation,
        lease.ttl_ms / 1000,
        lease.renew_every_ms / 1000
    );
    let held = lease.reference();
    let typed = "echo typed-under-a-lease\r";
    client
        .input(&session, &held, 1, typed, false)
        .unwrap_or_else(|e| fail(&format!("input 1: {e}")));
    client
        .input(&session, &held, 1, typed, false)
        .unwrap_or_else(|e| fail(&format!("input 1 again: {e}")));
    println!("Typed input 1, then sent it again: the terminal shows it once.");
    match client.input(&session, &held, 3, "echo skipped\r", false) {
        Err(HostError::InputOutOfOrder { expected, got, .. }) => {
            println!("Input {got} refused: expected {expected}.");
        }
        other => fail(&format!("input 3 should have been refused, got {other:?}")),
    }
    println!("Controller: {:?}.", client.controller(&session));

    println!("\nNow type anything into that terminal in the app, then press Enter here.");
    let _ = std::io::stdin().lock().lines().next();
    match client.input(&session, &held, 2, "echo too-late\r", false) {
        Err(HostError::LeaseLost(_)) => println!("Input 2 refused: the lease was lost."),
        other => println!("Input 2 was not refused ({other:?}): did you type in the app?"),
    }
    println!("Controller: {:?}.", client.controller(&session));
    let _ = client.release(&session, &held);
    let _ = client.detach(&session, watching);

    let items = items.lock().unwrap();
    let unbroken = items.windows(2).all(|pair| pair[1].0 == pair[0].0 + 1);
    let first = items.first().copied();
    println!(
        "\nWatched {} items from a snapshot at {:?}: {}.",
        items.len(),
        first.map(|(seq, _)| seq),
        if first.map(|(_, kind)| kind) == Some('s') && unbroken {
            "numbered without a gap or a repeat"
        } else {
            "NOT IN ORDER"
        }
    );
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}
