//! A device's channel to one host through the relay: the ticket and the relay's greeting
//! ([`Client::open_relay`]), then `Hello` and the remote protocol (27 §8–§11). The Rust side of
//! `packages/remote-client/src/channel.ts`, which the phone uses; the two are pinned by the same
//! fixtures in `remote-proto/fixtures/v1/`.
//!
//! One task reads the socket and one writes it. The reader matches each reply to its request,
//! hands a terminal's frames and events to its [`Attachment`] and checks that its items come
//! numbered without a gap, and hands activity to its [`Subscription`]. A held [`Control`] is
//! renewed by a task of its own until it is lost or let go.
//!
//! **Liveness** is the relay's: it pings, and a socket that says nothing at all for
//! [`remote_proto::relay::SILENCE_MS`] is taken to be gone ([`crate::LinkReader::next`]). Opening a
//! channel again is the caller's: it attaches again from [`Attachment::drawn`], and never sends
//! again what it typed ([`Control::input`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use remote_proto::account::Secret;
use remote_proto::relay::{CloseReason, TicketRequest};
use remote_proto::terminal::FrameError;
use remote_proto::{
    AcquireRequest, ActivityBatch, ActivityListParams, ActivityPage, ActivitySubscribeParams,
    ActivityUnsubscribeParams, AttachRequest, AttachStart, Attached, Controller, Cursor,
    DetachRequest, ErrorCode, Event, HarnessSummary, Hello, HelloReply, HostDescription, HostId,
    InputRequest, Lease, LeaseRef, Operation, OutputFrame, ProgramExit, ProtocolVersion,
    RemoteError, Reply, Request, RequestId, ResizeRequest, Response, SessionListParams,
    SessionPage, SessionResumeParams, SessionStartParams, Started, StreamId, StreamPosition,
    SubscriptionId, TermSize, TerminalId, WorkspaceDetail, WorkspaceId, WorkspaceListParams,
    WorkspacePage, WorkspaceRef, WorkspaceStartParams, MAX_CLIENT_NAME, MAX_CONTROL_FRAME,
    MAX_OUTPUT_CHUNK, PROTOCOL_MAJOR,
};
use tokio::sync::{mpsc, oneshot, watch};

use crate::relay::{Incoming, LinkReader, LinkWriter, RelayLink};
use crate::{Client, ClientError};

/// How long the host has to welcome a device, once the relay has connected it.
const WELCOME_TIMEOUT: Duration = Duration::from_secs(30);

/// The events this build reads. One a newer host sends is passed over.
const EVENTS: [&str; 6] = [
    "terminal.resized",
    "terminal.detached",
    "terminal.control",
    "terminal.exited",
    "activity.events",
    "activity.ended",
];

/// Why a channel ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndReason {
    /// The relay closed it, or it went: one of the relay's reasons.
    Relay(CloseReason),
    /// The host broke the protocol, so the channel was closed rather than drawn wrong.
    Invalid,
    /// [`Channel::close`].
    ClosedHere,
}

/// Why a channel ended, and a few words for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelEnd {
    pub reason: EndReason,
    pub text: String,
}

impl std::fmt::Display for ChannelEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ChannelError {
    /// The host refused, with its reason.
    #[error("{0}")]
    Remote(RemoteError),
    /// The channel ended before the answer came. For input, whether it arrived is not known.
    #[error("The connection closed: {0}.")]
    Closed(ChannelEnd),
    /// The host answered something this build does not understand.
    #[error("The computer's answer was not understood: {0}")]
    Unexpected(String),
    /// Refused here, before anything was sent.
    #[error("{0}")]
    Invalid(String),
    /// Opening it failed at the server or the relay.
    #[error("{0}")]
    Open(String, Option<ErrorCode>),
}

impl ChannelError {
    /// The host's or the server's reason, when it gave one.
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Remote(error) => Some(error.code),
            Self::Open(_, code) => *code,
            _ => None,
        }
    }
}

impl From<ClientError> for ChannelError {
    fn from(error: ClientError) -> Self {
        let code = error.code();
        Self::Open(error.to_string(), code)
    }
}

pub type Result<T> = std::result::Result<T, ChannelError>;

/// An open channel to one host. Dropping it closes it.
pub struct Channel {
    host: HostDescription,
    protocol: ProtocolVersion,
    link: Link,
    ended: watch::Receiver<Option<ChannelEnd>>,
}

impl Channel {
    /// Open a channel to `host`, under the grant the server has in force for this device.
    /// `client` says who this is, for the host's list of devices: `yardsort-desktop/0.23.0 (linux)`.
    pub async fn open(client: &Client, access: &Secret, host: &HostId, name: &str) -> Result<Self> {
        check_name(name)?;
        let request = TicketRequest::Device { host: host.clone() };
        let link = client.open_relay(access, &request).await?;
        if &link.host != host {
            return Err(ChannelError::Unexpected(
                "the relay connected to another computer".into(),
            ));
        }
        Self::over(link, name).await
    }

    /// Greet the host at the far end of a relay socket that is already open.
    pub async fn over(link: RelayLink, name: &str) -> Result<Self> {
        check_name(name)?;
        let RelayLink {
            mut reader,
            mut writer,
            ..
        } = link;
        let hello = Hello {
            client: name.to_owned(),
            protocols: vec![PROTOCOL_MAJOR],
        };
        let hello = serde_json::to_string(&hello).expect("a greeting serialises");
        writer
            .text(hello)
            .await
            .map_err(|e| ChannelError::Open(e.to_string(), None))?;
        let welcome = tokio::time::timeout(WELCOME_TIMEOUT, welcome(&mut reader))
            .await
            .map_err(|_| {
                ChannelError::Open("The computer did not answer in time.".into(), None)
            })??;

        let (outgoing, queued) = mpsc::unbounded_channel();
        let (ended_tx, ended) = watch::channel(None);
        let link = Link {
            shared: Arc::new(Mutex::new(State::default())),
            outgoing,
            closing: Arc::new(tokio::sync::Notify::new()),
        };
        tokio::spawn(write(writer, queued));
        tokio::spawn(read(reader, link.clone(), ended_tx));
        Ok(Self {
            protocol: welcome.protocol,
            host: welcome.host,
            link,
            ended,
        })
    }

    /// The host, as it described itself when it welcomed this device.
    pub fn host(&self) -> &HostDescription {
        &self.host
    }

    /// The protocol it speaks.
    pub fn protocol(&self) -> ProtocolVersion {
        self.protocol
    }

    /// Whether the channel still carries requests.
    pub fn is_open(&self) -> bool {
        self.ended.borrow().is_none()
    }

    /// Wait for the channel to end, and say why.
    pub async fn ended(&self) -> ChannelEnd {
        let mut ended = self.ended.clone();
        loop {
            if let Some(end) = ended.borrow_and_update().clone() {
                return end;
            }
            if ended.changed().await.is_err() {
                return gone();
            }
        }
    }

    /// Close the channel. Leases held through it lapse at the host.
    pub fn close(&self) {
        self.link
            .shared
            .lock()
            .expect("channel state")
            .end
            .get_or_insert_with(|| ChannelEnd {
                reason: EndReason::ClosedHere,
                text: "closed here".into(),
            });
        let _ = self.link.outgoing.send(Outgoing::Close);
        self.link.closing.notify_one();
    }

    /// Send one request and wait for its reply. A refusal is [`ChannelError::Remote`].
    pub async fn ask(&self, operation: Operation) -> Result<Reply> {
        self.link.ask(operation).await
    }

    pub async fn describe(&self) -> Result<HostDescription> {
        match self.ask(Operation::HostDescribe).await? {
            Reply::Host(host) => Ok(host),
            _ => Err(something_else()),
        }
    }

    pub async fn workspaces(&self, params: WorkspaceListParams) -> Result<WorkspacePage> {
        match self.ask(Operation::WorkspaceList(params)).await? {
            Reply::Workspaces(page) => Ok(page),
            _ => Err(something_else()),
        }
    }

    pub async fn workspace(&self, workspace: WorkspaceId) -> Result<WorkspaceDetail> {
        match self
            .ask(Operation::WorkspaceDescribe(WorkspaceRef { workspace }))
            .await?
        {
            Reply::Workspace(detail) => Ok(detail),
            _ => Err(something_else()),
        }
    }

    pub async fn sessions(&self, params: SessionListParams) -> Result<SessionPage> {
        match self.ask(Operation::SessionList(params)).await? {
            Reply::Sessions(page) => Ok(page),
            _ => Err(something_else()),
        }
    }

    /// The agents the host can start, its default first. Minor 5.
    pub async fn harnesses(&self) -> Result<Vec<HarnessSummary>> {
        match self.ask(Operation::HarnessList).await? {
            Reply::Harnesses(list) => Ok(list.harnesses),
            _ => Err(something_else()),
        }
    }

    /// A new workspace with an agent given `prompt`. The reply comes once the agent has started,
    /// which can take as long as the project's setup does. Minor 5.
    pub async fn start_workspace(&self, params: WorkspaceStartParams) -> Result<Started> {
        started(self.ask(Operation::WorkspaceStart(params)).await?)
    }

    /// Another agent in a workspace. Minor 5.
    pub async fn start_session(&self, params: SessionStartParams) -> Result<Started> {
        started(self.ask(Operation::SessionStart(params)).await?)
    }

    /// An ended conversation again, under its own record. Minor 6.
    pub async fn resume_session(&self, params: SessionResumeParams) -> Result<Started> {
        started(self.ask(Operation::SessionResume(params)).await?)
    }

    pub async fn activity(&self, params: ActivityListParams) -> Result<ActivityPage> {
        match self.ask(Operation::ActivityList(params)).await? {
            Reply::Activity(page) => Ok(page),
            _ => Err(something_else()),
        }
    }

    /// Watch a terminal: from a snapshot, or from just after `resume_after` when the host still
    /// has what came after it.
    pub async fn attach(
        &self,
        terminal: TerminalId,
        resume_after: Option<StreamPosition>,
    ) -> Result<Attachment> {
        let request = Operation::TerminalAttach(AttachRequest {
            terminal: terminal.clone(),
            resume_after,
        });
        let answer = self
            .link
            .send(request, Expect::Attach(terminal.clone()))?
            .await?;
        let Answer::Attached(attached, watched, tracker) = answer else {
            return Err(something_else());
        };
        Ok(Attachment {
            terminal,
            attached,
            watched,
            tracker,
            link: self.link.clone(),
        })
    }

    /// Ask for control of a terminal. It is renewed while held, and lost when taken or lapsed.
    pub async fn acquire(&self, terminal: TerminalId, take_over: bool) -> Result<Control> {
        let reply = self
            .ask(Operation::TerminalAcquire(AcquireRequest {
                terminal: terminal.clone(),
                take_over,
            }))
            .await?;
        let Reply::Lease(lease) = reply else {
            return Err(something_else());
        };
        Ok(Control::new(self.link.clone(), terminal, lease))
    }

    /// Follow activity as it is recorded: everything the grant reaches, unless `params` narrows it.
    pub async fn subscribe(&self, params: ActivitySubscribeParams) -> Result<Subscription> {
        let answer = self
            .link
            .send(Operation::ActivitySubscribe(params), Expect::Subscribe)?
            .await?;
        let Answer::Subscribed(id, newer, followed) = answer else {
            return Err(something_else());
        };
        Ok(Subscription {
            id,
            newer,
            followed,
            link: self.link.clone(),
        })
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.close();
    }
}

fn check_name(name: &str) -> Result<()> {
    if name.len() > MAX_CLIENT_NAME {
        return Err(ChannelError::Invalid(
            "The client's name is longer than a greeting carries.".into(),
        ));
    }
    Ok(())
}

fn started(reply: Reply) -> Result<Started> {
    match reply {
        Reply::Started(started) => Ok(started),
        _ => Err(something_else()),
    }
}

fn something_else() -> ChannelError {
    ChannelError::Unexpected("the computer answered something else".into())
}

fn gone() -> ChannelEnd {
    ChannelEnd {
        reason: EndReason::Relay(CloseReason::Gone),
        text: CloseReason::Gone.text().to_owned(),
    }
}

async fn welcome(reader: &mut LinkReader) -> Result<remote_proto::Welcome> {
    let text = match reader.next().await {
        Ok(Incoming::Text(text)) => text,
        Ok(Incoming::Binary(_)) => {
            return Err(ChannelError::Unexpected(
                "terminal output before the welcome".into(),
            ))
        }
        Err(reason) => {
            return Err(ChannelError::Open(
                format!(
                    "The relay closed the connection: {}.",
                    reader.describe(reason)
                ),
                None,
            ))
        }
    };
    match serde_json::from_str::<HelloReply>(&text) {
        Ok(HelloReply::Welcome(welcome)) if welcome.protocol.major == PROTOCOL_MAJOR => Ok(welcome),
        Ok(HelloReply::Welcome(_)) => Err(ChannelError::Unexpected(
            "the computer speaks another version of the protocol".into(),
        )),
        Ok(HelloReply::Error(error)) => Err(ChannelError::Remote(error)),
        Err(_) => Err(ChannelError::Unexpected(
            "the computer's welcome was not understood".into(),
        )),
    }
}

// --- inside --------------------------------------------------------------------------------------

enum Outgoing {
    Text(String),
    Close,
}

/// What a request waits for, besides its reply: an attachment or a subscription is kept by the
/// reader as the reply is read, before anything after it on the socket, since the snapshot's
/// frames follow the reply at once.
enum Expect {
    Reply,
    Attach(TerminalId),
    Subscribe,
}

enum Answer {
    Reply(Reply),
    Attached(
        Attached,
        mpsc::UnboundedReceiver<Watched>,
        Arc<Mutex<Tracker>>,
    ),
    Subscribed(
        SubscriptionId,
        Option<Cursor>,
        mpsc::UnboundedReceiver<Followed>,
    ),
}

struct Pending {
    expect: Expect,
    answer: oneshot::Sender<Result<Answer>>,
}

#[derive(Default)]
struct State {
    asked: u64,
    end: Option<ChannelEnd>,
    pending: HashMap<RequestId, Pending>,
    attachments: HashMap<StreamId, Watching>,
    subscriptions: HashMap<SubscriptionId, mpsc::UnboundedSender<Followed>>,
    controls: Vec<Arc<Mutex<Held>>>,
}

struct Watching {
    terminal: TerminalId,
    tx: mpsc::UnboundedSender<Watched>,
    tracker: Arc<Mutex<Tracker>>,
}

/// The channel as its attachments, controls and subscriptions use it.
#[derive(Clone)]
struct Link {
    shared: Arc<Mutex<State>>,
    outgoing: mpsc::UnboundedSender<Outgoing>,
    /// [`Channel::close`]: the reader stops at once, rather than wait for the far end to close.
    closing: Arc<tokio::sync::Notify>,
}

impl Link {
    async fn ask(&self, operation: Operation) -> Result<Reply> {
        match self.send(operation, Expect::Reply)?.await? {
            Answer::Reply(reply) => Ok(reply),
            _ => Err(something_else()),
        }
    }

    /// Send a request, or refuse it here, before anything is sent. The answer comes later.
    fn send(
        &self,
        operation: Operation,
        expect: Expect,
    ) -> Result<impl std::future::Future<Output = Result<Answer>>> {
        // Checked as a host would, so nothing malformed is ever sent.
        operation
            .validate()
            .map_err(|e| ChannelError::Invalid(format!("Not a request a host would take: {e}")))?;
        let (answer, answered) = oneshot::channel();
        {
            let mut state = self.shared.lock().expect("channel state");
            if let Some(end) = &state.end {
                return Err(ChannelError::Closed(end.clone()));
            }
            let request_id =
                RequestId::new(format!("r{}", state.asked + 1)).expect("a request id is short");
            let request = Request {
                request_id: request_id.clone(),
                protocol: ProtocolVersion::CURRENT,
                operation,
            };
            let wire = serde_json::to_string(&request).expect("a request serialises");
            if wire.len() > MAX_CONTROL_FRAME {
                return Err(ChannelError::Invalid(
                    "That request is larger than any request may be.".into(),
                ));
            }
            state.asked += 1;
            state.pending.insert(request_id, Pending { expect, answer });
            // Sent under the lock, so requests go in the order they were numbered.
            if self.outgoing.send(Outgoing::Text(wire)).is_err() {
                return Err(ChannelError::Closed(gone()));
            }
        }
        Ok(async move {
            answered
                .await
                .unwrap_or_else(|_| Err(ChannelError::Closed(gone())))
        })
    }

    /// Ask without waiting for the answer, as a gap's detach does: the channel may be on its way out.
    fn tell(&self, operation: Operation) {
        if let Ok(answer) = self.send(operation, Expect::Reply) {
            tokio::spawn(async move {
                let _ = answer.await;
            });
        }
    }
}

async fn write(mut writer: LinkWriter, mut queued: mpsc::UnboundedReceiver<Outgoing>) {
    while let Some(message) = queued.recv().await {
        match message {
            Outgoing::Text(text) => {
                if writer.text(text).await.is_err() {
                    break;
                }
            }
            Outgoing::Close => break,
        }
    }
    writer.close().await;
}

async fn read(mut reader: LinkReader, link: Link, ended: watch::Sender<Option<ChannelEnd>>) {
    let end = loop {
        let next = tokio::select! {
            next = reader.next() => next,
            () = link.closing.notified() => break ChannelEnd {
                reason: EndReason::ClosedHere,
                text: "closed here".into(),
            },
        };
        let message = match next {
            Ok(message) => message,
            Err(reason) => {
                break ChannelEnd {
                    reason: EndReason::Relay(reason),
                    text: reader.describe(reason),
                }
            }
        };
        let read = match message {
            Incoming::Text(text) => text_message(&link, &text),
            Incoming::Binary(bytes) => binary_message(&link, &bytes),
        };
        if let Err(what) = read {
            let _ = link.outgoing.send(Outgoing::Close);
            break ChannelEnd {
                reason: EndReason::Invalid,
                text: format!("the computer sent {what}"),
            };
        }
    };
    finish(&link, end.clone());
    let _ = ended.send(Some(end));
}

/// Everything waiting on the channel learns that it ended.
fn finish(link: &Link, end: ChannelEnd) {
    let (end, pending, attachments, subscriptions, controls) = {
        let mut state = link.shared.lock().expect("channel state");
        (
            state.end.get_or_insert(end).clone(),
            std::mem::take(&mut state.pending),
            std::mem::take(&mut state.attachments),
            std::mem::take(&mut state.subscriptions),
            std::mem::take(&mut state.controls),
        )
    };
    for (_, pending) in pending {
        let _ = pending.answer.send(Err(ChannelError::Closed(end.clone())));
    }
    for (_, watching) in attachments {
        let _ = watching
            .tx
            .send(Watched::Ended(AttachmentEnd::Closed(end.clone())));
    }
    for (_, followed) in subscriptions {
        let _ = followed.send(Followed::Ended(SubscriptionEnd::Closed(end.clone())));
    }
    for held in controls {
        held.lock()
            .expect("control")
            .lose(ControlLoss::Closed(end.clone()));
    }
}

/// A text message, or what broke the protocol in it.
fn text_message(link: &Link, text: &str) -> std::result::Result<(), &'static str> {
    if text.len() > MAX_CONTROL_FRAME {
        return Err("a message larger than any may be");
    }
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|_| "a message that is not JSON")?;
    if let Some(name) = value.get("event") {
        // An event a newer host sends is not this build's to read.
        if !name.as_str().is_some_and(|name| EVENTS.contains(&name)) {
            return Ok(());
        }
        let event: Event =
            serde_json::from_value(value).map_err(|_| "an event that does not read")?;
        event_message(link, event);
        return Ok(());
    }
    let response = match serde_json::from_value::<Response>(value.clone()) {
        Ok(response) => response,
        Err(error) => {
            // The request it answers, if that much can be read, learns that it was not understood.
            let id = value
                .get("requestId")
                .and_then(|id| id.as_str())
                .and_then(|id| RequestId::new(id).ok());
            let pending = id.and_then(|id| link.shared.lock().expect("state").pending.remove(&id));
            return match pending {
                Some(pending) => {
                    let _ = pending
                        .answer
                        .send(Err(ChannelError::Unexpected(error.to_string())));
                    Ok(())
                }
                None => Err("an answer that does not read"),
            };
        }
    };
    let Some(id) = response.request_id else {
        return Ok(());
    };
    let mut state = link.shared.lock().expect("channel state");
    let Some(pending) = state.pending.remove(&id) else {
        return Ok(());
    };
    let answer = match (response.outcome, pending.expect) {
        (remote_proto::Outcome::Error(error), _) => Err(ChannelError::Remote(error)),
        (remote_proto::Outcome::Result(Reply::Attached(attached)), Expect::Attach(terminal)) => {
            let (tx, rx) = mpsc::unbounded_channel();
            let tracker = Arc::new(Mutex::new(Tracker::new(&attached)));
            state.attachments.insert(
                attached.stream,
                Watching {
                    terminal,
                    tx,
                    tracker: Arc::clone(&tracker),
                },
            );
            Ok(Answer::Attached(attached, rx, tracker))
        }
        (remote_proto::Outcome::Result(Reply::Subscribed(subscribed)), Expect::Subscribe) => {
            let (tx, rx) = mpsc::unbounded_channel();
            state.subscriptions.insert(subscribed.subscription, tx);
            Ok(Answer::Subscribed(
                subscribed.subscription,
                subscribed.newer,
                rx,
            ))
        }
        (remote_proto::Outcome::Result(reply), _) => Ok(Answer::Reply(reply)),
    };
    drop(state);
    let _ = pending.answer.send(answer);
    Ok(())
}

fn event_message(link: &Link, event: Event) {
    let mut state = link.shared.lock().expect("channel state");
    match event {
        Event::TerminalResized(resized) => {
            let Some(watching) = state.attachments.get(&resized.stream) else {
                return;
            };
            let next = watching
                .tracker
                .lock()
                .expect("tracker")
                .resized(resized.position);
            match next {
                Ok(()) => {
                    let _ = watching.tx.send(Watched::Resized {
                        position: resized.position,
                        size: resized.size,
                    });
                }
                Err(gap) => end_on_gap(link, &mut state, resized.stream, gap),
            }
        }
        Event::TerminalControl(changed) => {
            if let Some(watching) = state.attachments.get(&changed.stream) {
                let _ = watching.tx.send(Watched::Control(changed.controller));
            }
        }
        Event::TerminalExited(exited) => {
            if let Some(watching) = state.attachments.get(&exited.stream) {
                let _ = watching.tx.send(Watched::Exited(exited.exit));
            }
        }
        Event::TerminalDetached(detached) => {
            if let Some(watching) = state.attachments.remove(&detached.stream) {
                let _ = watching
                    .tx
                    .send(Watched::Ended(AttachmentEnd::Detached(detached.error)));
            }
        }
        Event::ActivityEvents(batch) => {
            if let Some(followed) = state.subscriptions.get(&batch.subscription) {
                let _ = followed.send(Followed::Events(batch));
            }
        }
        Event::ActivityEnded(ended) => {
            if let Some(followed) = state.subscriptions.remove(&ended.subscription) {
                let _ = followed.send(Followed::Ended(SubscriptionEnd::Ended(ended.error)));
            }
        }
    }
}

fn binary_message(link: &Link, bytes: &[u8]) -> std::result::Result<(), &'static str> {
    if bytes.len() > remote_proto::terminal::OUTPUT_HEADER_LEN + MAX_OUTPUT_CHUNK {
        return Err("terminal output larger than a frame may be");
    }
    let frame = match OutputFrame::decode(bytes) {
        Ok(frame) => frame,
        // A frame of a kind or version from a newer host is not this build's to read.
        Err(FrameError::WrongKind | FrameError::UnknownVersion(_)) => return Ok(()),
        Err(_) => return Err("terminal output that does not read"),
    };
    let mut state = link.shared.lock().expect("channel state");
    let Some(watching) = state.attachments.get(&frame.stream) else {
        return Ok(());
    };
    let next = watching
        .tracker
        .lock()
        .expect("tracker")
        .output(frame.position, frame.bytes.len());
    match next {
        Ok(()) => {
            let _ = watching.tx.send(Watched::Output {
                position: frame.position,
                bytes: frame.bytes.to_vec(),
            });
        }
        Err(gap) => end_on_gap(link, &mut state, frame.stream, gap),
    }
    Ok(())
}

/// An item came out of order: the stream ends, and the host is told to stop sending what is no
/// longer drawn.
fn end_on_gap(link: &Link, state: &mut State, stream: StreamId, gap: AttachmentEnd) {
    let Some(watching) = state.attachments.remove(&stream) else {
        return;
    };
    let _ = watching.tx.send(Watched::Ended(gap));
    let detach = Operation::TerminalDetach(DetachRequest {
        terminal: watching.terminal,
        stream,
    });
    let link = link.clone();
    // Not under this lock: sending takes it.
    tokio::spawn(async move { link.tell(detach) });
}

// --- attachments ---------------------------------------------------------------------------------

/// What watching a terminal shows, in the order it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Watched {
    /// The terminal's own bytes, to the renderer unchanged. A snapshot comes this way first.
    Output {
        position: StreamPosition,
        bytes: Vec<u8>,
    },
    Resized {
        position: StreamPosition,
        size: TermSize,
    },
    /// Who controls the terminal from now on.
    Control(Controller),
    /// The program ended, and everything it printed has come.
    Exited(ProgramExit),
    /// The stream is over. Nothing more comes on it.
    Ended(AttachmentEnd),
}

/// Why a stream ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentEnd {
    /// The host detached it: `resnapshot_required` when this device fell behind.
    Detached(RemoteError),
    /// An item came out of order: attach again, for a snapshot.
    Gap {
        expected: StreamPosition,
        got: StreamPosition,
    },
    /// [`Attachment::detach`].
    DetachedHere,
    /// The channel closed: attach again on a new one, from [`Attachment::drawn`].
    Closed(ChannelEnd),
}

/// Where a stream has got to, kept by the reader as each item comes.
struct Tracker {
    epoch: u32,
    last: u64,
    /// Whether the last item was output, which a frame with the same number continues.
    in_output: bool,
    /// Whether that output may have more to come: its last frame was a full one. Every frame of
    /// a split piece but its last is full, so a shorter frame, or the next item, says it is whole.
    may_continue: bool,
    drew_any: bool,
}

impl Tracker {
    fn new(attached: &Attached) -> Self {
        let resumed = attached.start == AttachStart::Resumed;
        Self {
            epoch: attached.position.epoch,
            // A snapshot's frames carry the position's own number; a resumed stream goes on after it.
            last: if resumed {
                attached.position.seq
            } else {
                attached.position.seq.wrapping_sub(1)
            },
            in_output: false,
            may_continue: false,
            drew_any: resumed,
        }
    }

    fn output(&mut self, at: StreamPosition, len: usize) -> std::result::Result<(), AttachmentEnd> {
        let continues = self.in_output && at.seq == self.last;
        if at.epoch != self.epoch || (!continues && at.seq != self.last.wrapping_add(1)) {
            return Err(self.gap(at));
        }
        self.last = at.seq;
        self.in_output = true;
        self.may_continue = len == MAX_OUTPUT_CHUNK;
        self.drew_any = true;
        Ok(())
    }

    fn resized(&mut self, at: StreamPosition) -> std::result::Result<(), AttachmentEnd> {
        if at.epoch != self.epoch || at.seq != self.last.wrapping_add(1) {
            return Err(self.gap(at));
        }
        self.last = at.seq;
        self.in_output = false;
        self.may_continue = false;
        Ok(())
    }

    fn gap(&self, got: StreamPosition) -> AttachmentEnd {
        AttachmentEnd::Gap {
            expected: StreamPosition {
                epoch: self.epoch,
                seq: self.last.wrapping_add(1),
            },
            got,
        }
    }

    fn drawn(&self) -> Option<StreamPosition> {
        (self.drew_any && !self.may_continue).then_some(StreamPosition {
            epoch: self.epoch,
            seq: self.last,
        })
    }
}

/// A terminal being watched.
pub struct Attachment {
    terminal: TerminalId,
    attached: Attached,
    watched: mpsc::UnboundedReceiver<Watched>,
    tracker: Arc<Mutex<Tracker>>,
    link: Link,
}

impl Attachment {
    pub fn terminal(&self) -> &TerminalId {
        &self.terminal
    }

    /// The host's reply: the stream, how it starts, where, the size and who controls it.
    pub fn attached(&self) -> &Attached {
        &self.attached
    }

    /// What happened next, or `None` once the stream has ended and that was said.
    pub async fn next(&mut self) -> Option<Watched> {
        self.watched.recv().await
    }

    /// The last position drawn whole, to resume from on another channel. `None` when there is
    /// none, and the next attachment starts from a snapshot: before the snapshot has come, or
    /// while the last piece of output may have more frames to come. Resuming after a piece cut
    /// short would lose its end; resuming before it would draw its start twice.
    pub fn drawn(&self) -> Option<StreamPosition> {
        self.tracker.lock().expect("tracker").drawn()
    }

    /// Stop watching.
    pub async fn detach(self) -> Result<()> {
        let stream = self.attached.stream;
        let watching = self
            .link
            .shared
            .lock()
            .expect("channel state")
            .attachments
            .remove(&stream);
        if watching.is_none() {
            return Ok(());
        }
        self.link
            .ask(Operation::TerminalDetach(DetachRequest {
                terminal: self.terminal.clone(),
                stream,
            }))
            .await
            .map(|_| ())
    }
}

// --- control -------------------------------------------------------------------------------------

/// Why control was lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlLoss {
    /// The host said the lease is no longer the current one: taken over, taken back by the
    /// person there, or lapsed.
    Lost(RemoteError),
    /// [`Control::release`].
    Released,
    /// The channel closed.
    Closed(ChannelEnd),
}

struct Held {
    lease: Lease,
    seq: u64,
    lost: Option<ControlLoss>,
    told: watch::Sender<Option<ControlLoss>>,
}

impl Held {
    fn lose(&mut self, why: ControlLoss) {
        if self.lost.is_none() {
            self.lost = Some(why.clone());
            let _ = self.told.send(Some(why));
        }
    }
}

/// Typing into a terminal, under a lease. Input is numbered within the lease and never sent
/// again: when [`Control::input`] fails with [`ChannelError::Closed`], whether it arrived is not
/// known, and the person is the one to say whether to type it again.
pub struct Control {
    terminal: TerminalId,
    held: Arc<Mutex<Held>>,
    lost: watch::Receiver<Option<ControlLoss>>,
    link: Link,
    renewer: tokio::task::JoinHandle<()>,
}

impl Control {
    fn new(link: Link, terminal: TerminalId, lease: Lease) -> Self {
        let every = Duration::from_millis(u64::from(lease.renew_every_ms.max(1_000)));
        let (told, lost) = watch::channel(None);
        let held = Arc::new(Mutex::new(Held {
            lease,
            seq: 0,
            lost: None,
            told,
        }));
        link.shared
            .lock()
            .expect("channel state")
            .controls
            .push(Arc::clone(&held));
        let renewer = tokio::spawn(renew(
            link.clone(),
            terminal.clone(),
            Arc::clone(&held),
            every,
        ));
        Self {
            terminal,
            held,
            lost,
            link,
            renewer,
        }
    }

    pub fn lease(&self) -> Lease {
        self.held.lock().expect("control").lease.clone()
    }

    pub fn is_held(&self) -> bool {
        self.held.lock().expect("control").lost.is_none()
    }

    /// Wait until control is lost, and say why.
    pub async fn lost(&self) -> ControlLoss {
        let mut lost = self.lost.clone();
        loop {
            if let Some(why) = lost.borrow_and_update().clone() {
                return why;
            }
            if lost.changed().await.is_err() {
                return ControlLoss::Closed(gone());
            }
        }
    }

    /// Type `data`: keystrokes as the renderer made them, or text to paste. Input refused here is
    /// never sent and takes no number, so what is typed next is still the next one the host expects.
    pub async fn input(&self, data: &str, paste: bool) -> Result<()> {
        let sent = {
            let mut held = self.held.lock().expect("control");
            if held.lost.is_some() {
                return Err(ChannelError::Invalid("Control is no longer held.".into()));
            }
            let seq = held.seq + 1;
            let sent = self.link.send(
                Operation::TerminalInput(InputRequest {
                    lease: lease_ref(&self.terminal, &held.lease),
                    seq,
                    data: data.to_owned(),
                    paste,
                }),
                Expect::Reply,
            )?;
            held.seq = seq;
            sent
        };
        leased(&self.held, sent.await).map(|_| ())
    }

    /// Resize the terminal to fit this device. The size stands while control is held.
    pub async fn resize(&self, size: TermSize) -> Result<()> {
        let request = {
            let held = self.held.lock().expect("control");
            ResizeRequest {
                lease: lease_ref(&self.terminal, &held.lease),
                size,
            }
        };
        let sent = self
            .link
            .send(Operation::TerminalResize(request), Expect::Reply)?;
        leased(&self.held, sent.await).map(|_| ())
    }

    /// Hand control back.
    pub async fn release(self) -> Result<()> {
        let lease = {
            let mut held = self.held.lock().expect("control");
            if held.lost.is_some() {
                return Ok(());
            }
            held.lose(ControlLoss::Released);
            lease_ref(&self.terminal, &held.lease)
        };
        self.link
            .ask(Operation::TerminalRelease(lease))
            .await
            .map(|_| ())
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        self.renewer.abort();
    }
}

fn lease_ref(terminal: &TerminalId, lease: &Lease) -> LeaseRef {
    LeaseRef {
        terminal: terminal.clone(),
        lease: lease.id.clone(),
        generation: lease.generation,
    }
}

/// A lease's answer: one that says the lease is no longer current loses control.
fn leased(held: &Mutex<Held>, answer: Result<Answer>) -> Result<Reply> {
    match answer {
        Ok(Answer::Reply(reply)) => Ok(reply),
        Ok(_) => Err(something_else()),
        Err(ChannelError::Remote(error)) if error.code == ErrorCode::LeaseLost => {
            held.lock()
                .expect("control")
                .lose(ControlLoss::Lost(error.clone()));
            Err(ChannelError::Remote(error))
        }
        Err(error) => Err(error),
    }
}

async fn renew(link: Link, terminal: TerminalId, held: Arc<Mutex<Held>>, every: Duration) {
    let mut ticks = tokio::time::interval(every);
    ticks.tick().await;
    loop {
        ticks.tick().await;
        let sent = {
            let held = held.lock().expect("control");
            if held.lost.is_some() {
                return;
            }
            link.send(
                Operation::TerminalRenew(lease_ref(&terminal, &held.lease)),
                Expect::Reply,
            )
        };
        let Ok(sent) = sent else {
            return;
        };
        if let Ok(Reply::Lease(lease)) = leased(&held, sent.await) {
            held.lock().expect("control").lease = lease;
        }
    }
}

// --- subscriptions -------------------------------------------------------------------------------

/// What following activity hears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Followed {
    /// What was recorded next, oldest first, each event once.
    Events(ActivityBatch),
    /// The subscription is over.
    Ended(SubscriptionEnd),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionEnd {
    /// The host ended it: its error says why.
    Ended(RemoteError),
    Unsubscribed,
    Closed(ChannelEnd),
}

/// Activity being followed.
pub struct Subscription {
    id: SubscriptionId,
    newer: Option<Cursor>,
    followed: mpsc::UnboundedReceiver<Followed>,
    link: Link,
}

impl Subscription {
    pub fn id(&self) -> SubscriptionId {
        self.id
    }

    /// Where it got to: subscribe after this again on a new channel, and miss nothing still kept.
    pub fn newer(&self) -> Option<&Cursor> {
        self.newer.as_ref()
    }

    /// What was recorded next, or `None` once it has ended and that was said.
    pub async fn next(&mut self) -> Option<Followed> {
        let followed = self.followed.recv().await;
        if let Some(Followed::Events(batch)) = &followed {
            self.newer = Some(batch.newer.clone());
        }
        followed
    }

    pub async fn unsubscribe(self) -> Result<()> {
        let following = self
            .link
            .shared
            .lock()
            .expect("channel state")
            .subscriptions
            .remove(&self.id);
        if following.is_none() {
            return Ok(());
        }
        self.link
            .ask(Operation::ActivityUnsubscribe(ActivityUnsubscribeParams {
                subscription: self.id,
            }))
            .await
            .map(|_| ())
    }
}
