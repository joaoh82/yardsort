//! End-to-end tests against real processes in real PTYs. These run on Linux, macOS and Windows
//! in CI, which is the point: the PTY layer is where the platforms differ most.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pty_host::{
    HostError, HostEvent, LaunchPlan, Numbered, OutputSeq, PtyHost, SessionId, SessionState,
    TermSize,
};

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

/// One item a numbered viewer was sent, owned.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Item {
    Snapshot(OutputSeq, TermSize, Vec<u8>),
    Output(OutputSeq, Vec<u8>),
    Resized(OutputSeq, TermSize),
}

impl Item {
    fn seq(&self) -> OutputSeq {
        match self {
            Self::Snapshot(seq, ..) | Self::Output(seq, _) | Self::Resized(seq, _) => *seq,
        }
    }
}

/// Everything one numbered viewer was sent, in order.
#[derive(Clone, Default)]
struct Numbering(Arc<Mutex<Vec<Item>>>);

impl Numbering {
    fn attach(&self, host: &PtyHost, id: &SessionId) {
        let items = Arc::clone(&self.0);
        host.attach_numbered(
            id,
            Box::new(move |item| {
                items.lock().unwrap().push(match item {
                    Numbered::Snapshot { seq, size, bytes } => {
                        Item::Snapshot(seq, size, bytes.to_vec())
                    }
                    Numbered::Output { seq, bytes } => Item::Output(seq, bytes.to_vec()),
                    Numbered::Resized { seq, size } => Item::Resized(seq, size),
                });
                true
            }),
        )
        .unwrap();
    }

    fn items(&self) -> Vec<Item> {
        self.0.lock().unwrap().clone()
    }

    fn snapshot_seq(&self) -> OutputSeq {
        self.items()[0].seq()
    }

    /// The snapshot comes first, and every item after it carries the next number.
    fn assert_unbroken(&self) {
        let items = self.items();
        assert!(matches!(items[0], Item::Snapshot(..)), "{items:?}");
        for (n, pair) in items.windows(2).enumerate() {
            assert_eq!(pair[1].seq(), pair[0].seq() + 1, "a gap after item {n}");
            assert!(!matches!(pair[1], Item::Snapshot(..)));
        }
    }

    #[cfg(unix)]
    /// The screen a viewer draws from the items up to and including `until`.
    ///
    /// Bytes between resizes go to the emulator in one call, as one stream: `vt100` mishandles
    /// a character split across calls, and a real viewer is a stream parser.
    fn replay(&self, until: OutputSeq) -> vt100::Parser {
        let mut viewer: Option<vt100::Parser> = None;
        let mut pending = Vec::new();
        for item in self.items().into_iter().take_while(|i| i.seq() <= until) {
            match item {
                Item::Snapshot(_, size, bytes) => {
                    viewer = Some(vt100::Parser::new(size.rows, size.cols, 10_000));
                    pending = bytes;
                }
                Item::Output(_, bytes) => pending.extend_from_slice(&bytes),
                Item::Resized(_, size) => {
                    let viewer = viewer.as_mut().unwrap();
                    viewer.process(&std::mem::take(&mut pending));
                    viewer.screen_mut().set_size(size.rows, size.cols);
                }
            }
        }
        let mut viewer = viewer.unwrap();
        viewer.process(&pending);
        viewer
    }

    #[cfg(unix)]
    /// The screen drawn from every item.
    fn text(&self) -> String {
        let screen = self.replay(OutputSeq::MAX);
        screen.screen().contents()
    }

    /// Every byte sent, as text. ConPTY repaints in its own way, so on Windows the bytes are
    /// the safer thing to look in.
    fn bytes(&self) -> String {
        let mut all = Vec::new();
        for item in self.items() {
            if let Item::Snapshot(_, _, bytes) | Item::Output(_, bytes) = item {
                all.extend_from_slice(&bytes);
            }
        }
        String::from_utf8_lossy(&all).into_owned()
    }
}

#[cfg(unix)]
fn screen_of(viewer: &vt100::Parser) -> (Vec<u8>, (u16, u16)) {
    (
        viewer.screen().contents_formatted(),
        viewer.screen().cursor_position(),
    )
}

#[test]
fn numbered_viewers_get_every_item_once_and_in_order() {
    let (host, events) = host();
    let session = host
        .spawn(shell("echo numbered-first&& echo numbered-second"))
        .unwrap();
    // Only a numbered viewer is watching. On Windows that is also the proof that it does not
    // stand in for a terminal: ConPTY runs nothing until its cursor query is answered, and here
    // only the host can answer it.
    let early = Numbering::default();
    early.attach(&host, &session.id);
    wait_for_exit(&host, &events, &session.id);

    early.assert_unbroken();
    let text = early.bytes();
    assert!(text.contains("numbered-second"), "{text:?}");

    // After the exit, a snapshot of where it all ended, and nothing more.
    let late = Numbering::default();
    late.attach(&host, &session.id);
    assert_eq!(late.items().len(), 1);
    assert_eq!(late.snapshot_seq(), early.items().last().unwrap().seq());
}

#[cfg(unix)]
#[test]
fn a_numbered_snapshot_and_the_items_after_it_rebuild_the_screen_exactly() {
    // Colours, multibyte characters and a cursor move, in two halves with a pause for input
    // between them, so a second viewer can attach in the middle.
    let lines = |from: u32, to: u32| {
        format!(
            "i={from}; while [ $i -lt {to} ]; do \
             printf '\\033[3%dmline %d caf\\303\\251 \\342\\230\\203\\033[m\\r\\n' $((i % 7 + 1)) $i; \
             i=$((i + 1)); done"
        )
    };
    let script = format!(
        "{}; read go; {}; printf '\\033[2;5Hend'",
        lines(0, 150),
        lines(150, 300)
    );
    let (host, events) = host();
    let session = host.spawn(shell(&script)).unwrap();

    let first = Numbering::default();
    first.attach(&host, &session.id);
    let start = std::time::Instant::now();
    while !first.text().contains("line 149") {
        assert!(start.elapsed() < TIMEOUT, "the first half never arrived");
        std::thread::sleep(Duration::from_millis(20));
    }
    let second = Numbering::default();
    second.attach(&host, &session.id);
    host.resize(
        &session.id,
        TermSize {
            cols: 100,
            rows: 30,
        },
    )
    .unwrap();
    host.write(&session.id, b"go\r").unwrap();
    wait_for_exit(&host, &events, &session.id);
    let third = Numbering::default();
    third.attach(&host, &session.id);

    for viewer in [&first, &second, &third] {
        viewer.assert_unbroken();
    }
    let middle = second.snapshot_seq();
    let end = first.items().last().unwrap().seq();
    assert!(first.snapshot_seq() < middle && middle < end);
    assert_eq!(third.snapshot_seq(), end);

    // Where they overlap, the two live viewers were sent the same items.
    let after = |viewer: &Numbering| -> Vec<Item> {
        viewer
            .items()
            .into_iter()
            .filter(|i| i.seq() > middle)
            .collect()
    };
    assert_eq!(after(&first), after(&second));
    assert!(after(&first).iter().any(|i| matches!(
        i,
        Item::Resized(
            _,
            TermSize {
                cols: 100,
                rows: 30
            }
        )
    )));

    // The second viewer's snapshot is exactly what the first had drawn by the same number:
    // the number on a snapshot says precisely which output it holds.
    assert_eq!(
        screen_of(&first.replay(middle)),
        screen_of(&second.replay(middle))
    );
    // And from three different starts, one screen at the end.
    let ending = screen_of(&first.replay(end));
    assert_eq!(screen_of(&second.replay(end)), ending);
    assert_eq!(screen_of(&third.replay(end)), ending);
    let text = third.text();
    assert!(text.contains("line 299 café ☃"), "{text:?}");
    assert!(text.contains("end"), "{text:?}");
}

#[cfg(unix)]
#[test]
fn the_host_still_answers_queries_when_only_numbered_viewers_watch() {
    let (host, events) = host();
    let session = host
        .spawn(shell(&format!("sleep 1; {ASK_CURSOR_POSITION}")))
        .unwrap();
    let watching = Numbering::default();
    watching.attach(&host, &session.id);
    wait_for_exit(&host, &events, &session.id);
    let text = watching.text();
    assert!(text.contains("host-replied"), "{text:?}");
}

/// Ask for the cursor position once, then collect every reply that arrives within a second of
/// the last, and print them with the escapes taken out: `got:[1;1R`, or `got:[1;1R[2;2R` if two
/// terminals answered.
#[cfg(unix)]
const COUNT_CURSOR_REPLIES: &str = "stty raw -echo min 0 time 10; printf '\\033[6n'; \
     r=$(dd bs=1 count=24 2>/dev/null); stty sane; \
     printf 'got:%s\\n' \"$(printf '%s' \"$r\" | tr -d '\\033')\"";

/// A terminal viewer that answers every cursor query it is shown with `reply`, and counts them.
#[cfg(unix)]
fn attach_answering(
    host: &Arc<PtyHost>,
    id: &SessionId,
    capture: &Capture,
    reply: &'static [u8],
) -> (pty_host::AttachmentId, Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let asked = Arc::new(AtomicUsize::new(0));
    let (query_tx, query_rx) = mpsc::channel::<()>();
    let mut record = capture.sink();
    let counting = Arc::clone(&asked);
    let attachment = host
        .attach(
            id,
            Box::new(move |bytes| {
                for _ in bytes.windows(4).filter(|w| *w == b"\x1b[6n") {
                    counting.fetch_add(1, Ordering::SeqCst);
                    let _ = query_tx.send(());
                }
                record(bytes)
            }),
        )
        .unwrap();
    let (host, id) = (Arc::downgrade(host), id.clone());
    std::thread::spawn(move || {
        while query_rx.recv().is_ok() {
            let Some(host) = host.upgrade() else { break };
            let _ = host.write(&id, reply);
        }
    });
    (attachment, asked)
}

#[cfg(unix)]
#[test]
fn with_two_terminals_watching_only_the_newer_is_asked_and_the_program_hears_once() {
    let (host, events) = host();
    let session = host
        .spawn(shell(&format!("sleep 1; {COUNT_CURSOR_REPLIES}")))
        .unwrap();
    let (older, newer) = (Capture::default(), Capture::default());
    let (_, older_asked) = attach_answering(&host, &session.id, &older, b"\x1b[1;1R");
    let (_, newer_asked) = attach_answering(&host, &session.id, &newer, b"\x1b[2;2R");
    wait_for_exit(&host, &events, &session.id);

    use std::sync::atomic::Ordering;
    assert_eq!(newer_asked.load(Ordering::SeqCst), 1);
    assert_eq!(older_asked.load(Ordering::SeqCst), 0, "{:?}", older.text());
    // Both drew the same thing, and the program heard one answer.
    for capture in [&older, &newer] {
        assert!(
            capture.text().contains("got:[2;2R\r"),
            "{:?}",
            capture.text()
        );
    }
}

#[cfg(unix)]
#[test]
fn when_the_answering_terminal_leaves_the_one_before_it_answers() {
    let (host, events) = host();
    let session = host
        .spawn(shell(&format!("sleep 1; {COUNT_CURSOR_REPLIES}")))
        .unwrap();
    let (older, newer) = (Capture::default(), Capture::default());
    let (_, older_asked) = attach_answering(&host, &session.id, &older, b"\x1b[1;1R");
    let (attachment, newer_asked) = attach_answering(&host, &session.id, &newer, b"\x1b[2;2R");
    host.detach(&session.id, attachment).unwrap();
    wait_for_exit(&host, &events, &session.id);

    use std::sync::atomic::Ordering;
    assert_eq!(older_asked.load(Ordering::SeqCst), 1);
    assert_eq!(newer_asked.load(Ordering::SeqCst), 0);
    assert!(older.text().contains("got:[1;1R\r"), "{:?}", older.text());
}

#[cfg(unix)]
#[test]
fn while_a_lease_is_held_no_terminal_is_asked_and_the_host_answers() {
    let (host, events) = host();
    let session = host
        .spawn(shell(&format!("sleep 1; {COUNT_CURSOR_REPLIES}")))
        .unwrap();
    let capture = Capture::default();
    let (_, asked) = attach_answering(&host, &session.id, &capture, b"\x1b[9;9R");
    let watching = Numbering::default();
    watching.attach(&host, &session.id);
    host.acquire(&session.id, "phone", false).unwrap();
    wait_for_exit(&host, &events, &session.id);

    use std::sync::atomic::Ordering;
    assert_eq!(asked.load(Ordering::SeqCst), 0, "{:?}", capture.text());
    assert!(
        capture.text().contains("got:[1;1R\r"),
        "{:?}",
        capture.text()
    );
    assert!(
        !watching.bytes().contains("\x1b[6n"),
        "{:?}",
        watching.bytes()
    );
    assert!(
        watching.text().contains("got:[1;1R"),
        "{:?}",
        watching.text()
    );
}

#[cfg(unix)]
#[test]
fn input_under_a_lease_is_written_once_in_order_until_someone_here_types() {
    let (host, _events) = host();
    let session = host
        .spawn(shell(
            "stty -echo; while IFS= read -r line; do echo \"got:$line\"; done",
        ))
        .unwrap();
    let capture = Capture::default();
    attach_terminal(&host, &session.id, &capture);
    let id = &session.id;

    let lease = host.acquire(id, "phone", false).unwrap().reference();
    assert_eq!(
        host.controller(id).unwrap(),
        pty_host::Controller::Leased {
            holder: "phone".into()
        }
    );
    host.input(id, &lease, 1, "one\r", false).unwrap();
    // Sent again after a doubtful reconnect: not typed twice.
    host.input(id, &lease, 1, "one\r", false).unwrap();
    match host.input(id, &lease, 3, "three\r", false) {
        Err(HostError::InputOutOfOrder {
            expected: 2,
            got: 3,
            ..
        }) => {}
        other => panic!("a gap must be refused, got {other:?}"),
    }
    host.input(id, &lease, 2, "two\r", false).unwrap();
    capture.wait_for("got:two");

    // Someone at this computer types: control is theirs at once, and the lease is spent.
    host.write(id, b"here\r").unwrap();
    capture.wait_for("got:here");
    assert_eq!(host.controller(id).unwrap(), pty_host::Controller::Local);
    assert!(matches!(
        host.input(id, &lease, 3, "late\r", false),
        Err(HostError::LeaseLost(_))
    ));
    assert!(matches!(
        host.acquire(id, "phone", true),
        Err(HostError::ControlHeld {
            by: pty_host::Controller::Local,
            ..
        })
    ));
    std::thread::sleep(Duration::from_millis(300));
    let text = capture.text();
    assert_eq!(text.matches("got:one").count(), 1, "{text:?}");
    assert!(
        !text.contains("got:three") && !text.contains("got:late"),
        "{text:?}"
    );
    host.kill(id).unwrap();
}

#[test]
fn a_lease_holders_size_stands_until_control_comes_back_here() {
    let (host, _events) = host();
    let session = host.spawn(long_running()).unwrap();
    let id = &session.id;
    let size = |cols, rows| TermSize { cols, rows };
    let current = || host.info(id).unwrap().size;
    let watching = Numbering::default();
    watching.attach(&host, id);

    let lease = host.acquire(id, "phone", false).unwrap().reference();
    host.resize_leased(id, &lease, size(40, 12)).unwrap();
    assert_eq!(current(), size(40, 12));
    // The window here is resized meanwhile: noted, not applied.
    host.resize(id, size(100, 30)).unwrap();
    assert_eq!(current(), size(40, 12));
    // Given back, the terminal takes the size asked for here.
    host.release(id, &lease).unwrap();
    assert_eq!(current(), size(100, 30));
    assert!(matches!(
        host.resize_leased(id, &lease, size(40, 12)),
        Err(HostError::LeaseLost(_))
    ));

    // And taken back by typing here, the same.
    let lease = host.acquire(id, "phone", false).unwrap().reference();
    host.resize_leased(id, &lease, size(50, 15)).unwrap();
    host.write(id, b"\r").unwrap();
    assert_eq!(current(), size(100, 30));

    let resizes: Vec<_> = watching
        .items()
        .into_iter()
        .filter_map(|item| match item {
            Item::Resized(_, size) => Some(size),
            _ => None,
        })
        .collect();
    assert_eq!(
        resizes,
        [size(40, 12), size(100, 30), size(50, 15), size(100, 30)]
    );
    watching.assert_unbroken();
    host.kill(id).unwrap();
}

#[test]
fn an_exited_session_cannot_be_leased() {
    let (host, events) = host();
    let session = host.spawn(shell("echo done")).unwrap();
    wait_for_exit(&host, &events, &session.id);
    assert!(matches!(
        host.acquire(&session.id, "phone", false),
        Err(HostError::SessionExited(_))
    ));
}

#[cfg(unix)]
#[test]
fn a_terminal_answering_a_query_does_not_take_control() {
    let (host, events) = host();
    let session = host
        .spawn(shell(&format!("sleep 1; {COUNT_CURSOR_REPLIES}; sleep 5")))
        .unwrap();
    let capture = Capture::default();
    let (_, asked) = attach_answering(&host, &session.id, &capture, b"\x1b[24;80R");
    capture.wait_for("got:[24;80R");
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    // The answer was typed in from here, yet nobody holds control: a device may take it.
    assert_eq!(
        host.controller(&session.id).unwrap(),
        pty_host::Controller::Nobody
    );
    host.acquire(&session.id, "phone", false).unwrap();
    host.kill(&session.id).unwrap();
    wait_for_exit(&host, &events, &session.id);
}
