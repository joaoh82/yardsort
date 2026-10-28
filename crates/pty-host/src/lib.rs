//! The PTY host: owns pseudo-terminal sessions and the processes running in them.
//!
//! This crate deliberately knows nothing about Tauri, windows or the UI. Its API is shaped like
//! messages — serialisable requests in, byte streams and events out, sessions addressed by id —
//! which is what lets the very same host run in-process or, as it does in the shipping app, in
//! the `yardsortd` background process with the app as a client of it. See `pty-ipc`; nothing on
//! this side of the boundary knows the difference.
//!
//! ```text
//! spawn(LaunchPlan) -> SessionInfo        list() -> [SessionInfo]
//! attach(id, sink)  -> AttachmentId       detach(id, attachment)
//! attach_numbered(id, sink) -> AttachmentId
//! write(id, bytes) / paste(id, text)       resize(id, size)
//! kill(id)                                remove(id)
//! acquire / renew / release(id, …)         input / resize_leased(id, lease, …)
//! events: HostEvent::{Exited, Busy, Quiet}
//! ```
//!
//! [`TerminalHost`] is that API as a trait, implemented here and by the daemon's client.
//!
//! **One controller.** `write`, `paste` and `resize` are the person at this computer, who always
//! has control when they type. Anyone else asks for a lease first and types under it, numbered
//! (see [`control`]). **One responder.** Only one viewer is shown the program's terminal queries,
//! so only one terminal answers them; the others get the output without them (see [`queries`]).
//!
//! Each session keeps a headless terminal (`vt100`) fed with everything the process prints. A
//! viewer that attaches — for the first time or the tenth — first receives a snapshot that
//! repaints scrollback and screen, then the live stream, with nothing lost or duplicated between
//! the two — wherever the cut falls, even inside an escape sequence (see [`snapshot`]). A
//! numbered viewer is also told which item of the stream the snapshot holds.

pub mod control;
mod prompt;
mod queries;
mod session;
pub mod snapshot;
mod types;
mod unfinished;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

pub use control::{Controller, Lease, LeaseRef};
use session::Session;
pub use types::{
    AttachmentId, ExitInfo, HostError, HostEvent, LaunchPlan, Numbered, OutputSeq, PendingPrompt,
    Result, SessionId, SessionInfo, SessionState, TermSize,
};

/// Receives a session's output: first one snapshot, then live batches. Return `false` to detach.
///
/// Called while the session's lock is held — on its pump thread, or on the thread attaching —
/// so it must be quick and must not call back into the host.
pub type OutputSink = Box<dyn FnMut(&[u8]) -> bool + Send>;

/// Receives a session's output with its place in the stream: see [`PtyHost::attach_numbered`].
/// Return `false` to detach. Called under the session's lock, like an [`OutputSink`].
pub type NumberedSink = Box<dyn FnMut(Numbered<'_>) -> bool + Send>;

/// Receives host-wide events. Called from session threads.
pub type EventSink = Arc<dyn Fn(HostEvent) + Send + Sync>;

/// How long a session must print nothing before it counts as quiet. Long enough that a program
/// pausing between lines stays "busy"; agents animate a spinner while they think, so for them
/// silence really does mean "waiting for you".
pub const DEFAULT_QUIET_AFTER: Duration = Duration::from_secs(3);

pub struct PtyHost {
    sessions: Mutex<HashMap<SessionId, Arc<Session>>>,
    events: EventSink,
    quiet_after: Duration,
}

impl PtyHost {
    pub fn new(events: EventSink) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            events,
            quiet_after: DEFAULT_QUIET_AFTER,
        }
    }

    /// Use a different quiet period for sessions spawned from now on.
    pub fn with_quiet_after(mut self, quiet_after: Duration) -> Self {
        self.quiet_after = quiet_after;
        self
    }

    /// Start `plan.program` in a new PTY.
    pub fn spawn(&self, plan: LaunchPlan) -> Result<SessionInfo> {
        let pending = plan.prompt.clone();
        let session = Session::spawn(plan, Arc::clone(&self.events), self.quiet_after)?;
        let info = session.info();
        self.lock().insert(info.id.clone(), Arc::clone(&session));
        // Delivery runs here rather than in the client, so the message still arrives if whoever
        // asked for the session goes away while the program is still starting up.
        if let Some(pending) = pending {
            prompt::deliver(session, pending);
        }
        Ok(info)
    }

    /// Start receiving a session's output. The sink is handed a snapshot of the current terminal
    /// state before this returns, then live output. Works on exited sessions too, which is how
    /// their final screen stays viewable.
    pub fn attach(&self, id: &SessionId, sink: OutputSink) -> Result<AttachmentId> {
        Ok(self.get(id)?.attach(sink))
    }

    /// Watch a session's output knowing where each piece falls: a snapshot tagged with the item
    /// it reflects, then every later item numbered from there. What a viewer across a network
    /// needs in order to prove it lost nothing and doubled nothing.
    ///
    /// A numbered viewer watches; it is not a terminal. It is never shown the program's queries,
    /// and it does not stop the host from answering them when no terminal is attached. Detach
    /// with [`PtyHost::detach`].
    pub fn attach_numbered(&self, id: &SessionId, sink: NumberedSink) -> Result<AttachmentId> {
        Ok(self.get(id)?.attach_numbered(sink))
    }

    pub fn detach(&self, id: &SessionId, attachment: AttachmentId) -> Result<()> {
        self.get(id)?.detach(attachment);
        Ok(())
    }

    /// Type into the session from this computer. Takes control of it, from a lease holder if
    /// one has it.
    pub fn write(&self, id: &SessionId, data: &[u8]) -> Result<()> {
        self.get(id)?.local_write(data)
    }

    /// Paste text into the session, bracketed if the program enabled bracketed paste. Takes
    /// control, as [`PtyHost::write`] does.
    pub fn paste(&self, id: &SessionId, text: &str) -> Result<()> {
        self.get(id)?.local_paste(text)
    }

    /// Type for the app itself rather than for the person here, such as a workflow's step.
    /// Nobody typed it, so it takes no control; and while someone holds a lease it is refused
    /// with [`HostError::ControlHeld`] rather than type over them.
    pub fn type_unattended(&self, id: &SessionId, text: &str, paste: bool) -> Result<()> {
        self.get(id)?.unattended(text, paste)
    }

    /// Resize to this computer's view. While a lease holder controls the session, its size
    /// stands, and this one is put back when control returns.
    pub fn resize(&self, id: &SessionId, size: TermSize) -> Result<()> {
        self.get(id)?.resize(size)
    }

    /// Who may type into the session now.
    pub fn controller(&self, id: &SessionId) -> Result<Controller> {
        Ok(self.get(id)?.controller())
    }

    /// Ask for control of a session as `holder`. Refused while the person here has it, and
    /// while another holder does unless `take_over`.
    pub fn acquire(&self, id: &SessionId, holder: &str, take_over: bool) -> Result<Lease> {
        self.get(id)?.acquire(holder, take_over)
    }

    pub fn renew(&self, id: &SessionId, lease: &LeaseRef) -> Result<Lease> {
        self.get(id)?.renew(lease)
    }

    pub fn release(&self, id: &SessionId, lease: &LeaseRef) -> Result<()> {
        self.get(id)?.release(lease)
    }

    /// Type under a lease. `seq` is 1 for the first input under it, then one more each time:
    /// a repeat is dropped, and a gap refused.
    pub fn input(
        &self,
        id: &SessionId,
        lease: &LeaseRef,
        seq: u64,
        data: &str,
        paste: bool,
    ) -> Result<()> {
        self.get(id)?.input(lease, seq, data, paste)
    }

    pub fn resize_leased(&self, id: &SessionId, lease: &LeaseRef, size: TermSize) -> Result<()> {
        self.get(id)?.resize_leased(lease, size)
    }

    /// Ask the session's process to terminate. `Exited` follows once it has.
    pub fn kill(&self, id: &SessionId) -> Result<()> {
        self.get(id)?.kill()
    }

    /// Forget a session, killing it first if it is still running.
    pub fn remove(&self, id: &SessionId) -> Result<()> {
        let session = self
            .lock()
            .remove(id)
            .ok_or_else(|| HostError::UnknownSession(id.clone()))?;
        // Already-exited sessions report an error here; either way it is gone.
        let _ = session.kill();
        Ok(())
    }

    pub fn info(&self, id: &SessionId) -> Result<SessionInfo> {
        Ok(self.get(id)?.info())
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let mut all: Vec<_> = self.lock().values().map(|s| s.info()).collect();
        all.sort_by(|a, b| a.id.cmp(&b.id));
        all
    }

    fn get(&self, id: &SessionId) -> Result<Arc<Session>> {
        self.lock()
            .get(id)
            .cloned()
            .ok_or_else(|| HostError::UnknownSession(id.clone()))
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<SessionId, Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Pause between pasting a message and pressing Enter: some TUIs drop an Enter that arrives with
/// the paste.
pub const SUBMIT_DELAY: Duration = Duration::from_millis(150);

/// Type `text` into a running session as one paste, then press Enter — a message sent the way a
/// person would send it. Whatever was already in the program's input goes with it.
pub fn paste_and_submit(host: &dyn TerminalHost, id: &SessionId, text: &str) -> Result<()> {
    host.paste(id, text)?;
    std::thread::sleep(SUBMIT_DELAY);
    host.write(id, b"\r")
}

/// What a client of the PTY host can ask for, whether the host is in this process or in the
/// daemon at the other end of a socket. The app holds one of these and never knows which.
///
/// Numbered viewing and leases have defaults that refuse with [`HostError::ControlUnavailable`],
/// for hosts that cannot offer them: a daemon from before they existed, or a test's stand-in.
pub trait TerminalHost: Send + Sync {
    fn spawn(&self, plan: LaunchPlan) -> Result<SessionInfo>;
    fn attach(&self, id: &SessionId, sink: OutputSink) -> Result<AttachmentId>;
    fn detach(&self, id: &SessionId, attachment: AttachmentId) -> Result<()>;
    fn write(&self, id: &SessionId, data: &[u8]) -> Result<()>;
    fn paste(&self, id: &SessionId, text: &str) -> Result<()>;
    fn resize(&self, id: &SessionId, size: TermSize) -> Result<()>;
    fn kill(&self, id: &SessionId) -> Result<()>;
    fn remove(&self, id: &SessionId) -> Result<()>;
    fn info(&self, id: &SessionId) -> Result<SessionInfo>;
    fn list(&self) -> Vec<SessionInfo>;

    /// A host without leases has nobody to type over: it types as the person here would.
    fn type_unattended(&self, id: &SessionId, text: &str, paste: bool) -> Result<()> {
        if paste {
            self.paste(id, text)
        } else {
            self.write(id, text.as_bytes())
        }
    }

    fn attach_numbered(&self, _id: &SessionId, _sink: NumberedSink) -> Result<AttachmentId> {
        Err(HostError::ControlUnavailable)
    }
    fn controller(&self, _id: &SessionId) -> Result<Controller> {
        Err(HostError::ControlUnavailable)
    }
    fn acquire(&self, _id: &SessionId, _holder: &str, _take_over: bool) -> Result<Lease> {
        Err(HostError::ControlUnavailable)
    }
    fn renew(&self, _id: &SessionId, _lease: &LeaseRef) -> Result<Lease> {
        Err(HostError::ControlUnavailable)
    }
    fn release(&self, _id: &SessionId, _lease: &LeaseRef) -> Result<()> {
        Err(HostError::ControlUnavailable)
    }
    fn input(
        &self,
        _id: &SessionId,
        _lease: &LeaseRef,
        _seq: u64,
        _data: &str,
        _paste: bool,
    ) -> Result<()> {
        Err(HostError::ControlUnavailable)
    }
    fn resize_leased(&self, _id: &SessionId, _lease: &LeaseRef, _size: TermSize) -> Result<()> {
        Err(HostError::ControlUnavailable)
    }
}

impl TerminalHost for PtyHost {
    fn spawn(&self, plan: LaunchPlan) -> Result<SessionInfo> {
        PtyHost::spawn(self, plan)
    }

    fn attach(&self, id: &SessionId, sink: OutputSink) -> Result<AttachmentId> {
        PtyHost::attach(self, id, sink)
    }

    fn detach(&self, id: &SessionId, attachment: AttachmentId) -> Result<()> {
        PtyHost::detach(self, id, attachment)
    }

    fn write(&self, id: &SessionId, data: &[u8]) -> Result<()> {
        PtyHost::write(self, id, data)
    }

    fn paste(&self, id: &SessionId, text: &str) -> Result<()> {
        PtyHost::paste(self, id, text)
    }

    fn resize(&self, id: &SessionId, size: TermSize) -> Result<()> {
        PtyHost::resize(self, id, size)
    }

    fn kill(&self, id: &SessionId) -> Result<()> {
        PtyHost::kill(self, id)
    }

    fn remove(&self, id: &SessionId) -> Result<()> {
        PtyHost::remove(self, id)
    }

    fn info(&self, id: &SessionId) -> Result<SessionInfo> {
        PtyHost::info(self, id)
    }

    fn list(&self) -> Vec<SessionInfo> {
        PtyHost::list(self)
    }

    fn type_unattended(&self, id: &SessionId, text: &str, paste: bool) -> Result<()> {
        PtyHost::type_unattended(self, id, text, paste)
    }

    fn attach_numbered(&self, id: &SessionId, sink: NumberedSink) -> Result<AttachmentId> {
        PtyHost::attach_numbered(self, id, sink)
    }

    fn controller(&self, id: &SessionId) -> Result<Controller> {
        PtyHost::controller(self, id)
    }

    fn acquire(&self, id: &SessionId, holder: &str, take_over: bool) -> Result<Lease> {
        PtyHost::acquire(self, id, holder, take_over)
    }

    fn renew(&self, id: &SessionId, lease: &LeaseRef) -> Result<Lease> {
        PtyHost::renew(self, id, lease)
    }

    fn release(&self, id: &SessionId, lease: &LeaseRef) -> Result<()> {
        PtyHost::release(self, id, lease)
    }

    fn input(
        &self,
        id: &SessionId,
        lease: &LeaseRef,
        seq: u64,
        data: &str,
        paste: bool,
    ) -> Result<()> {
        PtyHost::input(self, id, lease, seq, data, paste)
    }

    fn resize_leased(&self, id: &SessionId, lease: &LeaseRef, size: TermSize) -> Result<()> {
        PtyHost::resize_leased(self, id, lease, size)
    }
}

impl Drop for PtyHost {
    fn drop(&mut self) {
        for session in self.lock().values() {
            let _ = session.kill();
        }
    }
}
