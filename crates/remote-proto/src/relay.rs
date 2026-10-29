//! The relay: how a device reaches a host through the server, with neither of them listening.
//!
//! Both sides connect out to the server. Each first asks for a **ticket** over HTTPS with its
//! access credential, then opens a WebSocket and presents the ticket as its first message. A
//! ticket is good once, for [`TICKET_TTL_MS`], and names what it was issued for: a host's own
//! link, or one device's way to one host under the grant the server has in force for it. The
//! access credential never travels on the WebSocket, and a ticket is never put in a URL.
//!
//! ```text
//! either  POST /v1/relay/tickets   TicketRequest            → RelayTicket
//! either  GET  /v1/relay?route=…   WebSocket upgrade, with the ticket's route if it has one
//!         → {"ticket": …}                                     first message, within FIRST_MESSAGE_MS
//!         ← Greeting                                          then the relay carries traffic
//! ```
//!
//! **A device's socket** then carries the remote protocol itself, untouched: the client's
//! [`Hello`](crate::Hello) and requests as text, the host's replies and [`Event`](crate::Event)s
//! as text, and terminal output as binary [`OutputFrame`](crate::OutputFrame)s.
//!
//! **A host's socket** carries every device's traffic at once, each in its own **channel**,
//! framed as a [`LinkFrame`]. The relay opens a channel when a device connects
//! ([`ChannelOpen`], naming the device and the grant the server has in force for it), and closes
//! it when the device leaves, its grant goes, or it falls too far behind ([`CloseReason`]). The
//! host closes a channel too, when its own record no longer lets the device in. Nothing a host
//! sends on a channel reaches any other device.
//!
//! **Channel 0 is the link's own** ([`NOTICE_CHANNEL`]): never a device's. The relay tells the
//! host something there, as text: a [`HostNotice`], such as a device asking to pair. A notice is
//! only a hint to ask the server again, never something to act on by itself, and a host that
//! does not know a notice ignores it. A host from before notices ignores the channel, since no
//! device is on it.
//!
//! **Where the socket goes.** A server can run as several instances, and a device's socket has
//! to reach the one holding its host's link. The server sends a device's ticket request there,
//! and the ticket says where that is ([`RelayTicket::route`]); the socket names it in its URL
//! ([`ROUTE_PARAM`]), and the server sends the socket on to it. A route names an instance of the
//! server, nothing about the device or the host, and is no secret. A ticket without one is from
//! a server that runs as one instance.
//!
//! **What the relay keeps:** nothing. Tickets and queues are in memory and go with the
//! connection or the process; the relay never logs or stores what it carries.
//!
//! A socket that breaks the rules — an oversized message, binary from a device, a frame for a
//! channel that is not open — is closed with [`CloseReason::Invalid`].

use serde::{Deserialize, Serialize};

use crate::account::Secret;
use crate::ids::{DeviceId, GrantId, HostId};
use crate::RemoteError;

/// How long a ticket can be presented.
pub const TICKET_TTL_MS: u64 = 60_000;
/// How long a new socket has to present its ticket before it is closed.
pub const FIRST_MESSAGE_MS: u64 = 10_000;
/// How often the relay pings each socket. Under the 60 seconds after which proxies (Fly.io's
/// among them) close a quiet connection.
pub const PING_EVERY_MS: u64 = 20_000;
/// A side that has heard nothing at all, not even a ping, for this long takes the link to be
/// dead and closes it.
pub const SILENCE_MS: u64 = 60_000;
/// Devices connected to one host at once. One more is refused with `rate_limited`.
pub const MAX_DEVICES_PER_HOST: usize = 16;
/// How much may wait for one socket before the relay gives up on it. A device that falls this
/// far behind is closed with [`CloseReason::Overloaded`] and attaches again for a snapshot; a
/// host that does is closed, and reconnects.
pub const MAX_QUEUED: usize = 2 * crate::MAX_QUEUED_OUTPUT;
/// Bytes before a [`LinkFrame`]'s payload.
pub const LINK_HEADER_LEN: usize = 5;
/// The largest message on either kind of socket: a terminal output frame, or a control message,
/// in a link frame.
pub const MAX_MESSAGE: usize = LINK_HEADER_LEN
    + if crate::MAX_CONTROL_FRAME > crate::terminal::OUTPUT_HEADER_LEN + crate::MAX_OUTPUT_CHUNK {
        crate::MAX_CONTROL_FRAME
    } else {
        crate::terminal::OUTPUT_HEADER_LEN + crate::MAX_OUTPUT_CHUNK
    };

/// `POST /v1/relay/tickets`: what the ticket is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "camelCase")]
pub enum TicketRequest {
    /// The host's own link. Only the device the host is signed in as may ask.
    Host { host: HostId },
    /// A device's way to a host. Only with a grant on that host the server has in force.
    Device { host: HostId },
}

impl TicketRequest {
    pub fn host(&self) -> &HostId {
        match self {
            Self::Host { host } | Self::Device { host } => host,
        }
    }
}

/// A ticket, good once within [`TICKET_TTL_MS`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayTicket {
    pub ticket: Secret,
    pub expires_in_ms: u64,
    /// Which instance of the server holds the ticket: the socket's URL names it as
    /// [`ROUTE_PARAM`]. Absent from a server that runs as one instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
}

/// The relay socket's query parameter naming a ticket's [`RelayTicket::route`].
pub const ROUTE_PARAM: &str = "route";
/// The longest route: letters, digits, `-` and `_`.
pub const MAX_ROUTE_LEN: usize = 64;

/// Whether `route` could name an instance: what a server checks before it acts on one.
pub fn valid_route(route: &str) -> bool {
    !route.is_empty()
        && route.len() <= MAX_ROUTE_LEN
        && route
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The first message on a relay socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presented {
    pub ticket: Secret,
}

/// The relay's answer to a presented ticket. After `refused`, it closes the socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Greeting {
    /// Connected. A device speaks to `host` from here on; a host waits for channels.
    Connected {
        host: HostId,
    },
    Refused(RemoteError),
}

/// Sent to a host when a device connects: which device, and the grant the server has in force
/// for it. The host lets it in only if its own record agrees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelOpen {
    pub device: DeviceId,
    pub grant: GrantId,
}

/// Why a socket or a channel was closed. On a socket, the WebSocket close code is
/// [`CloseReason::code`]; on a host's link, a [`LinkFrame::Close`] carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloseReason {
    /// The other end left.
    Gone,
    /// The ticket was missing, wrong, spent or late.
    Unauthenticated,
    /// The device's grant, the device, or the host is no longer in force.
    Revoked,
    /// The host is not connected, or left.
    HostOffline,
    /// It fell more than [`MAX_QUEUED`] behind. Attach again for a snapshot.
    Overloaded,
    /// The host connected again from elsewhere; this link is the old one.
    Replaced,
    /// The host would not let the device in: its own record does not have the grant.
    Refused,
    /// It broke the rules of the socket.
    Invalid,
    /// The relay is shutting down. Connect again.
    Restarting,
    /// A reason from a newer peer.
    #[serde(other)]
    Unknown,
}

impl CloseReason {
    /// The WebSocket close code: 4000 and up, the range left for applications.
    pub fn code(self) -> u16 {
        match self {
            Self::Gone => 4000,
            Self::Unauthenticated => 4001,
            Self::Revoked => 4003,
            Self::HostOffline => 4004,
            Self::Overloaded => 4008,
            Self::Replaced => 4009,
            Self::Refused => 4010,
            Self::Invalid => 4400,
            Self::Restarting => 4503,
            Self::Unknown => 4999,
        }
    }

    pub fn from_code(code: u16) -> Self {
        [
            Self::Gone,
            Self::Unauthenticated,
            Self::Revoked,
            Self::HostOffline,
            Self::Overloaded,
            Self::Replaced,
            Self::Refused,
            Self::Invalid,
            Self::Restarting,
        ]
        .into_iter()
        .find(|reason| reason.code() == code)
        .unwrap_or(Self::Unknown)
    }

    /// A few words for a WebSocket close frame and for a person: no ids, nothing of what was
    /// carried.
    pub fn text(self) -> &'static str {
        match self {
            Self::Gone => "the other end left",
            Self::Unauthenticated => "no valid ticket",
            Self::Revoked => "access was revoked",
            Self::HostOffline => "the host is offline",
            Self::Overloaded => "fell too far behind",
            Self::Replaced => "the host connected again",
            Self::Refused => "the host did not let this device in",
            Self::Invalid => "the socket broke its rules",
            Self::Restarting => "the server is restarting",
            Self::Unknown => "closed",
        }
    }
}

/// One channel is one device's connection, numbered by the relay.
pub type Channel = u32;

/// The channel no device is ever given: the relay's notices to the host go on it as text.
pub const NOTICE_CHANNEL: Channel = 0;

/// Something the relay tells a host on [`NOTICE_CHANNEL`]. A hint: the host asks the server
/// for what it says changed, with its own credential, as it would anyway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HostNotice {
    /// A device redeemed one of this host's pairing offers and waits for its person's answer:
    /// the grant is among the host's, pending.
    PairingRequested { grant: GrantId },
    /// A notice from a newer server. Ignored.
    #[serde(other)]
    Unknown,
}

/// A message on a host's link.
///
/// ```text
/// offset  size  field
///      0     1  kind      1 open, 2 close, 3 text, 4 binary
///      1     4  channel   u32, little-endian
///      5     …  payload   open: ChannelOpen JSON; close: CloseReason JSON;
///                         text: one text message; binary: one binary message
/// ```
///
/// The relay sends open, close, text and binary; a host sends close, text and binary. Text and
/// binary are a device's messages, as the device sent them or is to receive them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkFrame<'a> {
    Open(Channel, ChannelOpen),
    Close(Channel, CloseReason),
    Text(Channel, &'a [u8]),
    Binary(Channel, &'a [u8]),
}

/// Why a link frame could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("the frame is shorter than its header")]
    Truncated,
    #[error("unknown link frame kind {0}")]
    UnknownKind(u8),
    #[error("the frame is larger than any message")]
    TooLarge,
    #[error("the frame's payload does not read")]
    Malformed,
}

const KIND_OPEN: u8 = 1;
const KIND_CLOSE: u8 = 2;
const KIND_TEXT: u8 = 3;
const KIND_BINARY: u8 = 4;

impl<'a> LinkFrame<'a> {
    pub fn channel(&self) -> Channel {
        match self {
            Self::Open(c, _) | Self::Close(c, _) | Self::Text(c, _) | Self::Binary(c, _) => *c,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let (kind, payload): (u8, std::borrow::Cow<'_, [u8]>) = match self {
            Self::Open(_, open) => (
                KIND_OPEN,
                serde_json::to_vec(open).expect("serialises").into(),
            ),
            Self::Close(_, reason) => (
                KIND_CLOSE,
                serde_json::to_vec(reason).expect("serialises").into(),
            ),
            Self::Text(_, bytes) => (KIND_TEXT, (*bytes).into()),
            Self::Binary(_, bytes) => (KIND_BINARY, (*bytes).into()),
        };
        let mut out = Vec::with_capacity(LINK_HEADER_LEN + payload.len());
        out.push(kind);
        out.extend_from_slice(&self.channel().to_le_bytes());
        out.extend_from_slice(&payload);
        out
    }

    /// Read one frame, checking its size before anything else.
    pub fn decode(frame: &'a [u8]) -> Result<Self, LinkError> {
        if frame.len() > MAX_MESSAGE {
            return Err(LinkError::TooLarge);
        }
        if frame.len() < LINK_HEADER_LEN {
            return Err(LinkError::Truncated);
        }
        let channel = u32::from_le_bytes(frame[1..5].try_into().expect("four bytes"));
        let payload = &frame[LINK_HEADER_LEN..];
        Ok(match frame[0] {
            KIND_OPEN => Self::Open(
                channel,
                serde_json::from_slice(payload).map_err(|_| LinkError::Malformed)?,
            ),
            KIND_CLOSE => Self::Close(
                channel,
                serde_json::from_slice(payload).map_err(|_| LinkError::Malformed)?,
            ),
            KIND_TEXT => Self::Text(channel, payload),
            KIND_BINARY => Self::Binary(channel, payload),
            other => return Err(LinkError::UnknownKind(other)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_link_header_layout_is_fixed() {
        assert_eq!(
            LinkFrame::Text(0x0102_0304, b"{}").encode(),
            [3, 4, 3, 2, 1, b'{', b'}']
        );
        assert_eq!(
            LinkFrame::Close(7, CloseReason::Revoked).encode(),
            b"\x02\x07\0\0\0\"revoked\""
        );
    }

    #[test]
    fn link_frames_round_trip() {
        let open = ChannelOpen {
            device: DeviceId::new("phone").unwrap(),
            grant: GrantId::new("g1").unwrap(),
        };
        let output = [1u8, 1, 0, 0, 0xff];
        for frame in [
            LinkFrame::Open(1, open),
            LinkFrame::Close(2, CloseReason::Overloaded),
            LinkFrame::Text(3, br#"{"requestId":"r1"}"#),
            LinkFrame::Binary(u32::MAX, &output),
        ] {
            assert_eq!(LinkFrame::decode(&frame.encode()).unwrap(), frame);
        }
    }

    #[test]
    fn short_unknown_malformed_and_oversized_frames_are_refused() {
        assert_eq!(LinkFrame::decode(&[3, 0, 0]), Err(LinkError::Truncated));
        assert_eq!(
            LinkFrame::decode(&[9, 0, 0, 0, 0]),
            Err(LinkError::UnknownKind(9))
        );
        assert_eq!(
            LinkFrame::decode(b"\x01\0\0\0\0{\"device\":\"bad id\"}"),
            Err(LinkError::Malformed)
        );
        let mut big = vec![KIND_BINARY, 0, 0, 0, 0];
        big.resize(MAX_MESSAGE + 1, 0);
        assert_eq!(LinkFrame::decode(&big), Err(LinkError::TooLarge));
        big.truncate(MAX_MESSAGE);
        assert!(LinkFrame::decode(&big).is_ok());
    }

    #[test]
    fn a_route_is_a_short_plain_name() {
        assert!(valid_route("148e21eae69398"));
        assert!(valid_route("instance-a_1"));
        for bad in ["", "a b", "a/b", "a?b", "é", &"a".repeat(MAX_ROUTE_LEN + 1)] {
            assert!(!valid_route(bad), "{bad:?}");
        }
    }

    #[test]
    fn close_reasons_keep_their_codes() {
        for reason in [
            CloseReason::Gone,
            CloseReason::Unauthenticated,
            CloseReason::Revoked,
            CloseReason::HostOffline,
            CloseReason::Overloaded,
            CloseReason::Replaced,
            CloseReason::Refused,
            CloseReason::Invalid,
            CloseReason::Restarting,
        ] {
            assert!((4000..5000).contains(&reason.code()));
            assert_eq!(CloseReason::from_code(reason.code()), reason);
        }
        assert_eq!(CloseReason::from_code(1000), CloseReason::Unknown);
        let newer: CloseReason = serde_json::from_str("\"suspended\"").unwrap();
        assert_eq!(newer, CloseReason::Unknown);
    }
}
