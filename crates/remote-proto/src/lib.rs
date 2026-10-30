//! Yardsort's remote protocol: what a phone or another computer may ask a host, and what the
//! host answers. Data and validation only — no transport, no store, no listener.
//!
//! The protocol is versioned on its own, apart from the app's releases, the local daemon's
//! protocol (`pty_ipc::PROTOCOL`) and any future sync schema. One major is supported at a time.
//! Within a major, changes are additive: new optional fields, new operations announced as
//! capabilities. A peer speaking another major gets `protocol_incompatible`, with the majors
//! this side speaks, and nothing else — there is no "try anyway".
//!
//! ```text
//! client → host   Hello    { client, protocols }                   JSON, first, once
//! host → client   HelloReply: { welcome } | { error }              JSON
//! client → host   Request  { requestId, protocol, op, params }     JSON, ≤ MAX_CONTROL_FRAME
//! host → client   Response { requestId, result | error }           JSON, ≤ MAX_CONTROL_FRAME
//! host → client   Event    { event, … }                            JSON, unasked
//! host → client   terminal output                                  binary, see OutputFrame
//! ```
//!
//! A host's text message is an [`Event`] when it has an `event` field, and a [`Response`]
//! otherwise. Messages reach a host through the server's relay ([`relay`]), which carries them
//! unchanged.
//!
//! Hosts answer the read side (`host.describe`, `workspace.list`, `workspace.describe`,
//! `session.list`, `activity.list`, `memory.list`) and, since minor 1, the `terminal.*`
//! operations: see [`terminal`]. Since minor 3 a client can follow activity as it is recorded,
//! with `activity.subscribe`: see [`ActivitySubscribeParams`]. A host lists what it answers in
//! [`HostDescription::capabilities`].

pub mod account;
pub mod grant;
pub mod ids;
pub mod model;
pub mod pairing;
pub mod relay;
pub mod terminal;

use serde::{Deserialize, Serialize};

pub use grant::{Grant, ProjectGrant, Scope};
pub use ids::{
    AccountRef, DeviceId, EventId, GrantId, HostId, IdError, LeaseId, MemoryId, ProjectId,
    RequestId, RunId, ServerId, SessionRecordId, TerminalId, WorkspaceId,
};
pub use model::*;
pub use terminal::{
    AcquireRequest, AttachRequest, AttachStart, Attached, ControlChanged, Controller,
    DetachRequest, Detached, Exited, InputRequest, Lease, LeaseRef, OutputFrame, ProgramExit,
    ResizeRequest, Resized, StreamId, StreamPosition,
};

/// The major this build speaks. Changes only for a change an older peer would misread.
pub const PROTOCOL_MAJOR: u16 = 1;
/// The minor this build speaks. Raised for each additive change; never required of a peer.
///
/// - 1: [`HelloReply`], [`Event`]s, and hosts answering the `terminal.*` operations.
/// - 2: resuming a stream, and the `terminal.control` and `terminal.exited` events.
/// - 3: `activity.subscribe` and `activity.unsubscribe`, and the `activity.events` and
///   `activity.ended` events.
/// - 4: `terminalsLater` in `host.describe`, saying why terminals are not offered yet.
pub const PROTOCOL_MINOR: u16 = 4;

/// The largest JSON message either side reads, requests and responses alike. Checked before
/// parsing. A host keeps its responses within it: a page holds fewer items than asked when they
/// are large, and each item's fields have limits ([`MAX_TEXT`] and the rest) so that one always
/// fits.
pub const MAX_CONTROL_FRAME: usize = 64 * 1024;
/// The largest chunk of terminal output in one frame.
pub const MAX_OUTPUT_CHUNK: usize = 64 * 1024;
/// How much output may wait for one slow client before it is detached and must re-snapshot.
/// Detaching is the honest choice: dropping bytes and carrying on would draw a corrupt screen.
pub const MAX_QUEUED_OUTPUT: usize = 1024 * 1024;
/// The most input one `terminal.input` carries.
pub const MAX_INPUT_BYTES: usize = 32 * 1024;
/// A page of a list, unless a request asks for fewer.
pub const DEFAULT_PAGE: u32 = 50;
/// The most a page holds, whatever a request asks for.
pub const MAX_PAGE: u32 = 200;
/// A control lease lapses this long after it was last renewed.
pub const LEASE_TTL_MS: u32 = 30_000;
/// How often a lease holder renews.
pub const LEASE_RENEW_MS: u32 = 10_000;
/// Longest client name in a [`Hello`].
pub const MAX_CLIENT_NAME: usize = 128;
/// The most activity subscriptions one connection holds at once.
pub const MAX_SUBSCRIPTIONS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    pub const CURRENT: Self = Self {
        major: PROTOCOL_MAJOR,
        minor: PROTOCOL_MINOR,
    };
}

/// The first thing a client says on a connection: who it is and which majors it can speak.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    /// For a host's list of connected devices and for support: `yardsort-mobile/0.1.0 (ios)`.
    pub client: String,
    /// Majors the client speaks, any order.
    pub protocols: Vec<u16>,
}

/// The host's answer to a [`Hello`] it can serve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Welcome {
    pub protocol: ProtocolVersion,
    pub host: HostDescription,
}

/// The host's answer to a [`Hello`]: `{"welcome": …}` or `{"error": …}`. After an error the host
/// closes the connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HelloReply {
    Welcome(Welcome),
    Error(RemoteError),
}

/// Something a host tells a client without being asked, about a stream the client attached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum Event {
    /// The terminal changed size, at this point in its output.
    #[serde(rename = "terminal.resized")]
    TerminalResized(Resized),
    /// The stream ended, and nothing more comes on it. Today only for a client that fell too far
    /// behind (`resnapshot_required`); its error says why, so other reasons can come later.
    #[serde(rename = "terminal.detached")]
    TerminalDetached(Detached),
    /// Control of the terminal changed hands, or a hold lapsed. Since minor 2.
    #[serde(rename = "terminal.control")]
    TerminalControl(ControlChanged),
    /// The terminal's program ended. Since minor 2.
    #[serde(rename = "terminal.exited")]
    TerminalExited(Exited),
    /// Activity recorded since a subscription's last batch, or since it began. Since minor 3.
    #[serde(rename = "activity.events")]
    ActivityEvents(ActivityBatch),
    /// A subscription ended, and nothing more comes on it: its error says why. Since minor 3.
    #[serde(rename = "activity.ended")]
    ActivityEnded(SubscriptionEnded),
}

/// Pick the protocol to speak with a client, or say why there is none.
pub fn negotiate(hello: &Hello) -> Result<ProtocolVersion, RemoteError> {
    if hello.client.len() > MAX_CLIENT_NAME || hello.protocols.len() > 16 {
        return Err(RemoteError::new(
            ErrorCode::InvalidRequest,
            "The greeting is larger than any client sends.",
        ));
    }
    if hello.protocols.contains(&PROTOCOL_MAJOR) {
        Ok(ProtocolVersion::CURRENT)
    } else {
        Err(RemoteError::incompatible())
    }
}

/// A stable reason a request failed. Clients branch on this, never on the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// No valid credential came with the request.
    Unauthenticated,
    /// The credential is valid, but its grant does not include this.
    Forbidden,
    /// Nothing by that id — or nothing the caller's grant reaches, which looks the same on
    /// purpose, so ids cannot be probed.
    NotFound,
    /// The host is not connected. It may be asleep; nothing is running commands for it.
    HostOffline,
    /// The session's process has exited.
    SessionEnded,
    /// Someone else controls the terminal.
    ControlConflict,
    /// The lease named is not the current one: it lapsed, was released, or control was taken.
    LeaseLost,
    /// The client fell too far behind on a terminal's output; attach again for a snapshot.
    ResnapshotRequired,
    /// The cursor is no longer understood; start the list again.
    CursorExpired,
    RateLimited,
    /// The peers share no protocol major. See `supported_protocols`.
    ProtocolIncompatible,
    /// Malformed, oversized, or with parameters out of bounds.
    InvalidRequest,
    /// An operation this host does not answer (yet, or at all).
    Unsupported,
    Internal,
    /// A code from a newer peer this build does not know. Treat as not retryable.
    #[serde(other)]
    Unknown,
}

impl ErrorCode {
    /// Whether the same request might succeed later without anything else changing.
    pub fn retryable(self) -> bool {
        matches!(self, Self::HostOffline | Self::RateLimited | Self::Internal)
    }
}

/// A failed request: a code to branch on, a message safe to show a person, and whether trying
/// again could help. Messages never carry paths, terminal content, secrets or ids beyond the ones
/// the request named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    /// With `protocol_incompatible`: the majors this side speaks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_protocols: Option<Vec<u16>>,
}

impl RemoteError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: code.retryable(),
            supported_protocols: None,
        }
    }

    pub fn incompatible() -> Self {
        Self {
            supported_protocols: Some(vec![PROTOCOL_MAJOR]),
            ..Self::new(
                ErrorCode::ProtocolIncompatible,
                "This host speaks a different version of the remote protocol. Update the app \
                 that is older.",
            )
        }
    }

    pub fn not_found() -> Self {
        Self::new(ErrorCode::NotFound, "There is nothing by that id here.")
    }

    pub fn forbidden(scope: Scope) -> Self {
        let scope = serde_json::to_value(scope)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default();
        Self::new(
            ErrorCode::Forbidden,
            format!("This device has not been given {scope} on this host."),
        )
    }
}

impl std::fmt::Display for RemoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RemoteError {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceListParams {
    /// Only this project's.
    #[serde(default)]
    pub project: Option<ProjectId>,
    #[serde(default)]
    pub include_archived: bool,
    #[serde(default)]
    pub cursor: Option<Cursor>,
    /// At most [`MAX_PAGE`]; [`DEFAULT_PAGE`] when absent.
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRef {
    pub workspace: WorkspaceId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionListParams {
    pub workspace: WorkspaceId,
    /// Ended conversations too, not only those whose record says running.
    #[serde(default)]
    pub include_ended: bool,
    #[serde(default)]
    pub cursor: Option<Cursor>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityListParams {
    pub workspace: WorkspaceId,
    /// Only this conversation's events.
    #[serde(default)]
    pub session: Option<SessionRecordId>,
    /// A previous page's `older` or `newer`. Without one, the newest events.
    #[serde(default)]
    pub cursor: Option<Cursor>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// `activity.subscribe`: follow activity as it is recorded. Needs `activity:read`.
///
/// The host replies [`Subscribed`], then sends what was recorded after `after`, if given, and
/// from then on what is recorded, in `activity.events` events ([`ActivityBatch`]), oldest first
/// and each event once. Each batch's `newer` is where it got to: a client that loses its
/// connection subscribes again after the last one it took in, and misses nothing still kept.
/// When the host no longer keeps the event `after` names (the events were pruned or cleared, or
/// the workspace is no longer on show), the reply is `cursor_expired`: the client reads the list
/// again (`activity.list`) and subscribes after its `newer`.
///
/// A subscription holds nothing at the host but its position. A client that falls behind is
/// sent the rest when it catches up, not detached: the events are on disk, not in a queue.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySubscribeParams {
    /// Only this workspace's. Without one, every workspace the grant reaches, those made later
    /// included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceId>,
    /// Only this conversation's. Needs `workspace`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionRecordId>,
    /// A `newer` cursor: from an [`ActivityPage`], an [`ActivityBatch`] or a [`Subscribed`] of
    /// the same scope. Without one, only what is recorded from now on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Cursor>,
}

/// `activity.unsubscribe`: stop following. Needs `activity:read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityUnsubscribeParams {
    pub subscription: SubscriptionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryListParams {
    pub project: ProjectId,
    #[serde(default)]
    pub cursor: Option<Cursor>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Everything a client can ask a host. On the wire, `op` names it and `params` carries its
/// parameters.
///
/// There is deliberately no operation that approves, edits or proposes memory, launches a
/// process, deletes or archives a workspace, or reads a file. Each of those is its own scoped
/// increment, if it ever comes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", content = "params")]
pub enum Operation {
    #[serde(rename = "host.describe")]
    HostDescribe,
    #[serde(rename = "workspace.list")]
    WorkspaceList(WorkspaceListParams),
    #[serde(rename = "workspace.describe")]
    WorkspaceDescribe(WorkspaceRef),
    #[serde(rename = "session.list")]
    SessionList(SessionListParams),
    #[serde(rename = "activity.list")]
    ActivityList(ActivityListParams),
    #[serde(rename = "memory.list")]
    MemoryList(MemoryListParams),
    #[serde(rename = "terminal.attach")]
    TerminalAttach(AttachRequest),
    #[serde(rename = "terminal.detach")]
    TerminalDetach(DetachRequest),
    #[serde(rename = "terminal.acquire")]
    TerminalAcquire(AcquireRequest),
    #[serde(rename = "terminal.renew")]
    TerminalRenew(LeaseRef),
    #[serde(rename = "terminal.release")]
    TerminalRelease(LeaseRef),
    #[serde(rename = "terminal.input")]
    TerminalInput(InputRequest),
    #[serde(rename = "terminal.resize")]
    TerminalResize(ResizeRequest),
    #[serde(rename = "activity.subscribe")]
    ActivitySubscribe(ActivitySubscribeParams),
    #[serde(rename = "activity.unsubscribe")]
    ActivityUnsubscribe(ActivityUnsubscribeParams),
}

/// Every operation's wire name, in [`Operation`]'s order.
pub const OPERATIONS: &[&str] = &[
    "host.describe",
    "workspace.list",
    "workspace.describe",
    "session.list",
    "activity.list",
    "memory.list",
    "terminal.attach",
    "terminal.detach",
    "terminal.acquire",
    "terminal.renew",
    "terminal.release",
    "terminal.input",
    "terminal.resize",
    "activity.subscribe",
    "activity.unsubscribe",
];

impl Operation {
    pub fn name(&self) -> &'static str {
        let index = match self {
            Self::HostDescribe => 0,
            Self::WorkspaceList(_) => 1,
            Self::WorkspaceDescribe(_) => 2,
            Self::SessionList(_) => 3,
            Self::ActivityList(_) => 4,
            Self::MemoryList(_) => 5,
            Self::TerminalAttach(_) => 6,
            Self::TerminalDetach(_) => 7,
            Self::TerminalAcquire(_) => 8,
            Self::TerminalRenew(_) => 9,
            Self::TerminalRelease(_) => 10,
            Self::TerminalInput(_) => 11,
            Self::TerminalResize(_) => 12,
            Self::ActivitySubscribe(_) => 13,
            Self::ActivityUnsubscribe(_) => 14,
        };
        OPERATIONS[index]
    }

    /// The scope a caller needs for this, before anything else is looked at. `None` for
    /// `host.describe`, which any valid grant may ask.
    pub fn scope(&self) -> Option<Scope> {
        Some(match self {
            Self::HostDescribe => return None,
            Self::WorkspaceList(_) | Self::WorkspaceDescribe(_) | Self::SessionList(_) => {
                Scope::WorkspaceRead
            }
            Self::ActivityList(_) | Self::ActivitySubscribe(_) | Self::ActivityUnsubscribe(_) => {
                Scope::ActivityRead
            }
            Self::MemoryList(_) => Scope::MemoryRead,
            Self::TerminalAttach(_) | Self::TerminalDetach(_) => Scope::TerminalRead,
            Self::TerminalAcquire(_)
            | Self::TerminalRenew(_)
            | Self::TerminalRelease(_)
            | Self::TerminalInput(_)
            | Self::TerminalResize(_) => Scope::TerminalControl,
        })
    }

    /// Bounds a well-formed request must also meet.
    pub fn validate(&self) -> Result<(), RemoteError> {
        let invalid = |message: &str| Err(RemoteError::new(ErrorCode::InvalidRequest, message));
        match self {
            Self::WorkspaceList(WorkspaceListParams { limit, .. })
            | Self::SessionList(SessionListParams { limit, .. })
            | Self::ActivityList(ActivityListParams { limit, .. })
            | Self::MemoryList(MemoryListParams { limit, .. })
                if *limit == Some(0) =>
            {
                invalid("A page holds at least one item.")
            }
            Self::TerminalInput(input) if input.data.len() > MAX_INPUT_BYTES => {
                invalid("That input is larger than one request may carry.")
            }
            Self::TerminalInput(input) if input.seq == 0 => invalid("Input is numbered from 1."),
            Self::ActivitySubscribe(ActivitySubscribeParams {
                workspace: None,
                session: Some(_),
                ..
            }) => invalid("A conversation is followed within its workspace: name both."),
            Self::TerminalResize(resize) if resize.size.cols == 0 || resize.size.rows == 0 => {
                invalid("A terminal needs at least one row and one column.")
            }
            _ => Ok(()),
        }
    }
}

/// The page size to use for a request's `limit`.
pub fn page_size(limit: Option<u32>) -> usize {
    limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE) as usize
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub request_id: RequestId,
    pub protocol: ProtocolVersion,
    #[serde(flatten)]
    pub operation: Operation,
}

/// What a successful request returns, tagged with `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Reply {
    Host(HostDescription),
    Workspaces(WorkspacePage),
    Workspace(WorkspaceDetail),
    Sessions(SessionPage),
    Activity(ActivityPage),
    Memory(MemoryPage),
    Attached(Attached),
    Lease(Lease),
    Subscribed(Subscribed),
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Result(Reply),
    Error(RemoteError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    /// `None` only when the request was too broken to find its id.
    pub request_id: Option<RequestId>,
    #[serde(flatten)]
    pub outcome: Outcome,
}

impl Response {
    pub fn error(request_id: Option<RequestId>, error: RemoteError) -> Self {
        Self {
            request_id,
            outcome: Outcome::Error(error),
        }
    }
}

/// Read a request from untrusted bytes, or the response that says why it cannot be read.
///
/// Size is checked before parsing. When the request does not parse, it is looked at once more,
/// loosely, to give the most useful refusal: another protocol major is `protocol_incompatible`,
/// an operation this build does not know is `unsupported`, anything else `invalid_request`.
/// Refusals never echo the request's content back.
pub fn decode_request(bytes: &[u8]) -> Result<Request, Box<Response>> {
    if bytes.len() > MAX_CONTROL_FRAME {
        return Err(Box::new(Response::error(
            None,
            RemoteError::new(
                ErrorCode::InvalidRequest,
                "The request is larger than any request may be.",
            ),
        )));
    }
    match serde_json::from_slice::<Request>(bytes) {
        Ok(request) => match request.operation.validate() {
            Ok(()) => Ok(request),
            Err(error) => Err(Box::new(Response::error(Some(request.request_id), error))),
        },
        Err(_) => Err(Box::new(Response::error(
            probe(bytes).0,
            probe_error(bytes),
        ))),
    }
}

/// The parts of a request worth reading even when the whole does not parse.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Probe {
    #[serde(default)]
    request_id: Option<serde_json::Value>,
    #[serde(default)]
    protocol: Option<serde_json::Value>,
    #[serde(default)]
    op: Option<serde_json::Value>,
}

fn probe(bytes: &[u8]) -> (Option<RequestId>, Option<Probe>) {
    let Ok(probe) = serde_json::from_slice::<Probe>(bytes) else {
        return (None, None);
    };
    let id = probe
        .request_id
        .as_ref()
        .and_then(|v| v.as_str())
        .and_then(|s| RequestId::new(s).ok());
    (id, Some(probe))
}

fn probe_error(bytes: &[u8]) -> RemoteError {
    let invalid = || {
        RemoteError::new(
            ErrorCode::InvalidRequest,
            "The request is not a well-formed request.",
        )
    };
    let Some(probe) = probe(bytes).1 else {
        return invalid();
    };
    let major = probe
        .protocol
        .as_ref()
        .and_then(|p| p.get("major"))
        .and_then(|m| m.as_u64());
    if let Some(major) = major {
        if major != u64::from(PROTOCOL_MAJOR) {
            return RemoteError::incompatible();
        }
    }
    match probe.op.as_ref().and_then(|op| op.as_str()) {
        Some(op) if !OPERATIONS.contains(&op) => RemoteError::new(
            ErrorCode::Unsupported,
            "This host does not know that operation.",
        ),
        _ => invalid(),
    }
}

/// Write a response for the wire.
pub fn encode_response(response: &Response) -> Vec<u8> {
    // Every type in a response serialises without failing: no maps with non-string keys, no
    // custom serialisers that can refuse.
    serde_json::to_vec(response).expect("a response always serialises")
}

#[cfg(test)]
mod tests;
