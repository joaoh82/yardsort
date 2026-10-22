//! Rebuilding a terminal's screen for a viewer that attaches late.
//!
//! Replaying raw bytes is not enough: a full-screen program may have entered the alternate screen
//! megabytes ago. Instead the host keeps a headless terminal and serialises its *state* — a byte
//! sequence that, written to a fresh emulator of the same size, reproduces scrollback, screen,
//! cursor and input modes.

use crate::unfinished::Unfinished;

/// The terminal a session keeps for snapshots: a `vt100` parser fed everything the program prints,
/// and what it takes to cut a snapshot between any two bytes.
///
/// Batches end wherever a read ended, so the cut can fall inside a sequence. Two things make that
/// safe. A snapshot ends with the open sequence's raw bytes, so the next batch completes it in
/// the viewer (see [`Unfinished`]). And a character the batch ends inside is held back from the
/// parser and handed to it with the next batch, because `vt100` 0.16 drops the byte after a
/// character split across two `process` calls when more multibyte text follows. The viewers'
/// own stream is never held back.
pub(crate) struct Headless {
    parser: vt100::Parser,
    unfinished: Unfinished,
    /// The start of a character the last batch ended inside, not yet given to the parser.
    held: Vec<u8>,
}

impl Headless {
    pub(crate) fn new(rows: u16, cols: u16, scrollback: usize) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, scrollback),
            unfinished: Unfinished::default(),
            held: Vec::new(),
        }
    }

    /// Take in a batch of the program's output.
    pub(crate) fn feed(&mut self, batch: &[u8]) {
        self.unfinished.feed(batch);
        let mut input = std::mem::take(&mut self.held);
        input.extend_from_slice(batch);
        if self.unfinished.in_character() {
            let cut = input.len().saturating_sub(self.unfinished.bytes().len());
            self.held = input.split_off(cut);
        }
        self.parser.process(&input);
    }

    /// What a viewer that attaches now is sent first: scrollback, screen, modes, and the open
    /// sequence's bytes. Leaves the scrollback view at the bottom.
    pub(crate) fn snapshot(&mut self) -> Vec<u8> {
        let mut out = render(&mut self.parser);
        out.extend_from_slice(self.unfinished.bytes());
        out
    }

    pub(crate) fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub(crate) fn set_size(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows, cols);
    }
}

/// Serialise `parser`'s scrollback and screen. Leaves the parser's scrollback view at the bottom.
pub(crate) fn render(parser: &mut vt100::Parser) -> Vec<u8> {
    let (rows, cols) = parser.screen().size();
    let mut out = Vec::new();

    // Start from a known state, whatever the viewer showed before.
    out.extend_from_slice(b"\x1bc");

    // Scrollback exists only on the primary screen.
    if !parser.screen().alternate_screen() {
        let history = scrollback_rows(parser, rows, cols);
        if !history.is_empty() {
            for row in &history {
                out.extend_from_slice(row);
                out.extend_from_slice(b"\x1b[m\r\n");
            }
            // Push every history line off the visible area and into the viewer's own
            // scrollback, because the screen state below begins by clearing the display.
            out.extend(std::iter::repeat_n(b'\n', usize::from(rows)));
        }
    }

    if parser.screen().alternate_screen() {
        out.extend_from_slice(b"\x1b[?1049h");
    }
    out.extend_from_slice(&parser.screen().state_formatted());
    out
}

/// Read a snapshot back as plain text: history first, then the screen, with the colours and
/// cursor positioning dropped.
///
/// Replayed through a real terminal rather than stripped with a pattern, because a snapshot is a
/// *state* — it positions the cursor and clears regions to rebuild a screen, so the bytes are not
/// the text in order. `size` must be the session's own, or the replay wraps differently than the
/// program drew.
pub fn to_text(snapshot: &[u8], rows: u16, cols: u16) -> String {
    let mut viewer = vt100::Parser::new(rows, cols, SCROLLBACK_FOR_READING);
    viewer.process(snapshot);

    // `render` pushes history off the screen with a run of newlines, so the blank lines that run
    // creates land in the scrollback behind it. They are an artefact of the transport, not
    // something the program printed.
    let mut history = read_scrollback(&mut viewer, rows, cols);
    while history.last().is_some_and(|line| line.trim().is_empty()) {
        history.pop();
    }

    let mut lines = history;
    lines.extend(viewer.screen().rows(0, cols));
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Enough to hold anything a session is allowed to keep. See `session::SCROLLBACK_LINES`.
const SCROLLBACK_FOR_READING: usize = 10_000;

/// The replayed viewer's history as plain text, oldest first.
fn read_scrollback(viewer: &mut vt100::Parser, rows: u16, cols: u16) -> Vec<String> {
    let page = usize::from(rows);
    viewer.screen_mut().set_scrollback(usize::MAX);
    let total = viewer.screen().scrollback();

    let mut lines = Vec::with_capacity(total);
    let mut offset = total;
    while offset > 0 {
        viewer.screen_mut().set_scrollback(offset);
        let take = offset.min(page);
        lines.extend(viewer.screen().rows(0, cols).take(take));
        offset -= take;
    }
    viewer.screen_mut().set_scrollback(0);
    lines
}

/// Every scrollback line, oldest first, each as formatted bytes without a line terminator.
fn scrollback_rows(parser: &mut vt100::Parser, rows: u16, cols: u16) -> Vec<Vec<u8>> {
    let page = usize::from(rows);
    // `set_scrollback` clamps to what is available, which is how we learn the length.
    parser.screen_mut().set_scrollback(usize::MAX);
    let total = parser.screen().scrollback();

    let mut history = Vec::with_capacity(total);
    let mut offset = total;
    while offset > 0 {
        // With the view scrolled back by `offset`, its first `min(offset, page)` rows are history.
        parser.screen_mut().set_scrollback(offset);
        let take = offset.min(page);
        history.extend(parser.screen().rows_formatted(0, cols).take(take));
        offset -= take;
    }
    parser.screen_mut().set_scrollback(0);
    history
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Output that exercises every kind of sequence, and text around them. Not scroll regions
    /// or a saved cursor: `state_formatted` carries neither, so no snapshot does, wherever it
    /// is cut.
    const CORPUS: &[&[u8]] = &[
        b"plain \x1b[31mred\x1b[m and \x1b[1;38;2;10;20;30mtrue colour\x1b[0m\r\n",
        "caf\u{e9} \u{2603} snow \u{1f600} face\r\n".as_bytes(),
        b"\x1b]0;a window title\x07after the title\r\n",
        b"\x1b]8;;https://example.invalid/\x1b\\a link\x1b]8;;\x1b\\\r\n",
        b"\x1b[?1049h\x1b[2J\x1b[Hfull screen\x1b[5;3Hhere\x1b[?25l",
        b"\x1b(0lqk\x1b(B box\r\n\x1bPq#0;2;0;0;0\x1b\\after dcs",
        b"\x1b[3\x18cancelled\x1b[32\x1b[33myellow\x1b[m\r\n",
        b"\x1b[4;1Hmoved\x1b[Kerased\x1b[1Aup\x1b[2@in\x1b[3Pdel\x1b[?2004h\x1b[5 q",
        // Controls inside a sequence are carried out at once, and the sequence goes on.
        b"abc\x1b[31\nmRED\x1b[1\t;4mtab\x08\x1b[\x08\x0832mback\x1b(\r0lqk\x1b(B\x1b\n=keypad\x1b>!",
    ];

    /// Screen, cursor, and the modes and attributes the next byte would be printed with.
    fn state(screen: &vt100::Screen) -> (Vec<u8>, (u16, u16), Vec<u8>) {
        (
            screen.contents_formatted(),
            screen.cursor_position(),
            screen.state_formatted(),
        )
    }

    /// Cut the output at every byte. A host fed it in two batches, and a viewer that attached
    /// between them, both end up exactly where one fed it whole does.
    ///
    /// The viewer takes snapshot and tail in one `process` call, as a stream parser such as
    /// xterm.js sees one stream: `vt100` itself would trip on a character split across calls,
    /// which is the very thing [`Headless`] works around on the host's side.
    #[test]
    fn a_snapshot_cut_at_any_byte_continues_exactly() {
        for output in CORPUS {
            let mut whole = vt100::Parser::new(6, 30, 100);
            whole.process(output);
            let expected = state(whole.screen());
            for cut in 0..=output.len() {
                let (head, tail) = output.split_at(cut);
                let what = format!("cut at {cut} of {:?}", String::from_utf8_lossy(output));
                let mut host = Headless::new(6, 30, 100);
                host.feed(head);
                let mut stream = host.snapshot();
                stream.extend_from_slice(tail);
                let mut viewer = vt100::Parser::new(6, 30, 100);
                viewer.process(&stream);
                assert_eq!(state(viewer.screen()), expected, "viewer, {what}");

                host.feed(tail);
                assert_eq!(state(host.screen()), expected, "host, {what}");
            }
        }
    }

    #[test]
    fn a_long_string_cut_mid_way_is_still_swallowed_by_the_viewer() {
        let mut copy = b"\x1b]52;c;".to_vec();
        copy.extend(std::iter::repeat_n(b'A', 10_000));
        let (head, tail) = copy.split_at(6_000);
        let mut host = Headless::new(3, 20, 0);
        host.feed(head);
        let mut stream = host.snapshot();
        stream.extend_from_slice(tail);
        stream.extend_from_slice(b"\x07visible");
        let mut viewer = vt100::Parser::new(3, 20, 0);
        viewer.process(&stream);
        assert_eq!(viewer.screen().contents(), "visible");
    }

    /// Feed `snapshot` to a fresh terminal and return it.
    fn replay(snapshot: &[u8], rows: u16, cols: u16) -> vt100::Parser {
        let mut viewer = vt100::Parser::new(rows, cols, 1000);
        viewer.process(snapshot);
        viewer
    }

    #[test]
    fn read_back_as_text_gives_the_lines_the_program_printed() {
        let mut source = vt100::Parser::new(4, 20, 1000);
        source.process(b"first\r\nsecond \x1b[31mred\x1b[m\r\nthird\r\n");

        let text = to_text(&render(&mut source), 4, 20);

        assert_eq!(text, "first\nsecond red\nthird");
    }

    /// The history a viewer would scroll back to is part of what was printed, so it comes out
    /// too — and the blank run `render` uses to push it off the screen does not.
    #[test]
    fn read_back_as_text_includes_scrollback_and_not_the_gap_behind_it() {
        let mut source = vt100::Parser::new(3, 20, 1000);
        for n in 1..=8 {
            source.process(format!("line {n}\r\n").as_bytes());
        }

        let text = to_text(&render(&mut source), 3, 20);

        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            ["line 1", "line 2", "line 3", "line 4", "line 5", "line 6", "line 7", "line 8"],
            "every line should appear once, in order, with no blank gap"
        );
    }

    /// A full-screen program's screen is what matters; its alternate screen has no scrollback.
    #[test]
    fn read_back_as_text_handles_a_full_screen_program() {
        let mut source = vt100::Parser::new(3, 20, 1000);
        source.process(b"scrolled away\r\n\x1b[?1049h\x1b[H\x1b[2Jmenu item\r\nchosen");

        let text = to_text(&render(&mut source), 3, 20);

        assert_eq!(text, "menu item\nchosen");
    }

    #[test]
    fn reproduces_screen_cursor_and_colours() {
        let mut source = vt100::Parser::new(5, 20, 1000);
        source.process(b"plain \x1b[31mred\x1b[m\r\nsecond line\x1b[2;4H");

        let viewer = replay(&render(&mut source), 5, 20);

        assert_eq!(viewer.screen().contents(), source.screen().contents());
        assert_eq!(viewer.screen().cursor_position(), (1, 3));
        let cell = viewer.screen().cell(0, 6).unwrap();
        assert_eq!(cell.contents(), "r");
        assert_eq!(cell.fgcolor(), vt100::Color::Idx(1));
    }

    #[test]
    fn carries_scrollback_in_order() {
        let mut source = vt100::Parser::new(4, 20, 1000);
        for n in 1..=25 {
            source.process(format!("line {n}\r\n").as_bytes());
        }

        let mut viewer = replay(&render(&mut source), 4, 20);

        assert_eq!(viewer.screen().contents(), source.screen().contents());
        let theirs = scrollback_text(&mut viewer);
        let ours = scrollback_text(&mut source);
        assert_eq!(ours.first().map(String::as_str), Some("line 1"));
        // The viewer holds our history, in order, followed only by padding it scrolled away.
        assert_eq!(&theirs[..ours.len()], &ours[..]);
        assert!(theirs[ours.len()..].iter().all(String::is_empty));
    }

    #[test]
    fn restores_the_alternate_screen_and_input_modes() {
        let mut source = vt100::Parser::new(5, 20, 1000);
        source.process(b"shell prompt\r\n\x1b[?1049h\x1b[?2004h\x1b[?25lfull-screen app");

        let viewer = replay(&render(&mut source), 5, 20);

        assert!(viewer.screen().alternate_screen());
        assert!(viewer.screen().bracketed_paste());
        assert!(viewer.screen().hide_cursor());
        assert_eq!(viewer.screen().contents(), "full-screen app");
    }

    #[test]
    fn leaves_the_source_scrolled_to_the_bottom() {
        let mut source = vt100::Parser::new(3, 10, 100);
        for n in 1..=10 {
            source.process(format!("{n}\r\n").as_bytes());
        }
        let before = source.screen().contents();
        render(&mut source);
        assert_eq!(source.screen().scrollback(), 0);
        assert_eq!(source.screen().contents(), before);
    }

    fn scrollback_text(parser: &mut vt100::Parser) -> Vec<String> {
        let (rows, cols) = parser.screen().size();
        scrollback_rows(parser, rows, cols)
            .into_iter()
            .map(|row| {
                let mut p = vt100::Parser::new(1, cols, 0);
                p.process(&row);
                p.screen().contents()
            })
            .collect()
    }
}
