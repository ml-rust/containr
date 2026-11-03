//! Packet iterators. Each holds a handle to the packet index, so iterating
//! does not borrow the demuxer, and the caller can read packet bytes in the
//! loop.
//!
//! The demuxer indexes every frame at open, so iteration never fails. The
//! items are `Result`s to match the MP4 iterators.

use std::sync::Arc;

use crate::error::Result;
use crate::packet::Packet;

/// The packets of every track in file order.
#[derive(Clone, Debug)]
pub struct Packets {
    packets: Arc<[Packet]>,
    pos: usize,
}

impl Packets {
    pub(crate) fn new(packets: Arc<[Packet]>) -> Self {
        Self { packets, pos: 0 }
    }
}

impl Iterator for Packets {
    type Item = Result<Packet>;

    fn next(&mut self) -> Option<Self::Item> {
        let packet = *self.packets.get(self.pos)?;
        self.pos += 1;
        Some(Ok(packet))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.packets.len().saturating_sub(self.pos);
        (left, Some(left))
    }
}

/// The packets of one track in storage order, which Matroska defines as
/// decode order.
#[derive(Clone, Debug)]
pub struct TrackPackets {
    packets: Arc<[Packet]>,
    track_id: u32,
    pos: usize,
}

impl TrackPackets {
    pub(crate) fn new(packets: Arc<[Packet]>, track_id: u32) -> Self {
        Self {
            packets,
            track_id,
            pos: 0,
        }
    }
}

impl Iterator for TrackPackets {
    type Item = Result<Packet>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(&packet) = self.packets.get(self.pos) {
            self.pos += 1;
            if packet.track_id == self.track_id {
                return Some(Ok(packet));
            }
        }
        None
    }
}
