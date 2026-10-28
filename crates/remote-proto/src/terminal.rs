//! Terminal attachment and control: the schema, not yet the service.
//!
//! **Nothing here is answered by a host yet.** A host does not advertise these operations, and
//! asking for one gets `unsupported`. They are defined now so the read side and the terminal side
//! are one coherent protocol, and so the hard rules are written into types before anyone builds
//! on a looser shape:
//!
//! - **Many viewers, one controller.** Attaching is viewing. Typing and resizing need a control
//!   lease, held by one client at a time, the desktop included. A viewer never resizes the
//!   program; it renders at the host's size.
//! - **Every control frame names its lease generation.** Reclaiming control — which the person
//!   at the host can always do at once — bumps the generation, so input from the previous holder
//!   is refused even if its socket is still open.
//! - **Input is never replayed.** Each input carries a sequence number within its lease. The host
//!   drops duplicates and refuses gaps. After a reconnect, a client does not resend typing it
//!   was unsure about: it shows the person that delivery is uncertain, and the lease it had is
//!   gone anyway.
//! - **One terminal answers terminal queries.** The host shows "where is the cursor?" and its kin
//!   to one terminal only: the one attached last at the host, or, while a device controls, none,
//!   and it answers the cursor itself. A remote viewer is sent the output without them, and its
//!   renderer must not answer anyway.
//! - **A snapshot is a boundary.** An attachment begins with a snapshot that is exactly the
//!   screen after output `snapshot_seq`, then output `snapshot_seq + 1` onward, with nothing lost
//!   or doubled. The PTY host already makes this atomic for local viewers; sequence numbers are
//!   what make it checkable across a network.
//! - **Epochs, not guesses.** Sequence numbers count within an epoch. A new epoch (the daemon
//!   restarted, the replay window was lost, a client fell too far behind) means a fresh snapshot
//!   that *replaces* the screen — never one appended to an old screen.

use serde::{Deserialize, Serialize};

use crate::ids::{DeviceId, LeaseId, TerminalId};
use crate::model::TermSize;

/// One attachment's output stream on one connection. Chosen by the host.
pub type StreamId = u32;

/// A point in a terminal's output: the `seq`th chunk of `epoch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamPosition {
    pub epoch: u32,
    pub seq: u64,
}

/// `terminal.attach`: start viewing. Needs `terminal:read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachRequest {
    pub terminal: TerminalId,
    /// Where a previous attachment got to. The host resumes from there when it still can, and
    /// otherwise sends a fresh snapshot; the reply says which.
    #[serde(default)]
    pub resume_after: Option<StreamPosition>,
}

/// How an attachment begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AttachStart {
    /// A snapshot follows: clear the view and draw it.
    Snapshot,
    /// Output continues right after the position the client gave.
    Resumed,
}

/// Who may type into a terminal right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Controller {
    /// Nobody has control: the next to ask gets it.
    Nobody,
    /// The person at the host, in the desktop app or `ys attach`.
    Local,
    /// An enrolled device.
    Device { device: DeviceId, name: String },
}

/// Reply to `terminal.attach`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attached {
    pub stream: StreamId,
    pub start: AttachStart,
    /// The snapshot (or the resumed stream) covers output up to and including this position.
    /// The first output frame after it has `seq + 1` in the same epoch.
    pub position: StreamPosition,
    pub size: TermSize,
    pub controller: Controller,
}

/// `terminal.detach`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachRequest {
    pub terminal: TerminalId,
    pub stream: StreamId,
}

/// `terminal.acquire`: ask for control. Needs `terminal:control`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquireRequest {
    pub terminal: TerminalId,
    /// Take control from another device that holds it. Without this, a held terminal answers
    /// `control_conflict`. A device can never take control from `Local` this way: the person at
    /// the host releases it, or it lapses.
    #[serde(default)]
    pub take_over: bool,
}

/// A control lease. Renew it before `ttl_ms` runs out or it lapses on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub id: LeaseId,
    /// Increases every time control changes hands on this terminal.
    pub generation: u64,
    pub ttl_ms: u32,
    /// How often the holder should renew.
    pub renew_every_ms: u32,
}

/// Names a lease in every request that needs one: `terminal.renew`, `terminal.release`,
/// `terminal.input`, `terminal.resize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseRef {
    pub terminal: TerminalId,
    pub lease: LeaseId,
    pub generation: u64,
}

/// `terminal.input`: type into the terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputRequest {
    #[serde(flatten)]
    pub lease: LeaseRef,
    /// 1 for the first input under this lease, then one more each time. A repeat is dropped; a
    /// gap is refused.
    pub seq: u64,
    /// What the terminal renderer produced: keystrokes, or text to paste. At most
    /// [`crate::MAX_INPUT_BYTES`].
    pub data: String,
    /// Send as a paste — bracketed, if the program asked for that.
    #[serde(default)]
    pub paste: bool,
}

/// `terminal.resize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeRequest {
    #[serde(flatten)]
    pub lease: LeaseRef,
    pub size: TermSize,
}

/// Why an output frame could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("the frame is shorter than its header")]
    Truncated,
    #[error("not a terminal output frame")]
    WrongKind,
    #[error("unknown output frame version {0}")]
    UnknownVersion(u8),
    #[error("the frame carries {0} bytes, more than the limit")]
    TooLarge(usize),
}

/// Terminal output on the wire: a binary frame, never JSON.
///
/// ```text
/// offset  size  field
///      0     1  kind       = 0x01 (terminal output)
///      1     1  version    = 0x01
///      2     2  reserved   = 0, ignored
///      4     4  stream     u32, little-endian
///      8     4  epoch      u32, little-endian
///     12     8  seq        u64, little-endian
///     20     …  bytes      at most MAX_OUTPUT_CHUNK
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputFrame<'a> {
    pub stream: StreamId,
    pub position: StreamPosition,
    pub bytes: &'a [u8],
}

const OUTPUT_KIND: u8 = 0x01;
const OUTPUT_VERSION: u8 = 0x01;
/// Bytes before an output frame's payload.
pub const OUTPUT_HEADER_LEN: usize = 20;

impl<'a> OutputFrame<'a> {
    /// Append the frame to `out`. A chunk over [`crate::MAX_OUTPUT_CHUNK`] is refused: split it.
    pub fn encode(&self, out: &mut Vec<u8>) -> Result<(), FrameError> {
        if self.bytes.len() > crate::MAX_OUTPUT_CHUNK {
            return Err(FrameError::TooLarge(self.bytes.len()));
        }
        out.reserve(OUTPUT_HEADER_LEN + self.bytes.len());
        out.extend_from_slice(&[OUTPUT_KIND, OUTPUT_VERSION, 0, 0]);
        out.extend_from_slice(&self.stream.to_le_bytes());
        out.extend_from_slice(&self.position.epoch.to_le_bytes());
        out.extend_from_slice(&self.position.seq.to_le_bytes());
        out.extend_from_slice(self.bytes);
        Ok(())
    }

    /// Read a frame that arrived whole (one WebSocket binary message). Checks the size before
    /// anything else, so an oversized frame costs nothing to refuse.
    pub fn decode(frame: &'a [u8]) -> Result<Self, FrameError> {
        if frame.len() > OUTPUT_HEADER_LEN + crate::MAX_OUTPUT_CHUNK {
            return Err(FrameError::TooLarge(frame.len() - OUTPUT_HEADER_LEN));
        }
        if frame.len() < OUTPUT_HEADER_LEN {
            return Err(FrameError::Truncated);
        }
        if frame[0] != OUTPUT_KIND {
            return Err(FrameError::WrongKind);
        }
        if frame[1] != OUTPUT_VERSION {
            return Err(FrameError::UnknownVersion(frame[1]));
        }
        let u32_at = |at: usize| u32::from_le_bytes(frame[at..at + 4].try_into().unwrap());
        Ok(Self {
            stream: u32_at(4),
            position: StreamPosition {
                epoch: u32_at(8),
                seq: u64::from_le_bytes(frame[12..20].try_into().unwrap()),
            },
            bytes: &frame[OUTPUT_HEADER_LEN..],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_output_frame_round_trips_raw_bytes() {
        // Escape sequences and invalid UTF-8 alike: terminal bytes are opaque.
        let bytes = b"\x1b[2J\x1b[?1049h\xff\xfeok";
        let frame = OutputFrame {
            stream: 7,
            position: StreamPosition {
                epoch: 3,
                seq: u64::MAX - 1,
            },
            bytes,
        };
        let mut wire = Vec::new();
        frame.encode(&mut wire).unwrap();
        assert_eq!(wire.len(), OUTPUT_HEADER_LEN + bytes.len());
        assert_eq!(OutputFrame::decode(&wire).unwrap(), frame);
    }

    #[test]
    fn the_header_layout_is_fixed() {
        let mut wire = Vec::new();
        OutputFrame {
            stream: 0x0102_0304,
            position: StreamPosition { epoch: 5, seq: 6 },
            bytes: b"x",
        }
        .encode(&mut wire)
        .unwrap();
        assert_eq!(
            wire,
            [
                1, 1, 0, 0, // kind, version, reserved
                4, 3, 2, 1, // stream
                5, 0, 0, 0, // epoch
                6, 0, 0, 0, 0, 0, 0, 0, // seq
                b'x'
            ]
        );
    }

    #[test]
    fn short_foreign_and_oversized_frames_are_refused() {
        assert_eq!(OutputFrame::decode(&[1, 1, 0]), Err(FrameError::Truncated));
        let mut wire = Vec::new();
        OutputFrame {
            stream: 1,
            position: StreamPosition { epoch: 0, seq: 0 },
            bytes: b"",
        }
        .encode(&mut wire)
        .unwrap();
        let mut foreign = wire.clone();
        foreign[0] = 9;
        assert_eq!(OutputFrame::decode(&foreign), Err(FrameError::WrongKind));
        let mut future = wire.clone();
        future[1] = 2;
        assert_eq!(
            OutputFrame::decode(&future),
            Err(FrameError::UnknownVersion(2))
        );

        let big = vec![0u8; crate::MAX_OUTPUT_CHUNK + 1];
        let frame = OutputFrame {
            stream: 1,
            position: StreamPosition { epoch: 0, seq: 0 },
            bytes: &big,
        };
        assert!(matches!(
            frame.encode(&mut Vec::new()),
            Err(FrameError::TooLarge(_))
        ));
        let mut oversized = wire;
        oversized.extend_from_slice(&big);
        assert!(matches!(
            OutputFrame::decode(&oversized),
            Err(FrameError::TooLarge(_))
        ));
    }

    #[test]
    fn input_names_its_lease_on_the_wire() {
        let input: InputRequest = serde_json::from_str(
            r#"{"terminal":"t1","lease":"l1","generation":4,"seq":1,"data":"y\r"}"#,
        )
        .unwrap();
        assert_eq!(input.lease.generation, 4);
        assert!(!input.paste);
        // No lease, no input: the fields are required, not defaulted.
        let missing: Result<InputRequest, _> =
            serde_json::from_str(r#"{"terminal":"t1","seq":1,"data":"y"}"#);
        assert!(missing.is_err());
    }
}
