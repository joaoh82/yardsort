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
    AttachmentId, Controller, EventSink, HostError, HostEvent, LaunchPlan, Lease, LeaseRef,
    NumberedSink, OutputSink, Result, SessionId, SessionInfo, TermSize, TerminalHost,
};

use crate::endpoint::Endpoint;
use crate::proto::{
    DaemonInfo, Frame, Op, OpResult, Request, RequestId, Response, StreamId, CONTROL_PROTOCOL,
    KIND_EVENT, KIND_NUMBERED, KIND_OUTPUT, KIND_REQUEST, KIND_RESPONSE, OLDEST_PROTOCOL, PROTOCOL,
};

/// How long to wait for the daemon to answer. Generous: a spawn forks a process, and the machine
/// may be busy. Short enough that a wedged daemon surfaces as an error instead of a frozen app.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

type Pending = Mutex<HashMap<RequestId, Sender<OpResult>>>;
type Sinks = Mutex<HashMap<StreamId, OutputSink>>;
type Watchers = Mutex<HashMap<StreamId, NumberedSink>>;

pub struct DaemonClient {
    /// One socket, read by the reader thread and written under `sending`. `&Stream` is both
    /// `Read` and `Write`, so both sides share the one connection rather than opening two.
    conn: Arc<Stream>,
    sending: Mutex<()>,
    pending: Arc<Pending>,
    sinks: Arc<Sinks>,
    watchers: Arc<Watchers>,
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
        let watchers: Arc<Watchers> = Arc::default();
        let connected = Arc::new(AtomicBool::new(true));

        let reading = Arc::clone(&conn);
        let (their_pending, their_flag) = (Arc::clone(&pending), Arc::clone(&connected));
        let streams = Streams {
            sinks: Arc::clone(&sinks),
            watchers: Arc::clone(&watchers),
        };
        std::thread::Builder::new()
            .name("ipc-reader".into())
            .spawn(move || {
                let mut reader = BufReader::new(&*reading);
                read_loop(&mut reader, &their_pending, &streams, &events);
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
            watchers,
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

    /// Whether this build can run terminals in the daemon at all: it speaks our protocol, or an
    /// older one whose every message we still know.
    pub fn understood(&self) -> bool {
        (OLDEST_PROTOCOL..=PROTOCOL).contains(&self.info.protocol)
    }

    /// Whether the daemon numbers output and hands out control leases. One that does not
    /// still runs terminals; they just cannot be shared beyond this computer.
    pub fn shares_control(&self) -> bool {
        (CONTROL_PROTOCOL..=PROTOCOL).contains(&self.info.protocol)
    }

    /// Send a request only a daemon that shares control can read. An older one would drop the
    /// connection on it, taking every terminal this client shows with it.
    fn control_request(&self, op: Op) -> Result<OpResult> {
        if !self.shares_control() {
            return Err(HostError::ControlUnavailable);
        }
        self.request(op)
    }

    fn expect_lease(&self, op: Op) -> Result<Lease> {
        match self.control_request(op)? {
            OpResult::Leased { lease } => Ok(lease),
            OpResult::Failed { error } => Err(error.into()),
            other => Err(unexpected(&other)),
        }
    }

    fn expect_control_done(&self, op: Op) -> Result<()> {
        match self.control_request(op)? {
            OpResult::Done => Ok(()),
            OpResult::Failed { error } => Err(error.into()),
            other => Err(unexpected(&other)),
        }
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
        self.watchers
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

    /// As [`TerminalHost::attach`], numbered. Detach with [`TerminalHost::detach`].
    fn attach_numbered(&self, id: &SessionId, sink: NumberedSink) -> Result<AttachmentId> {
        if !self.shares_control() {
            return Err(HostError::ControlUnavailable);
        }
        let stream = self.next_stream.fetch_add(1, Ordering::Relaxed);
        self.watchers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(stream, sink);
        match self.expect_control_done(Op::Watch {
            session: id.clone(),
            stream,
        }) {
            Ok(()) => Ok(stream),
            Err(error) => {
                self.watchers
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&stream);
                Err(error)
            }
        }
    }

    fn controller(&self, id: &SessionId) -> Result<Controller> {
        match self.control_request(Op::Controller {
            session: id.clone(),
        })? {
            OpResult::Controller { controller } => Ok(controller),
            OpResult::Failed { error } => Err(error.into()),
            other => Err(unexpected(&other)),
        }
    }

    fn acquire(&self, id: &SessionId, holder: &str, take_over: bool) -> Result<Lease> {
        self.expect_lease(Op::Acquire {
            session: id.clone(),
            holder: holder.to_owned(),
            take_over,
        })
    }

    fn renew(&self, id: &SessionId, lease: &LeaseRef) -> Result<Lease> {
        self.expect_lease(Op::Renew {
            session: id.clone(),
            lease: lease.clone(),
        })
    }

    fn release(&self, id: &SessionId, lease: &LeaseRef) -> Result<()> {
        self.expect_control_done(Op::Release {
            session: id.clone(),
            lease: lease.clone(),
        })
    }

    fn input(
        &self,
        id: &SessionId,
        lease: &LeaseRef,
        seq: u64,
        data: &str,
        paste: bool,
    ) -> Result<()> {
        self.expect_control_done(Op::Input {
            session: id.clone(),
            lease: lease.clone(),
            seq,
            data: data.to_owned(),
            paste,
        })
    }

    fn resize_leased(&self, id: &SessionId, lease: &LeaseRef, size: TermSize) -> Result<()> {
        self.expect_control_done(Op::ResizeLeased {
            session: id.clone(),
            lease: lease.clone(),
            size,
        })
    }
}

/// Where output frames go: plain viewers, and numbered ones.
struct Streams {
    sinks: Arc<Sinks>,
    watchers: Arc<Watchers>,
}

/// Demultiplex everything the daemon sends: responses to whoever is waiting, events to the app's
/// sink, output to the viewer it belongs to.
fn read_loop(
    reader: &mut impl std::io::Read,
    pending: &Pending,
    streams: &Streams,
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
                let mut sinks = streams.sinks.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some(sink) = sinks.get_mut(&stream) {
                    // A sink that says it is done — a closed webview channel — is dropped here;
                    // the daemon notices at its own pace when the connection goes.
                    if !sink(bytes) {
                        sinks.remove(&stream);
                    }
                }
            }
            KIND_NUMBERED => {
                let Ok((stream, item)) = frame.as_numbered() else {
                    continue;
                };
                let mut watchers = streams
                    .watchers
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if let Some(sink) = watchers.get_mut(&stream) {
                    if !sink(item) {
                        watchers.remove(&stream);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{Response, KIND_RESPONSE};
    use interprocess::local_socket::traits::ListenerExt as _;
    use interprocess::local_socket::ListenerOptions;

    /// A daemon of protocol 1 drops the connection on a request it cannot read, and every
    /// terminal the app shows through that connection with it. So a client never sends one
    /// there: it refuses, and the daemon hears nothing.
    #[test]
    fn an_older_daemon_still_runs_terminals_and_is_never_sent_what_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::for_data_dir(dir.path());
        let listener = ListenerOptions::new()
            .name(endpoint.to_name().unwrap())
            .create_sync()
            .unwrap();
        let (heard_tx, heard) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            let stream = listener.incoming().next().unwrap().unwrap();
            let mut reader = BufReader::new(&stream);
            while let Ok(frame) = Frame::read_from(&mut reader) {
                let request: Request = frame.parse().unwrap();
                let op = serde_json::to_value(&request.op).unwrap()["op"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let result = match op.as_str() {
                    "hello" => OpResult::Welcome {
                        daemon: DaemonInfo {
                            protocol: 1,
                            version: "0.20.2".into(),
                            pid: 1,
                            started_at: 0,
                            running_sessions: 2,
                        },
                    },
                    "list" => OpResult::Sessions { sessions: vec![] },
                    _ => OpResult::Done,
                };
                let _ = heard_tx.send(op);
                let response = Frame::json(
                    KIND_RESPONSE,
                    &Response {
                        id: request.id,
                        result,
                    },
                )
                .unwrap();
                let mut writer: &interprocess::local_socket::Stream = &stream;
                response.write_to(&mut writer).unwrap();
            }
        });

        let client = DaemonClient::connect(&endpoint, "test", Arc::new(|_| {})).unwrap();
        assert!(client.understood());
        assert!(!client.speaks_our_protocol());
        assert!(!client.shares_control());
        let id = SessionId("s".into());
        let lease = LeaseRef {
            id: "l".into(),
            generation: 1,
        };
        let refused = |result: Result<()>| {
            assert!(
                matches!(result, Err(HostError::ControlUnavailable)),
                "{result:?}"
            );
        };
        refused(client.attach_numbered(&id, Box::new(|_| true)).map(|_| ()));
        refused(client.controller(&id).map(|_| ()));
        refused(client.acquire(&id, "phone", false).map(|_| ()));
        refused(client.renew(&id, &lease).map(|_| ()));
        refused(client.release(&id, &lease));
        refused(client.input(&id, &lease, 1, "x", false));
        refused(client.resize_leased(&id, &lease, TermSize::DEFAULT));
        // What protocol 1 has still works.
        assert!(client.list().is_empty());

        let heard: Vec<String> = heard.try_iter().collect();
        assert_eq!(heard, ["hello", "list"]);
    }
}
