//! One output track while packets arrive: timing checks and blocks not yet
//! written.
//!
//! A packet is checked in full before anything changes, so a refused packet
//! leaves the track as it was. The newest block of a track waits in the queue,
//! since only the next packet tells whether it is the last. The last block
//! always carries `BlockDuration`.

use std::collections::VecDeque;

use crate::error::{Container, Error, Result};
use crate::mkv::mux::bytes::refuse;
use crate::mkv::mux::spec::{PacketIn, TrackSpec};
use crate::mkv::mux::timing::{Inexact, rescale};

/// One packet in output ticks, waiting for its place in a Cluster.
#[derive(Debug)]
pub(crate) struct Block {
    pub data: Vec<u8>,
    /// The block timestamp, 0 or more.
    pub ticks: i64,
    /// The interleave key: the largest `ticks` of the track up to this block.
    pub key: i64,
    pub duration: Option<u64>,
    pub sync: bool,
    pub discardable: bool,
    pub discard_padding: Option<i64>,
}

/// One output track.
#[derive(Debug)]
pub(crate) struct TrackState {
    /// The `TrackNumber`.
    pub number: u32,
    pub uid: u64,
    pub spec: TrackSpec,
    /// What a reader takes as the duration of a block with none:
    /// `DefaultDuration` over the scale, rounded down.
    pub default_ticks: Option<u64>,
    pub queue: VecDeque<Block>,
    /// The largest `ticks + duration` so far.
    pub end: i64,
    /// The timestamp of the block last written.
    pub previous: Option<i64>,
    scale: u64,
    /// Packets accepted so far.
    packets: u64,
    key: i64,
    /// True when the newest packet has a known duration.
    last_known: bool,
}

impl TrackState {
    pub fn new(number: u32, uid: u64, spec: TrackSpec, scale: u64) -> Self {
        let default_ticks = spec
            .matroska
            .default_duration
            .map(|nanoseconds| nanoseconds / scale.max(1));
        Self {
            number,
            uid,
            spec,
            default_ticks,
            queue: VecDeque::new(),
            end: 0,
            previous: None,
            scale,
            packets: 0,
            key: 0,
            last_known: true,
        }
    }

    pub fn is_video(&self) -> bool {
        self.spec.matroska.track_type == 1
    }

    fn invalid(&self, reason: String) -> Error {
        Error::InvalidPacket {
            container: Container::Matroska,
            track_id: self.number,
            index: self.packets,
            reason,
        }
    }

    /// `value` in output ticks: exact, or refused by the timing rule.
    fn scaled(&self, value: i128, what: &str) -> Result<i64> {
        let timebase = self.spec.timebase;
        match rescale(value, timebase, self.scale) {
            Ok(ticks) => i64::try_from(ticks).map_err(|_| {
                self.invalid(format!(
                    "the {what} {value} is {ticks} output ticks, outside 64 signed bits. Use a \
                     coarser `MkvOptions::timestamp_scale`"
                ))
            }),
            Err(Inexact::NoNanoseconds) => Err(refuse(format!(
                "the {what} {value} of track {} at {}/{} seconds per tick: that time is no whole \
                 number of nanoseconds, and `TimestampScale` counts whole nanoseconds. To round \
                 a stream copy to the nearest tick, use `StreamCopy::plan_rounded`",
                self.number, timebase.num, timebase.den
            ))),
            Err(Inexact::NotMultiple { nanos, divisor }) => Err(self.invalid(format!(
                "the {what} {value} is {nanos} ns, no whole multiple of the `TimestampScale` {}. \
                 Set `MkvOptions::timestamp_scale` to {divisor}, or to a value \
                 `StreamCopy::timestamp_scale` gives",
                self.scale
            ))),
        }
    }

    /// Check `packet`, and give the block to queue.
    pub fn accept(&self, packet: &PacketIn<'_>) -> Result<Block> {
        if packet.pts < 0 {
            return Err(refuse(format!(
                "packet {} of track {} at pts {}: a Matroska block timestamp is never negative, \
                 and an MP4 edit list holds a negative start",
                self.packets, self.number, packet.pts
            )));
        }
        let ticks = self.scaled(i128::from(packet.pts), "pts")?;
        let duration = match packet.duration {
            Some(duration) => Some(
                self.scaled(i128::from(duration), "duration")?
                    .unsigned_abs(),
            ),
            None => None,
        };
        Ok(Block {
            data: packet.data.to_vec(),
            ticks,
            key: self.key.max(ticks),
            duration,
            sync: packet.sync,
            discardable: packet.discardable,
            discard_padding: packet.discard_padding,
        })
    }

    /// Queue a block `accept` gave. Returns its bytes.
    pub fn push(&mut self, block: Block) -> usize {
        let duration = block.duration.or(self.default_ticks);
        let span = i64::try_from(duration.unwrap_or(0)).unwrap_or(i64::MAX);
        self.end = self.end.max(block.ticks.saturating_add(span));
        self.key = block.key;
        self.last_known = duration.is_some();
        self.packets += 1;
        let bytes = block.data.len();
        self.queue.push_back(block);
        bytes
    }

    /// The `BlockDuration` to write. With `explicit`, a block always writes
    /// its duration when one is known. Otherwise only a duration that differs
    /// from what a reader takes from `DefaultDuration` needs writing.
    pub fn written_duration(&self, block: &Block, explicit: bool) -> Option<u64> {
        if explicit {
            block.duration.or(self.default_ticks)
        } else {
            block
                .duration
                .filter(|&duration| Some(duration) != self.default_ticks)
        }
    }

    /// The `ReferenceBlock` of a block that is not a keyframe: the distance
    /// back to the block before it. Readers take any `ReferenceBlock` as "not
    /// a keyframe", and 0 is not a valid distance.
    pub fn reference(&self, block: &Block) -> Option<i64> {
        if block.sync {
            return None;
        }
        let back = self
            .previous
            .map_or(-1, |previous| previous.saturating_sub(block.ticks));
        Some(if back == 0 { -1 } else { back })
    }

    /// Refuse a track whose last packet has no duration from any source.
    pub fn check_last(&self) -> Result<()> {
        match self.packets.checked_sub(1) {
            Some(index) if !self.last_known => Err(Error::MissingDuration {
                track_id: self.number,
                index,
            }),
            _ => Ok(()),
        }
    }
}

/// The slot of the track whose first waiting block comes earliest.
pub(crate) fn earliest(tracks: &[TrackState]) -> Option<usize> {
    tracks
        .iter()
        .enumerate()
        .filter_map(|(slot, track)| track.queue.front().map(|block| (block.key, slot)))
        .min()
        .map(|(_, slot)| slot)
}
