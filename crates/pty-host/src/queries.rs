//! Terminal queries in a session's output, and the same output without them.
//!
//! A program can ask its terminal things: where the cursor is, what the terminal is, which
//! colours it uses, whether a mode is set. The terminal answers by typing the reply into the
//! program's input. With several viewers attached, every terminal among them would answer, and
//! the program would read the second answer as typed text. So exactly one viewer — the
//! *responder* — is shown the queries, and every other viewer gets the output with them taken
//! out. A query draws nothing, so the screens stay identical.
//!
//! [`Queries`] follows the output with the same state machine as [`crate::unfinished`], and
//! reports each query it sees completed, wherever the batches were cut. It looks only at the
//! terminal protocol, never at what a program printed for a person to read.
//!
//! A query cut across two batches has already gone out half-sent to every viewer when it turns
//! out to be one. A quiet viewer then gets `CAN` in place of the rest, which abandons the open
//! sequence in its parser exactly as if it had never begun. Nothing is held back or delayed.
//!
//! The responder's answer comes back as a write from this computer, which would otherwise take
//! control of the session from anyone holding a lease. [`is_reply`] tells an answer from typing,
//! so that it does not.

use crate::unfinished::{step, State};

/// Longer than any query. A sequence that outgrows this is not one.
const MAX_QUERY: usize = 256;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const DEL: u8 = 0x7f;

/// Follows a session's output, batch by batch, for queries.
#[derive(Debug, Default)]
pub(crate) struct Queries {
    state: State,
    /// The open sequence's bytes, from its `ESC`, without controls carried out inside it.
    open: Vec<u8>,
    overflowed: bool,
    /// Where the open sequence began in the batch being scanned; `None` when it began in an
    /// earlier batch, or when no sequence is open.
    start: Option<usize>,
    /// Controls carried out inside the open sequence, by position in the batch being scanned.
    /// They take effect in the terminal, so a copy without the query must keep them.
    kept: Vec<usize>,
}

/// The queries one batch completed.
#[derive(Debug, Default)]
pub(crate) struct Scan {
    spans: Vec<Span>,
    cursor: usize,
}

/// One query's bytes in a batch.
#[derive(Debug)]
struct Span {
    start: usize,
    end: usize,
    /// It began in an earlier batch, which every viewer has already been sent.
    continued: bool,
    kept: Vec<usize>,
}

impl Scan {
    /// No query ended in this batch: every viewer can have it as it is.
    pub(crate) fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// How many of them asked where the cursor is (`ESC [ 6 n`), the one query the host can
    /// answer itself.
    pub(crate) fn cursor_queries(&self) -> usize {
        self.cursor
    }

    /// `batch` as a viewer that does not answer queries should see it.
    pub(crate) fn without(&self, batch: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(batch.len());
        let mut at = 0;
        for span in &self.spans {
            out.extend_from_slice(&batch[at..span.start]);
            if span.continued {
                out.push(CAN);
            }
            out.extend(span.kept.iter().map(|&k| batch[k]));
            at = span.end;
        }
        out.extend_from_slice(&batch[at..]);
        out
    }
}

impl Queries {
    /// Follow `batch`, the next piece of the session's output, and say which queries it ended.
    pub(crate) fn feed(&mut self, batch: &[u8]) -> Scan {
        let mut scan = Scan::default();
        self.start = None;
        self.kept.clear();
        for (i, &byte) in batch.iter().enumerate() {
            let before = self.state;
            let (next, restart) = step(before, byte);
            self.state = next;
            let ended = match before {
                // `ESC` ends a string as well as starting what follows; xterm.js and `vte` both
                // act on the string there. The `ESC \` that usually follows is harmless on its
                // own, so it is left in place.
                State::String { ends_on_bel } => match byte {
                    BEL if ends_on_bel => Some(i + 1),
                    ESC => Some(i),
                    CAN | SUB => {
                        self.forget();
                        None
                    }
                    _ => {
                        self.push(byte);
                        None
                    }
                },
                State::Escape | State::EscapeIntermediate | State::Csi => {
                    if restart {
                        // Abandoned for whatever comes next: never a query.
                        self.forget();
                        None
                    } else if next == State::Ground {
                        self.push(byte);
                        Some(i + 1)
                    } else if byte < 0x20 {
                        self.kept.push(i);
                        None
                    } else {
                        if byte != DEL {
                            self.push(byte);
                        }
                        None
                    }
                }
                State::Ground | State::Utf8(_) => None,
            };
            if let Some(end) = ended {
                if let Some(kind) = (!self.overflowed).then(|| classify(&self.open)).flatten() {
                    scan.cursor += usize::from(kind == Kind::Cursor);
                    scan.spans.push(Span {
                        start: self.start.unwrap_or(0),
                        end,
                        continued: self.start.is_none(),
                        kept: std::mem::take(&mut self.kept),
                    });
                }
                self.forget();
            }
            if byte == ESC {
                self.open.push(ESC);
                self.start = Some(i);
            }
        }
        scan
    }

    fn push(&mut self, byte: u8) {
        if self.open.len() < MAX_QUERY {
            self.open.push(byte);
        } else {
            self.overflowed = true;
        }
    }

    fn forget(&mut self) {
        self.open.clear();
        self.overflowed = false;
        self.start = None;
        self.kept.clear();
    }
}

/// Whether `input` is nothing but answers to queries — the shapes a terminal replies in, and that
/// no key sends: reports ending `R`, `c`, `n`, `y`, `t`, `x` or `|`, kitty's `CSI ? … u`,
/// XTQMODKEYS's `CSI > … m`, and any OSC or DCS string. Asked only just after a query went out,
/// so a key that happens to share a reply's shape (Shift+F3 is `CSI 1 ; 2 R`) is mistaken for
/// one only then — and is still typed; it just does not count as taking control.
pub(crate) fn is_reply(input: &[u8]) -> bool {
    let mut state = State::Ground;
    let mut sequence = Vec::new();
    let mut replies = 0;
    for &byte in input {
        let before = state;
        let (next, restart) = step(before, byte);
        state = next;
        match before {
            State::Ground | State::Utf8(_) if byte != ESC => return false,
            State::String { ends_on_bel } if byte == ESC || (ends_on_bel && byte == BEL) => {
                replies += 1;
                sequence.clear();
            }
            State::Escape | State::EscapeIntermediate | State::Csi if restart => return false,
            State::Escape | State::EscapeIntermediate | State::Csi if next == State::Ground => {
                sequence.push(byte);
                if !answer(&sequence) {
                    return false;
                }
                replies += 1;
                sequence.clear();
            }
            _ => sequence.push(byte),
        }
        if byte == ESC {
            sequence.clear();
            sequence.push(ESC);
        }
    }
    // Everything ended, bar the `ESC \` that closes a string.
    replies > 0 && (state == State::Ground || sequence == [ESC])
}

/// A whole escape or control sequence, from its `ESC`, in the shape of an answer. A string's
/// closing `ESC \` counts as part of the string before it.
fn answer(sequence: &[u8]) -> bool {
    match sequence {
        [ESC, b'\\'] => true,
        [ESC, b'[', body @ ..] => {
            let Some((&last, rest)) = body.split_last() else {
                return false;
            };
            match last {
                b'R' | b'c' | b'n' | b'y' | b't' | b'x' | b'|' => true,
                b'u' => rest.first() == Some(&b'?'),
                b'm' => rest.first() == Some(&b'>'),
                _ => false,
            }
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `ESC [ 6 n`, where is the cursor.
    Cursor,
    Other,
}

/// Whether a whole sequence, from its `ESC`, asks the terminal for an answer. The list is
/// xterm's (its "ctlseqs"), plus the kitty keyboard and colour queries.
fn classify(sequence: &[u8]) -> Option<Kind> {
    let (&introducer, body) = sequence.get(1..)?.split_first()?;
    let query = match introducer {
        b'[' => return csi(body),
        // OSC: colours (4, 5, 10–19, kitty's 21) and the clipboard (52), when asked with `?`.
        b']' => {
            let mut fields = body.split(|&b| b == b';');
            let command = fields.next()?;
            let asks = |field: &[u8]| field == b"?" || field.ends_with(b"=?");
            matches!(
                command,
                b"4" | b"5"
                    | b"10"
                    | b"11"
                    | b"12"
                    | b"13"
                    | b"14"
                    | b"15"
                    | b"16"
                    | b"17"
                    | b"18"
                    | b"19"
                    | b"21"
                    | b"52"
            ) && fields.any(asks)
        }
        // DCS: request a setting (DECRQSS) or a terminfo capability (XTGETTCAP).
        b'P' => body.starts_with(b"$q") || body.starts_with(b"+q"),
        // DECID, an old way of asking what the terminal is.
        b'Z' => body.is_empty(),
        _ => false,
    };
    query.then_some(Kind::Other)
}

/// A control sequence, after `ESC [`: whether it asks for an answer.
fn csi(body: &[u8]) -> Option<Kind> {
    let (&last, rest) = body.split_last()?;
    let (prefix, rest) = match rest.first() {
        Some(&p @ (b'<' | b'=' | b'>' | b'?')) => (Some(p), &rest[1..]),
        _ => (None, rest),
    };
    let split = rest
        .iter()
        .position(|b| (0x20..=0x2f).contains(b))
        .unwrap_or(rest.len());
    let (params, intermediates) = rest.split_at(split);
    let first = params.split(|&b| b == b';').next().unwrap_or_default();
    let query = match (prefix, intermediates, last) {
        (None, b"", b'n') if params == b"6" => return Some(Kind::Cursor),
        // Device status (5, 6, and DEC's own), and device attributes, all three kinds.
        (None | Some(b'?'), b"", b'n') => true,
        (None | Some(b'>' | b'='), b"", b'c') => true,
        // DECRQM, whether a mode is set.
        (None | Some(b'?'), b"$", b'p') => true,
        // Window reports: state, position, sizes, titles. The rest of `t` changes the window.
        (None, b"", b't') => matches!(
            first,
            b"11" | b"13" | b"14" | b"15" | b"16" | b"18" | b"19" | b"20" | b"21"
        ),
        // XTVERSION; kitty's keyboard flags; XTQMODKEYS; XTSMGRAPHICS.
        (Some(b'>'), b"", b'q') | (Some(b'?'), b"", b'u' | b'm' | b'S') => true,
        // DECREQTPARM, DECRQTSR, DECRQUPSS, DECRQPSR, DECRQCRA, XTREPORTSGR.
        (None, b"", b'x') => true,
        (None, b"$", b'u' | b'w') | (None, b"&", b'u') | (None, b"*", b'y') => true,
        (None, b"#", b'|') => true,
        _ => false,
    };
    query.then_some(Kind::Other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(bytes: &[u8]) -> (Vec<u8>, usize, usize) {
        let scan = Queries::default().feed(bytes);
        (scan.without(bytes), scan.spans.len(), scan.cursor_queries())
    }

    #[test]
    fn queries_are_told_from_everything_else() {
        let queries: &[&[u8]] = &[
            b"\x1b[6n",
            b"\x1b[5n",
            b"\x1b[?6n",
            b"\x1b[?996n",
            b"\x1b[c",
            b"\x1b[0c",
            b"\x1b[>c",
            b"\x1b[>0c",
            b"\x1b[=c",
            b"\x1b[?2004$p",
            b"\x1b[4$p",
            b"\x1b[18t",
            b"\x1b[14;2t",
            b"\x1b[>q",
            b"\x1b[>0q",
            b"\x1b[?u",
            b"\x1b[?4m",
            b"\x1b[?1;1;0S",
            b"\x1b[x",
            b"\x1b[1$w",
            b"\x1b[1;1;2;2#|",
            b"\x1b]11;?\x07",
            b"\x1b]10;?\x1b\\",
            b"\x1b]4;1;?\x07",
            b"\x1b]52;c;?\x07",
            b"\x1b]21;foreground=?\x1b\\",
            b"\x1bP$qm\x1b\\",
            b"\x1bP+q544e\x1b\\",
            b"\x1bZ",
        ];
        for query in queries {
            let (quiet, found, _) = scan(query);
            assert_eq!(found, 1, "{:?} is a query", String::from_utf8_lossy(query));
            // An ST after a string is left behind: on its own it does nothing.
            assert!(
                quiet.is_empty() || quiet == b"\x1b\\",
                "{:?} left {:?}",
                String::from_utf8_lossy(query),
                String::from_utf8_lossy(&quiet)
            );
        }
        let others: &[&[u8]] = &[
            b"\x1b[31m",
            b"\x1b[6m",
            b"\x1b[>4;1m",
            b"\x1b[>1u",
            b"\x1b[<u",
            b"\x1b[=1;1u",
            b"\x1b[!p",
            b"\x1b[2 q",
            b"\x1b[8;24;80t",
            b"\x1b[22;0t",
            b"\x1b[>0n",
            b"\x1b[?25h",
            b"\x1b[s",
            b"\x1b]0;?\x07",
            b"\x1b]8;;https://example.com/?\x1b\\",
            b"\x1b]11;#000000\x07",
            b"\x1b]52;c;aGVsbG8=\x07",
            b"\x1bP+p544e\x1b\\",
            b"\x1b_Gf=24;AAAA\x1b\\",
            b"\x1b7",
            b"\x1b(B",
            b"plain text ?\x07",
        ];
        for other in others {
            let (quiet, found, _) = scan(other);
            assert_eq!(found, 0, "{:?} is no query", String::from_utf8_lossy(other));
            assert_eq!(&quiet, other);
        }
    }

    #[test]
    fn answers_are_told_from_typing() {
        let answers: &[&[u8]] = &[
            b"\x1b[24;80R",
            b"\x1b[?1;2c",
            b"\x1b[>0;276;0c",
            b"\x1b[0n",
            b"\x1b[?2004;2$y",
            b"\x1b[8;24;80t",
            b"\x1b[?0u",
            b"\x1b[>4;2m",
            b"\x1b]11;rgb:0000/0000/0000\x1b\\",
            b"\x1b]10;rgb:ffff/ffff/ffff\x07",
            b"\x1bP1$r0m\x1b\\",
            b"\x1bP>|xterm.js(5.5.0)\x1b\\",
            b"\x1b[1;1R\x1b]11;rgb:0000/0000/0000\x07\x1b[?62;22c",
        ];
        for answer in answers {
            assert!(is_reply(answer), "{:?}", String::from_utf8_lossy(answer));
        }
        let typing: &[&[u8]] = &[
            b"",
            b"a",
            b"\r",
            b"\x1b",
            b"\x1b[A",
            b"\x1b[1;5D",
            b"\x1b[3~",
            b"\x1bOP",
            b"\x1b[97;5u",
            b"\x1b[200~text\x1b[201~",
            b"\x1b[24;80Rls\r",
            b"\x1b[24;80",
            b"\x1b]11;rgb:0000/0000/0000",
        ];
        for keys in typing {
            assert!(!is_reply(keys), "{:?}", String::from_utf8_lossy(keys));
        }
    }

    #[test]
    fn only_the_cursor_question_is_counted_for_the_host_to_answer() {
        let (_, found, cursor) = scan(b"\x1b[6n\x1b[c\x1b[?6n\x1b]11;?\x07\x1b[6n");
        assert_eq!((found, cursor), (5, 2));
    }

    #[test]
    fn controls_carried_out_inside_a_query_stay() {
        let (quiet, found, cursor) = scan(b"a\x1b[6\nn\rb");
        assert_eq!((found, cursor), (1, 1));
        assert_eq!(quiet, b"a\n\rb");
    }

    #[test]
    fn a_query_too_long_to_be_one_is_left_alone() {
        let mut long = b"\x1b]11;".to_vec();
        long.extend(std::iter::repeat_n(b'x', MAX_QUERY));
        long.extend_from_slice(b";?\x07");
        let (quiet, found, _) = scan(&long);
        assert_eq!(found, 0);
        assert_eq!(quiet, long);
    }

    #[test]
    fn an_abandoned_sequence_is_no_query() {
        for bytes in [&b"\x1b[6\x1b[m"[..], b"\x1b[6\x18n", b"\x1b]11;?\x18"] {
            let (quiet, found, _) = scan(bytes);
            assert_eq!(found, 0, "{:?}", String::from_utf8_lossy(bytes));
            assert_eq!(quiet, bytes);
        }
    }

    /// What it is all for: cut the output anywhere, and a viewer sent the copies without the
    /// queries draws exactly what a viewer sent everything draws, and is never asked anything.
    #[test]
    fn wherever_the_output_is_cut_the_quiet_copy_draws_the_same_and_asks_nothing() {
        let stream: &[u8] = b"\x1b[?1049h\x1b[2J\x1b[H\x1b[38;2;10;20;30mcaf\xc3\xa9\x1b[m \
            \x1b[6n\x1b]11;?\x1b\\\x1b[1;5Hmid\x1b[c\x1b]0;title\x07\
            \x1b[3\n1m red\x1b[m\x1bP$qm\x1b\\\x1b[?2004$p tail \xf0\x9f\x98\x80\x1b[6n\r\nend";
        let mut whole = vt100::Parser::new(10, 40, 0);
        whole.process(stream);
        let total = Queries::default().feed(stream);
        assert_eq!((total.spans.len(), total.cursor_queries()), (6, 2));

        for cut in 0..=stream.len() {
            let (first, second) = stream.split_at(cut);
            let mut queries = Queries::default();
            let (a, b) = (queries.feed(first), queries.feed(second));
            assert_eq!(
                a.cursor_queries() + b.cursor_queries(),
                2,
                "cut at {cut}: every cursor question counted once"
            );
            let mut quiet = a.without(first);
            quiet.extend(b.without(second));

            let mut viewer = vt100::Parser::new(10, 40, 0);
            viewer.process(&quiet);
            assert_eq!(
                viewer.screen().contents_formatted(),
                whole.screen().contents_formatted(),
                "cut at {cut}"
            );
            assert_eq!(
                viewer.screen().cursor_position(),
                whole.screen().cursor_position(),
                "cut at {cut}"
            );
            assert!(
                Queries::default().feed(&quiet).is_empty(),
                "cut at {cut}: {:?}",
                String::from_utf8_lossy(&quiet)
            );
        }
    }
}
