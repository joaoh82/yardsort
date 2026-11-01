//! **Send result to agent**: a run's report typed into an agent's terminal, by the workflow
//! driver's rule — never into a working agent, never over somebody else's control, and only
//! into a session still running. The app keeps a delivery that is not yet possible and tries
//! again on every tick, which the host's quiet and exit events wake.

use pty_host::{HostError, SessionId, SessionState, TerminalHost};

/// A result on its way to an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub run_id: String,
    pub session_id: String,
    pub text: String,
}

/// How one try at a delivery went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tried {
    Delivered,
    /// The agent was busy, or controlled from elsewhere: try again when it is quiet.
    NotYet,
    /// It will never go: the session ended, or could not be typed into.
    Gone(String),
}

/// Type the result into the agent now, if it is quiet: the text as one paste, then Enter.
pub fn try_deliver(host: &dyn TerminalHost, delivery: &Delivery) -> Tried {
    let id = SessionId(delivery.session_id.clone());
    match host.info(&id) {
        Ok(info) if !matches!(info.state, SessionState::Running) => {
            return Tried::Gone("The agent's terminal ended before it was quiet.".into());
        }
        Err(_) => return Tried::Gone("The agent's terminal is gone.".into()),
        Ok(info) if info.busy => return Tried::NotYet,
        Ok(_) => {}
    }
    match host.type_unattended(&id, &delivery.text, true) {
        Ok(()) => {}
        Err(HostError::ControlHeld { .. }) => return Tried::NotYet,
        Err(error) => return Tried::Gone(format!("Could not type into the agent: {error}")),
    }
    std::thread::sleep(pty_host::SUBMIT_DELAY);
    match host.type_unattended(&id, "\r", false) {
        Ok(()) => Tried::Delivered,
        Err(error) => Tried::Gone(format!("Could not type into the agent: {error}")),
    }
}

// The tests drive real PTYs with `cat`, `sh` and `true`, so they are Unix-only. Gating the
// whole module keeps its imports and helpers from being dead code on Windows, where `-D
// warnings` would fail the build.
#[cfg(all(test, unix))]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use pty_host::{LaunchPlan, PtyHost, TermSize};

    use super::*;

    fn host() -> PtyHost {
        PtyHost::new(Arc::new(|_| {})).with_quiet_after(Duration::from_millis(200))
    }

    fn plan(program: &str, args: &[&str]) -> LaunchPlan {
        LaunchPlan {
            program: program.into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            cwd: None,
            env: Vec::new(),
            clear_env: false,
            size: TermSize { cols: 80, rows: 24 },
            labels: Default::default(),
            prompt: None,
        }
    }

    fn delivery(session_id: &str, text: &str) -> Delivery {
        Delivery {
            run_id: "r1".into(),
            session_id: session_id.into(),
            text: text.into(),
        }
    }

    fn until(what: &str, done: impl Fn() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "timed out waiting until {what}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_quiet_session_gets_the_text_and_an_enter_a_busy_one_waits_an_ended_one_never() {
        let host = host();
        // `cat` echoes what it is given: what the agent would have received.
        let quiet = host.spawn(plan("cat", &[])).unwrap();
        let heard = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink = Arc::clone(&heard);
        host.attach(
            &quiet.id,
            Box::new(move |bytes| {
                sink.lock().unwrap().extend_from_slice(bytes);
                true
            }),
        )
        .unwrap();
        until("the session is quiet", || {
            !host.info(&quiet.id).unwrap().busy
        });
        assert_eq!(
            try_deliver(&host, &delivery(&quiet.id.0, "failed  login  (1 of 2)")),
            Tried::Delivered
        );
        until("cat echoes the result", || {
            String::from_utf8_lossy(&heard.lock().unwrap()).contains("failed  login  (1 of 2)")
        });
        host.kill(&quiet.id).unwrap();

        // A session printing without pause is busy: the text waits rather than interrupts.
        let busy = host
            .spawn(plan("sh", &["-c", "while :; do echo working; done"]))
            .unwrap();
        until("the session is busy", || host.info(&busy.id).unwrap().busy);
        assert_eq!(
            try_deliver(&host, &delivery(&busy.id.0, "later")),
            Tried::NotYet
        );
        host.kill(&busy.id).unwrap();

        // One that ended will never read it.
        let ended = host.spawn(plan("true", &[])).unwrap();
        until("the session ends", || {
            !matches!(host.info(&ended.id).unwrap().state, SessionState::Running)
        });
        assert!(matches!(
            try_deliver(&host, &delivery(&ended.id.0, "never")),
            Tried::Gone(_)
        ));
        assert!(matches!(
            try_deliver(&host, &delivery("no-such-session", "never")),
            Tried::Gone(_)
        ));
    }
}
