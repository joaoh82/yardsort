//! `ys attach` — put a running session on this terminal.
//!
//! Detaching is not stopping. The session belongs to the daemon either way; this command only
//! decides whether its output is coming here. That is the same relationship the app's window has
//! with it, which is why both can be attached at once.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pty_host::{HostEvent, SessionId, TermSize, TerminalHost};

use crate::target::{self, NoMatch};
use crate::{Failure, Output, Yardsort};

/// Byte 0x1D, `Ctrl-]`. Telnet's escape for the same job, and vanishingly rare in a TUI — but it
/// is intercepted here, so a program that wanted it cannot have it.
const DETACH: u8 = 0x1d;

/// How often the local terminal's size is compared with what the session was last told.
///
/// Polled rather than driven by `SIGWINCH` so that all three platforms take one path; a fifth of
/// a second is below noticing, and a resize that arrives late only redraws late.
const RESIZE_POLL: Duration = Duration::from_millis(200);

pub fn run(
    ys: &Yardsort,
    wanted: Option<String>,
    no_resize: bool,
    out: &Output,
) -> Result<(), Failure> {
    if out.json {
        return Err(Failure::new(
            "`attach` is a terminal, not a report: --json means nothing here.",
        ));
    }
    // Raw mode needs a terminal on both sides. Refusing now beats discovering it halfway, with
    // escape sequences already written into somebody's pipe.
    if !std::io::IsTerminal::is_terminal(&std::io::stdin())
        || !std::io::IsTerminal::is_terminal(&std::io::stdout())
    {
        return Err(Failure::new(
            "`ys attach` needs a terminal on stdin and stdout; it cannot be piped or redirected.",
        ));
    }

    // The session is found before the daemon is asked to send anything, so a mistyped name costs
    // nothing and never leaves an attachment behind.
    let exited = Arc::new(AtomicBool::new(false));
    let target_id = Arc::new(Mutex::new(None::<String>));
    let client = {
        let (exited, target_id) = (Arc::clone(&exited), Arc::clone(&target_id));
        ys.daemon_watching(Arc::new(move |event: HostEvent| {
            if let HostEvent::Exited { id, .. } = &event {
                let wanted = target_id
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if wanted.as_deref() == Some(id.0.as_str()) {
                    exited.store(true, Ordering::SeqCst);
                }
            }
        }))
    }
    .ok_or_else(|| {
        Failure::new(
            "No terminal daemon is running for this profile, so nothing is attached to anything.\n\
             `ys session list` shows what the records say; `ys doctor` says where to look.",
        )
    })?;

    let candidates = target::candidates(
        &client.list(),
        &ys.store.workspaces()?,
        &ys.store.all_sessions()?,
    );
    let target = target::resolve(&candidates, wanted.as_deref()).map_err(describe)?;
    *target_id
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(target.pty.clone());
    let session = SessionId(target.pty.clone());

    println!(
        "Attaching to {} — Ctrl-] detaches, and leaves it running.",
        target.label
    );
    let restore = RawMode::enter()?;

    // The snapshot arrives inside `attach`, before it returns, so the screen is repainted before
    // the first live byte. Nothing in this sink may ask the daemon anything: it runs on the
    // client's reader thread, which is the thread that would have to deliver the answer.
    let attachment = client
        .attach(
            &session,
            Box::new(move |bytes: &[u8]| {
                let mut stdout = std::io::stdout().lock();
                stdout.write_all(bytes).is_ok() && stdout.flush().is_ok()
            }),
        )
        .map_err(|e| Failure::new(format!("cannot attach: {e}")))?;

    if !no_resize {
        // Match the session to this terminal before anything is typed, or the program is drawing
        // to a width nobody is looking at.
        if let Some(size) = usable_size() {
            let _ = client.resize(&session, size);
        }
    }
    let watching = (!no_resize).then(|| watch_size(Arc::clone(&client), session.clone()));

    let reason = pump(&*client, &session, &exited);

    // Put the terminal back first, so whatever is said next is readable.
    drop(restore);
    if let Some(watching) = watching {
        watching.store(false, Ordering::SeqCst);
    }
    let _ = client.detach(&session, attachment);

    match reason {
        Ended::Detached => println!("\nDetached. {} is still running.", target.label),
        Ended::Exited => println!("\n{} exited.", target.label),
        Ended::Input(error) => return Err(Failure::new(format!("stopped reading input: {error}"))),
    }
    Ok(())
}

enum Ended {
    Detached,
    Exited,
    Input(std::io::Error),
}

/// How long the loop will sit on an idle keyboard before looking at the session again.
const IDLE_TICK: Duration = Duration::from_millis(150);

/// Forward what is typed until the detach key, or until the session goes away.
///
/// Reading happens on a thread of its own so that an agent finishing while nobody is typing ends
/// this promptly. A blocking read in the loop itself would hold on until the next keystroke, and
/// the whole point of attaching is to watch something you are not driving.
fn pump(client: &dyn TerminalHost, session: &SessionId, exited: &AtomicBool) -> Ended {
    let (typed, keys) = std::sync::mpsc::channel::<std::io::Result<Vec<u8>>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buffer = [0u8; 4096];
        loop {
            let message = match stdin.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => Ok(buffer[..n].to_vec()),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => Err(error),
            };
            let failed = message.is_err();
            // The receiver is gone once the loop below has finished; nothing left to send to.
            if typed.send(message).is_err() || failed {
                break;
            }
        }
    });

    loop {
        if exited.load(Ordering::SeqCst) {
            return Ended::Exited;
        }
        let typed = match keys.recv_timeout(IDLE_TICK) {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) => return Ended::Input(error),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            // Standard input ended: nothing more can be typed, so there is nothing to stay for.
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ended::Detached,
        };
        if let Some(at) = typed.iter().position(|b| *b == DETACH) {
            // Whatever came before the detach key in the same read still belongs to the program.
            if at > 0 {
                let _ = client.write(session, &typed[..at]);
            }
            return Ended::Detached;
        }
        if client.write(session, &typed).is_err() {
            return Ended::Exited;
        }
    }
}

/// Keep the session's size matched to this terminal for as long as the returned flag is true.
fn watch_size(client: Arc<pty_ipc::DaemonClient>, session: SessionId) -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&running);
    std::thread::spawn(move || {
        let mut last = usable_size();
        while flag.load(Ordering::SeqCst) {
            std::thread::sleep(RESIZE_POLL);
            // A size we cannot use is not a change: leave the session as it is and wait. Acting
            // on it would be the bug this guards against, one poll at a time.
            let Some(now) = usable_size() else { continue };
            if Some(now) != last {
                let _ = client.resize(&session, now);
                last = Some(now);
            }
        }
    });
    running
}

/// This terminal's size, if it is one a session could actually be drawn at.
fn usable_size() -> Option<TermSize> {
    crossterm::terminal::size().ok().and_then(usable)
}

/// Reject a degenerate size instead of passing it on.
///
/// A terminal can report zero — a pty opened without a size ever being set does, which is what
/// `script` gives a piped command — and resizing a session to zero columns throws away the
/// screen the daemon is holding. That screen is what repaints the app's window and the next
/// attach, so a bad size here destroys something somebody else was relying on.
fn usable(size: (u16, u16)) -> Option<TermSize> {
    let (cols, rows) = size;
    (cols > 0 && rows > 0).then_some(TermSize { cols, rows })
}

/// Raw mode for as long as this lives, including through a panic.
///
/// Without it the shell is left without an echo and without line editing, which looks like a
/// hung terminal to anyone who does not know to type `reset`.
struct RawMode;

impl RawMode {
    fn enter() -> Result<Self, Failure> {
        crossterm::terminal::enable_raw_mode()
            .map_err(|e| Failure::new(format!("cannot put this terminal into raw mode: {e}")))?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn describe(no_match: NoMatch) -> Failure {
    Failure::new(match no_match {
        NoMatch::Nothing => "Nothing is running to attach to. `ys session list --all` shows what \
                             there has been."
            .to_owned(),
        NoMatch::Unknown { choices } => format!(
            "No running session by that name. Running now: {}.",
            choices.join(", ")
        ),
        NoMatch::Several { choices } => format!(
            "Which one? {} — name a workspace, or the start of an id from `ys session list`.",
            choices.join(", ")
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_dimension_is_not_a_size_worth_resizing_to() {
        assert_eq!(usable((0, 0)), None);
        assert_eq!(usable((80, 0)), None, "no rows is still no screen");
        assert_eq!(usable((0, 24)), None, "no columns is still no screen");
        assert_eq!(usable((80, 24)), Some(TermSize { cols: 80, rows: 24 }));
        assert_eq!(usable((1, 1)), Some(TermSize { cols: 1, rows: 1 }));
    }
}
