//! One output track while packets arrive: timing checks, sample tables and
//! chunks not yet written.
//!
//! A packet is checked in full before anything changes, so a refused packet
//! leaves the track as it was.

use std::collections::VecDeque;

use crate::error::{Container, Error, Result};
use crate::mp4::mux::options::{MAX_CHUNK_BYTES, MAX_CHUNK_SAMPLES};
use crate::mp4::mux::spec::{PacketFormat, PacketIn, TrackSpec};
use crate::mp4::mux::table::SampleTable;

/// An empty `tx3g` sample: a text length of 0.
const EMPTY_TEXT: [u8; 2] = [0, 0];

/// Packet bytes waiting to be written as one chunk.
#[derive(Debug)]
pub(crate) struct Chunk {
    /// The decode time of the first sample.
    pub first_dts: i64,
    pub data: Vec<u8>,
    pub samples: u32,
}

/// The newest sample, whose duration the next packet sets.
#[derive(Clone, Copy, Debug)]
struct Pending {
    dts: i64,
    pts: i64,
    duration: Option<u64>,
    index: u64,
}

/// What a checked packet adds to the track.
struct Accepted {
    /// The duration of the pending sample.
    pending_delta: Option<u32>,
    /// An empty text sample before the packet: its decode time and duration.
    filler: Option<(i64, u32)>,
    offset: i64,
    size: u32,
}

/// One output track.
#[derive(Debug)]
pub(crate) struct TrackState {
    pub id: u32,
    pub spec: TrackSpec,
    /// The whole sample entry box.
    pub entry: Vec<u8>,
    pub table: SampleTable,
    pending: Option<Pending>,
    /// Packets accepted so far.
    packets: u64,
    /// For [`PacketFormat::PlainText`]: where the last cue ends.
    text_end: i64,
    /// The longest chunk, in track ticks of decode time.
    chunk_ticks: u64,
    pub open: Option<Chunk>,
    pub closed: VecDeque<Chunk>,
    last_dts: Option<i64>,
}

impl TrackState {
    pub fn new(id: u32, spec: TrackSpec, entry: Vec<u8>, chunk_ms: u32) -> Self {
        let chunk_ticks = (u64::from(chunk_ms) * u64::from(spec.timescale) / 1000).max(1);
        Self {
            id,
            spec,
            entry,
            table: SampleTable::default(),
            pending: None,
            packets: 0,
            text_end: 0,
            chunk_ticks,
            open: None,
            closed: VecDeque::new(),
            last_dts: None,
        }
    }

    fn invalid(&self, reason: String) -> Error {
        Error::InvalidPacket {
            container: Container::Mp4,
            track_id: self.id,
            index: self.packets,
            reason,
        }
    }

    /// The `stts` delta from the sample `prev` to a sample at `dts`.
    fn delta(&self, prev: &Pending, dts: i64) -> Result<u32> {
        if dts <= prev.dts {
            return Err(Error::DtsOutOfOrder {
                track_id: self.id,
                index: self.packets,
                dts,
                previous: prev.dts,
            });
        }
        let delta = dts
            .checked_sub(prev.dts)
            .and_then(|d| u32::try_from(d).ok());
        let Some(delta) = delta else {
            return Err(self.invalid(format!(
                "dts {dts} is more than 2^32 - 1 ticks after the previous dts {}, the \
                 largest `stts` delta",
                prev.dts
            )));
        };
        if let Some(declared) = prev.duration
            && declared != u64::from(delta)
        {
            return Err(Error::InvalidPacket {
                container: Container::Mp4,
                track_id: self.id,
                index: prev.index,
                reason: format!(
                    "the duration is {declared}, but the next packet's dts {dts} gives \
                     {delta}. MP4 stores durations as dts deltas. Make them agree, or \
                     leave the duration unset on every packet but the last"
                ),
            });
        }
        Ok(delta)
    }

    /// Check a composition offset against what `ctts` can store.
    fn offset(&self, packet: &PacketIn<'_>, signed: bool) -> Result<i64> {
        let refuse = |reason: String| Error::CompositionOffset {
            track_id: self.id,
            index: self.packets,
            pts: packet.pts,
            dts: packet.dts,
            reason,
        };
        let offset = packet
            .pts
            .checked_sub(packet.dts)
            .ok_or_else(|| refuse("pts - dts overflows 64 bits".to_owned()))?;
        if offset < 0 && !signed {
            return Err(refuse(
                "pts is before dts, and the `mp41` brand stores unsigned composition \
                 offsets. Use `Brand::Isom`, `Brand::Mp42` or `Brand::QuickTime`"
                    .to_owned(),
            ));
        }
        // Version 0 `ctts` can store up to 2^32 - 1, but readers such as
        // libavformat and this crate read every offset as signed.
        if i32::try_from(offset).is_err() {
            return Err(refuse(format!(
                "the composition offset {offset} is outside the range readers accept, \
                 -2^31 to 2^31 - 1. Lower the distance between pts and dts"
            )));
        }
        Ok(offset)
    }

    fn accept(&self, packet: &PacketIn<'_>, signed: bool) -> Result<Accepted> {
        let framing = match self.spec.packet_format {
            PacketFormat::AsIs => 0,
            PacketFormat::PlainText => 2,
        };
        let size = u32::try_from(packet.data.len() + framing)
            .ok()
            .filter(|_| framing == 0 || packet.data.len() <= usize::from(u16::MAX));
        let Some(size) = size else {
            return Err(self.invalid(format!(
                "the packet holds {} bytes, above the sample limit of {}. Split the packet",
                packet.data.len(),
                if framing == 0 {
                    u64::from(u32::MAX)
                } else {
                    u64::from(u16::MAX)
                }
            )));
        };
        let mut filler = None;
        let mut prev = self.pending;
        if self.spec.packet_format == PacketFormat::PlainText {
            if packet.pts != packet.dts {
                return Err(self.invalid(format!(
                    "a text cue has pts {} and dts {}. Text cues need pts == dts",
                    packet.pts, packet.dts
                )));
            }
            if packet.duration.is_none() && self.spec.default_duration.is_none() {
                return Err(Error::MissingDuration {
                    track_id: self.id,
                    index: self.packets,
                });
            }
            if packet.dts < self.text_end {
                return Err(Error::Unsupported {
                    container: Container::Mp4,
                    what: format!(
                        "overlapping text cues: track {} cue {} starts at {} before the \
                         previous cue ends at {}",
                        self.id, self.packets, packet.dts, self.text_end
                    ),
                    use_instead: Container::Matroska,
                });
            }
            if packet.dts > self.text_end {
                let gap = u32::try_from(packet.dts - self.text_end).map_err(|_| {
                    self.invalid(format!(
                        "the gap of {} ticks before the cue exceeds 2^32 - 1. Send a cue inside the gap",
                        packet.dts - self.text_end
                    ))
                })?;
                filler = Some((self.text_end, gap));
            }
        }
        let mut pending_delta = None;
        if let Some((filler_dts, _)) = filler {
            if let Some(prev) = &prev {
                pending_delta = Some(self.delta(prev, filler_dts)?);
            }
            prev = Some(Pending {
                dts: filler_dts,
                pts: filler_dts,
                duration: None,
                index: self.packets,
            });
        }
        match &prev {
            Some(prev) => {
                let delta = self.delta(prev, packet.dts)?;
                if filler.is_none() {
                    pending_delta = Some(delta);
                }
            }
            None if packet.dts != 0 => {
                return Err(self.invalid(format!(
                    "the first packet has dts {}, and MP4 decodes the first sample at 0. \
                     Shift the track's timestamps so its first dts is 0, and put the delay \
                     in the edit list",
                    packet.dts
                )));
            }
            None => {}
        }
        let samples = u64::from(self.table.count) + 1 + u64::from(filler.is_some());
        if samples > u64::from(u32::MAX) {
            return Err(
                self.invalid("the track holds 2^32 - 1 samples, the most `stsz` counts. Split the output across files".to_owned())
            );
        }
        let offset = self.offset(packet, signed)?;
        if let Some(duration) = packet.duration
            && u32::try_from(duration).is_err()
        {
            return Err(self.invalid(format!(
                "the duration {duration} exceeds 2^32 - 1, the largest `stts` delta. Use a shorter duration"
            )));
        }
        Ok(Accepted {
            pending_delta,
            filler,
            offset,
            size,
        })
    }

    /// Check `packet` and add it. Returns the bytes added to the chunk
    /// buffer.
    pub fn push(&mut self, packet: &PacketIn<'_>, signed: bool) -> Result<usize> {
        let accepted = self.accept(packet, signed)?;
        if let Some(delta) = accepted.pending_delta
            && let Some(pending) = self.pending.take()
        {
            self.table.push_duration(delta, pending.pts);
        }
        let mut added = 0;
        if let Some((dts, gap)) = accepted.filler {
            self.table.push_sample(2, 0, true);
            self.table.push_duration(gap, dts);
            added += self.append(dts, &[&EMPTY_TEXT[..]]);
        }
        self.table
            .push_sample(accepted.size, accepted.offset, packet.sync);
        added += match self.spec.packet_format {
            PacketFormat::AsIs => self.append(packet.dts, &[packet.data]),
            PacketFormat::PlainText => {
                // accept() bounds the text length by u16::MAX.
                let len = (packet.data.len() as u16).to_be_bytes();
                self.append(packet.dts, &[&len[..], packet.data])
            }
        };
        let duration = packet
            .duration
            .or_else(|| self.spec.default_duration.map(u64::from));
        if self.spec.packet_format == PacketFormat::PlainText {
            // accept() bounds the duration by u32::MAX.
            self.text_end = packet.dts.saturating_add(duration.unwrap_or(0) as i64);
        }
        self.pending = Some(Pending {
            dts: packet.dts,
            pts: packet.pts,
            duration: packet.duration,
            index: self.packets,
        });
        self.last_dts = Some(packet.dts);
        self.packets += 1;
        Ok(added)
    }

    /// Add one sample's bytes to the open chunk. Returns the byte count.
    fn append(&mut self, dts: i64, parts: &[&[u8]]) -> usize {
        if let Some(open) = &self.open
            && dts.saturating_sub(open.first_dts) as u64 >= self.chunk_ticks
        {
            self.close_open();
        }
        let chunk = self.open.get_or_insert_with(|| Chunk {
            first_dts: dts,
            data: Vec::new(),
            samples: 0,
        });
        let mut added = 0;
        for part in parts {
            chunk.data.extend_from_slice(part);
            added += part.len();
        }
        chunk.samples += 1;
        if chunk.data.len() >= MAX_CHUNK_BYTES || chunk.samples >= MAX_CHUNK_SAMPLES {
            self.close_open();
        }
        added
    }

    /// Move the open chunk to the write queue.
    pub fn close_open(&mut self) {
        if let Some(chunk) = self.open.take() {
            self.closed.push_back(chunk);
        }
    }

    /// The earliest decode time this track can still write: its first
    /// queued chunk, else its last packet. `None` before its first packet.
    pub fn lower_bound(&self) -> Option<i64> {
        self.closed
            .front()
            .or(self.open.as_ref())
            .map(|chunk| chunk.first_dts)
            .or(self.last_dts)
    }

    /// End the track: set the last sample's duration and queue the open
    /// chunk.
    pub fn finish(&mut self) -> Result<()> {
        if let Some(pending) = self.pending {
            let duration = pending
                .duration
                .or_else(|| self.spec.default_duration.map(u64::from))
                .ok_or(Error::MissingDuration {
                    track_id: self.id,
                    index: pending.index,
                })?;
            // accept() bounds a packet duration by u32::MAX, and the
            // default is a u32.
            let duration = u32::try_from(duration).unwrap_or(u32::MAX);
            self.table.push_duration(duration, pending.pts);
            self.pending = None;
        }
        self.close_open();
        Ok(())
    }
}
