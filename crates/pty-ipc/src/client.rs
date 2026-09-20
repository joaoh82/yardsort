//! The app side: a [`TerminalHost`] whose sessions live in another process.
//!
//! Everything above this — `terminal.rs`, `sessions.rs`, the whole webview — is written against
//! the same trait the in-process host implements, and cannot tell the difference.

use std::collections::HashMap;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use interprocess::local_socket::traits::Stream as _;
use interprocess::local_socket::Stream;
use pty_host::{
    AttachmentId, EventSink, HostError, HostEvent, LaunchPlan, OutputSink, Result, SessionId,
    SessionInfo, TermSize, TerminalHost,
};

use crate::endpoint::Endpoint;
use crate::proto::{
    DaemonInfo, Frame, Op, OpResult, Request, RequestId, Response, StreamId, KIND_EVENT,
    KIND_OUTPUT, KIND_REQUEST, KIND_RESPONSE, PROTOCOL,
};

/// How long to wait for the daemon to answer. Generous: a spawn forks a process, and the machine
/// may be busy. Short enough that a wedged daemon surfaces as an error instead of a frozen app.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

type Pending = Mutex<HashMap<RequestId, Sender<OpResult>>>;
type Sinks = Mutex<HashMap<StreamId, OutputSink>>;

pub struct DaemonClient {
    /// One socket, read by the reader thread and written under `sending`. `&Stream` is both
    /// `Read` and `Write`, so both sides share the one connection rather than opening two.
    conn: Arc<Stream>,
    sending: Mutex<()>,
    pending: Arc<Pending>,
    sinks: Arc<Sinks>,
    connected: Arc<AtomicBool>,
    next_request: AtomicU64,
    next_stream: AtomicU32,
    info: DaemonInfo,
}

impl DaemonClient {
    /// Connect to a daemon that is already listening, and shake hands with it.
    ///
    /// This does not start one: when to spawn a daemon, and what to do about one too old to talk
    /// to, is the app's decision — see its `daemon` module.
    pub fn connect(endpoint: &Endpoint, client: &str, events: EventSink) -> std::io::Result<Self> {
        let conn = Arc::new(Stream::connect(endpoint.to_name()?)?);
        let pending: Arc<Pending> = Arc::default();
        let sinks: Arc<Sinks> = Arc::default();
        let connected = Arc::new(AtomicBool::new(true));

        let reading = Arc::clone(&conn);
        let (their_pending, their_sinks, their_flag) = (
            Arc::clone(&pending),
            Arc::clone(&sinks),
            Arc::clone(&connected),
        );
        std::thread::Builder::new()
            .name("ipc-reader".into())
            .spawn(move || {
                let mut reader = BufReader::new(&*reading);
                read_loop(&mut reader, &their_pending, &their_sinks, &events);
                // Nothing more will arrive: fail everyone still waiting instead of leaving them
                // to time out one by one.
                their_flag.store(false, Ordering::Relaxed);
                their_pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clear();
            })?;

        let who = format!("{client} (pid {})", std::process::id());
        let mut client = Self {
            conn,
            sending: Mutex::new(()),
            pending,
            sinks,
            connected,
            next_request: AtomicU64::new(1),
            next_stream: AtomicU32::new(1),
            info: DaemonInfo {
                protocol: 0,
                version: String::new(),
                pid: 0,
                started_at: 0,
                running_sessions: 0,
            },
        };
        let hello = Op::Hello {
            protocol: PROTOCOL,
            client: who,
        };
        match client.request(hello) {
            Ok(OpResult::Welcome { daemon }) => {
                client.info = daemon;
                Ok(client)
            }
            Ok(_) => Err(std::io::Error::other("the daemon did not introduce itself")),
            Err(error) => Err(std::io::Error::other(error.to_string())),
        }
    }

    /// What answered the handshake: protocol, version, pid, and how many agents it is running.
    /// Named apart from [`TerminalHost::info`], which asks about a *session*.
    pub fn daemon_info(&self) -> &DaemonInfo {
        &self.info
    }

    /// Whether this build and the daemon speak the same language.
    pub fn speaks_our_protocol(&self) -> bool {
        self.info.protocol == PROTOCOL
    }

    /// Hang up. The daemon closes its side, which ends our reader thread and closes the socket
    /// for real — without this the thread would sit blocked on a read for ever, and the daemon
    /// would go on counting a client that is no longer there.
    ///
    /// Nothing this client started is affected: that is the whole point of the daemon.
    pub fn disconnect(&self) {
        let id = self.next_request.fetch_add(1, Ordering::Relaxed);
        let _ = self.send(&Request {
            id,
            op: Op::Goodbye,
        });
    }

    /// Ask the daemon to stop. With `stop_sessions`, everything it runs is killed first.
    pub fn shutdown(&self, stop_sessions: bool) -> Result<()> {
        self.expect_done(Op::Shutdown { stop_sessions })
    }

    /// Send a request and wait for its answer.
    fn request(&self, op: Op) -> Result<OpResult> {
        let id = self.next_request.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, tx);

        if let Err(error) = self.send(&Request { id, op }) {
            self.forget(id);
            return Err(error);
        }
        match rx.recv_timeout(REQUEST_TIMEOUT) {
            Ok(result) => Ok(result),
            // The reader clears every pending sender when the connection ends, so this is "the
            // daemon is gone" at least as often as it is "the daemon is stuck".
            Err(RecvTimeoutError::Disconnected) => Err(self.lost()),
            Err(RecvTimeoutError::Timeout) => {
                self.forget(id);
                Err(HostError::Io(std::io::Error::other(
                    "the terminal daemon did not answer in time",
                )))
            }
        }
    }

    fn forget(&self, id: RequestId) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
    }

    fn send(&self, request: &Request) -> Result<()> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(self.lost());
        }
        let frame = Frame::json(KIND_REQUEST, request).map_err(HostError::Io)?;
        let _one_at_a_time = self.sending.lock().unwrap_or_else(PoisonError::into_inner);
        let mut socket: &Stream = &self.conn;
        frame.write_to(&mut socket).map_err(|_| self.lost())
    }

    fn lost(&self) -> HostError {
        self.connected.store(false, Ordering::Relaxed);
        HostError::Io(std::io::Error::other(
            "lost the connection to the terminal daemon",
        ))
    }

    fn expect_done(&self, op: Op) -> Result<()> {
        match self.request(op)? {
            OpResult::Done => Ok(()),
            OpResult::Failed { error } => Err(error.into()),
            other => Err(unexpected(&other)),
        }
    }

    fn expect_session(&self, op: Op) -> Result<SessionInfo> {
        match self.request(op)? {
            OpResult::Session { session } => Ok(*session),
            OpResult::Failed { error } => Err(error.into()),
            other => Err(unexpected(&other)),
        }
    }
}

fn unexpected(result: &OpResult) -> HostError {
    HostError::Io(std::io::Error::other(format!(
        "the terminal daemon answered with something unexpected: {result:?}"
    )))
}

impl TerminalHost for DaemonClient {
    fn spawn(&self, plan: LaunchPlan) -> Result<SessionInfo> {
        self.expect_session(Op::Spawn {
            plan: Box::new(plan),
        })
    }

    /// The returned [`AttachmentId`] is the stream id this client chose, which is what `detach`
    /// takes back — the daemon keeps the host's own attachment id on its side of the socket.
    fn attach(&self, id: &SessionId, sink: OutputSink) -> Result<AttachmentId> {
        // Register before asking: the snapshot starts arriving while the daemon is still inside
        // its own `attach`, which is to say before the response to this request does.
        let stream = self.next_stream.fetch_add(1, Ordering::Relaxed);
        self.sinks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(stream, sink);
        match self.expect_done(Op::Attach {
            session: id.clone(),
            stream,
        }) {
            Ok(()) => Ok(stream),
            Err(error) => {
                self.sinks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&stream);
                Err(error)
            }
        }
    }

    fn detach(&self, id: &SessionId, attachment: AttachmentId) -> Result<()> {
        self.sinks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&attachment);
        self.expect_done(Op::Detach {
            session: id.clone(),
            stream: attachment,
        })
    }

    fn write(&self, id: &SessionId, data: &[u8]) -> Result<()> {
        self.expect_done(Op::Write {
            session: id.clone(),
            data: data.to_vec(),
        })
    }

    fn paste(&self, id: &SessionId, text: &str) -> Result<()> {
        self.expect_done(Op::Paste {
            session: id.clone(),
            text: text.to_owned(),
        })
    }

    fn resize(&self, id: &SessionId, size: TermSize) -> Result<()> {
        self.expect_done(Op::Resize {
            session: id.clone(),
            size,
        })
    }

    fn kill(&self, id: &SessionId) -> Result<()> {
        self.expect_done(Op::Kill {
            session: id.clone(),
        })
    }

    fn remove(&self, id: &SessionId) -> Result<()> {
        self.expect_done(Op::Remove {
            session: id.clone(),
        })
    }

    fn info(&self, id: &SessionId) -> Result<SessionInfo> {
        self.expect_session(Op::Info {
            session: id.clone(),
        })
    }

    fn list(&self) -> Vec<SessionInfo> {
        match self.request(Op::List) {
            Ok(OpResult::Sessions { sessions }) => sessions,
            // `list` has nowhere to report a failure, and an unreachable daemon has no sessions
            // to show; the app finds out through the next call that can return an error. Say so
            // on the way past, though — an empty list is far too quiet a way to fail.
            other => {
                eprintln!("could not list the daemon's sessions: {other:?}");
                Vec::new()
            }
        }
    }
}

/// Demultiplex everything the daemon sends: responses to whoever is waiting, events to the app's
/// sink, output to the viewer it belongs to.
fn read_loop(
    reader: &mut impl std::io::Read,
    pending: &Pending,
    sinks: &Sinks,
    events: &EventSink,
) {
    while let Ok(frame) = Frame::read_from(reader) {
        match frame.kind {
            KIND_RESPONSE => {
                let Ok(response) = frame.parse::<Response>() else {
                    continue;
                };
                if let Some(waiting) = pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&response.id)
                {
                    let _ = waiting.send(response.result);
                }
            }
            KIND_EVENT => {
                if let Ok(event) = frame.parse::<HostEvent>() {
                    events(event);
                }
            }
            KIND_OUTPUT => {
                let Ok((stream, bytes)) = frame.as_output() else {
                    continue;
                };
                let mut sinks = sinks.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(sink) = sinks.get_mut(&stream) {
                    // A sink that says it is done — a closed webview channel — is dropped here;
                    // the daemon notices at its own pace when the connection goes.
                    if !sink(bytes) {
                        sinks.remove(&stream);
                    }
                }
            }
            _ => {}
        }
    }
}

impl Drop for DaemonClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}
