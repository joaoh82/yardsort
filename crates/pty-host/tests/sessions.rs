//! End-to-end tests against real processes in real PTYs. These run on Linux, macOS and Windows
//! in CI, which is the point: the PTY layer is where the platforms differ most.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pty_host::{HostError, HostEvent, LaunchPlan, PtyHost, SessionId, SessionState, TermSize};

const TIMEOUT: Duration = Duration::from_secs(20);

fn host() -> (Arc<PtyHost>, Receiver<HostEvent>) {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let host = PtyHost::new(Arc::new(move |event| {
        let _ = tx.lock().unwrap().send(event);
    }));
    (Arc::new(host), rx)
}

/// Attach `capture` the way a terminal emulator attaches: it records output *and* answers
/// cursor-position queries, as xterm.js does in the app. A viewer that stays silent is not a
/// terminal — and ConPTY will not start the program until a terminal has answered.
fn attach_terminal(
    host: &Arc<PtyHost>,
    id: &SessionId,
    capture: &Capture,
) -> pty_host::AttachmentId {
    let (query_tx, query_rx) = mpsc::channel::<()>();
    let mut record = capture.sink();
    let attachment = host
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

    // Sinks must not call back into the host, so replies are written from a thread of their own.
    // It holds the host weakly and ends when the session (and with it the sink) is gone.
    let (host, id) = (Arc::downgrade(host), id.clone());
    std::thread::spawn(move || {
        while query_rx.recv().is_ok() {
            let Some(host) = host.upgrade() else { break };
            let _ = host.write(&id, b"\x1b[1;1R");
        }
    });
    attachment
}

/// Run `script` with the platform's shell.
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

/// A process that stays alive until killed.
fn long_running() -> LaunchPlan {
    shell(if cfg!(windows) {
        "ping -n 60 127.0.0.1"
    } else {
        "sleep 60"
    })
}

/// Collects everything a session sends to one viewer.
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

    /// Only the Unix-only tests need to wait on output mid-run.
    #[cfg(unix)]
    fn wait_for(&self, needle: &str) {
        let start = std::time::Instant::now();
        while !self.text().contains(needle) {
            assert!(
                start.elapsed() < TIMEOUT,
                "timed out waiting for {needle:?}; got {:?}",
                self.text()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn wait_for_exit(
    host: &PtyHost,
    events: &Receiver<HostEvent>,
    id: &SessionId,
) -> pty_host::ExitInfo {
    loop {
        // On a timeout, say what the host believes: a session still `Running` means the child
        // never reported its exit; a small `idle_ms` means the PTY never went quiet.
        match events
            .recv_timeout(TIMEOUT)
            .unwrap_or_else(|_| panic!("timed out waiting for exit; host says {:?}", host.info(id)))
        {
            HostEvent::Exited { id: exited, exit } if &exited == id => return exit,
            _ => {}
        }
    }
}

#[test]
fn streams_output_then_reports_the_exit_code() {
    let (host, events) = host();
    let session = host.spawn(shell("echo hello-from-pty&& exit 3")).unwrap();
    let capture = Capture::default();
    attach_terminal(&host, &session.id, &capture);

    let exit = wait_for_exit(&host, &events, &session.id);

    // All output is delivered before `Exited` is announced.
    assert!(
        capture.text().contains("hello-from-pty"),
        "{:?}",
        capture.text()
    );
    assert_eq!(exit.code, 3);
    assert!(!exit.success);
    assert!(matches!(
        host.info(&session.id).unwrap().state,
        SessionState::Exited { .. }
    ));
}

#[test]
fn a_late_viewer_gets_a_snapshot_of_what_it_missed() {
    let (host, events) = host();
    let session = host
        .spawn(shell("echo printed-before-anyone-watched"))
        .unwrap();
    wait_for_exit(&host, &events, &session.id);

    let capture = Capture::default();
    attach_terminal(&host, &session.id, &capture);

    // Delivered synchronously by `attach`, even though the process is long gone.
    assert!(
        capture.text().contains("printed-before-anyone-watched"),
        "{:?}",
        capture.text()
    );
}

#[test]
fn detached_viewers_stop_receiving() {
    let (host, events) = host();
    let script = if cfg!(windows) {
        "ping -n 2 127.0.0.1 >NUL&& echo second-part"
    } else {
        "sleep 1; echo second-part"
    };
    let session = host.spawn(shell(script)).unwrap();
    let (stays, leaves) = (Capture::default(), Capture::default());
    attach_terminal(&host, &session.id, &stays);
    let leaving = host.attach(&session.id, leaves.sink()).unwrap();
    host.detach(&session.id, leaving).unwrap();

    wait_for_exit(&host, &events, &session.id);

    assert!(stays.text().contains("second-part"));
    assert!(!leaves.text().contains("second-part"));
}

#[cfg(unix)]
#[test]
fn input_reaches_the_process() {
    let (host, events) = host();
    let session = host.spawn(shell("read line; echo \"got:$line\"")).unwrap();
    let capture = Capture::default();
    attach_terminal(&host, &session.id, &capture);

    host.write(&session.id, b"ping\n").unwrap();

    capture.wait_for("got:ping");
    assert!(wait_for_exit(&host, &events, &session.id).success);
}

#[cfg(unix)]
#[test]
fn output_into_a_quiet_session_is_not_held_for_the_batch_window() {
    // Output is gathered for a few milliseconds so a repainting TUI cannot flood the IPC channel
    // with hundreds of tiny messages. That window must not be charged to output arriving into a
    // terminal that has been sitting still: there is nothing to gather it with, and the delay
    // lands squarely on the echo of whatever the user just typed. Timing the window from the
    // previous delivery rather than from the arriving chunk is what keeps both properties.
    //
    // Unix-only because it needs a program that echoes bytes straight back; the batching it
    // covers has no platform-specific paths.
    let (host, _events) = host();
    let session = host.spawn(shell("cat")).unwrap();
    let (tx, arrivals) = mpsc::channel();
    let tx = Mutex::new(tx);
    host.attach(
        &session.id,
        Box::new(move |_| tx.lock().unwrap().send(std::time::Instant::now()).is_ok()),
    )
    .unwrap();

    // Let the shell settle, and drop whatever it printed on the way up.
    std::thread::sleep(Duration::from_millis(300));
    while arrivals.try_recv().is_ok() {}

    let mut round_trips = Vec::new();
    for _ in 0..10 {
        let sent = std::time::Instant::now();
        host.write(&session.id, b"x").unwrap();
        let arrived = arrivals
            .recv_timeout(TIMEOUT)
            .expect("the echo never came back");
        round_trips.push(arrived.duration_since(sent));
        // Stay quiet long enough that the next byte is a fresh burst, not a continuation.
        std::thread::sleep(Duration::from_millis(50));
    }
    round_trips.sort();

    // The median, so one descheduled sample on a loaded CI runner cannot fail the run. Half the
    // window is a deliberately loose bound: holding the byte gives ~8 ms, delivering it at once
    // gives well under 1 ms.
    let median = round_trips[round_trips.len() / 2];
    assert!(
        median < Duration::from_millis(4),
        "a lone keystroke echo waited {median:?}; it is being held for the batch window"
    );
}

#[cfg(unix)]
#[test]
fn the_process_sees_the_terminal_size_and_resizes() {
    let (host, events) = host();
    let mut plan = shell("stty size; read _; stty size");
    plan.size = TermSize {
        cols: 100,
        rows: 30,
    };
    let session = host.spawn(plan).unwrap();
    let capture = Capture::default();
    attach_terminal(&host, &session.id, &capture);
    capture.wait_for("30 100");

    host.resize(
        &session.id,
        TermSize {
            cols: 132,
            rows: 43,
        },
    )
    .unwrap();
    host.write(&session.id, b"\n").unwrap();

    capture.wait_for("43 132");
    assert_eq!(
        host.info(&session.id).unwrap().size,
        TermSize {
            cols: 132,
            rows: 43
        }
    );
    wait_for_exit(&host, &events, &session.id);
}

#[test]
fn kill_ends_a_running_session() {
    let (host, events) = host();
    let session = host.spawn(long_running()).unwrap();
    assert_eq!(host.info(&session.id).unwrap().state, SessionState::Running);

    host.kill(&session.id).unwrap();

    assert!(!wait_for_exit(&host, &events, &session.id).success);
    assert!(matches!(
        host.kill(&session.id),
        Err(HostError::SessionExited(_))
    ));
    assert!(matches!(
        host.write(&session.id, b"x"),
        Err(HostError::SessionExited(_))
    ));
}

#[test]
fn sessions_are_listed_until_removed() {
    let (host, _events) = host();
    let a = host.spawn(long_running()).unwrap();
    let b = host.spawn(long_running()).unwrap();
    let mut expected = vec![a.id.clone(), b.id.clone()];
    expected.sort();
    assert_eq!(
        host.list().into_iter().map(|s| s.id).collect::<Vec<_>>(),
        expected
    );

    host.remove(&a.id).unwrap();

    assert_eq!(
        host.list().into_iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![b.id]
    );
    assert!(matches!(
        host.info(&a.id),
        Err(HostError::UnknownSession(_))
    ));
}

#[test]
fn a_burst_of_output_is_reported_as_busy_then_quiet() {
    let (tx, events) = mpsc::channel();
    let tx = Mutex::new(tx);
    let host = Arc::new(
        PtyHost::new(Arc::new(move |event| {
            let _ = tx.lock().unwrap().send(event);
        }))
        .with_quiet_after(Duration::from_millis(300)),
    );
    // Print, stay silent well past the quiet period, print again, then linger.
    let script = if cfg!(windows) {
        "echo one&& ping -n 3 127.0.0.1 >NUL&& echo two&& ping -n 3 127.0.0.1 >NUL"
    } else {
        "echo one; sleep 1.5; echo two; sleep 1.5"
    };
    let session = host.spawn(shell(script)).unwrap();
    let capture = Capture::default();
    attach_terminal(&host, &session.id, &capture);

    let mut seen = Vec::new();
    loop {
        match events
            .recv_timeout(TIMEOUT)
            .expect("timed out waiting for events")
        {
            HostEvent::Busy { .. } => seen.push("busy"),
            HostEvent::Quiet { busy_ms, .. } => {
                assert!(
                    busy_ms < 1_000,
                    "a one-line burst is short, got {busy_ms} ms"
                );
                seen.push("quiet");
            }
            HostEvent::Exited { .. } => break,
        }
    }
    // ConPTY adds bursts of its own (startup, repaints), so only the shape is asserted:
    // bursts alternate, there were at least two, and the session is idle once it has exited.
    assert!(seen.len() >= 4, "{seen:?}");
    assert_eq!(seen[0], "busy");
    assert!(seen.windows(2).all(|pair| pair[0] != pair[1]), "{seen:?}");
    assert!(!host.info(&session.id).unwrap().busy);
}

#[test]
fn labels_are_stored_and_reported_back_untouched() {
    let (host, _events) = host();
    let mut plan = long_running();
    plan.labels.insert("workspace".into(), "ws-42".into());
    let session = host.spawn(plan).unwrap();
    assert_eq!(session.labels["workspace"], "ws-42");
    assert_eq!(host.list()[0].labels["workspace"], "ws-42");
}

#[test]
fn a_missing_program_is_a_spawn_error() {
    let (host, _events) = host();
    let mut plan = shell("");
    plan.program = "yardsort-no-such-program".into();
    match host.spawn(plan) {
        Err(HostError::Spawn { program, .. }) => assert_eq!(program, "yardsort-no-such-program"),
        other => panic!("expected a spawn error, got {other:?}"),
    }
}

/// Ask the terminal for the cursor position, read the 6-byte reply, and say which one came.
#[cfg(unix)]
const ASK_CURSOR_POSITION: &str = "stty raw -echo; printf '\\033[6n'; \
     r=$(dd bs=1 count=6 2>/dev/null); stty sane; \
     case \"$r\" in *'[1;1R') echo host-replied;; *'[9;9R') echo viewer-replied;; *) echo no-reply;; esac";

#[cfg(unix)]
#[test]
fn the_host_answers_cursor_queries_when_nobody_is_watching() {
    // What ConPTY does at startup on Windows: it blocks until the terminal reports the cursor.
    let (host, events) = host();
    let session = host.spawn(shell(ASK_CURSOR_POSITION)).unwrap();
    wait_for_exit(&host, &events, &session.id);

    let capture = Capture::default();
    attach_terminal(&host, &session.id, &capture);
    assert!(
        capture.text().contains("host-replied"),
        "{:?}",
        capture.text()
    );
}

#[cfg(unix)]
#[test]
fn an_attached_viewer_answers_cursor_queries_itself() {
    let (host, events) = host();
    let session = host
        .spawn(shell(&format!("sleep 1; {ASK_CURSOR_POSITION}")))
        .unwrap();

    // A viewer that behaves like a terminal emulator: it replies to the query it is shown.
    let capture = Capture::default();
    let (seen_tx, seen_rx) = mpsc::channel();
    let mut inner = capture.sink();
    host.attach(
        &session.id,
        Box::new(move |bytes| {
            if bytes.windows(4).any(|w| w == b"\x1b[6n") {
                let _ = seen_tx.send(());
            }
            inner(bytes)
        }),
    )
    .unwrap();
    seen_rx
        .recv_timeout(TIMEOUT)
        .expect("the viewer never saw the query");
    host.write(&session.id, b"\x1b[9;9R").unwrap();

    wait_for_exit(&host, &events, &session.id);
    // Exactly one reply reached the program, and it was the viewer's.
    assert!(
        capture.text().contains("viewer-replied"),
        "{:?}",
        capture.text()
    );
}

#[cfg(unix)]
#[test]
fn paste_is_bracketed_only_for_programs_that_asked() {
    let (host, events) = host();
    // `cat -v` shows control characters; the first run enables bracketed paste, the second not.
    for (setup, bracketed) in [("printf '\\033[?2004h'; ", true), ("", false)] {
        let session = host
            .spawn(shell(&format!(
                "{setup}stty -echo; echo ready; head -c 40 | cat -v; echo; echo done"
            )))
            .unwrap();
        let capture = Capture::default();
        attach_terminal(&host, &session.id, &capture);
        capture.wait_for("ready");
        assert!(host.info(&session.id).unwrap().has_output);

        host.paste(&session.id, "two\nlines").unwrap();
        host.write(&session.id, &[b'\n'; 40]).unwrap(); // let `head` finish
        capture.wait_for("done");
        let text = capture.text();
        assert!(text.contains("two") && text.contains("lines"), "{text:?}");
        assert_eq!(text.contains("^[[200~two"), bracketed, "{text:?}");
        assert_eq!(text.contains("lines^[[201~"), bracketed, "{text:?}");
        wait_for_exit(&host, &events, &session.id);
    }
}

#[cfg(unix)]
#[test]
fn the_plan_controls_the_environment() {
    let (host, events) = host();
    let mut plan = shell("echo \"[$SY_TEST|$TERM|${HOME:-unset}]\"");
    plan.clear_env = true;
    plan.env = vec![("SY_TEST".into(), "yes".into())];
    let session = host.spawn(plan).unwrap();
    let capture = Capture::default();
    attach_terminal(&host, &session.id, &capture);
    wait_for_exit(&host, &events, &session.id);
    assert!(
        capture.text().contains("[yes|xterm-256color|unset]"),
        "{:?}",
        capture.text()
    );
}

/// A plan's prompt, against a real program in a real PTY: a "harness" that draws a prompt, then
/// reads a line. The message must arrive only once the program has gone quiet, and be submitted.
///
/// The host does this itself, so it still lands when whoever asked for the session has gone —
/// the window closed while the harness was starting up.
#[cfg(unix)]
#[test]
fn a_plans_prompt_is_typed_in_once_the_program_is_ready() {
    let (host, _events) = host();
    let mut plan =
        shell("printf 'starting'; sleep 0.3; printf ' > '; read line; echo \"got:[$line]\"");
    plan.prompt = Some(pty_host::PendingPrompt {
        text: "fix the bug".into(),
        quiet_ms: 600,
    });
    let session = host.spawn(plan).unwrap();

    let capture = Capture::default();
    attach_terminal(&host, &session.id, &capture);
    capture.wait_for("got:[fix the bug]");
}
