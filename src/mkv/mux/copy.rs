//! Stream copy: a track spec and packet timing from a demuxed track.
//!
//! A Matroska track copies as it is: every `MatroskaTrack` fact, `pts`,
//! duration, flags and `DiscardPadding`. An MP4 track maps to its Matroska
//! form, and every timestamp and duration must be a whole number of
//! nanoseconds, or [`StreamCopy::plan`] refuses the track and names MP4.
//!
//! An MP4 edit list has no Matroska element, so the plan expresses it as
//! [`edit_plan`] describes, and refuses one with no Matroska form. A frame the
//! edit cuts from the start needs a negative timestamp, so it is refused.
//! An audio edit that ends before the last packet does becomes `DiscardPadding`.
//! A `tx3g` sample loses its 16-bit length, and an empty sample writes nothing.

use crate::error::{Error, Result};
use crate::mkv::mux::bytes::{invalid, refuse};
use crate::mkv::mux::from_mp4::{is_tx3g, spec_from_mp4};
use crate::mkv::mux::mp4_edit::{EditPlan, edit_plan};
use crate::mkv::mux::spec::{PacketIn, TrackSpec};
use crate::mkv::mux::timing::{gcd, nanos_of};
use crate::packet::Packet;
use crate::track::{Track, TrackKind};

/// The scale [`StreamCopy::timestamp_scale`] starts from: 1 ms, the Matroska
/// default. A coarser scale holds fewer values.
const PREFERRED_SCALE: u64 = 1_000_000;

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

/// One packet's timing, in the track's ticks.
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
    /// The largest divisor of 1 ms that holds every value of this track.
    scale: u64,
    /// True when a sample holds a 16-bit length, then the text.
    framed_text: bool,
    track_id: u32,
}

impl StreamCopy {
    /// Plan the copy of `track`. `packets` holds every packet of the track,
    /// in decode order, as the demuxer's `track_packets` yields them.
    pub fn plan(track: &Track, packets: &[Packet]) -> Result<Self> {
        for (n, packet) in packets.iter().enumerate() {
            if packet.track_id != track.id || packet.index != n as u64 {
                return Err(invalid(format!(
                    "`StreamCopy::plan` got packet {} of track {} at position {n} for track \
                     {}. Pass the track's packets in decode order",
                    packet.index, packet.track_id, track.id
                )));
            }
        }
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
            framed_text,
            track_id: track.id,
        })
    }

    /// The `TimestampScale` for a set of tracks: the largest divisor of 1 ms
    /// that holds every value of every plan. Set it as
    /// [`MkvOptions::timestamp_scale`](crate::mkv::MkvOptions::timestamp_scale).
    pub fn timestamp_scale(plans: &[StreamCopy]) -> u64 {
        plans.iter().fold(PREFERRED_SCALE, |scale, plan| {
            u64::try_from(gcd(u128::from(scale), u128::from(plan.scale))).unwrap_or(1)
        })
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

fn matroska_timing(track: &Track, packets: &[Packet]) -> Result<Vec<Option<PacketTiming>>> {
    packets
        .iter()
        .map(|packet| {
            if packet.pts < 0 {
                return Err(negative(track, packet.index, packet.pts));
            }
            Ok(Some(PacketTiming {
                pts: packet.pts,
                duration: packet.duration,
                discard_padding: packet.discard_padding,
            }))
        })
        .collect()
}

fn negative(track: &Track, index: u64, pts: i64) -> Error {
    refuse(format!(
        "packet {index} of track {track} at pts {pts}: a Matroska block timestamp is never \
         negative, and an MP4 edit list holds a frame the edit cuts",
        track = track.id
    ))
}

fn mp4_timing(
    track: &Track,
    edit: &EditPlan,
    framed_text: bool,
    packets: &[Packet],
) -> Result<Vec<Option<PacketTiming>>> {
    let mut out = Vec::with_capacity(packets.len());
    // The last block's slot, and the largest pts + duration, before the shift.
    let mut last = None;
    let mut media_end = 0i128;
    for (n, packet) in packets.iter().enumerate() {
        if packet.description_index != 1 {
            return Err(Error::SampleDescriptionChange {
                track_id: track.id,
                descriptions: u64::from(packet.description_index),
            });
        }
        if framed_text && packet.size == 2 {
            out.push(None);
            continue;
        }
        let pts = packet
            .pts
            .checked_add(edit.shift)
            .ok_or_else(|| negative(track, packet.index, packet.pts))?;
        if pts < 0 {
            return Err(negative(track, packet.index, pts));
        }
        let end = i128::from(packet.pts) + i128::from(packet.duration.unwrap_or(0));
        media_end = media_end.max(end);
        last = Some(n);
        out.push(Some(PacketTiming {
            pts,
            duration: packet.duration,
            discard_padding: None,
        }));
    }
    let padding = end_trim(track, edit, media_end)?;
    if let (Some(padding), Some(slot)) = (padding, last)
        && let Some(Some(timing)) = out.get_mut(slot)
    {
        let covered = timing
            .duration
            .is_some_and(|duration| i128::from(duration) >= i128::from(padding.ticks));
        if !covered {
            return Err(refuse(format!(
                "the edit list of track {} cuts {} ticks from the end, more than its last \
                 packet lasts: `DiscardPadding` trims one block",
                track.id, padding.ticks
            )));
        }
        timing.discard_padding = Some(padding.nanos);
    }
    Ok(out)
}

/// A trim of the last packet.
struct EndTrim {
    ticks: i64,
    nanos: i64,
}

/// The trim an edit asks for at the end of the media. A gap of up to one
/// movie tick is the rounding of the edit's own duration, and trims nothing.
fn end_trim(track: &Track, edit: &EditPlan, media_end: i128) -> Result<Option<EndTrim>> {
    let Some(end_scaled) = edit.end_scaled else {
        return Ok(None);
    };
    let movie = edit.movie_timescale;
    let gap_scaled = media_end * movie - end_scaled;
    if gap_scaled <= i128::from(track.timebase.den) {
        return Ok(None);
    }
    if track.kind != TrackKind::Audio {
        return Err(refuse(format!(
            "the edit list of track {}: it ends before the last packet does, and Matroska trims \
             the end of audio only",
            track.id
        )));
    }
    let nanos = (gap_scaled % movie == 0)
        .then(|| nanos_of(gap_scaled / movie, track.timebase))
        .flatten()
        .and_then(|nanos| i64::try_from(nanos).ok());
    let ticks = i64::try_from(gap_scaled / movie).ok();
    let (Some(nanos), Some(ticks)) = (nanos, ticks) else {
        return Err(refuse(format!(
            "the edit list of track {}: its end trim of {gap_scaled} over {movie} ticks at {} \
             ticks per second is no whole number of nanoseconds",
            track.id, track.timebase.den
        )));
    };
    Ok(Some(EndTrim { ticks, nanos }))
}

/// Refuse a value with no whole number of nanoseconds, and name it.
fn exact_nanos(track: &Track, what: &str, value: i128) -> Result<i128> {
    nanos_of(value, track.timebase).ok_or_else(|| {
        refuse(format!(
            "{what} {value} of track {} at {}/{} seconds per tick: that time is no whole number \
             of nanoseconds, and `TimestampScale` counts whole nanoseconds. No scale holds it \
             exactly",
            track.id, track.timebase.num, track.timebase.den
        ))
    })
}

/// `CodecDelay` from the priming an audio edit drops.
fn set_codec_delay(track: &Track, edit: &EditPlan, spec: &mut TrackSpec) -> Result<()> {
    if edit.delay <= 0 {
        return Ok(());
    }
    let nanos = exact_nanos(track, "the edit list media time", i128::from(edit.delay))?;
    spec.matroska.codec_delay = u64::try_from(nanos).map_err(|_| {
        invalid(format!(
            "track {}: the `CodecDelay` of {nanos} ns exceeds 64 bits. Use a shorter edit",
            track.id
        ))
    })?;
    Ok(())
}

/// `DefaultDuration` when every block but the last lasts the same, in whole
/// nanoseconds. A track of one block stores no default.
fn uniform_duration(track: &Track, timing: &[Option<PacketTiming>]) -> Result<Option<u64>> {
    let mut blocks = timing.iter().flatten();
    let Some(first) = blocks.next() else {
        return Ok(None);
    };
    let mut rest = blocks.peekable();
    if rest.peek().is_none() {
        return Ok(None);
    }
    let Some(duration) = first.duration.filter(|&duration| duration > 0) else {
        return Ok(None);
    };
    // Every block but the last matches.
    while let Some(block) = rest.next() {
        if rest.peek().is_some() && block.duration != Some(duration) {
            return Ok(None);
        }
    }
    let nanos = exact_nanos(track, "the duration", i128::from(duration))?;
    Ok(u64::try_from(nanos).ok())
}

/// The largest divisor of 1 ms that holds every `pts` and duration of a track,
/// or the refusal of the first value no scale holds.
fn exact_scale(track: &Track, timing: &[Option<PacketTiming>]) -> Result<u64> {
    let mut scale = u128::from(PREFERRED_SCALE);
    for block in timing.iter().flatten() {
        let pts = exact_nanos(track, "the pts", i128::from(block.pts))?;
        scale = gcd(scale, pts.unsigned_abs());
        if let Some(duration) = block.duration {
            let nanos = exact_nanos(track, "the duration", i128::from(duration))?;
            scale = gcd(scale, nanos.unsigned_abs());
        }
    }
    Ok(u64::try_from(scale).unwrap_or(1))
}
