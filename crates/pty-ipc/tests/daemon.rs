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
            .arg(dir.path().join("spool"))
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

    fn spool(&self) -> pty_ipc::spool::Spool {
        pty_ipc::spool::Spool::new(self._dir.path().join("spool"))
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

/// The reason the spool exists: an agent finishes while nothing is connected to hear it. The
/// exit must be waiting on disk for the next client, with the session's labels, and a client
/// that *was* connected finds the same exit there too — a duplicate it can recognise.
#[test]
fn an_exit_nobody_heard_is_waiting_in_the_spool_with_its_labels() {
    let daemon = Daemon::start();
    let (client, _events) = daemon.connect();
    let mut plan = shell(if cfg!(windows) {
        "ping -n 2 127.0.0.1 >NUL && exit 7"
    } else {
        "sleep 1 && exit 7"
    });
    plan.labels.insert("workspace".into(), "ws-1".into());
    plan.labels.insert("run".into(), "run-1".into());
    let session = client.spawn(plan).unwrap();
    // Gone before the process is: exactly the closed-window case.
    client.disconnect();
    drop(client);

    let spool = daemon.spool();
    let deadline = Instant::now() + TIMEOUT;
    let entry = loop {
        let waiting = spool.entries().unwrap();
        if let Some(path) = waiting.first() {
            break pty_ipc::spool::Spool::read(path).unwrap();
        }
        assert!(Instant::now() < deadline, "the exit was never spooled");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(entry.session, session.id);
    assert_eq!(entry.exit.code, 7);
    assert_eq!(entry.labels["workspace"], "ws-1");
    assert_eq!(entry.labels["run"], "run-1");
    assert_eq!(entry.version, pty_ipc::spool::SPOOL_VERSION);
    assert!(entry.at_ms > 0);

    // A second exit, heard live this time, is spooled all the same: the client decides what
    // is a duplicate, and a daemon cannot know whether the client that heard it kept it.
    let (client, events) = daemon.connect();
    let heard = client.spawn(shell("exit 3")).unwrap();
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(HostEvent::Exited { id, exit }) if id == heard.id => {
                assert_eq!(exit.code, 3);
                break;
            }
            Ok(_) => {}
            Err(_) => panic!("no live exit"),
        }
    }
    let deadline = Instant::now() + TIMEOUT;
    while spool.entries().unwrap().len() < 2 {
        assert!(Instant::now() < deadline, "the live exit was not spooled");
        std::thread::sleep(Duration::from_millis(50));
    }
    let codes: Vec<u32> = spool
        .entries()
        .unwrap()
        .iter()
        .map(|path| pty_ipc::spool::Spool::read(path).unwrap().exit.code)
        .collect();
    assert_eq!(codes, [7, 3], "oldest first");
}

/// One item a numbered viewer was sent: its number, what it was, and its bytes.
type WatchedItem = (u64, &'static str, Vec<u8>);

/// Everything one numbered viewer is sent over the socket.
#[derive(Clone, Default)]
struct Watching(Arc<Mutex<Vec<WatchedItem>>>);

impl Watching {
    fn sink(&self) -> pty_host::NumberedSink {
        let items = Arc::clone(&self.0);
        Box::new(move |item| {
            let entry = match item {
                pty_host::Numbered::Snapshot { seq, bytes, .. } => {
                    (seq, "snapshot", bytes.to_vec())
                }
                pty_host::Numbered::Output { seq, bytes } => (seq, "output", bytes.to_vec()),
                pty_host::Numbered::Resized { seq, size } => (
                    seq,
                    "resized",
                    format!("{}x{}", size.cols, size.rows).into_bytes(),
                ),
            };
            items.lock().unwrap().push(entry);
            true
        })
    }

    fn items(&self) -> Vec<WatchedItem> {
        self.0.lock().unwrap().clone()
    }

    fn text(&self) -> String {
        let bytes: Vec<u8> = self
            .items()
            .into_iter()
            .filter(|(_, kind, _)| *kind != "resized")
            .flat_map(|(_, _, bytes)| bytes)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[test]
fn a_numbered_viewer_over_the_socket_gets_every_item_once_and_in_order() {
    let daemon = Daemon::start();
    let (client, events) = daemon.connect();
    assert!(client.shares_control());
    let session = client.spawn(long_running_echo()).unwrap();
    let watching = Watching::default();
    let stream = client
        .attach_numbered(&session.id, watching.sink())
        .unwrap();
    client
        .resize(
            &session.id,
            TermSize {
                cols: 100,
                rows: 30,
            },
        )
        .unwrap();
    wait_for_exit(&events, &session.id);
    client.detach(&session.id, stream).unwrap();

    let items = watching.items();
    assert_eq!(items[0].1, "snapshot", "{items:?}");
    for pair in items.windows(2) {
        assert_eq!(pair[1].0, pair[0].0 + 1, "a gap or a repeat: {items:?}");
    }
    assert!(items
        .iter()
        .any(|(_, kind, size)| *kind == "resized" && size == b"100x30"));
    assert!(
        watching.text().contains("numbered-end"),
        "{:?}",
        watching.text()
    );
}

/// Prints, pauses long enough to be resized, and prints again.
fn long_running_echo() -> LaunchPlan {
    shell(if cfg!(windows) {
        "echo numbered-start&& ping -n 2 127.0.0.1 >NUL&& echo numbered-end"
    } else {
        "echo numbered-start; sleep 1; echo numbered-end"
    })
}

#[test]
fn a_lease_taken_over_the_socket_holds_until_someone_here_types() {
    let daemon = Daemon::start();
    let (here, _) = daemon.connect();
    let (remote, _) = daemon.connect();
    let session = here.spawn(echo_lines()).unwrap();
    let capture = Capture::default();
    attach_terminal(&here, &session.id, &capture);
    let id = &session.id;

    let lease = remote.acquire(id, "phone", false).unwrap();
    assert_eq!((lease.ttl_ms, lease.renew_every_ms), (30_000, 10_000));
    let lease = lease.reference();
    remote
        .input(id, &lease, 1, "from-the-lease\r", false)
        .unwrap();
    remote
        .input(id, &lease, 1, "from-the-lease\r", false)
        .unwrap();
    capture.wait_for("got:from-the-lease");
    assert!(matches!(
        remote.input(id, &lease, 5, "skipped\r", false),
        Err(pty_host::HostError::InputOutOfOrder {
            expected: 2,
            got: 5,
            ..
        })
    ));
    assert_eq!(
        remote.renew(id, &lease).unwrap().generation,
        lease.generation
    );
    assert_eq!(
        here.controller(id).unwrap(),
        pty_host::Controller::Leased {
            holder: "phone".into()
        }
    );
    // The app typing for itself, as a workflow does, waits rather than type over the lease.
    assert!(matches!(
        here.type_unattended(id, "from-a-workflow\r", false),
        Err(pty_host::HostError::ControlHeld { .. })
    ));
    assert_eq!(
        remote.renew(id, &lease).unwrap().generation,
        lease.generation
    );

    // Typed here, on the other connection: control comes here, and the lease is spent.
    here.write(id, b"from-here\r").unwrap();
    capture.wait_for("got:from-here");
    assert!(matches!(
        remote.input(id, &lease, 2, "late\r", false),
        Err(pty_host::HostError::LeaseLost(_))
    ));
    assert!(matches!(
        remote.acquire(id, "phone", true),
        Err(pty_host::HostError::ControlHeld {
            by: pty_host::Controller::Local,
            ..
        })
    ));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(capture.text().matches("got:from-the-lease").count(), 1);
    assert!(!capture.text().contains("got:late"));
    assert!(!capture.text().contains("got:from-a-workflow"));
    here.kill(id).unwrap();
}

/// Echoes every line it reads, `got:` in front.
fn echo_lines() -> LaunchPlan {
    if cfg!(windows) {
        // PowerShell is on every Windows CI runner; cmd has no loop that reads lines.
        LaunchPlan {
            program: "powershell.exe".into(),
            args: vec![
                "-NoProfile".into(),
                "-Command".into(),
                "while ($true) { $l = [Console]::In.ReadLine(); if ($l -eq $null) { break }; \
                 Write-Output \"got:$l\" }"
                    .into(),
            ],
            ..shell("")
        }
    } else {
        shell("stty -echo; while IFS= read -r line; do echo \"got:$line\"; done")
    }
}

#[cfg(unix)]
#[test]
fn with_two_clients_watching_only_the_newer_is_asked() {
    let daemon = Daemon::start();
    let (first, events) = daemon.connect();
    let (second, _) = daemon.connect();
    let session = first
        .spawn(shell(
            "sleep 1; stty raw -echo min 0 time 10; printf '\\033[6n'; \
             r=$(dd bs=1 count=24 2>/dev/null); stty sane; \
             printf 'got:%s\\n' \"$(printf '%s' \"$r\" | tr -d '\\033')\"",
        ))
        .unwrap();
    let (older, newer) = (Capture::default(), Capture::default());
    attach_terminal(&first, &session.id, &older);
    attach_terminal(&second, &session.id, &newer);
    wait_for_exit(&events, &session.id);
    // Each would answer `[1;1R` if asked: one answer, so one was asked.
    assert!(newer.text().contains("got:[1;1R\r"), "{:?}", newer.text());
    assert!(!older.text().contains("\x1b[6n"), "{:?}", older.text());
    assert!(newer.text().contains("\x1b[6n"));
}
