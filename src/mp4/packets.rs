//! Packet iterators. Each holds its own table handles, so iterating does not
//! borrow the demuxer, and the caller can read packet bytes in the loop.

use crate::error::{Error, Result};
use crate::mp4::tables::SampleCursor;
use crate::packet::Packet;

/// The packets of one track in decode order.
#[derive(Clone, Debug)]
pub struct TrackPackets {
    cursor: SampleCursor,
}

impl TrackPackets {
    pub(crate) fn new(cursor: SampleCursor) -> Self {
        Self { cursor }
    }
}

impl Iterator for TrackPackets {
    type Item = Result<Packet>;

    fn next(&mut self) -> Option<Self::Item> {
        self.cursor.next_packet().transpose()
    }
}

/// The packets of every track in file order: by byte offset, with each
/// track's packets in decode order. Packets at one offset come in track
/// order. After an error the iterator yields nothing more.
#[derive(Debug)]
pub struct Packets {
    cursors: Vec<SampleCursor>,
    heads: Vec<Option<Packet>>,
    primed: bool,
    pending: Option<Error>,
    done: bool,
}

impl Packets {
    pub(crate) fn new(cursors: Vec<SampleCursor>) -> Self {
        let heads = vec![None; cursors.len()];
        Self {
            cursors,
            heads,
            primed: false,
            pending: None,
            done: false,
        }
    }

    /// Load the next packet of track `slot` into its head.
    fn refill(&mut self, slot: usize) -> Result<()> {
        if let (Some(cursor), Some(head)) = (self.cursors.get_mut(slot), self.heads.get_mut(slot)) {
            *head = cursor.next_packet()?;
        }
        Ok(())
    }
}

impl Iterator for Packets {
    type Item = Result<Packet>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if let Some(err) = self.pending.take() {
            self.done = true;
            return Some(Err(err));
        }
        if !self.primed {
            self.primed = true;
            for slot in 0..self.cursors.len() {
                if let Err(err) = self.refill(slot) {
                    self.done = true;
                    return Some(Err(err));
                }
            }
        }
        let slot = self
            .heads
            .iter()
            .enumerate()
            .filter_map(|(slot, head)| head.map(|packet| (packet.offset, slot)))
            .min()
            .map(|(_, slot)| slot);
        let Some(slot) = slot else {
            self.done = true;
            return None;
        };
        let packet = self.heads.get_mut(slot).and_then(Option::take);
        if let Err(err) = self.refill(slot) {
            self.pending = Some(err);
        }
        packet.map(Ok)
    }
}
