//! The daemon as it really runs: a separate process, a real socket, real PTYs.
//!
//! In-process tests could not show the one thing this crate exists for — that sessions outlive
//! the client — so these start an actual daemon and talk to it over an actual socket.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pty_host::{HostEvent, LaunchPlan, TermSize, TerminalHost};
use pty_ipc::{DaemonClient, Endpoint};

const TIMEOUT: Duration = Duration::from_secs(20);

/// A daemon in its own process, killed when the test ends however it ends.
struct Daemon {
    child: std::process::Child,
    endpoint: Endpoint,
    _dir: tempfile::TempDir,
}

impl Daemon {
    fn start() -> Self {
        // Long enough that no test trips over it by accident.
        Self::start_with_idle_grace(60_000)
    }

    fn start_with_idle_grace(ms: u64) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let endpoint = Endpoint::for_data_dir(dir.path());
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_pty-daemon"))
            .arg(endpoint.as_os_str())
            .env("PTY_DAEMON_IDLE_GRACE_MS", ms.to_string())
            .spawn()
            .expect("could not start the daemon");
        let daemon = Self {
            child,
            endpoint,
            _dir: dir,
        };
        daemon.wait_until_listening();
        daemon
    }

    fn wait_until_listening(&self) {
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            if let Ok(client) = self.try_connect() {
                drop(client);
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the daemon never started listening on {}", self.endpoint);
    }

    fn try_connect(&self) -> std::io::Result<(Arc<DaemonClient>, Receiver<HostEvent>)> {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let client = DaemonClient::connect(
            &self.endpoint,
            "test",
            Arc::new(move |event| {
                let _ = tx.lock().unwrap().send(event);
            }),
        )?;
        Ok((Arc::new(client), rx))
    }

    fn connect(&self) -> (Arc<DaemonClient>, Receiver<HostEvent>) {
        self.try_connect().expect("could not connect")
    }

    fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run `script` with the platform's shell.
///
/// A script that has to run on Windows too is written for `cmd.exe` as well as `sh`: `&&`
/// separates commands in both, `;` in neither — cmd would echo it as part of the text and go on
/// to exit 0. Tests needing real shell syntax are `#[cfg(unix)]`.
fn shell(script: &str) -> LaunchPlan {
    let (program, flag) = if cfg!(windows) {
        ("cmd.exe", "/C")
    } else {
        ("/bin/sh", "-c")
    };
    LaunchPlan {
        program: program.into(),
        args: vec![flag.into(), script.into()],
        cwd: None,
        env: vec![],
        clear_env: false,
        size: TermSize { cols: 80, rows: 24 },
        labels: Default::default(),
        prompt: None,
    }
}

/// Attach `capture` the way a terminal emulator attaches: it records output **and** answers
/// cursor-position queries, as xterm.js does in the app. A viewer that stays silent is not a
/// terminal — and ConPTY will not let the program start until a terminal has answered.
///
/// The reply cannot be written from the sink: sinks run on the client's reader thread, and a
/// request made from there would be waiting on the very thread that has to deliver its answer.
/// So the query is passed to a thread of its own, exactly as `pty-host`'s tests do.
fn attach_terminal(client: &Arc<DaemonClient>, id: &pty_host::SessionId, capture: &Capture) -> u32 {
    let (query_tx, query_rx) = mpsc::channel::<()>();
    let mut record = capture.sink();
    let attachment = client
        .attach(
            id,
            Box::new(move |bytes| {
                for _ in bytes.windows(4).filter(|w| *w == b"\x1b[6n") {
                    let _ = query_tx.send(());
                }
                record(bytes)
            }),
        )
        .unwrap();

    let (client, id) = (Arc::downgrade(client), id.clone());
    std::thread::spawn(move || {
        while query_rx.recv().is_ok() {
            let Some(client) = client.upgrade() else {
                break;
            };
            let _ = client.write(&id, b"\x1b[1;1R");
        }
    });
    attachment
}

/// Collects everything one attached viewer is sent.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn sink(&self) -> pty_host::OutputSink {
        let buf = Arc::clone(&self.0);
        Box::new(move |bytes| {
            buf.lock().unwrap().extend_from_slice(bytes);
            true
        })
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }

    fn wait_for(&self, needle: &str) {
        let deadline = Instant::now() + TIMEOUT;
        while !self.text().contains(needle) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {needle:?}; got {:?}",
                self.text()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn wait_for_exit(events: &Receiver<HostEvent>, id: &pty_host::SessionId) -> pty_host::ExitInfo {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(HostEvent::Exited { id: got, exit }) if &got == id => return exit,
            Ok(_) => continue,
            Err(error) => panic!("no exit for {id}: {error}"),
        }
    }
}

#[test]
fn a_session_runs_over_the_socket_and_reports_its_exit() {
    let daemon = Daemon::start();
    let (client, events) = daemon.connect();
    assert!(client.speaks_our_protocol());
    assert!(client.daemon_info().pid > 0);

    let session = client.spawn(shell("echo over-the-wire&& exit 3")).unwrap();
    let capture = Capture::default();
    attach_terminal(&client, &session.id, &capture);

    capture.wait_for("over-the-wire");
    let exit = wait_for_exit(&events, &session.id);
    assert_eq!(exit.code, 3);
    assert!(!exit.success);
}

#[cfg(unix)]
#[test]
fn input_typed_on_one_side_reaches_the_program_on_the_other() {
    let daemon = Daemon::start();
    let (client, _events) = daemon.connect();

    let session = client
        .spawn(shell("read line; echo \"got:[$line]\""))
        .unwrap();
    let capture = Capture::default();
    attach_terminal(&client, &session.id, &capture);

    client.write(&session.id, b"hello\r").unwrap();
    capture.wait_for("got:[hello]");
}

/// The reason the daemon exists: closing the app is a disconnect, not a kill.
#[cfg(unix)]
#[test]
fn sessions_outlive_the_client_that_started_them() {
    let daemon = Daemon::start();
    let (client, _events) = daemon.connect();

    let session = client.spawn(shell("echo still-here; sleep 60")).unwrap();
    let capture = Capture::default();
    attach_terminal(&client, &session.id, &capture);
    capture.wait_for("still-here");

    // The window closes.
    drop(client);

    // Somebody opens Yardsort again.
    let (returning, _events) = {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Ok(pair) = daemon.try_connect() {
                break pair;
            }
            assert!(Instant::now() < deadline, "could not reconnect");
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    let listed = returning.list();
    assert_eq!(listed.len(), 1, "the session should still be there");
    assert_eq!(listed[0].id, session.id);
    assert!(matches!(listed[0].state, pty_host::SessionState::Running));

    // Attaching afresh repaints what happened before anyone was watching.
    let again = Capture::default();
    attach_terminal(&returning, &session.id, &again);
    again.wait_for("still-here");

    returning.kill(&session.id).unwrap();
}

#[test]
fn a_shutdown_that_is_asked_to_stops_the_agents_and_the_daemon() {
    let mut daemon = Daemon::start();
    let (client, events) = daemon.connect();
    let session = client.spawn(shell(if cfg!(windows) {
        "ping -n 60 127.0.0.1"
    } else {
        "sleep 60"
    }));
    let session = session.unwrap();

    client.shutdown(true).unwrap();
    wait_for_exit(&events, &session.id);

    let deadline = Instant::now() + TIMEOUT;
    while daemon.is_running() {
        assert!(
            Instant::now() < deadline,
            "the daemon outlived its shutdown"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn asking_about_a_session_nobody_has_heard_of_says_so() {
    let daemon = Daemon::start();
    let (client, _events) = daemon.connect();
    let error = TerminalHost::info(&*client, &pty_host::SessionId("nope".into())).unwrap_err();
    assert!(
        matches!(error, pty_host::HostError::UnknownSession(id) if id.0 == "nope"),
        "the error should survive the trip as itself"
    );
}

/// A daemon with nothing to look after must not linger for ever: every data directory ever used
/// would leave a process behind. It waits a while first, so restarting the app does not take it
/// down between the disconnect and the reconnect.
#[test]
fn an_idle_daemon_gives_up_but_not_immediately() {
    let mut daemon = Daemon::start_with_idle_grace(1_500);
    let (client, _events) = daemon.connect();
    drop(client);

    std::thread::sleep(Duration::from_millis(500));
    assert!(
        daemon.is_running(),
        "it should still be there for an app that is merely restarting"
    );

    let deadline = Instant::now() + TIMEOUT;
    while daemon.is_running() {
        assert!(
            Instant::now() < deadline,
            "an idle daemon should eventually exit"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// ...but one with an agent still working stays, however long nobody is watching.
#[cfg(unix)]
#[test]
fn a_daemon_with_an_agent_still_running_stays_put() {
    let mut daemon = Daemon::start_with_idle_grace(300);
    let (client, _events) = daemon.connect();
    let session = client.spawn(shell("sleep 60")).unwrap();
    drop(client);

    std::thread::sleep(Duration::from_millis(2_000));
    assert!(daemon.is_running(), "the agent is still working");

    let (returning, _events) = daemon.connect();
    assert_eq!(returning.list()[0].id, session.id);
}

/// A daemon must start even when its socket lands somewhere it cannot tighten: `/tmp` belongs to
/// root, and it is both the last fallback for an over-long data directory and where somebody
/// starting a daemon by hand is likely to put one. What keeps others out is the socket's own
/// mode, which we do own.
#[cfg(unix)]
#[test]
fn a_socket_in_a_directory_we_do_not_own_still_works_and_is_private() {
    use std::os::unix::fs::PermissionsExt;

    let endpoint = Endpoint::Path(
        std::env::temp_dir().join(format!("yardsort-test-{}.sock", std::process::id())),
    );
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_pty-daemon"))
        .arg(endpoint.as_os_str())
        .env("PTY_DAEMON_IDLE_GRACE_MS", "60000")
        .spawn()
        .expect("could not start the daemon");
    let mut daemon = Daemon {
        child,
        endpoint,
        _dir: tempfile::tempdir().unwrap(),
    };
    daemon.wait_until_listening();

    let Endpoint::Path(path) = &daemon.endpoint else {
        unreachable!("unix uses socket files")
    };
    let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "the socket is the lock, not the directory");

    let (client, _events) = daemon.connect();
    assert!(client.speaks_our_protocol());
    assert!(daemon.is_running());
}
