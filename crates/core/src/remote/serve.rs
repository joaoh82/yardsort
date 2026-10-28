//! One device's channel through the relay, as the host answers it: its greeting, its requests,
//! the terminals it watches and the leases it holds.
//!
//! The relay opens a channel with the device and the grant the server has in force for it
//! ([`remote_proto::relay::ChannelOpen`]). [`Visitor::open`] lets it in only if this host's own
//! record agrees ([`host::grant_for`]), and every request after that looks the grant up again,
//! so a grant revoked here stops the next request at once, and [`Visitor::allowed`] tells the
//! connector to close the channel.
//!
//! Read requests go to the [`HostFacade`]. The terminal operations are answered here, after the
//! facade's checks, because they outlive one request:
//!
//! - **Watching** is a numbered attachment to the daemon. The reply comes first, then the
//!   snapshot, then everything after it, each numbered (see [`remote_proto::terminal`]). A device
//!   more than [`remote_proto::MAX_QUEUED_OUTPUT`] behind is detached with
//!   `resnapshot_required`: the daemon never waits for it.
//! - **Control** is a daemon lease, held as `device:<id>`. A lease is this channel's: another
//!   device naming its id gets `lease_lost`, exactly as for a lease that is gone.
//! - **Closing the channel** — the device left, its grant went, the relay or the network failed —
//!   detaches everything it watched and releases every lease it held. The terminals and their
//!   programs are untouched: they belong to the daemon, not to the connection.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use pty_host::{AttachmentId, HostError, Numbered, SessionId, TerminalHost};
use remote_proto::relay::CloseReason;
use remote_proto::terminal::{
    AcquireRequest, AttachRequest, AttachStart, Attached, Controller, DetachRequest, Detached,
    InputRequest, Lease, LeaseRef, OutputFrame, ResizeRequest, Resized, StreamId, StreamPosition,
};
use remote_proto::{
    DeviceId, ErrorCode, Event, Grant, GrantId, Hello, HelloReply, LeaseId, Operation, Outcome,
    RemoteError, Reply, Request, Response, TermSize, MAX_OUTPUT_CHUNK,
};

use super::{host, HostFacade, HostIdentity};
use crate::git::Git;
use crate::store::Store;

/// The epoch of every stream. Output numbers are the daemon's, which count from a terminal's
/// start for its whole life, so they never need a new epoch; a client that asks to resume is
/// sent a fresh snapshot, since no replay is kept.
const EPOCH: u32 = 0;

/// The prefix of a device's name as a lease holder in the daemon.
const HOLDER_PREFIX: &str = "device:";

/// Where a channel's messages go: to the relay, for the device.
pub trait Outbound: Send + Sync {
    /// Queue a text message.
    fn text(&self, text: String);
    /// Queue terminal output. `false` when the device is more than
    /// [`remote_proto::MAX_QUEUED_OUTPUT`] behind: nothing was queued.
    fn output(&self, frame: Vec<u8>) -> bool;
}

/// What a channel needs of this host to answer.
pub struct Context<'a> {
    pub store: &'a Store,
    pub git: &'a Git,
    pub terminals: &'a dyn TerminalHost,
    pub host: &'a HostIdentity,
    /// The daemon shares control (its protocol 2): the terminal operations are answered.
    pub interactive: bool,
}

impl Context<'_> {
    fn facade(&self) -> HostFacade<'_> {
        HostFacade {
            store: self.store,
            git: self.git,
            terminals: Some(self.terminals),
            host: self.host,
            interactive: self.interactive,
        }
    }
}

/// One device's channel.
pub struct Visitor {
    device: DeviceId,
    grant: GrantId,
    out: Arc<dyn Outbound>,
    greeted: AtomicBool,
    next_stream: AtomicU32,
    streams: Mutex<HashMap<StreamId, Watch>>,
    /// The leases this channel holds: each one's terminal and generation.
    leases: Mutex<HashMap<LeaseId, (SessionId, u64)>>,
}

struct Watch {
    session: SessionId,
    attachment: AttachmentId,
}

/// A stream as its output arrives: held back until the reply that starts it has gone.
struct Flow {
    stream: StreamId,
    out: Arc<dyn Outbound>,
    /// Items that came before the reply was sent; `None` once it has been.
    held: Option<Vec<Item>>,
    snapshot: Option<(u64, pty_host::TermSize)>,
    /// Detached for falling behind: everything after is refused.
    over: bool,
}

enum Item {
    Bytes(u64, Vec<u8>),
    Resized(u64, pty_host::TermSize),
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Visitor {
    /// A device the relay connected, if this host's record lets it in under `grant`.
    pub fn open(
        store: &Store,
        device: DeviceId,
        grant: GrantId,
        out: Arc<dyn Outbound>,
    ) -> Result<Self, CloseReason> {
        match host::grant_for(store, &device, &grant) {
            Ok(Some(_)) => Ok(Self {
                device,
                grant,
                out,
                greeted: AtomicBool::new(false),
                next_stream: AtomicU32::new(1),
                streams: Mutex::new(HashMap::new()),
                leases: Mutex::new(HashMap::new()),
            }),
            Ok(None) => Err(CloseReason::Refused),
            Err(_) => Err(CloseReason::Gone),
        }
    }

    pub fn device(&self) -> &DeviceId {
        &self.device
    }

    /// Whether this host still lets the device in. When not, close the channel.
    pub fn allowed(&self, store: &Store) -> bool {
        matches!(
            host::grant_for(store, &self.device, &self.grant),
            Ok(Some(_))
        )
    }

    /// Answer one message from the device. `Err` means close the channel, for that reason.
    pub fn handle(&self, context: &Context<'_>, text: &str) -> Result<(), CloseReason> {
        if !self.greeted.load(Ordering::SeqCst) {
            let reply = match serde_json::from_str::<Hello>(text) {
                Ok(hello) => match context.facade().welcome(&hello) {
                    Ok(welcome) => HelloReply::Welcome(welcome),
                    Err(error) => HelloReply::Error(error),
                },
                Err(_) => HelloReply::Error(RemoteError::new(
                    ErrorCode::InvalidRequest,
                    "The first message is a hello.",
                )),
            };
            let welcomed = matches!(reply, HelloReply::Welcome(_));
            self.out
                .text(serde_json::to_string(&reply).expect("serialises"));
            if !welcomed {
                return Err(CloseReason::Invalid);
            }
            self.greeted.store(true, Ordering::SeqCst);
            return Ok(());
        }
        let request = match remote_proto::decode_request(text.as_bytes()) {
            Ok(request) => request,
            Err(refusal) => {
                self.respond(&refusal);
                return Ok(());
            }
        };
        let Ok(Some(grant)) = host::grant_for(context.store, &self.device, &self.grant) else {
            self.respond(&Response::error(
                Some(request.request_id.clone()),
                RemoteError::new(
                    ErrorCode::Unauthenticated,
                    "This host no longer lets this device in.",
                ),
            ));
            return Err(CloseReason::Revoked);
        };
        if !is_terminal(&request.operation) || !context.interactive {
            self.respond(&context.facade().handle(&grant, &request));
            return Ok(());
        }
        let facade = context.facade();
        let answered = facade
            .check(&grant, &request)
            .and_then(|()| self.terminal(context, &facade, &grant, &request));
        match answered {
            // An attachment's reply was sent already, ahead of its output.
            Ok(None) => {}
            Ok(Some(reply)) => self.respond(&Response {
                request_id: Some(request.request_id),
                outcome: Outcome::Result(reply),
            }),
            Err(error) => self.respond(&Response::error(Some(request.request_id), error)),
        }
        Ok(())
    }

    /// Detach everything this channel watched, and release every lease it held.
    pub fn close(&self, terminals: &dyn TerminalHost) {
        for (_, watch) in lock(&self.streams).drain() {
            let _ = terminals.detach(&watch.session, watch.attachment);
        }
        for (lease, (session, generation)) in lock(&self.leases).drain() {
            let _ = terminals.release(&session, &daemon_lease(&lease, generation));
        }
    }

    fn respond(&self, response: &Response) {
        let bytes = remote_proto::encode_response(response);
        self.out
            .text(String::from_utf8(bytes).expect("JSON is UTF-8"));
    }

    fn holder(&self) -> String {
        format!("{HOLDER_PREFIX}{}", self.device)
    }

    /// A terminal operation. `None` when the reply has been sent already.
    fn terminal(
        &self,
        context: &Context<'_>,
        facade: &HostFacade<'_>,
        grant: &Grant,
        request: &Request,
    ) -> Result<Option<Reply>, RemoteError> {
        let terminals = context.terminals;
        Ok(Some(match &request.operation {
            Operation::TerminalAttach(attach) => {
                self.attach(context, facade, grant, attach, request)?;
                return Ok(None);
            }
            Operation::TerminalDetach(DetachRequest { terminal, stream }) => {
                let watch = {
                    let mut streams = lock(&self.streams);
                    match streams.get(stream) {
                        Some(w) if w.session.0 == terminal.as_str() => streams.remove(stream),
                        _ => None,
                    }
                }
                .ok_or_else(RemoteError::not_found)?;
                let _ = terminals.detach(&watch.session, watch.attachment);
                Reply::Done
            }
            Operation::TerminalAcquire(AcquireRequest {
                terminal,
                take_over,
            }) => {
                let info = facade.reachable_terminal(grant, terminal)?;
                let lease = terminals
                    .acquire(&info.id, &self.holder(), *take_over)
                    .map_err(terminal_error)?;
                let id = LeaseId::new(lease.id.clone()).map_err(|_| internal())?;
                lock(&self.leases).insert(id.clone(), (info.id, lease.generation));
                Reply::Lease(Lease {
                    id,
                    generation: lease.generation,
                    ttl_ms: lease.ttl_ms,
                    renew_every_ms: lease.renew_every_ms,
                })
            }
            Operation::TerminalRenew(lease) => {
                let session = self.leased(facade, grant, lease)?;
                let renewed = terminals
                    .renew(&session, &daemon_lease(&lease.lease, lease.generation))
                    .map_err(|e| self.lost(lease, e))?;
                Reply::Lease(Lease {
                    id: lease.lease.clone(),
                    generation: renewed.generation,
                    ttl_ms: renewed.ttl_ms,
                    renew_every_ms: renewed.renew_every_ms,
                })
            }
            Operation::TerminalRelease(lease) => {
                let session = self.leased(facade, grant, lease)?;
                lock(&self.leases).remove(&lease.lease);
                terminals
                    .release(&session, &daemon_lease(&lease.lease, lease.generation))
                    .map_err(terminal_error)?;
                Reply::Done
            }
            Operation::TerminalInput(InputRequest {
                lease,
                seq,
                data,
                paste,
            }) => {
                let session = self.leased(facade, grant, lease)?;
                terminals
                    .input(
                        &session,
                        &daemon_lease(&lease.lease, lease.generation),
                        *seq,
                        data,
                        *paste,
                    )
                    .map_err(|e| self.lost(lease, e))?;
                Reply::Done
            }
            Operation::TerminalResize(ResizeRequest { lease, size }) => {
                let session = self.leased(facade, grant, lease)?;
                terminals
                    .resize_leased(
                        &session,
                        &daemon_lease(&lease.lease, lease.generation),
                        pty_host::TermSize {
                            cols: size.cols,
                            rows: size.rows,
                        },
                    )
                    .map_err(|e| self.lost(lease, e))?;
                Reply::Done
            }
            _ => return Err(internal()),
        }))
    }

    /// The terminal a lease of this channel is for, while the grant still reaches it. Another
    /// channel's lease, or one this channel no longer holds, is `lease_lost`.
    fn leased(
        &self,
        facade: &HostFacade<'_>,
        grant: &Grant,
        lease: &LeaseRef,
    ) -> Result<SessionId, RemoteError> {
        let session = lock(&self.leases)
            .get(&lease.lease)
            .filter(|(s, generation)| {
                s.0 == lease.terminal.as_str() && *generation == lease.generation
            })
            .map(|(s, _)| s.clone())
            .ok_or_else(lease_lost)?;
        facade.reachable_terminal(grant, &lease.terminal)?;
        Ok(session)
    }

    /// The daemon's refusal of a lease operation, forgetting a lease it says is gone.
    fn lost(&self, lease: &LeaseRef, error: HostError) -> RemoteError {
        if matches!(error, HostError::LeaseLost(_) | HostError::SessionExited(_)) {
            lock(&self.leases).remove(&lease.lease);
        }
        terminal_error(error)
    }

    fn attach(
        &self,
        context: &Context<'_>,
        facade: &HostFacade<'_>,
        grant: &Grant,
        attach: &AttachRequest,
        request: &Request,
    ) -> Result<(), RemoteError> {
        let terminals = context.terminals;
        let info = facade.reachable_terminal(grant, &attach.terminal)?;
        let stream = self.next_stream.fetch_add(1, Ordering::Relaxed);
        let flow = Arc::new(Mutex::new(Flow {
            stream,
            out: Arc::clone(&self.out),
            held: Some(Vec::new()),
            snapshot: None,
            over: false,
        }));
        let feeding = Arc::clone(&flow);
        let attachment = terminals
            .attach_numbered(&info.id, Box::new(move |item| lock(&feeding).take(item)))
            .map_err(terminal_error)?;
        let controller = terminals
            .controller(&info.id)
            .map(|c| controller(context.store, c))
            .unwrap_or(Controller::Nobody);
        let mut flow = lock(&flow);
        let Some((seq, size)) = flow.snapshot else {
            drop(flow);
            let _ = terminals.detach(&info.id, attachment);
            return Err(internal());
        };
        // The reply, then what was held back, with nothing from the daemon in between: it waits
        // on this lock to hand over anything newer.
        self.respond(&Response {
            request_id: Some(request.request_id.clone()),
            outcome: Outcome::Result(Reply::Attached(Attached {
                stream,
                start: AttachStart::Snapshot,
                position: StreamPosition { epoch: EPOCH, seq },
                size: TermSize {
                    cols: size.cols,
                    rows: size.rows,
                },
                controller,
            })),
        });
        for item in flow.held.take().unwrap_or_default() {
            if !flow.send(&item) {
                break;
            }
        }
        let over = flow.over;
        drop(flow);
        if over {
            let _ = terminals.detach(&info.id, attachment);
        } else {
            lock(&self.streams).insert(
                stream,
                Watch {
                    session: info.id,
                    attachment,
                },
            );
        }
        Ok(())
    }
}

impl Flow {
    /// An item from the daemon. `false` detaches this stream there.
    fn take(&mut self, item: Numbered<'_>) -> bool {
        if self.over {
            return false;
        }
        let item = match item {
            Numbered::Snapshot { seq, size, bytes } => {
                self.snapshot = Some((seq, size));
                Item::Bytes(seq, bytes.to_vec())
            }
            Numbered::Output { seq, bytes } => Item::Bytes(seq, bytes.to_vec()),
            Numbered::Resized { seq, size } => Item::Resized(seq, size),
        };
        match &mut self.held {
            Some(held) => {
                held.push(item);
                true
            }
            None => self.send(&item),
        }
    }

    /// Send one item to the device: output as frames, a resize as an event. Past the device's
    /// queue, the stream is over and the device is told.
    fn send(&mut self, item: &Item) -> bool {
        let position = |seq| StreamPosition { epoch: EPOCH, seq };
        match item {
            Item::Bytes(seq, bytes) => {
                // A snapshot or output always sends at least one frame, empty or not.
                let chunks = bytes.chunks(MAX_OUTPUT_CHUNK);
                let chunks: Box<dyn Iterator<Item = &[u8]>> = if bytes.is_empty() {
                    Box::new(std::iter::once(&[][..]))
                } else {
                    Box::new(chunks)
                };
                for chunk in chunks {
                    let mut frame = Vec::new();
                    OutputFrame {
                        stream: self.stream,
                        position: position(*seq),
                        bytes: chunk,
                    }
                    .encode(&mut frame)
                    .expect("chunks fit");
                    if !self.out.output(frame) {
                        self.over = true;
                        self.out.text(event(&Event::TerminalDetached(Detached {
                            stream: self.stream,
                            error: RemoteError::new(
                                ErrorCode::ResnapshotRequired,
                                "This device fell too far behind the terminal. Attach again.",
                            ),
                        })));
                        return false;
                    }
                }
            }
            Item::Resized(seq, size) => self.out.text(event(&Event::TerminalResized(Resized {
                stream: self.stream,
                position: position(*seq),
                size: TermSize {
                    cols: size.cols,
                    rows: size.rows,
                },
            }))),
        }
        true
    }
}

fn event(event: &Event) -> String {
    serde_json::to_string(event).expect("serialises")
}

fn is_terminal(operation: &Operation) -> bool {
    matches!(
        operation,
        Operation::TerminalAttach(_)
            | Operation::TerminalDetach(_)
            | Operation::TerminalAcquire(_)
            | Operation::TerminalRenew(_)
            | Operation::TerminalRelease(_)
            | Operation::TerminalInput(_)
            | Operation::TerminalResize(_)
    )
}

fn daemon_lease(id: &LeaseId, generation: u64) -> pty_host::LeaseRef {
    pty_host::LeaseRef {
        id: id.as_str().to_owned(),
        generation,
    }
}

/// Who controls a terminal, as a device is told: a device by the name this host knows it by,
/// and anything else on this computer as the person here.
fn controller(store: &Store, controller: pty_host::Controller) -> Controller {
    match controller {
        pty_host::Controller::Nobody => Controller::Nobody,
        pty_host::Controller::Local => Controller::Local,
        pty_host::Controller::Leased { holder } => holder
            .strip_prefix(HOLDER_PREFIX)
            .and_then(|id| DeviceId::new(id).ok())
            .map(|device| {
                let name = host::local_grants(store)
                    .ok()
                    .and_then(|grants| {
                        grants
                            .into_iter()
                            .find(|g| g.grant.device == device)
                            .map(|g| g.device_name)
                    })
                    .unwrap_or_default();
                Controller::Device {
                    device,
                    name: remote_proto::clip(&name, remote_proto::MAX_TEXT),
                }
            })
            .unwrap_or(Controller::Local),
    }
}

fn lease_lost() -> RemoteError {
    RemoteError::new(
        ErrorCode::LeaseLost,
        "That lease is no longer in force. Ask for control again.",
    )
}

fn internal() -> RemoteError {
    RemoteError::new(
        ErrorCode::Internal,
        "The host could not do that. Try again in a moment.",
    )
}

/// The daemon's refusal, as a device is told it. Nothing of the daemon's own text goes out.
fn terminal_error(error: HostError) -> RemoteError {
    match error {
        HostError::UnknownSession(_) => RemoteError::not_found(),
        HostError::SessionExited(_) => RemoteError::new(
            ErrorCode::SessionEnded,
            "That terminal's program has ended.",
        ),
        HostError::LeaseLost(_) => lease_lost(),
        HostError::ControlHeld { by, .. } => RemoteError::new(
            ErrorCode::ControlConflict,
            match by {
                pty_host::Controller::Local => {
                    "Someone at the host is typing in this terminal. Try again once they stop."
                }
                _ => "Another device controls this terminal.",
            },
        ),
        HostError::InputOutOfOrder { .. } => RemoteError::new(
            ErrorCode::InvalidRequest,
            "That input came out of order, and was not typed. Nothing after it will be.",
        ),
        HostError::ControlUnavailable => RemoteError::new(
            ErrorCode::Unsupported,
            "This host's terminals cannot be shared until its terminal service restarts.",
        ),
        _ => internal(),
    }
}

#[cfg(test)]
mod tests;
