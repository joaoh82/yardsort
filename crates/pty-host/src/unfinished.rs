//! The bytes of a sequence a program has begun printing and not finished.
//!
//! Output arrives in chunks cut wherever a read happened to end, so a batch can stop halfway
//! through an escape sequence (`ESC [ 3` … `1 m`) or a character (the first byte of `é`). The
//! headless terminal holds that half in its parser, but a snapshot is rendered from the screen,
//! not the parser, so the half is not in it. A viewer that attached at that moment would then
//! receive the second half on its own and draw it as text — `1mred` instead of red `red` — or
//! drop a character outright.
//!
//! [`Unfinished`] follows the same state machine as the parser (the DEC ANSI parser that `vte`
//! implements) closely enough to know, after each byte, whether a sequence is still open, and
//! keeps that sequence's bytes. A snapshot ends with them, so the next batch completes the
//! sequence in the viewer exactly as it does in the host. It only scans bytes; nothing is held
//! back from viewers and nothing is delayed.

/// More than any sequence a terminal program prints in practice, short of an image or a
/// clipboard copy. A string longer than this keeps only its introducer: the viewer then reads
/// the rest of it as a (truncated) string, rather than printing it as text.
const MAX_UNFINISHED: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    /// Inside a UTF-8 character, with this many continuation bytes still to come.
    Utf8(u8),
    /// After `ESC`.
    Escape,
    /// After `ESC` and one or more intermediate bytes.
    EscapeIntermediate,
    /// `ESC [`, until its final byte.
    Csi,
    /// `ESC ]` (`ends_on_bel`), or `ESC P`, `ESC X`, `ESC ^`, `ESC _`: until `ESC \`.
    String {
        ends_on_bel: bool,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct Unfinished {
    state: State,
    bytes: Vec<u8>,
    /// The open sequence outgrew [`MAX_UNFINISHED`]; only its introducer is kept.
    overflowed: bool,
}

impl Default for Unfinished {
    fn default() -> Self {
        Self {
            state: State::Ground,
            bytes: Vec::new(),
            overflowed: false,
        }
    }
}

impl Unfinished {
    /// Follow `data`, which the headless terminal has just been given too.
    pub(crate) fn feed(&mut self, data: &[u8]) {
        for &byte in data {
            let (next, restart) = step(self.state, byte);
            self.state = next;
            if next == State::Ground || restart {
                self.bytes.clear();
                self.overflowed = false;
                if next == State::Ground {
                    continue;
                }
            }
            if self.overflowed {
                continue;
            }
            if self.bytes.len() < MAX_UNFINISHED {
                self.bytes.push(byte);
            } else {
                // Keep the introducer only, from here to the end of the sequence.
                self.bytes.truncate(2);
                self.overflowed = true;
            }
        }
    }

    /// The open sequence's bytes; empty between sequences.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Whether what is open is a character, rather than an escape sequence or a string.
    pub(crate) fn in_character(&self) -> bool {
        matches!(self.state, State::Utf8(_))
    }
}

const ESC: u8 = 0x1b;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const BEL: u8 = 0x07;

/// The state after `byte`, and whether `byte` begins a new sequence (abandoning any open one).
fn step(state: State, byte: u8) -> (State, bool) {
    // From anywhere: ESC starts a sequence, ending whatever was open; CAN and SUB cancel.
    match byte {
        ESC => return (State::Escape, true),
        CAN | SUB => return (State::Ground, true),
        _ => {}
    }
    let next = match state {
        State::Ground => return (ground(byte), true),
        State::Utf8(left) => match byte {
            0x80..=0xbf if left > 1 => State::Utf8(left - 1),
            0x80..=0xbf => State::Ground,
            // A broken character: the parser shows a replacement for it, then reads this byte
            // afresh.
            _ => return (ground(byte), true),
        },
        State::Escape => match byte {
            0x20..=0x2f => State::EscapeIntermediate,
            b'[' => State::Csi,
            b']' => State::String { ends_on_bel: true },
            b'P' | b'X' | b'^' | b'_' => State::String { ends_on_bel: false },
            0x30..=0x7e => State::Ground,
            // C0 controls are carried out without leaving the sequence; the rest is ignored.
            _ => State::Escape,
        },
        State::EscapeIntermediate => match byte {
            0x30..=0x7e => State::Ground,
            _ => State::EscapeIntermediate,
        },
        State::Csi => match byte {
            0x40..=0x7e => State::Ground,
            _ => State::Csi,
        },
        State::String { ends_on_bel } => match byte {
            BEL if ends_on_bel => State::Ground,
            _ => state,
        },
    };
    (next, false)
}

/// What a byte starts when no sequence is open.
fn ground(byte: u8) -> State {
    match byte {
        0xc2..=0xdf => State::Utf8(1),
        0xe0..=0xef => State::Utf8(2),
        0xf0..=0xf4 => State::Utf8(3),
        _ => State::Ground,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_kept_between_sequences() {
        let mut unfinished = Unfinished::default();
        unfinished.feed(b"text \x1b[31mred\x1b[m \xc3\xa9");
        assert!(unfinished.bytes().is_empty());
        unfinished.feed(b"\x1b[3");
        assert_eq!(unfinished.bytes(), b"\x1b[3");
        assert!(!unfinished.in_character());
        unfinished.feed(b"1m");
        assert!(unfinished.bytes().is_empty());
        unfinished.feed(&"\u{1f600}".as_bytes()[..2]);
        assert_eq!(unfinished.bytes(), &"\u{1f600}".as_bytes()[..2]);
        assert!(unfinished.in_character());
        // An escape abandons the open sequence, and starts its own.
        unfinished.feed(b"\x1b[3\x1b]0;ti");
        assert_eq!(unfinished.bytes(), b"\x1b]0;ti");
    }

    #[test]
    fn a_long_string_keeps_its_introducer() {
        let mut copy = b"\x1b]52;c;".to_vec();
        copy.extend(std::iter::repeat_n(b'A', MAX_UNFINISHED * 2));
        let mut unfinished = Unfinished::default();
        unfinished.feed(&copy);
        assert_eq!(unfinished.bytes(), b"\x1b]");
        unfinished.feed(b"\x07");
        assert!(unfinished.bytes().is_empty());
    }
}
