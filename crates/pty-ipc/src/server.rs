//! The daemon side: one [`PtyHost`], however many clients.
//!
//! A client going away is an ordinary event here — that is the whole point of the daemon — so
//! disconnecting never touches a session. Sessions die when they are killed, when their program
//! exits, or when the daemon is told to shut down.

use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use interprocess::local_socket::traits::ListenerExt as _;
use interprocess::local_socket::{ListenerOptions, Stream};
use pty_host::{AttachmentId, HostEvent, PtyHost, SessionState};

use crate::endpoint::Endpoint;
use crate::proto::{
    event_frame, DaemonInfo, Frame, Op, OpResult, Request, Response, StreamId, WireError,
    KIND_REQUEST, KIND_RESPONSE, PROTOCOL,
};

/// How long the daemon lingers with no clients and no sessions before exiting. Long enough that
/// restarting the app, or reloading the webview, does not take the daemon down with it.
pub const IDLE_GRACE: Duration = Duration::from_secs(10);

/// A client that stops reading must not be allowed to stall the session writing to it: output is
/// queued for the connection's own writer thread, and a connection that lets this much pile up
/// is dropped. Nothing is corrupted by that — the client reconnects and reattaches, and an
/// attach always starts with a snapshot.
const MAX_QUEUED_BYTES: usize = 32 * 1024 * 1024;

/// How long a shutdown waits for the sessions it killed to actually go.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
/// The gap between a session's state changing and its `Exited` event being broadcast.
const EXIT_ANNOUNCE_GRACE: Duration = Duration::from_millis(200);

/// Run the daemon until it is told to stop, or until `idle_grace` has passed with nothing to
/// look after — no client connected and no session running.
pub fn serve(endpoint: &Endpoint, version: &str, idle_grace: Duration) -> std::io::Result<()> {
    if let Some(dir) = endpoint.parent_dir() {
        std::fs::create_dir_all(dir)?;
        private(dir)?;
    }
    let listener = ListenerOptions::new()
        .name(endpoint.to_name()?)
        // A daemon that died without tidying up leaves a socket nobody can connect to. The app
        // only reaches this point after failing to connect, so taking it over is right.
        .try_overwrite(true)
        .create_sync()?;

    let conns: Conns = Arc::new(Mutex::new(Vec::new()));
    let broadcasting = Arc::clone(&conns);
    let host = Arc::new(PtyHost::new(Arc::new(move |event| {
        broadcast(&broadcasting, &event);
    })));

    let daemon = Arc::new(Daemon {
        host,
        conns,
        version: version.to_owned(),
        started_at: epoch_ms(),
    });
    daemon.watch_for_idleness(idle_grace);

    eprintln!("yardsort daemon {version} listening on {endpoint}");
    for incoming in listener.incoming() {
        let stream = match incoming {
            Ok(stream) => stream,
            // One failed accept is not a reason to abandon the agents already running.
            Err(error) => {
                eprintln!("incoming connection failed: {error}");
                continue;
            }
        };
        let daemon = Arc::clone(&daemon);
        std::thread::Builder::new()
            .name("ipc-conn".into())
            .spawn(move || daemon.serve_connection(stream))?;
    }
    Ok(())
}

type Conns = Arc<Mutex<Vec<Weak<Conn>>>>;

struct Daemon {
    host: Arc<PtyHost>,
    conns: Conns,
    version: String,
    started_at: u64,
}

impl Daemon {
    fn info(&self) -> DaemonInfo {
        DaemonInfo {
            protocol: PROTOCOL,
            version: self.version.clone(),
            pid: std::process::id(),
            started_at: self.started_at,
            running_sessions: running(&self.host),
        }
    }

    /// Exit once there is nothing left to look after: no client watching, no session running.
    /// Without this, every profile ever used would leave a process behind for ever.
    fn watch_for_idleness(self: &Arc<Self>, grace: Duration) {
        let daemon = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("ipc-idle".into())
            .spawn(move || {
                let mut idle_since: Option<Instant> = None;
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    let busy = !daemon.live_conns().is_empty() || running(&daemon.host) > 0;
                    if busy {
                        idle_since = None;
                        continue;
                    }
                    let since = *idle_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= grace {
                        eprintln!("nothing left to look after; exiting");
                        std::process::exit(0);
                    }
                }
            });
        if let Err(error) = spawned {
            eprintln!("could not watch for idleness: {error}");
        }
    }

    fn live_conns(&self) -> Vec<Arc<Conn>> {
        let mut conns = self.conns.lock().unwrap_or_else(PoisonError::into_inner);
        conns.retain(|conn| conn.strong_count() > 0);
        conns.iter().filter_map(Weak::upgrade).collect()
    }

    fn serve_connection(&self, stream: Stream) {
        let stream = Arc::new(stream);
        let conn = Conn::new(Arc::clone(&stream));
        self.conns
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::downgrade(&conn));

        let mut reader = BufReader::new(&*stream);
        // Reading stops when the client goes, which includes the ordinary case: the app closed.
        while let Ok(frame) = Frame::read_from(&mut reader) {
            if frame.kind != KIND_REQUEST {
                continue;
            }
            let request: Request = match frame.parse() {
                Ok(request) => request,
                Err(error) => {
                    eprintln!("unintelligible request: {error}");
                    break;
                }
            };
            if matches!(request.op, Op::Goodbye) {
                break;
            }
            let shutdown = matches!(request.op, Op::Shutdown { .. });
            let result = self.handle(&conn, request.op);
            let response = Response {
                id: request.id,
                result,
            };
            match Frame::json(KIND_RESPONSE, &response) {
                Ok(frame) => conn.send(frame),
                Err(error) => eprintln!("could not encode a response: {error}"),
            }
            if shutdown {
                // Let the answer reach the client before the process goes.
                conn.flush(Duration::from_secs(2));
                break;
            }
        }
        // Whatever this client was watching, it is not watching any more. The sessions carry on.
        for (session, attachment) in conn.take_attachments() {
            let _ = self.host.detach(&session, attachment);
        }
    }

    fn handle(&self, conn: &Arc<Conn>, op: Op) -> OpResult {
        match op {
            Op::Hello { protocol, client } => {
                if protocol != PROTOCOL {
                    eprintln!(
                        "client {client} speaks protocol {protocol}, this daemon speaks {PROTOCOL}"
                    );
                }
                // Answered whether or not the versions match: the reply is how the app finds out
                // what it is talking to, and how many agents are at stake.
                OpResult::Welcome {
                    daemon: self.info(),
                }
            }
            Op::Spawn { plan } => match self.host.spawn(*plan) {
                Ok(info) => OpResult::Session {
                    session: Box::new(info),
                },
                Err(error) => failed(error),
            },
            Op::Attach { session, stream } => {
                let out = Arc::clone(conn);
                let sink = Box::new(move |bytes: &[u8]| out.send_output(stream, bytes));
                match self.host.attach(&session, sink) {
                    Ok(attachment) => {
                        conn.remember_attachment(session, stream, attachment);
                        OpResult::Done
                    }
                    Err(error) => failed(error),
                }
            }
            Op::Detach { session, stream } => {
                if let Some(attachment) = conn.forget_attachment(stream) {
                    let _ = self.host.detach(&session, attachment);
                }
                OpResult::Done
            }
            Op::Write { session, data } => done(self.host.write(&session, &data)),
            Op::Paste { session, text } => done(self.host.paste(&session, &text)),
            Op::Resize { session, size } => done(self.host.resize(&session, size)),
            Op::Kill { session } => done(self.host.kill(&session)),
            Op::Remove { session } => done(self.host.remove(&session)),
            Op::Info { session } => match self.host.info(&session) {
                Ok(info) => OpResult::Session {
                    session: Box::new(info),
                },
                Err(error) => failed(error),
            },
            Op::List => OpResult::Sessions {
                sessions: self.host.list(),
            },
            // Handled before it gets here: the connection simply ends.
            Op::Goodbye => OpResult::Done,
            Op::Shutdown { stop_sessions } => {
                if stop_sessions {
                    for session in self.host.list() {
                        let _ = self.host.kill(&session.id);
                    }
                    // "Stop them" has to mean stopped by the time this answers. Killing only
                    // asks; the processes go at their own pace, and on Windows tearing a
                    // pseudo-console down is not quick. Without this wait the daemon could exit
                    // before the exits were announced, and the app would quit with its session
                    // records still claiming to be running.
                    let deadline = Instant::now() + STOP_TIMEOUT;
                    while running(&self.host) > 0 && Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    // A session's state flips to exited just *before* its `Exited` event goes
                    // out, so give the last ones a moment to reach the queue that `flush` drains.
                    std::thread::sleep(EXIT_ANNOUNCE_GRACE);
                }
                // The process ends once the answer is out; see `serve_connection`.
                std::thread::Builder::new()
                    .name("ipc-shutdown".into())
                    .spawn(|| {
                        std::thread::sleep(Duration::from_secs(3));
                        std::process::exit(0);
                    })
                    .ok();
                OpResult::Done
            }
        }
    }
}

fn done(result: pty_host::Result<()>) -> OpResult {
    match result {
        Ok(()) => OpResult::Done,
        Err(error) => failed(error),
    }
}

fn failed(error: pty_host::HostError) -> OpResult {
    OpResult::Failed {
        error: WireError::from(error),
    }
}

fn running(host: &Arc<PtyHost>) -> u32 {
    let live = host
        .list()
        .iter()
        .filter(|s| matches!(s.state, SessionState::Running))
        .count();
    u32::try_from(live).unwrap_or(u32::MAX)
}

fn broadcast(conns: &Conns, event: &HostEvent) {
    let Ok(frame) = event_frame(event) else {
        return;
    };
    let mut conns = conns.lock().unwrap_or_else(PoisonError::into_inner);
    conns.retain(|conn| conn.strong_count() > 0);
    for conn in conns.iter().filter_map(Weak::upgrade) {
        conn.send(frame.clone());
    }
}

/// One connected client. Everything it is sent — responses, events, output — goes out through a
/// single queue, so frames never interleave and a session's pump thread never waits on a socket.
struct Conn {
    queue: Sender<Vec<u8>>,
    queued: Arc<AtomicUsize>,
    gone: Arc<AtomicBool>,
    /// What this client is watching: its stream id → the host's attachment.
    attachments: Mutex<HashMap<StreamId, (pty_host::SessionId, AttachmentId)>>,
}

impl Conn {
    fn new(stream: Arc<Stream>) -> Arc<Self> {
        let (queue, rx) = mpsc::channel::<Vec<u8>>();
        let queued = Arc::new(AtomicUsize::new(0));
        let gone = Arc::new(AtomicBool::new(false));

        let (counter, flag) = (Arc::clone(&queued), Arc::clone(&gone));
        let writer = std::thread::Builder::new()
            .name("ipc-writer".into())
            .spawn(move || {
                let mut out = BufWriter::new(&*stream);
                while let Ok(bytes) = rx.recv() {
                    counter.fetch_sub(bytes.len(), Ordering::Relaxed);
                    if out.write_all(&bytes).and_then(|()| out.flush()).is_err() {
                        break;
                    }
                }
                flag.store(true, Ordering::Relaxed);
            });
        if let Err(error) = writer {
            eprintln!("could not start a connection writer: {error}");
            gone.store(true, Ordering::Relaxed);
        }

        Arc::new(Self {
            queue,
            queued,
            gone,
            attachments: Mutex::new(HashMap::new()),
        })
    }

    fn send(&self, frame: Frame) {
        if self.gone.load(Ordering::Relaxed) {
            return;
        }
        let bytes = frame.encode();
        let pending = self.queued.fetch_add(bytes.len(), Ordering::Relaxed) + bytes.len();
        if pending > MAX_QUEUED_BYTES {
            eprintln!("a client stopped reading with {pending} bytes queued; dropping it");
            self.gone.store(true, Ordering::Relaxed);
            return;
        }
        if self.queue.send(bytes).is_err() {
            self.gone.store(true, Ordering::Relaxed);
        }
    }

    /// Returns false when this viewer is gone, which is how the host detaches it.
    fn send_output(&self, stream: StreamId, bytes: &[u8]) -> bool {
        self.send(Frame::output(stream, bytes));
        !self.gone.load(Ordering::Relaxed)
    }

    /// Wait for the queue to drain, so a last frame is not lost to a process exiting.
    fn flush(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while self.queued.load(Ordering::Relaxed) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn remember_attachment(
        &self,
        session: pty_host::SessionId,
        stream: StreamId,
        attachment: AttachmentId,
    ) {
        self.attachments
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(stream, (session, attachment));
    }

    fn forget_attachment(&self, stream: StreamId) -> Option<AttachmentId> {
        self.attachments
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&stream)
            .map(|(_, attachment)| attachment)
    }

    fn take_attachments(&self) -> Vec<(pty_host::SessionId, AttachmentId)> {
        std::mem::take(
            &mut *self
                .attachments
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
        .into_values()
        .collect()
    }
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Keep the socket's directory to its owner. On Windows the pipe's default security descriptor
/// already limits it to the creating user's session.
#[cfg(unix)]
fn private(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn private(_dir: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}
