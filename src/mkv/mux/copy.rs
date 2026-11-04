//! Stream copy: a track spec and packet timing from a demuxed track.
//!
//! A Matroska track copies as it is: every `MatroskaTrack` fact, `pts`,
//! duration, flags and `DiscardPadding`. An MP4 track maps to its Matroska
//! form, and every timestamp and duration must be a whole number of
//! nanoseconds, or [`StreamCopy::plan`] refuses the track and names MP4.
//! [`StreamCopy::plan_rounded`] is the opt-in that rounds instead.
//!
//! An MP4 edit list has no Matroska element, so the plan expresses it as
//! [`edit_plan`] describes, and refuses one with no Matroska form. A frame the
//! edit cuts from the start needs a negative timestamp, so it is refused.
//! An audio edit that ends before the last packet does becomes `DiscardPadding`.
//! A `tx3g` sample loses its 16-bit length, and an empty sample writes nothing.

use crate::error::Result;
use crate::mkv::mux::bytes::{invalid, refuse};
use crate::mkv::mux::copy_timing::{
    exact_scale, matroska_timing, mp4_timing, set_codec_delay, uniform_duration,
};
use crate::mkv::mux::from_mp4::{is_tx3g, spec_from_mp4};
use crate::mkv::mux::mp4_edit::edit_plan;
use crate::mkv::mux::rounded::{TimestampRounding, round_mp4};
use crate::mkv::mux::spec::{PacketIn, TrackSpec};
use crate::mkv::mux::timing::gcd;
use crate::packet::Packet;
use crate::track::{Timebase, Track};

/// The scale [`StreamCopy::timestamp_scale`] starts from: 1 ms, the Matroska
/// default. A coarser scale holds fewer values.
pub(super) const PREFERRED_SCALE: u64 = 1_000_000;

impl TrackSpec {
    /// A spec from a demuxed Matroska or MP4 track.
    ///
    /// A Matroska track keeps its raw `CodecID`, `CodecPrivate`, UID, tags and
    /// every other `MatroskaTrack` fact. An MP4 track maps by sample entry,
    /// and an entry with no Matroska form gives
    /// [`Error::Unsupported`], which names MP4. Use [`StreamCopy::plan`] for
    /// packet timing and for the facts an MP4 track's packets and edit list
    /// set: `DefaultDuration` and `CodecDelay`.
    pub fn copy_from(track: &Track) -> Result<Self> {
        match &track.matroska {
            Some(mkv) => Ok(Self {
                timebase: track.timebase,
                matroska: mkv.clone(),
                metadata: track.metadata.clone(),
            }),
            None => spec_from_mp4(track),
        }
    }
}

/// One packet's timing, in ticks of the plan's [`TrackSpec::timebase`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PacketTiming {
    /// Presentation timestamp.
    pub pts: i64,
    /// Duration. `None` leaves the packet to `DefaultDuration`.
    pub duration: Option<u64>,
    /// The `DiscardPadding` in nanoseconds.
    pub discard_padding: Option<i64>,
}

/// A track spec and packet timing for the stream copy of one track.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamCopy {
    /// The spec to pass to [`Muxer::add_track`](crate::mkv::Muxer::add_track).
    pub spec: TrackSpec,
    /// One entry per source packet. `None` writes no block.
    timing: Vec<Option<PacketTiming>>,
    /// The largest divisor of 1 ms that holds every value of this track, or
    /// the scale a rounded plan names.
    scale: u64,
    /// What a rounded plan did. `None` for an exact plan.
    rounding: Option<TimestampRounding>,
    /// True when a sample holds a 16-bit length, then the text.
    framed_text: bool,
    track_id: u32,
}

impl StreamCopy {
    /// Plan the copy of `track`. `packets` holds every packet of the track,
    /// in decode order, as the demuxer's `track_packets` yields them.
    ///
    /// Every value converts exactly, or the plan refuses the track. An MP4
    /// value with no whole number of nanoseconds is refused, and the error
    /// names MP4 and [`StreamCopy::plan_rounded`].
    pub fn plan(track: &Track, packets: &[Packet]) -> Result<Self> {
        check_order(track, packets, "plan")?;
        let mut spec = TrackSpec::copy_from(track)?;
        let framed_text = track.matroska.is_none() && is_tx3g(track);
        let timing = if track.matroska.is_some() {
            matroska_timing(track, packets)?
        } else {
            let edit = edit_plan(track)?;
            set_codec_delay(track, &edit, &mut spec)?;
            let timing = mp4_timing(track, &edit, framed_text, packets)?;
            if !framed_text {
                spec.matroska.default_duration = uniform_duration(track, &timing)?;
            }
            timing
        };
        let scale = exact_scale(track, &timing)?;
        Ok(Self {
            spec,
            timing,
            scale,
            rounding: None,
            framed_text,
            track_id: track.id,
        })
    }

    /// Plan the copy of an MP4 `track`, rounding to `scale` nanoseconds per
    /// tick, 1 to 2^32 - 1. `packets` holds every packet of the track, in
    /// decode order.
    ///
    /// Each timestamp rounds on its own, from its exact source value, to the
    /// nearest tick, ties to even. Its error stays at most half a tick, and no
    /// error accumulates across packets. A packet's duration is the distance
    /// to the next packet in presentation order. The last packet in that
    /// order rounds from its exact end. Decode order and `sync` flags stay as
    /// the source has them.
    ///
    /// `CodecDelay`, `DiscardPadding` and `DefaultDuration` count in
    /// nanoseconds, so each rounds to the nearest nanosecond, ties to even.
    /// The edit list's start delay rounds with the timestamps.
    ///
    /// The refusals name the input: two packets on one tick, with a finer
    /// scale to use, a Matroska track, and a scale out of range. The spec's
    /// timebase becomes `scale`, and [`PacketTiming`] counts in that tick.
    /// [`StreamCopy::rounding`] reports the largest error.
    pub fn plan_rounded(track: &Track, packets: &[Packet], scale: u64) -> Result<Self> {
        check_order(track, packets, "plan_rounded")?;
        let tick = u32::try_from(scale).ok().filter(|&tick| tick > 0).ok_or_else(|| {
            invalid(format!(
                "`StreamCopy::plan_rounded` got scale {scale}. Use 1 to {} nanoseconds per tick",
                u32::MAX
            ))
        })?;
        if track.matroska.is_some() {
            return Err(invalid(format!(
                "`StreamCopy::plan_rounded` got Matroska track {}. It rounds MP4 tracks. Use \
                 `StreamCopy::plan`",
                track.id
            )));
        }
        let mut spec = TrackSpec::copy_from(track)?;
        let framed_text = is_tx3g(track);
        let rounded = round_mp4(track, framed_text, packets, tick)?;
        spec.timebase = Timebase::from_nanoseconds(tick);
        spec.matroska.codec_delay = rounded.codec_delay;
        spec.matroska.default_duration = rounded.default_duration;
        Ok(Self {
            spec,
            timing: rounded.timing,
            scale,
            rounding: Some(rounded.rounding),
            framed_text,
            track_id: track.id,
        })
    }

    /// The `TimestampScale` for a set of tracks. Set it as
    /// [`MkvOptions::timestamp_scale`](crate::mkv::MkvOptions::timestamp_scale).
    ///
    /// It is the largest value that divides the scale of every plan. An exact
    /// plan's scale is the largest divisor of 1 ms that holds its values. A
    /// rounded plan's scale is the one the caller named. With no plans, it is
    /// 1 ms.
    pub fn timestamp_scale(plans: &[StreamCopy]) -> u64 {
        let scale = plans.iter().fold(0u64, |scale, plan| {
            u64::try_from(gcd(u128::from(scale), u128::from(plan.scale))).unwrap_or(1)
        });
        if scale == 0 { PREFERRED_SCALE } else { scale }
    }

    /// What [`StreamCopy::plan_rounded`] did. `None` for an exact plan.
    pub fn rounding(&self) -> Option<TimestampRounding> {
        self.rounding
    }

    /// The timing of every packet, in decode order. `None` writes no block.
    pub fn timing(&self) -> &[Option<PacketTiming>] {
        &self.timing
    }

    /// The packet to write for a demuxed `packet` and its bytes. `None` when
    /// the packet writes no block: an empty `tx3g` gap sample.
    pub fn packet<'a>(&self, packet: &Packet, data: &'a [u8]) -> Result<Option<PacketIn<'a>>> {
        let timing = usize::try_from(packet.index)
            .ok()
            .and_then(|index| self.timing.get(index))
            .ok_or_else(|| {
                invalid(format!(
                    "packet {} is past the {} packets planned for track {}. Plan the copy with \
                     every packet of the track",
                    packet.index,
                    self.timing.len(),
                    self.track_id
                ))
            })?;
        let Some(timing) = timing else {
            return Ok(None);
        };
        let data = if self.framed_text {
            cue_text(data, self.track_id, packet.index)?
        } else {
            data
        };
        Ok(Some(PacketIn {
            data,
            pts: timing.pts,
            duration: timing.duration,
            sync: packet.sync,
            discardable: packet.discardable,
            discard_padding: timing.discard_padding,
        }))
    }
}

/// Refuse packets that are not the track's own, in decode order.
fn check_order(track: &Track, packets: &[Packet], call: &str) -> Result<()> {
    for (n, packet) in packets.iter().enumerate() {
        if packet.track_id != track.id || packet.index != n as u64 {
            return Err(invalid(format!(
                "`StreamCopy::{call}` got packet {} of track {} at position {n} for track {}. \
                 Pass the track's packets in decode order",
                packet.index, packet.track_id, track.id
            )));
        }
    }
    Ok(())
}

/// The text of a `tx3g` sample. Style boxes after the text have no
/// `S_TEXT/UTF8` form.
fn cue_text(data: &[u8], track_id: u32, index: u64) -> Result<&[u8]> {
    let [high, low, text @ ..] = data else {
        return Err(invalid(format!(
            "`tx3g` sample {index} of track {track_id} holds {} bytes, below its 2-byte length. \
             Remux the source with `ffmpeg -c copy`",
            data.len()
        )));
    };
    let len = usize::from(u16::from_be_bytes([*high, *low]));
    if text.len() != len {
        return Err(refuse(format!(
            "`tx3g` sample {index} of track {track_id}: {} bytes follow a text of {len} bytes. \
             `S_TEXT/UTF8` holds no style boxes",
            text.len()
        )));
    }
    Ok(text)
}
