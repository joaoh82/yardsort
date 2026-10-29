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
//!   that asks to resume is sent only what came after the position it gives, when the daemon
//!   still keeps all of it, and a snapshot otherwise. Who controls the terminal, and that its
//!   program ended, come as events. A device more than [`remote_proto::MAX_QUEUED_OUTPUT`]
//!   behind is detached with `resnapshot_required`: the daemon never waits for it.
//! - **Control** is a daemon lease, held as `device:<id>`. A lease is this channel's: another
//!   device naming its id gets `lease_lost`, exactly as for a lease that is gone.
//! - **Following activity** is a position per subscription, nothing more: each
//!   [`Visitor::poll`] reads what was recorded after it from the store and sends it in
//!   `activity.events` batches, and moves it on. The host's owner calls `poll` about every
//!   [`POLL_EVERY`] while [`Visitor::following`]. A batch goes only while little waits for the
//!   device ([`Outbound::activity`]), so catching up never crowds out a terminal's output; what
//!   did not go is read again at the next poll, because it is on disk, not in a queue.
//! - **Closing the channel** — the device left, its grant went, the relay or the network failed —
//!   detaches everything it watched and releases every lease it held, and ends its
//!   subscriptions. The terminals and their programs are untouched: they belong to the daemon,
//!   not to the connection.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use pty_host::{AttachmentId, HostError, Numbered, SessionId, TerminalHost};
use remote_proto::relay::CloseReason;
use remote_proto::terminal::{
    AcquireRequest, AttachRequest, AttachStart, Attached, ControlChanged, Controller,
    DetachRequest, Detached, Exited, InputRequest, Lease, LeaseRef, OutputFrame, ProgramExit,
    ResizeRequest, Resized, StreamId, StreamPosition,
};
use remote_proto::{
    ActivityBatch, ActivitySubscribeParams, ActivityUnsubscribeParams, DeviceId, ErrorCode, Event,
    Grant, GrantId, Hello, HelloReply, LeaseId, Operation, Outcome, RemoteError, Reply, Request,
    Response, SessionRecordId, Subscribed, SubscriptionEnded, SubscriptionId, TermSize,
    WorkspaceId, MAX_OUTPUT_CHUNK, MAX_QUEUED_OUTPUT, MAX_SUBSCRIPTIONS,
};

use super::{host, HostFacade, HostIdentity};
use crate::git::Git;
use crate::store::Store;

/// The epoch of every stream. Output numbers are the daemon's, which count from a terminal's
/// start for its whole life, so they never need a new epoch. A position in another epoch is not
/// one of this terminal's, and gets a snapshot.
const EPOCH: u32 = 0;

/// The prefix of a device's name as a lease holder in the daemon.
const HOLDER_PREFIX: &str = "device:";

/// How often a channel that follows activity looks for more: how late, at most, a device hears
/// of an event once it is in the store.
pub const POLL_EVERY: Duration = Duration::from_secs(1);

/// How much may wait for a device before activity holds back: a quarter of what it may fall
/// behind on a terminal, so the rest stays for terminals' output.
pub const ACTIVITY_WAITING: usize = MAX_QUEUED_OUTPUT / 4;

/// Where a channel's messages go: to the relay, for the device.
pub trait Outbound: Send + Sync {
    /// Queue a text message.
    fn text(&self, text: String);
    /// Queue terminal output. `false` when the device is more than
    /// [`remote_proto::MAX_QUEUED_OUTPUT`] behind: nothing was queued.
    fn output(&self, frame: Vec<u8>) -> bool;
    /// Queue a batch of activity, counted with output, unless more than [`ACTIVITY_WAITING`]
    /// waits for the device already: `false` when nothing was queued, to send at a later poll.
    fn activity(&self, text: String) -> bool;
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
            subscriptions: true,
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
    names: Names,
    next_subscription: AtomicU32,
    subscriptions: Mutex<HashMap<SubscriptionId, Following>>,
}

/// One activity subscription: what it follows, and the sequence number of the last event its
/// device was sent or need not be.
struct Following {
    workspace: Option<WorkspaceId>,
    session: Option<SessionRecordId>,
    after: i64,
}

/// What this host calls each device it let in, for telling a device who controls a terminal.
/// Kept here because the daemon's items arrive where the store cannot be asked: a sink must not
/// wait on anything. Read again whenever the grants are checked, and on each attach and lease.
type Names = Arc<Mutex<HashMap<DeviceId, String>>>;

struct Watch {
    session: SessionId,
    attachment: AttachmentId,
}

/// A stream as its output arrives: held back until the reply that starts it has gone.
struct Flow {
    stream: StreamId,
    out: Arc<dyn Outbound>,
    names: Names,
    /// Items that came before the reply was sent; `None` once it has been.
    held: Option<Vec<Item>>,
    /// How the stream began: a snapshot or a resume, after which item, at which size.
    start: Option<(AttachStart, u64, pty_host::TermSize)>,
    /// Who controlled the terminal when the reply was sent, as the daemon last said.
    controller: Option<pty_host::Controller>,
    /// Detached for falling behind: everything after is refused.
    over: bool,
}

enum Item {
    Bytes(u64, Vec<u8>),
    Resized(u64, pty_host::TermSize),
    Control(pty_host::Controller),
    Exited(pty_host::ExitInfo),
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
                names: Names::default(),
                next_subscription: AtomicU32::new(1),
                subscriptions: Mutex::new(HashMap::new()),
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
        self.learn_names(store);
        matches!(
            host::grant_for(store, &self.device, &self.grant),
            Ok(Some(_))
        )
    }

    /// Read again what this host calls each device it let in. Left as it was if the store
    /// cannot say.
    fn learn_names(&self, store: &Store) {
        if let Ok(grants) = host::local_grants(store) {
            *lock(&self.names) = grants
                .into_iter()
                .map(|g| (g.grant.device, g.device_name))
                .collect();
        }
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
        let facade = context.facade();
        let answered = match &request.operation {
            operation if is_terminal(operation) && context.interactive => facade
                .check(&grant, &request)
                .and_then(|()| self.terminal(context, &facade, &grant, &request)),
            Operation::ActivitySubscribe(params) => facade
                .check(&grant, &request)
                .and_then(|()| self.subscribe(context, &facade, &grant, params, &request)),
            Operation::ActivityUnsubscribe(ActivityUnsubscribeParams { subscription }) => {
                facade.check(&grant, &request).and_then(|()| {
                    lock(&self.subscriptions)
                        .remove(subscription)
                        .map(|_| Some(Reply::Done))
                        .ok_or_else(RemoteError::not_found)
                })
            }
            _ => {
                self.respond(&facade.handle(&grant, &request));
                return Ok(());
            }
        };
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

    /// Whether this channel follows any activity: then call [`Self::poll`] about every
    /// [`POLL_EVERY`].
    pub fn following(&self) -> bool {
        !lock(&self.subscriptions).is_empty()
    }

    /// Send each subscription what was recorded since its last batch, as much as the device's
    /// queue takes now. A subscription whose workspace the device can no longer see ends, and
    /// the device is told; one the store cannot answer for now waits for the next poll.
    pub fn poll(&self, context: &Context<'_>) {
        if !self.following() {
            return;
        }
        // A grant gone here ends the channel at the next check; until then, nothing more.
        let Ok(Some(grant)) = host::grant_for(context.store, &self.device, &self.grant) else {
            return;
        };
        let facade = context.facade();
        let mut subscriptions = lock(&self.subscriptions);
        let mut ended = Vec::new();
        'subscriptions: for (&subscription, following) in subscriptions.iter_mut() {
            loop {
                let (events, last) = match facade.recorded_after(
                    &grant,
                    following.workspace.as_ref(),
                    following.session.as_ref(),
                    following.after,
                ) {
                    Ok(read) => read,
                    Err(error) if error.code == ErrorCode::NotFound => {
                        ended.push((subscription, error));
                        continue 'subscriptions;
                    }
                    Err(_) => continue 'subscriptions,
                };
                if last == following.after {
                    continue 'subscriptions;
                }
                if !events.is_empty() {
                    let Ok(newer) = super::activity_cursor('n', last) else {
                        continue 'subscriptions;
                    };
                    let batch = event(&Event::ActivityEvents(ActivityBatch {
                        subscription,
                        events,
                        newer,
                    }));
                    // The device has enough waiting: the rest goes at a later poll.
                    if !self.out.activity(batch) {
                        break 'subscriptions;
                    }
                }
                following.after = last;
            }
        }
        for (subscription, error) in ended {
            subscriptions.remove(&subscription);
            self.out
                .text(event(&Event::ActivityEnded(SubscriptionEnded {
                    subscription,
                    error,
                })));
        }
    }

    /// Detach everything this channel watched, and release every lease it held.
    pub fn close(&self, terminals: &dyn TerminalHost) {
        lock(&self.subscriptions).clear();
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
                self.learn_names(context.store);
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

    /// Start following activity: the reply, then at once what was recorded after the position
    /// it starts from. `None`: the reply has been sent.
    fn subscribe(
        &self,
        context: &Context<'_>,
        facade: &HostFacade<'_>,
        grant: &Grant,
        params: &ActivitySubscribeParams,
        request: &Request,
    ) -> Result<Option<Reply>, RemoteError> {
        if lock(&self.subscriptions).len() >= MAX_SUBSCRIPTIONS {
            return Err(RemoteError::new(
                ErrorCode::InvalidRequest,
                "This device follows as much activity as one connection may. Stop following \
                 something first.",
            ));
        }
        let (after, newer) = facade.subscription_start(grant, params)?;
        let subscription = self.next_subscription.fetch_add(1, Ordering::Relaxed);
        lock(&self.subscriptions).insert(
            subscription,
            Following {
                workspace: params.workspace.clone(),
                session: params.session.clone(),
                after,
            },
        );
        self.respond(&Response {
            request_id: Some(request.request_id.clone()),
            outcome: Outcome::Result(Reply::Subscribed(Subscribed {
                subscription,
                newer,
            })),
        });
        self.poll(context);
        Ok(None)
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
        self.learn_names(context.store);
        let stream = self.next_stream.fetch_add(1, Ordering::Relaxed);
        let flow = Arc::new(Mutex::new(Flow {
            stream,
            out: Arc::clone(&self.out),
            names: Arc::clone(&self.names),
            held: Some(Vec::new()),
            start: None,
            controller: None,
            over: false,
        }));
        let feeding = Arc::clone(&flow);
        let sink: pty_host::NumberedSink =
            Box::new(move |item: Numbered<'_>| lock(&feeding).take(item));
        let attachment = match attach.resume_after {
            Some(after) if after.epoch == EPOCH => {
                terminals.resume_numbered(&info.id, after.seq, sink)
            }
            _ => terminals.attach_numbered(&info.id, sink),
        }
        .map_err(terminal_error)?;
        // A daemon from before it told watchers who controls a terminal is asked instead.
        let asked = lock(&flow)
            .controller
            .is_none()
            .then(|| terminals.controller(&info.id).ok())
            .flatten();
        let mut flow = lock(&flow);
        let Some((start, seq, size)) = flow.start else {
            drop(flow);
            let _ = terminals.detach(&info.id, attachment);
            return Err(internal());
        };
        let controller = flow
            .controller
            .clone()
            .or(asked)
            .map_or(Controller::Nobody, |c| device_controller(&flow.names, c));
        // The reply, then what was held back, with nothing from the daemon in between: it waits
        // on this lock to hand over anything newer.
        self.respond(&Response {
            request_id: Some(request.request_id.clone()),
            outcome: Outcome::Result(Reply::Attached(Attached {
                stream,
                start,
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
                self.start = Some((AttachStart::Snapshot, seq, size));
                Item::Bytes(seq, bytes.to_vec())
            }
            Numbered::Resumed { seq, size } => {
                self.start = Some((AttachStart::Resumed, seq, size));
                return true;
            }
            Numbered::Output { seq, bytes } => Item::Bytes(seq, bytes.to_vec()),
            Numbered::Resized { seq, size } => Item::Resized(seq, size),
            // Before the reply, the reply says who controls the terminal.
            Numbered::Control(controller) if self.held.is_some() => {
                self.controller = Some(controller);
                return true;
            }
            Numbered::Control(controller) => Item::Control(controller),
            Numbered::Exited(exit) => Item::Exited(exit),
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
            Item::Control(controller) => {
                self.out.text(event(&Event::TerminalControl(ControlChanged {
                    stream: self.stream,
                    controller: device_controller(&self.names, controller.clone()),
                })));
            }
            Item::Exited(exit) => self.out.text(event(&Event::TerminalExited(Exited {
                stream: self.stream,
                exit: ProgramExit {
                    code: exit.code,
                    success: exit.success,
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

/// Who controls a terminal, as a device is told: a device by the name this host knows it by
/// (empty for one let in since the names were last read), and anything else on this computer as
/// the person here.
fn device_controller(names: &Names, controller: pty_host::Controller) -> Controller {
    match controller {
        pty_host::Controller::Nobody => Controller::Nobody,
        pty_host::Controller::Local => Controller::Local,
        pty_host::Controller::Leased { holder } => holder
            .strip_prefix(HOLDER_PREFIX)
            .and_then(|id| DeviceId::new(id).ok())
            .map(|device| {
                let name = lock(names).get(&device).cloned().unwrap_or_default();
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
