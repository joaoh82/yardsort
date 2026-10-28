//! What crosses the socket, and how it is framed.
//!
//! Every frame is `[u32 length][u8 kind][payload]`, little-endian. Control traffic — requests,
//! responses, events — is JSON, reusing the `serde` implementations the host types already
//! carry. Terminal output is not: it gets a frame kind of its own and travels as raw bytes, the
//! same bytes the webview hands to xterm.js, so a busy agent does not pay for JSON on every
//! repaint. Numbered output has a kind of its own too, with its place in the stream in front.

use std::io::{self, Read, Write};

use pty_host::{
    Controller, HostError, HostEvent, LaunchPlan, Lease, LeaseRef, Numbered, OutputSeq, SessionId,
    SessionInfo, TermSize,
};
use serde::{Deserialize, Serialize};

/// The language this build speaks. Bumped whenever a frame or message changes shape, and
/// **independently of the app version**: most releases leave it alone, and a daemon holding live
/// agents is only in the way when it really cannot be talked to.
///
/// 2 added numbered output ([`Op::Watch`]) and control leases ([`Op::Acquire`] and the rest),
/// and nothing else: every message of 1 kept its shape. So a client of 2 still uses a daemon of
/// 1 for everything 1 had, which keeps agents running under one through an update.
pub const PROTOCOL: u32 = 2;
/// The oldest daemon this build can still run terminals in.
pub const OLDEST_PROTOCOL: u32 = 1;
/// The first protocol with numbered output and control leases.
pub const CONTROL_PROTOCOL: u32 = 2;

pub const KIND_REQUEST: u8 = 1;
pub const KIND_RESPONSE: u8 = 2;
pub const KIND_EVENT: u8 = 3;
pub const KIND_OUTPUT: u8 = 4;
/// Numbered output for one watching viewer; see [`Frame::numbered`]. Protocol 2.
pub const KIND_NUMBERED: u8 = 5;

/// Refuse anything larger rather than allocate it. Output is batched well below this
/// (`MAX_BATCH` in the host is 512 KiB) and control messages are tiny.
pub const MAX_FRAME: usize = 8 * 1024 * 1024;

pub type RequestId = u64;

/// Identifies one attached viewer *on one connection*. The **client** picks it, so it can
/// register the sink before asking to attach — the snapshot starts arriving during the attach
/// call, ahead of the response.
pub type StreamId = u32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: RequestId,
    pub op: Op,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "op")]
pub enum Op {
    /// Always the first request on a connection.
    Hello {
        protocol: u32,
        client: String,
    },
    Spawn {
        plan: Box<LaunchPlan>,
    },
    Attach {
        session: SessionId,
        stream: StreamId,
    },
    Detach {
        session: SessionId,
        stream: StreamId,
    },
    /// Keystrokes and control sequences: a handful of bytes at a time. Anything long enough for
    /// a JSON byte array to matter is a paste, and has its own op.
    Write {
        session: SessionId,
        data: Vec<u8>,
    },
    Paste {
        session: SessionId,
        text: String,
    },
    Resize {
        session: SessionId,
        size: TermSize,
    },
    Kill {
        session: SessionId,
    },
    Remove {
        session: SessionId,
    },
    Info {
        session: SessionId,
    },
    List,
    /// "I am leaving." The daemon answers nothing and closes its side, which is what lets the
    /// client's reader thread end and the socket actually close. Sessions carry on regardless.
    Goodbye,
    /// Stop the daemon. Running sessions are killed first only if asked.
    Shutdown {
        stop_sessions: bool,
    },
    // Protocol 2 from here on. A daemon of 1 cannot read these, and drops the connection on
    // them, so the client never sends one there.
    /// Watch a session, numbered: as `Attach`, but output comes as numbered frames.
    Watch {
        session: SessionId,
        stream: StreamId,
    },
    /// Who controls a session.
    Controller {
        session: SessionId,
    },
    Acquire {
        session: SessionId,
        holder: String,
        take_over: bool,
    },
    Renew {
        session: SessionId,
        lease: LeaseRef,
    },
    Release {
        session: SessionId,
        lease: LeaseRef,
    },
    /// Type under a lease: `seq` numbers the inputs under it from 1.
    Input {
        session: SessionId,
        lease: LeaseRef,
        seq: u64,
        data: String,
        paste: bool,
    },
    /// Type for the app itself: takes no control, refused while a lease is held.
    TypeUnattended {
        session: SessionId,
        text: String,
        paste: bool,
    },
    ResizeLeased {
        session: SessionId,
        lease: LeaseRef,
        size: TermSize,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: RequestId,
    pub result: OpResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// Struct variants throughout, not newtypes: serde's internal tagging cannot encode a newtype
// variant that wraps a sequence, and `Sessions` is one.
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "ok")]
pub enum OpResult {
    Welcome {
        daemon: DaemonInfo,
    },
    Session {
        session: Box<SessionInfo>,
    },
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    /// Attach, detach, write, resize, kill, remove, shutdown: nothing to report but success.
    Done,
    Failed {
        error: WireError,
    },
    Controller {
        controller: Controller,
    },
    Leased {
        lease: Lease,
    },
}

/// Who answered the handshake. Shown in the status bar and in bug reports, and how the app
/// notices it is talking to a daemon from before an update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonInfo {
    pub protocol: u32,
    pub version: String,
    pub pid: u32,
    /// Milliseconds since the Unix epoch.
    pub started_at: u64,
    /// Sessions the daemon is running right now. Lets the app decide whether a daemon it cannot
    /// talk to can simply be replaced, or whether there is work to ask about first.
    pub running_sessions: u32,
}

/// [`HostError`] as it travels. `io::Error` is not serialisable, and the two variants the app
/// branches on must survive the trip intact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum WireError {
    UnknownSession {
        id: SessionId,
    },
    SessionExited {
        id: SessionId,
    },
    OpenPty {
        message: String,
    },
    Spawn {
        program: String,
        reason: String,
    },
    Io {
        message: String,
    },
    LeaseLost {
        id: SessionId,
    },
    ControlHeld {
        id: SessionId,
        by: Controller,
    },
    InputOutOfOrder {
        id: SessionId,
        expected: u64,
        got: u64,
    },
    InvalidHolder,
    ControlUnavailable,
}

impl From<HostError> for WireError {
    fn from(error: HostError) -> Self {
        match error {
            HostError::UnknownSession(id) => Self::UnknownSession { id },
            HostError::SessionExited(id) => Self::SessionExited { id },
            HostError::OpenPty(message) => Self::OpenPty { message },
            HostError::Spawn { program, reason } => Self::Spawn { program, reason },
            HostError::Io(error) => Self::Io {
                message: error.to_string(),
            },
            HostError::LeaseLost(id) => Self::LeaseLost { id },
            HostError::ControlHeld { id, by } => Self::ControlHeld { id, by },
            HostError::InputOutOfOrder { id, expected, got } => {
                Self::InputOutOfOrder { id, expected, got }
            }
            HostError::InvalidHolder => Self::InvalidHolder,
            HostError::ControlUnavailable => Self::ControlUnavailable,
        }
    }
}

impl From<WireError> for HostError {
    fn from(error: WireError) -> Self {
        match error {
            WireError::UnknownSession { id } => Self::UnknownSession(id),
            WireError::SessionExited { id } => Self::SessionExited(id),
            WireError::OpenPty { message } => Self::OpenPty(message),
            WireError::Spawn { program, reason } => Self::Spawn { program, reason },
            WireError::Io { message } => Self::Io(io::Error::other(message)),
            WireError::LeaseLost { id } => Self::LeaseLost(id),
            WireError::ControlHeld { id, by } => Self::ControlHeld { id, by },
            WireError::InputOutOfOrder { id, expected, got } => {
                Self::InputOutOfOrder { id, expected, got }
            }
            WireError::InvalidHolder => Self::InvalidHolder,
            WireError::ControlUnavailable => Self::ControlUnavailable,
        }
    }
}

/// A frame as it was read: its kind and its payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn json<T: Serialize>(kind: u8, value: &T) -> io::Result<Self> {
        Ok(Self {
            kind,
            payload: serde_json::to_vec(value).map_err(io::Error::other)?,
        })
    }

    /// Terminal output for one attached viewer: `[u32 stream][bytes]`.
    pub fn output(stream: StreamId, bytes: &[u8]) -> Self {
        let mut payload = Vec::with_capacity(4 + bytes.len());
        payload.extend_from_slice(&stream.to_le_bytes());
        payload.extend_from_slice(bytes);
        Self {
            kind: KIND_OUTPUT,
            payload,
        }
    }

    /// One numbered item for one watching viewer:
    /// `[u32 stream][u8 item][u64 seq]`, then for a snapshot `[u16 cols][u16 rows][bytes]`, for
    /// output `[bytes]`, and for a resize `[u16 cols][u16 rows]`.
    pub fn numbered(stream: StreamId, item: &Numbered<'_>) -> Self {
        let (tag, seq, size, bytes) = match *item {
            Numbered::Snapshot { seq, size, bytes } => (ITEM_SNAPSHOT, seq, Some(size), bytes),
            Numbered::Output { seq, bytes } => (ITEM_OUTPUT, seq, None, bytes),
            Numbered::Resized { seq, size } => (ITEM_RESIZED, seq, Some(size), &[][..]),
        };
        let mut payload = Vec::with_capacity(NUMBERED_HEADER + 4 + bytes.len());
        payload.extend_from_slice(&stream.to_le_bytes());
        payload.push(tag);
        payload.extend_from_slice(&seq.to_le_bytes());
        if let Some(size) = size {
            payload.extend_from_slice(&size.cols.to_le_bytes());
            payload.extend_from_slice(&size.rows.to_le_bytes());
        }
        payload.extend_from_slice(bytes);
        Self {
            kind: KIND_NUMBERED,
            payload,
        }
    }

    /// Read a numbered frame back: the viewer it is for, and the item.
    pub fn as_numbered(&self) -> io::Result<(StreamId, Numbered<'_>)> {
        let truncated = || io::Error::other("truncated numbered frame");
        let head = self.payload.get(..NUMBERED_HEADER).ok_or_else(truncated)?;
        let stream = StreamId::from_le_bytes(head[..4].try_into().expect("four bytes"));
        let seq = OutputSeq::from_le_bytes(head[5..13].try_into().expect("eight bytes"));
        let rest = &self.payload[NUMBERED_HEADER..];
        let size = || -> io::Result<(TermSize, &[u8])> {
            let (size, bytes) = rest.split_at_checked(4).ok_or_else(truncated)?;
            Ok((
                TermSize {
                    cols: u16::from_le_bytes([size[0], size[1]]),
                    rows: u16::from_le_bytes([size[2], size[3]]),
                },
                bytes,
            ))
        };
        let item = match head[4] {
            ITEM_SNAPSHOT => {
                let (size, bytes) = size()?;
                Numbered::Snapshot { seq, size, bytes }
            }
            ITEM_OUTPUT => Numbered::Output { seq, bytes: rest },
            ITEM_RESIZED => Numbered::Resized {
                seq,
                size: size()?.0,
            },
            other => return Err(io::Error::other(format!("unknown numbered item {other}"))),
        };
        Ok((stream, item))
    }

    pub fn parse<T: for<'de> Deserialize<'de>>(&self) -> io::Result<T> {
        serde_json::from_slice(&self.payload).map_err(io::Error::other)
    }

    /// Split an output frame back into the viewer it belongs to and its bytes.
    pub fn as_output(&self) -> io::Result<(StreamId, &[u8])> {
        let (head, rest) = self
            .payload
            .split_at_checked(4)
            .ok_or_else(|| io::Error::other("truncated output frame"))?;
        let stream = StreamId::from_le_bytes(head.try_into().expect("four bytes"));
        Ok((stream, rest))
    }

    pub fn encode(&self) -> Vec<u8> {
        let len =
            u32::try_from(self.payload.len() + 1).expect("frames are capped well below 4 GiB");
        let mut out = Vec::with_capacity(4 + 1 + self.payload.len());
        out.extend_from_slice(&len.to_le_bytes());
        out.push(self.kind);
        out.extend_from_slice(&self.payload);
        out
    }

    pub fn write_to(&self, writer: &mut impl Write) -> io::Result<()> {
        writer.write_all(&self.encode())?;
        writer.flush()
    }

    /// Read one whole frame, blocking until it has arrived. A clean end of stream is reported as
    /// `UnexpectedEof`, which is how both sides notice the other has gone.
    pub fn read_from(reader: &mut impl Read) -> io::Result<Self> {
        let mut header = [0u8; 5];
        reader.read_exact(&mut header)?;
        let len = u32::from_le_bytes(header[..4].try_into().expect("four bytes")) as usize;
        if len == 0 {
            return Err(io::Error::other("empty frame"));
        }
        if len > MAX_FRAME {
            return Err(io::Error::other(format!(
                "frame of {len} bytes is too large"
            )));
        }
        let mut payload = vec![0u8; len - 1];
        reader.read_exact(&mut payload)?;
        Ok(Self {
            kind: header[4],
            payload,
        })
    }
}

const ITEM_SNAPSHOT: u8 = 0;
const ITEM_OUTPUT: u8 = 1;
const ITEM_RESIZED: u8 = 2;
/// Stream, item and number, before what each item carries.
const NUMBERED_HEADER: usize = 4 + 1 + 8;

/// An event on its way to every connected client.
pub fn event_frame(event: &HostEvent) -> io::Result<Frame> {
    Frame::json(KIND_EVENT, event)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frames survive a stream that hands them over a few bytes at a time — which is what a
    /// socket does under load, and the classic way a hand-rolled codec goes wrong.
    #[test]
    fn frames_survive_being_read_in_dribs_and_drabs() {
        let frames = [
            Frame::output(7, b"hello \x1b[31mworld"),
            Frame::json(
                KIND_EVENT,
                &HostEvent::Busy {
                    id: SessionId("a".into()),
                },
            )
            .unwrap(),
            Frame::output(0, &[]),
        ];
        let mut wire = Vec::new();
        for frame in &frames {
            wire.extend_from_slice(&frame.encode());
        }

        /// A reader that never gives more than three bytes at a time.
        struct Trickle<'a>(&'a [u8]);
        impl Read for Trickle<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let n = self.0.len().min(buf.len()).min(3);
                buf[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }

        let mut reader = Trickle(&wire);
        for expected in &frames {
            assert_eq!(&Frame::read_from(&mut reader).unwrap(), expected);
        }
        assert_eq!(
            Frame::read_from(&mut reader).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof,
            "the end of the stream is how we notice the other side has gone"
        );
    }

    #[test]
    fn an_absurd_length_is_refused_rather_than_allocated() {
        let mut header = Vec::new();
        header.extend_from_slice(&u32::MAX.to_le_bytes());
        header.push(KIND_OUTPUT);
        let err = Frame::read_from(&mut header.as_slice()).unwrap_err();
        assert!(err.to_string().contains("too large"), "{err}");
    }

    #[test]
    fn output_frames_carry_their_viewer() {
        let frame = Frame::output(42, b"bytes");
        assert_eq!(frame.as_output().unwrap(), (42, &b"bytes"[..]));
        let truncated = Frame {
            kind: KIND_OUTPUT,
            payload: vec![1, 2],
        };
        assert!(truncated.as_output().is_err());
    }

    #[test]
    fn numbered_items_keep_their_place_and_size() {
        let size = TermSize {
            cols: 132,
            rows: 43,
        };
        let items = [
            Numbered::Snapshot {
                seq: 7,
                size,
                bytes: b"\x1bc screen",
            },
            Numbered::Output {
                seq: 8,
                bytes: b"more",
            },
            Numbered::Output { seq: 9, bytes: b"" },
            Numbered::Resized {
                seq: u64::MAX,
                size,
            },
        ];
        for item in items {
            let frame = Frame::numbered(3, &item);
            let read = Frame::read_from(&mut frame.encode().as_slice()).unwrap();
            assert_eq!(read.as_numbered().unwrap(), (3, item));
        }
        for cut in 0..NUMBERED_HEADER + 3 {
            let mut frame = Frame::numbered(3, &items[3]);
            frame.payload.truncate(cut);
            assert!(frame.as_numbered().is_err(), "cut at {cut}");
        }
        let mut unknown = Frame::numbered(3, &items[1]);
        unknown.payload[4] = 9;
        assert!(unknown.as_numbered().is_err());
    }

    /// The errors the app and the remote side branch on must come back as themselves, not as a
    /// string.
    #[test]
    fn host_errors_round_trip() {
        let id = SessionId("s1".into());
        for original in [
            HostError::UnknownSession(id.clone()),
            HostError::SessionExited(id.clone()),
            HostError::Spawn {
                program: "claude".into(),
                reason: "not found".into(),
            },
            HostError::LeaseLost(id.clone()),
            HostError::ControlHeld {
                id: id.clone(),
                by: Controller::Leased {
                    holder: "phone".into(),
                },
            },
            HostError::InputOutOfOrder {
                id: id.clone(),
                expected: 2,
                got: 4,
            },
            HostError::InvalidHolder,
            HostError::ControlUnavailable,
        ] {
            let text = original.to_string();
            let wire = WireError::from(original);
            let decoded: WireError =
                serde_json::from_slice(&serde_json::to_vec(&wire).unwrap()).unwrap();
            assert_eq!(decoded, wire);
            assert_eq!(HostError::from(decoded).to_string(), text);
        }
    }
}
