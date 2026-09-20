//! What crosses the socket, and how it is framed.
//!
//! Every frame is `[u32 length][u8 kind][payload]`, little-endian. Control traffic — requests,
//! responses, events — is JSON, reusing the `serde` implementations the host types already
//! carry. Terminal output is not: it gets a frame kind of its own and travels as raw bytes, the
//! same bytes the webview hands to xterm.js, so a busy agent does not pay for JSON on every
//! repaint.

use std::io::{self, Read, Write};

use pty_host::{HostError, HostEvent, LaunchPlan, SessionId, SessionInfo, TermSize};
use serde::{Deserialize, Serialize};

/// The language this build speaks. Bumped whenever a frame or message changes shape, and
/// **independently of the app version**: most releases leave it alone, and a daemon holding live
/// agents is only in the way when it really cannot be talked to.
pub const PROTOCOL: u32 = 1;

pub const KIND_REQUEST: u8 = 1;
pub const KIND_RESPONSE: u8 = 2;
pub const KIND_EVENT: u8 = 3;
pub const KIND_OUTPUT: u8 = 4;

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
    UnknownSession { id: SessionId },
    SessionExited { id: SessionId },
    OpenPty { message: String },
    Spawn { program: String, reason: String },
    Io { message: String },
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

    /// The two errors the app branches on must come back as themselves, not as a string.
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
