//! The last stretch of a session's numbered stream, kept so a viewer that lost its connection can
//! carry on from the last item it saw instead of being sent the whole screen again.
//!
//! It is kept only for sessions someone has watched numbered, and only the last
//! [`REPLAY_BYTES`] of it: a phone that was away for longer than that, or a session that never had
//! a numbered viewer, gets a snapshot, which is always correct. What is kept is what numbered
//! viewers were sent, so the program's terminal queries are not in it.

use std::collections::VecDeque;

use crate::types::{Numbered, OutputSeq, TermSize};

/// How much of the stream a session keeps for resuming. A minute of an agent's ordinary output
/// fits several times over; a full-screen program repainting fast fills it in seconds, and the
/// viewer then gets a snapshot.
pub const REPLAY_BYTES: usize = 256 * 1024;
/// What a kept resize counts as against [`REPLAY_BYTES`]: its own size, roughly.
const RESIZE_COST: usize = 16;

pub(crate) struct Replay {
    /// The item just before the first one kept, and the size the terminal was at after it.
    base: OutputSeq,
    base_size: TermSize,
    /// Every item after `base`, numbered from `base + 1` without a gap.
    items: VecDeque<Kept>,
    bytes: usize,
}

enum Kept {
    Output(Vec<u8>),
    Resized(TermSize),
}

impl Kept {
    fn cost(&self) -> usize {
        match self {
            Self::Output(bytes) => bytes.len(),
            Self::Resized(_) => RESIZE_COST,
        }
    }
}

impl Replay {
    /// Keep the stream from after item `seq`, at which the terminal is `size`.
    pub(crate) fn new(seq: OutputSeq, size: TermSize) -> Self {
        Self {
            base: seq,
            base_size: size,
            items: VecDeque::new(),
            bytes: 0,
        }
    }

    /// The item after the last one kept was output.
    pub(crate) fn output(&mut self, bytes: &[u8]) {
        self.push(Kept::Output(bytes.to_vec()));
    }

    /// The item after the last one kept was a resize.
    pub(crate) fn resized(&mut self, size: TermSize) {
        self.push(Kept::Resized(size));
    }

    fn push(&mut self, item: Kept) {
        self.bytes += item.cost();
        self.items.push_back(item);
        while self.bytes > REPLAY_BYTES {
            let Some(oldest) = self.items.pop_front() else {
                break;
            };
            self.bytes -= oldest.cost();
            self.base += 1;
            if let Kept::Resized(size) = oldest {
                self.base_size = size;
            }
        }
    }

    /// The items after item `after`, if they are all still kept, and the size the terminal was
    /// at after it. `None` when some are gone, or `after` is not an item of this stream yet.
    pub(crate) fn since(
        &self,
        after: OutputSeq,
    ) -> Option<(TermSize, impl Iterator<Item = Numbered<'_>>)> {
        let skip = usize::try_from(after.checked_sub(self.base)?).ok()?;
        if skip > self.items.len() {
            return None;
        }
        let size = self
            .items
            .iter()
            .take(skip)
            .fold(self.base_size, |size, item| match item {
                Kept::Resized(to) => *to,
                Kept::Output(_) => size,
            });
        let items = self
            .items
            .iter()
            .enumerate()
            .skip(skip)
            .map(move |(at, item)| {
                let seq = self.base + at as OutputSeq + 1;
                match item {
                    Kept::Output(bytes) => Numbered::Output { seq, bytes },
                    Kept::Resized(size) => Numbered::Resized { seq, size: *size },
                }
            });
        Some((size, items))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: TermSize = TermSize { cols: 80, rows: 24 };
    const WIDE: TermSize = TermSize {
        cols: 120,
        rows: 40,
    };

    fn seqs(replay: &Replay, after: OutputSeq) -> Option<(TermSize, Vec<OutputSeq>)> {
        replay.since(after).map(|(size, items)| {
            let seqs = items
                .map(|item| match item {
                    Numbered::Output { seq, .. } | Numbered::Resized { seq, .. } => seq,
                    other => panic!("not a stream item: {other:?}"),
                })
                .collect();
            (size, seqs)
        })
    }

    #[test]
    fn what_came_after_a_kept_item_is_handed_back_in_order_with_the_size_then() {
        let mut replay = Replay::new(4, SMALL);
        replay.output(b"five");
        replay.resized(WIDE);
        replay.output(b"seven");
        assert_eq!(seqs(&replay, 4), Some((SMALL, vec![5, 6, 7])));
        assert_eq!(seqs(&replay, 5), Some((SMALL, vec![6, 7])));
        // After the resize, the viewer's terminal was already the new size.
        assert_eq!(seqs(&replay, 6), Some((WIDE, vec![7])));
        // Up to date: nothing to send, which is still a resume.
        assert_eq!(seqs(&replay, 7), Some((WIDE, vec![])));
        // Before what was kept, or an item that has not happened: no.
        assert_eq!(seqs(&replay, 3), None);
        assert_eq!(seqs(&replay, 8), None);
    }

    #[test]
    fn the_oldest_items_go_first_once_the_window_is_full_and_the_size_goes_with_them() {
        let mut replay = Replay::new(0, SMALL);
        replay.resized(WIDE);
        let chunk = vec![b'x'; REPLAY_BYTES / 4];
        for _ in 0..4 {
            replay.output(&chunk);
        }
        // One more pushes the resize, then the first output, out.
        replay.output(&chunk);
        assert_eq!(seqs(&replay, 0), None);
        assert_eq!(seqs(&replay, 1), None);
        assert_eq!(seqs(&replay, 2), Some((WIDE, vec![3, 4, 5, 6])));
        assert!(replay.bytes <= REPLAY_BYTES);
    }

    #[test]
    fn output_larger_than_the_window_leaves_only_the_present_to_resume_from() {
        let mut replay = Replay::new(0, SMALL);
        replay.output(&vec![b'x'; REPLAY_BYTES + 1]);
        assert_eq!(seqs(&replay, 0), None);
        assert_eq!(seqs(&replay, 1), Some((SMALL, vec![])));
    }
}
