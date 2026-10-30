//! One PTY, the process in it, and the three threads that service it:
//!
//! - **reader** blocks on the PTY and forwards raw chunks;
//! - **waiter** blocks on the child and reports its exit;
//! - **pump** coalesces chunks into batches, feeds the headless terminal and fans out to viewers.
//!
//! The view lock also holds who controls the session ([`Control`]), because that decides which
//! viewer is shown the program's terminal queries (see [`crate::queries`]).

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::control::{Control, Controller, Delivery, Holds, Lease, LeaseRef, Refusal};
use crate::queries::{self, Queries};
use crate::replay::Replay;
use crate::snapshot::Headless;
use crate::types::{
    AttachmentId, ExitInfo, HostError, HostEvent, LaunchPlan, Numbered, OutputSeq, Result,
    SessionId, SessionInfo, SessionState, TermSize,
};
use crate::{EventSink, NumberedSink, OutputSink};

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
/// How long after a terminal is shown a query its answer may come back. A terminal answers in
/// milliseconds; a webview busy with something else, in rather more.
const REPLY_WINDOW: Duration = Duration::from_secs(2);

enum Msg {
    Output(Vec<u8>),
    Eof,
    ChildExited(ExitInfo),
    /// A hold now lapses sooner than the pump may be waiting for: look again.
    Wake,
}

pub(crate) struct Session {
    id: SessionId,
    plan: LaunchPlan,
    pid: Option<u32>,
    io: Mutex<Io>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    view: Mutex<View>,
    /// The pump's own channel, to wake it.
    wake: Mutex<Sender<Msg>>,
}

/// The write side. Dropped when the session ends, which also unblocks the reader on Windows.
struct Io {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
}

/// What viewers see: the headless terminal plus whoever is currently watching.
struct View {
    terminal: Headless,
    size: TermSize,
    sinks: Vec<(AttachmentId, Viewer)>,
    next_attachment: AttachmentId,
    /// The last item of output (or resize) handed out. See [`OutputSeq`].
    seq: OutputSeq,
    state: SessionState,
    last_output: Instant,
    has_output: bool,
    busy: bool,
    control: Control,
    queries: Queries,
    /// The size the person here last asked for. A lease holder's size wins while it holds
    /// control; this one comes back when control does.
    local_size: TermSize,
    /// Until when the responder's answer to the last query it was shown may arrive.
    answers_until: Option<Instant>,
    /// The last stretch of the numbered stream, from the first numbered viewer on.
    replay: Option<Replay>,
    /// Who numbered viewers were last told controls the session.
    announced: Controller,
}

enum Viewer {
    /// A terminal emulator, which answers the program's queries itself.
    Terminal(OutputSink),
    /// A numbered watcher, which never answers.
    Numbered(NumberedSink),
}

impl View {
    fn next_attachment(&mut self) -> AttachmentId {
        let id = self.next_attachment;
        self.next_attachment += 1;
        id
    }

    /// The one viewer shown the program's queries, which its terminal then answers: the
    /// terminal attached most recently, the one the person most likely just opened. `None` when
    /// a lease holder controls the session, or no terminal is attached, and the host answers
    /// what it can itself. A lease holder's own terminal does not answer yet: the remote side
    /// only watches, numbered.
    fn responder(&mut self, now: Instant) -> Option<AttachmentId> {
        if self.control.leased(now) {
            return None;
        }
        self.sinks
            .iter()
            .rev()
            .find(|(_, viewer)| matches!(viewer, Viewer::Terminal(_)))
            .map(|(id, _)| *id)
    }

    /// Tell numbered viewers who controls the session, if that changed since they were last
    /// told: control changing hands, or a hold lapsing, which nobody else would notice.
    fn announce(&mut self, now: Instant) {
        let controller = self.control.controller(now);
        if controller == self.announced {
            return;
        }
        self.announced = controller.clone();
        self.sinks.retain_mut(|(_, viewer)| match viewer {
            Viewer::Terminal(_) => true,
            Viewer::Numbered(sink) => sink(Numbered::Control(controller.clone())),
        });
    }
}

impl Session {
    pub(crate) fn spawn(
        plan: LaunchPlan,
        events: EventSink,
        quiet_after: Duration,
        holds: Holds,
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

        let (tx, rx) = mpsc::channel();
        let session = Arc::new(Self {
            wake: Mutex::new(tx.clone()),
            id: SessionId::generate(),
            pid: child.process_id(),
            killer: Mutex::new(child.clone_killer()),
            io: Mutex::new(Io {
                master: Some(pair.master),
                writer: Some(writer),
            }),
            view: Mutex::new(View {
                terminal: Headless::new(size.rows, size.cols, SCROLLBACK_LINES),
                size,
                sinks: Vec::new(),
                next_attachment: 1,
                seq: 0,
                state: SessionState::Running,
                last_output: Instant::now(),
                has_output: false,
                busy: false,
                control: Control::new(holds),
                queries: Queries::default(),
                local_size: size,
                answers_until: None,
                replay: None,
                announced: Controller::Nobody,
            }),
            plan,
        });

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
        let id = view.next_attachment();
        let snapshot = view.terminal.snapshot();
        if sink(&snapshot) {
            view.sinks.push((id, Viewer::Terminal(sink)));
        }
        id
    }

    /// As [`Session::attach`], with every item numbered: the snapshot is the terminal after item
    /// `seq`, and the first item after it is `seq + 1`. With `after`, the items since that one
    /// instead, when they are all still kept (see [`crate::replay`]).
    pub(crate) fn attach_numbered(
        &self,
        after: Option<OutputSeq>,
        mut sink: NumberedSink,
    ) -> AttachmentId {
        let mut view = self.view();
        let view = &mut *view;
        let id = view.next_attachment();
        let replay = view
            .replay
            .get_or_insert_with(|| Replay::new(view.seq, view.size));
        let mut open = match after.and_then(|after| Some((after, replay.since(after)?))) {
            Some((seq, (size, items))) => {
                let mut open = sink(Numbered::Resumed { seq, size });
                for item in items {
                    open = open && sink(item);
                }
                open
            }
            None => sink(Numbered::Snapshot {
                seq: view.seq,
                size: view.size,
                bytes: &view.terminal.snapshot(),
            }),
        };
        view.announce(Instant::now());
        open = open && sink(Numbered::Control(view.announced.clone()));
        if let SessionState::Exited { exit } = &view.state {
            open = open && sink(Numbered::Exited(exit.clone()));
        }
        if open {
            view.sinks.push((id, Viewer::Numbered(sink)));
        }
        id
    }

    pub(crate) fn detach(&self, attachment: AttachmentId) {
        self.view().sinks.retain(|(id, _)| *id != attachment);
    }

    /// Something typed at this computer: it takes control of the session, from a lease holder
    /// if there is one, and puts back the size the person here asked for. A terminal answering
    /// a query it was just shown is not typing, and takes nothing.
    pub(crate) fn local_write(&self, data: &[u8]) -> Result<()> {
        let answering = {
            let view = self.view();
            view.answers_until
                .is_some_and(|until| Instant::now() < until)
                && queries::is_reply(data)
        };
        if !answering {
            self.take_local_control()?;
        }
        self.write(data)
    }

    /// As [`Session::local_write`], for a paste.
    pub(crate) fn local_paste(&self, text: &str) -> Result<()> {
        self.take_local_control()?;
        self.paste(text)
    }

    /// Typing the app does on its own, such as a workflow's step: it takes no control, and is
    /// refused while a lease holder has the session, so it never types over someone in control
    /// from elsewhere.
    pub(crate) fn unattended(&self, text: &str, paste: bool) -> Result<()> {
        let payload = self.controlling(|view, now| {
            if view.control.leased(now) {
                let by = view.control.controller(now);
                return Err(self.refused(Refusal::Held(by)));
            }
            Ok(if paste {
                pasted(text, view.terminal.screen().bracketed_paste())
            } else {
                text.to_owned()
            })
        })?;
        self.write(payload.as_bytes())
    }

    fn take_local_control(&self) -> Result<()> {
        self.controlling(|view, now| {
            view.control.local_input(now);
            let size = view.local_size;
            self.apply_size(view, size)
        })
    }

    /// Do something that may change who controls the session, or find that a hold lapsed, and
    /// then tell numbered viewers if it did. Whether `f` succeeded or not: a refusal can be the
    /// first to notice a lapse.
    ///
    /// A hold that now lapses sooner than before wakes the pump, which waits for lapses (see
    /// [`Session::pump`]); one that lapses later needs nothing: the pump wakes early, finds it
    /// held, and waits again.
    fn controlling<T>(&self, f: impl FnOnce(&mut View, Instant) -> Result<T>) -> Result<T> {
        let mut view = self.view();
        let now = Instant::now();
        let before = view.control.lapses_at();
        let result = f(&mut view, now);
        view.announce(now);
        if let Some(after) = view.control.lapses_at() {
            if before.is_none_or(|before| after < before) {
                let _ = self
                    .wake
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .send(Msg::Wake);
            }
        }
        result
    }

    /// Write to the program, whoever controls it: the host's own replies, and a launch's first
    /// message.
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

    /// Paste `text`, whoever controls the session (see [`pasted`]): a launch's first message.
    pub(crate) fn paste(&self, text: &str) -> Result<()> {
        let bracketed = self.view().terminal.screen().bracketed_paste();
        self.write(pasted(text, bracketed).as_bytes())
    }

    /// The person here resized their view. While a lease holder controls the session its size
    /// stands, and this one is kept for when control comes back.
    pub(crate) fn resize(&self, size: TermSize) -> Result<()> {
        let size = size.sanitized();
        self.controlling(|view, now| {
            view.local_size = size;
            if view.control.leased(now) {
                return Ok(());
            }
            self.apply_size(view, size)
        })
    }

    pub(crate) fn controller(&self) -> Controller {
        let mut view = self.view();
        view.announce(Instant::now());
        view.announced.clone()
    }

    pub(crate) fn acquire(&self, holder: &str, take_over: bool) -> Result<Lease> {
        self.controlling(|view, now| {
            if matches!(view.state, SessionState::Exited { .. }) {
                return Err(HostError::SessionExited(self.id.clone()));
            }
            view.control
                .acquire(holder, take_over, now)
                .map_err(|refusal| self.refused(refusal))
        })
    }

    pub(crate) fn renew(&self, lease: &LeaseRef) -> Result<Lease> {
        self.controlling(|view, now| {
            view.control
                .renew(lease, now)
                .map_err(|refusal| self.refused(refusal))
        })
    }

    /// Give control back, and the person here their size.
    pub(crate) fn release(&self, lease: &LeaseRef) -> Result<()> {
        self.controlling(|view, now| {
            if view.control.release(lease, now) {
                let size = view.local_size;
                self.apply_size(view, size)?;
            }
            Ok(())
        })
    }

    /// Input number `seq` from a lease holder: written once, in order, and only while the
    /// lease is in force.
    pub(crate) fn input(&self, lease: &LeaseRef, seq: u64, data: &str, paste: bool) -> Result<()> {
        let payload = self.controlling(|view, now| {
            Ok(match view.control.input(lease, seq, now) {
                Ok(Delivery::Fresh) if paste => {
                    Some(pasted(data, view.terminal.screen().bracketed_paste()))
                }
                Ok(Delivery::Fresh) => Some(data.to_owned()),
                Ok(Delivery::Repeat) => None,
                Err(refusal) => return Err(self.refused(refusal)),
            })
        })?;
        match payload {
            Some(payload) => self.write(payload.as_bytes()),
            None => Ok(()),
        }
    }

    /// A lease holder resizing the terminal to its own view.
    pub(crate) fn resize_leased(&self, lease: &LeaseRef, size: TermSize) -> Result<()> {
        self.controlling(|view, now| {
            view.control
                .check(lease, now)
                .map_err(|refusal| self.refused(refusal))?;
            self.apply_size(view, size.sanitized())
        })
    }

    fn refused(&self, refusal: Refusal) -> HostError {
        let id = self.id.clone();
        match refusal {
            Refusal::Lost => HostError::LeaseLost(id),
            Refusal::OutOfOrder { expected, got } => {
                HostError::InputOutOfOrder { id, expected, got }
            }
            Refusal::Held(by) => HostError::ControlHeld { id, by },
            Refusal::BadHolder => HostError::InvalidHolder,
        }
    }

    /// Resize the PTY and the headless terminal, and tell numbered viewers. Called with the view
    /// lock held, and held across the PTY's own resize, so no output the program draws at the
    /// new size can reach a numbered viewer before the item that says the size changed.
    fn apply_size(&self, view: &mut View, size: TermSize) -> Result<()> {
        if view.size == size {
            return Ok(());
        }
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
        view.size = size;
        view.terminal.set_size(size.rows, size.cols);
        view.seq += 1;
        let seq = view.seq;
        if let Some(replay) = &mut view.replay {
            replay.resized(size);
        }
        view.sinks.retain_mut(|(_, viewer)| match viewer {
            Viewer::Terminal(_) => true,
            Viewer::Numbered(sink) => sink(Numbered::Resized { seq, size }),
        });
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
        // When output last went out, so the batch window can be measured from it.
        let mut last_deliver: Option<Instant> = None;

        while !(eof && exit.is_some()) {
            // When the hold in force lapses. Asked here, so the lock is let go before waiting.
            let lapses_at = self.view().control.lapses_at();
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
            } else if let Some(at) = lapses_at {
                // A hold lapsing changes who controls the session with nothing happening: wake
                // when it does, to say so.
                match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                    Ok(msg) => msg,
                    Err(RecvTimeoutError::Timeout) => {
                        self.view().announce(Instant::now());
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
                Msg::Wake => {}
            };
            absorb(first, &mut batch);

            // Gather what follows into the same batch, but only until `BATCH_WINDOW` after the
            // *previous* delivery — not after this chunk turned up. Output that arrives into a
            // quiet terminal has nothing to be coalesced with, so it goes out at once; only a
            // stream fast enough to keep refilling the window is held back, which is the case
            // the window exists for. Timing from this chunk instead puts the whole window in
            // front of every keystroke echo.
            let deadline = last_deliver.map_or_else(Instant::now, |at| at + BATCH_WINDOW);
            while batch.len() < MAX_BATCH {
                let wait = deadline.saturating_duration_since(Instant::now());
                if wait.is_zero() {
                    break;
                }
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
                last_deliver = Some(Instant::now());
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
            view.sinks.retain_mut(|(_, viewer)| match viewer {
                Viewer::Terminal(_) => true,
                Viewer::Numbered(sink) => sink(Numbered::Exited(exit.clone())),
            });
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
    /// A program can ask its terminal things and wait for the answer, and exactly one terminal
    /// must answer: two answers and the program reads the second as typing. So only the
    /// responder (see [`View::responder`]) is sent the queries; every other viewer, numbered
    /// ones included, gets the batch without them. Deciding under the view lock means the
    /// choice cannot change halfway through a batch.
    ///
    /// When there is no responder, the host answers where the cursor is (`ESC [ 6 n`) from its
    /// headless terminal. This is not an edge case: ConPTY asks exactly this at startup and runs
    /// nothing until it hears back, so without an answer a Windows session started before its
    /// view attaches would hang forever.
    fn deliver(&self, batch: &[u8]) -> Vec<u8> {
        let mut view = self.view();
        let now = Instant::now();
        view.announce(now);
        let responder = view.responder(now);
        let scan = view.queries.feed(batch);
        let without = (!scan.is_empty()).then(|| scan.without(batch));
        let quiet = without.as_deref().unwrap_or(batch);
        view.seq += 1;
        let seq = view.seq;
        if let Some(replay) = &mut view.replay {
            replay.output(quiet);
        }
        // Hand the bytes to the viewers before parsing them into the headless terminal. The
        // parse is only there for snapshots, and nothing below needs it except the cursor
        // reply, which is only reached when there is no terminal to have waited for it.
        view.sinks.retain_mut(|(id, viewer)| {
            let bytes = if Some(*id) == responder { batch } else { quiet };
            match viewer {
                Viewer::Terminal(_) if bytes.is_empty() => true,
                Viewer::Terminal(sink) => sink(bytes),
                Viewer::Numbered(sink) => sink(Numbered::Output { seq, bytes }),
            }
        });
        view.terminal.feed(batch);
        view.last_output = Instant::now();
        view.has_output = true;

        if responder.is_some() && !scan.is_empty() {
            view.answers_until = Some(Instant::now() + REPLY_WINDOW);
        }
        let mut replies = Vec::new();
        if responder.is_none() && scan.cursor_queries() > 0 {
            let (row, col) = view.terminal.screen().cursor_position();
            let report = format!("\x1b[{};{}R", row + 1, col + 1);
            for _ in 0..scan.cursor_queries() {
                replies.extend_from_slice(report.as_bytes());
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

/// `text` the way a terminal delivers a paste: wrapped in bracketed-paste markers if the program
/// asked for them (so newlines stay part of the text instead of submitting it), otherwise as
/// typed input with carriage returns for line breaks.
fn pasted(text: &str, bracketed: bool) -> String {
    let normalized = text.replace("\r\n", "\n");
    if bracketed {
        // The end marker inside the text would end the paste early; no program needs it.
        format!("\x1b[200~{}\x1b[201~", normalized.replace("\x1b[201~", ""))
    } else {
        normalized.replace('\n', "\r")
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
