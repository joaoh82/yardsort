//! One PTY, the process in it, and the three threads that service it:
//!
//! - **reader** blocks on the PTY and forwards raw chunks;
//! - **waiter** blocks on the child and reports its exit;
//! - **pump** coalesces chunks into batches, feeds the headless terminal and fans out to viewers.

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::types::{
    AttachmentId, ExitInfo, HostError, HostEvent, LaunchPlan, Result, SessionId, SessionInfo,
    SessionState, TermSize,
};
use crate::{snapshot, EventSink, OutputSink};

/// Lines of history kept per session for snapshots.
const SCROLLBACK_LINES: usize = 10_000;
const READ_CHUNK: usize = 64 * 1024;
/// A full-screen program repaints in bursts of small writes. Gathering them for a few
/// milliseconds turns hundreds of IPC messages per second into at most ~120.
const BATCH_WINDOW: Duration = Duration::from_millis(8);
const MAX_BATCH: usize = 512 * 1024;
/// After the child exits, how long the PTY must stay quiet before we stop waiting for more
/// output. Needed where end-of-file never arrives: ConPTY on Windows, or a background
/// grandchild that still holds the terminal open.
const EXIT_DRAIN_QUIET: Duration = Duration::from_millis(150);

/// DSR 6, "report cursor position". The reply is `ESC [ row ; col R`.
const CURSOR_POSITION_QUERY: &[u8] = b"\x1b[6n";

enum Msg {
    Output(Vec<u8>),
    Eof,
    ChildExited(ExitInfo),
}

pub(crate) struct Session {
    id: SessionId,
    plan: LaunchPlan,
    pid: Option<u32>,
    io: Mutex<Io>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    view: Mutex<View>,
}

/// The write side. Dropped when the session ends, which also unblocks the reader on Windows.
struct Io {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
}

/// What viewers see: the headless terminal plus whoever is currently watching.
struct View {
    parser: vt100::Parser,
    size: TermSize,
    sinks: Vec<(AttachmentId, OutputSink)>,
    next_attachment: AttachmentId,
    state: SessionState,
    last_output: Instant,
    has_output: bool,
    busy: bool,
}

impl Session {
    pub(crate) fn spawn(
        plan: LaunchPlan,
        events: EventSink,
        quiet_after: Duration,
    ) -> Result<Arc<Self>> {
        let size = plan.size.sanitized();
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| HostError::OpenPty(e.to_string()))?;

        let mut child =
            pair.slave
                .spawn_command(command_for(&plan))
                .map_err(|e| HostError::Spawn {
                    program: plan.program.clone(),
                    reason: e.to_string(),
                })?;
        // Keep no handle to the slave side, or we would never see end-of-file.
        drop(pair.slave);

        // `portable-pty` reports failures as `anyhow::Error`; only the message matters to us.
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| HostError::OpenPty(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| HostError::OpenPty(e.to_string()))?;

        let session = Arc::new(Self {
            id: SessionId::generate(),
            pid: child.process_id(),
            killer: Mutex::new(child.clone_killer()),
            io: Mutex::new(Io {
                master: Some(pair.master),
                writer: Some(writer),
            }),
            view: Mutex::new(View {
                parser: vt100::Parser::new(size.rows, size.cols, SCROLLBACK_LINES),
                size,
                sinks: Vec::new(),
                next_attachment: 1,
                state: SessionState::Running,
                last_output: Instant::now(),
                has_output: false,
                busy: false,
            }),
            plan,
        });

        let (tx, rx) = mpsc::channel();
        let name = |role: &str| format!("pty-{role}-{}", &session.id.0[..8]);

        let reader_tx = tx.clone();
        thread::Builder::new()
            .name(name("reader"))
            .spawn(move || read_loop(reader, &reader_tx))?;

        thread::Builder::new().name(name("waiter")).spawn(move || {
            let exit = match child.wait() {
                Ok(status) => ExitInfo {
                    code: status.exit_code(),
                    success: status.success(),
                    signal: status.signal().map(str::to_owned),
                },
                Err(_) => ExitInfo {
                    code: 1,
                    success: false,
                    signal: None,
                },
            };
            let _ = tx.send(Msg::ChildExited(exit));
        })?;

        let pumped = Arc::clone(&session);
        thread::Builder::new()
            .name(name("pump"))
            .spawn(move || pumped.pump(&rx, &events, quiet_after))?;

        Ok(session)
    }

    pub(crate) fn id(&self) -> &SessionId {
        &self.id
    }

    pub(crate) fn info(&self) -> SessionInfo {
        let view = self.view();
        SessionInfo {
            id: self.id.clone(),
            program: self.plan.program.clone(),
            args: self.plan.args.clone(),
            cwd: self.plan.cwd.clone(),
            pid: self.pid,
            size: view.size,
            labels: self.plan.labels.clone(),
            state: view.state.clone(),
            has_output: view.has_output,
            busy: view.busy,
            idle_ms: u32::try_from(view.last_output.elapsed().as_millis()).unwrap_or(u32::MAX),
        }
    }

    pub(crate) fn attach(&self, mut sink: OutputSink) -> AttachmentId {
        // Snapshot and registration happen under the lock the pump takes to deliver output, so
        // the viewer sees every byte exactly once: in the snapshot or in the stream, never both.
        let mut view = self.view();
        let id = view.next_attachment;
        view.next_attachment += 1;
        let snapshot = snapshot::render(&mut view.parser);
        if sink(&snapshot) {
            view.sinks.push((id, sink));
        }
        id
    }

    pub(crate) fn detach(&self, attachment: AttachmentId) {
        self.view().sinks.retain(|(id, _)| *id != attachment);
    }

    pub(crate) fn write(&self, data: &[u8]) -> Result<()> {
        let mut io = self.io();
        let writer = io
            .writer
            .as_mut()
            .ok_or_else(|| HostError::SessionExited(self.id.clone()))?;
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }

    /// Deliver `text` the way a terminal delivers a paste: wrapped in bracketed-paste markers if
    /// the program asked for them (so newlines stay part of the text instead of submitting it),
    /// otherwise as typed input with carriage returns for line breaks.
    pub(crate) fn paste(&self, text: &str) -> Result<()> {
        let bracketed = self.view().parser.screen().bracketed_paste();
        let normalized = text.replace("\r\n", "\n");
        let payload = if bracketed {
            // The end marker inside the text would end the paste early; no program needs it.
            format!("\x1b[200~{}\x1b[201~", normalized.replace("\x1b[201~", ""))
        } else {
            normalized.replace('\n', "\r")
        };
        self.write(payload.as_bytes())
    }

    pub(crate) fn resize(&self, size: TermSize) -> Result<()> {
        let size = size.sanitized();
        if let Some(master) = self.io().master.as_ref() {
            master
                .resize(PtySize {
                    rows: size.rows,
                    cols: size.cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| HostError::Io(std::io::Error::other(e.to_string())))?;
        }
        // Exited sessions still resize their headless terminal so late snapshots fit the viewer.
        let mut view = self.view();
        view.size = size;
        view.parser.screen_mut().set_size(size.rows, size.cols);
        Ok(())
    }

    pub(crate) fn kill(&self) -> Result<()> {
        if matches!(self.view().state, SessionState::Exited { .. }) {
            return Err(HostError::SessionExited(self.id.clone()));
        }
        let result = self
            .killer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .kill();
        // portable-pty 0.9 inverts the success check of `TerminateProcess` in its cloned killer,
        // so on Windows a successful kill comes back as an error (carrying a stale OS error).
        // The waiter thread is the source of truth either way: `Exited` follows a real kill.
        if cfg!(windows) {
            return Ok(());
        }
        Ok(result?)
    }

    fn pump(&self, rx: &Receiver<Msg>, events: &EventSink, quiet_after: Duration) {
        let mut exit: Option<ExitInfo> = None;
        let mut eof = false;
        let mut batch = Vec::new();
        // When the current burst of output began, while there is one.
        let mut busy_since: Option<Instant> = None;

        while !(eof && exit.is_some()) {
            // Once the child is gone, stop as soon as the PTY goes quiet.
            let first = if exit.is_some() {
                match rx.recv_timeout(EXIT_DRAIN_QUIET) {
                    Ok(msg) => msg,
                    Err(_) => break,
                }
            } else if let Some(since) = busy_since {
                // Mid-burst: wake up if the output stops, to say so.
                match rx.recv_timeout(quiet_after) {
                    Ok(msg) => msg,
                    Err(RecvTimeoutError::Timeout) => {
                        busy_since = None;
                        self.view().busy = false;
                        events(HostEvent::Quiet {
                            id: self.id.clone(),
                            // The burst ended when the output did, not when we noticed.
                            busy_ms: millis(since.elapsed().saturating_sub(quiet_after)),
                        });
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match rx.recv() {
                    Ok(msg) => msg,
                    Err(_) => break,
                }
            };

            let mut absorb = |msg: Msg, batch: &mut Vec<u8>| match msg {
                Msg::Output(bytes) => batch.extend_from_slice(&bytes),
                Msg::Eof => eof = true,
                Msg::ChildExited(info) => exit = Some(info),
            };
            absorb(first, &mut batch);

            let deadline = Instant::now() + BATCH_WINDOW;
            while batch.len() < MAX_BATCH {
                let wait = deadline.saturating_duration_since(Instant::now());
                match rx.recv_timeout(wait) {
                    Ok(msg) => absorb(msg, &mut batch),
                    Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
                }
            }

            if !batch.is_empty() {
                if busy_since.is_none() {
                    busy_since = Some(Instant::now());
                    self.view().busy = true;
                    events(HostEvent::Busy {
                        id: self.id.clone(),
                    });
                }
                let replies = self.deliver(&batch);
                batch.clear();
                if !replies.is_empty() {
                    // Best effort: the process may already be gone.
                    let _ = self.write(&replies);
                }
            }
        }

        let exit = exit.unwrap_or(ExitInfo {
            code: 1,
            success: false,
            signal: None,
        });

        // Announce the exit *before* tearing the PTY down, and tear it down on a thread of its
        // own: on Windows, dropping the master calls `ClosePseudoConsole`, which can block until
        // conhost has flushed and gone. Nothing may wait on that — least of all the exit event.
        let (master, writer) = {
            let mut io = self.io();
            (io.master.take(), io.writer.take())
        };
        {
            let mut view = self.view();
            view.busy = false;
            view.state = SessionState::Exited { exit: exit.clone() };
        }
        events(HostEvent::Exited {
            id: self.id.clone(),
            exit,
        });
        let _ = thread::Builder::new()
            .name(format!("pty-close-{}", &self.id.0[..8]))
            .spawn(move || {
                drop(writer);
                drop(master);
            });
    }

    /// Feed a batch to the headless terminal and the viewers. Returns bytes the *terminal* owes
    /// the program in response, which the caller writes back to the PTY.
    ///
    /// A program can ask the terminal where the cursor is (`ESC [ 6 n`) and wait for the answer.
    /// Normally the attached xterm.js replies. With nobody attached there is no terminal to
    /// reply, so the host answers from its headless one. This is not an edge case: ConPTY asks
    /// exactly this at startup and runs nothing until it hears back, so without an answer a
    /// Windows session started before its view attaches would hang forever. Deciding under the
    /// view lock guarantees exactly one reply: ours, or the viewer's.
    fn deliver(&self, batch: &[u8]) -> Vec<u8> {
        let mut view = self.view();
        let unattended = view.sinks.is_empty();
        view.parser.process(batch);
        view.last_output = Instant::now();
        view.has_output = true;
        view.sinks.retain_mut(|(_, sink)| sink(batch));

        let mut replies = Vec::new();
        if unattended {
            let queries = batch
                .windows(CURSOR_POSITION_QUERY.len())
                .filter(|w| *w == CURSOR_POSITION_QUERY)
                .count();
            if queries > 0 {
                let (row, col) = view.parser.screen().cursor_position();
                let report = format!("\x1b[{};{}R", row + 1, col + 1);
                for _ in 0..queries {
                    replies.extend_from_slice(report.as_bytes());
                }
            }
        }
        replies
    }

    fn view(&self) -> MutexGuard<'_, View> {
        self.view.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn io(&self) -> MutexGuard<'_, Io> {
        self.io.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn millis(duration: Duration) -> u32 {
    u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}

fn read_loop(mut reader: Box<dyn Read + Send>, tx: &Sender<Msg>) {
    let mut buf = vec![0u8; READ_CHUNK];
    loop {
        match reader.read(&mut buf) {
            // Linux reports a closed PTY as EIO rather than end-of-file; both mean "done".
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if tx.send(Msg::Output(buf[..n].to_vec())).is_err() {
                    return;
                }
            }
        }
    }
    let _ = tx.send(Msg::Eof);
}

fn command_for(plan: &LaunchPlan) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(&plan.program);
    cmd.args(&plan.args);
    if let Some(cwd) = &plan.cwd {
        cmd.cwd(cwd);
    }
    if plan.clear_env {
        cmd.env_clear();
    }
    // Every session is a colour-capable xterm as far as the program is concerned, unless told
    // otherwise by the plan.
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    for (key, value) in &plan.env {
        cmd.env(key, value);
    }
    cmd
}
