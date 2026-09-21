//! Typing a first message into a program that will not take one on its command line.
//!
//! The host does this itself rather than leaving it to whoever asked for the session: the
//! delivery outlives the client, so a prompt still arrives when the window is closed — or the
//! app quits — in the seconds a harness spends starting up.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::session::Session;
use crate::types::{PendingPrompt, SessionState};

/// How long to wait for a program to become ready before pasting anyway.
const READY_TIMEOUT: Duration = Duration::from_secs(20);
const READY_POLL: Duration = Duration::from_millis(100);
/// Pause between pasting and pressing Enter; some TUIs drop an Enter that arrives with the paste.
const SUBMIT_DELAY: Duration = Duration::from_millis(150);

#[derive(Debug, PartialEq, Eq)]
enum Readiness {
    Ready,
    /// Still running at the timeout; we paste anyway rather than lose the message.
    TimedOut,
    Gone,
}

/// Paste `prompt` into the session once it is ready, then submit it. Runs on its own thread.
pub(crate) fn deliver(session: Arc<Session>, prompt: PendingPrompt) {
    let spawned = std::thread::Builder::new()
        .name(format!("pty-prompt-{}", &session.id().0[..8]))
        .spawn(move || {
            let quiet = Duration::from_millis(u64::from(prompt.quiet_ms));
            let readiness = wait_until_ready(quiet, READY_TIMEOUT, READY_POLL, || {
                let info = session.info();
                matches!(info.state, SessionState::Running).then(|| {
                    (
                        info.has_output,
                        Duration::from_millis(u64::from(info.idle_ms)),
                    )
                })
            });
            if readiness == Readiness::Gone || session.paste(&prompt.text).is_err() {
                return;
            }
            std::thread::sleep(SUBMIT_DELAY);
            let _ = session.write(b"\r");
        });
    if let Err(error) = spawned {
        eprintln!("could not start prompt delivery: {error}");
    }
}

/// Wait until the program has printed something and then stayed quiet for `quiet` — a TUI that
/// has finished drawing itself and is waiting for input. We never parse what it printed.
/// `observe` reports `(has_output, idle)`, or `None` once the session is over.
fn wait_until_ready(
    quiet: Duration,
    timeout: Duration,
    poll: Duration,
    mut observe: impl FnMut() -> Option<(bool, Duration)>,
) -> Readiness {
    let started = Instant::now();
    loop {
        match observe() {
            None => return Readiness::Gone,
            Some((true, idle)) if idle >= quiet => return Readiness::Ready,
            Some(_) if started.elapsed() >= timeout => return Readiness::TimedOut,
            Some(_) => std::thread::sleep(poll),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_is_output_followed_by_quiet() {
        let ms = Duration::from_millis;
        let run = |script: Vec<Option<(bool, u64)>>, timeout: u64| {
            let mut steps = script.into_iter();
            let mut last = None;
            wait_until_ready(ms(500), ms(timeout), ms(1), move || {
                last = steps.next().or(last);
                last.flatten().map(|(out, idle)| (out, ms(idle)))
            })
        };
        // Silent at first, then drawing (idle resets), then quiet long enough.
        let drawing = vec![
            Some((false, 900)),
            Some((true, 10)),
            Some((true, 200)),
            Some((true, 600)),
        ];
        assert_eq!(run(drawing, 5_000), Readiness::Ready);
        // Quiet from the start does not count: nothing was ever printed.
        assert_eq!(run(vec![Some((false, 10_000))], 30), Readiness::TimedOut);
        // Never settles: give up waiting rather than lose the message.
        assert_eq!(run(vec![Some((true, 5))], 30), Readiness::TimedOut);
        assert_eq!(run(vec![Some((true, 5)), None], 5_000), Readiness::Gone);
    }
}
