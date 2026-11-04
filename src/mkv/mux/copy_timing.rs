//! The timing rules of an exact stream copy: edit lists, end trims, and the scale.

use crate::error::{Error, Result};
use crate::mkv::mux::bytes::{invalid, refuse};
use crate::mkv::mux::copy::{PREFERRED_SCALE, PacketTiming};
use crate::mkv::mux::mp4_edit::EditPlan;
use crate::mkv::mux::spec::TrackSpec;
use crate::mkv::mux::timing::{gcd, nanos_of};
use crate::packet::Packet;
use crate::track::{Track, TrackKind};

pub(super) fn matroska_timing(
    track: &Track,
    packets: &[Packet],
) -> Result<Vec<Option<PacketTiming>>> {
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

pub(super) fn negative(track: &Track, index: u64, pts: i64) -> Error {
    refuse(format!(
        "packet {index} of track {track} at pts {pts}: a Matroska block timestamp is never \
         negative, and an MP4 edit list holds a frame the edit cuts",
        track = track.id
    ))
}

pub(super) fn mp4_timing(
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
    let Some(gap_scaled) = end_gap(track, edit, media_end)? else {
        return Ok(None);
    };
    let movie = edit.movie_timescale;
    let nanos = (gap_scaled % movie == 0)
        .then(|| nanos_of(gap_scaled / movie, track.timebase))
        .flatten()
        .and_then(|nanos| i64::try_from(nanos).ok());
    let ticks = i64::try_from(gap_scaled / movie).ok();
    let (Some(nanos), Some(ticks)) = (nanos, ticks) else {
        return Err(refuse(format!(
            "the edit list of track {}: its end trim of {gap_scaled} over {movie} ticks at {} \
             ticks per second is no whole number of nanoseconds. `StreamCopy::plan_rounded` \
             rounds it",
            track.id, track.timebase.den
        )));
    };
    Ok(Some(EndTrim { ticks, nanos }))
}

/// The media an edit cuts from the end, in track ticks as `gap / movie
/// timescale`. `None` when it cuts nothing. Matroska trims the end of audio
/// only, so any other track is refused.
pub(super) fn end_gap(track: &Track, edit: &EditPlan, media_end: i128) -> Result<Option<i128>> {
    let Some(end_scaled) = edit.end_scaled else {
        return Ok(None);
    };
    let gap_scaled = media_end * edit.movie_timescale - end_scaled;
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
    Ok(Some(gap_scaled))
}

/// Refuse a value with no whole number of nanoseconds, and name it.
fn exact_nanos(track: &Track, what: &str, value: i128) -> Result<i128> {
    nanos_of(value, track.timebase).ok_or_else(|| {
        refuse(format!(
            "{what} {value} of track {} at {}/{} seconds per tick: that time is no whole number \
             of nanoseconds, and `TimestampScale` counts whole nanoseconds. No scale holds it \
             exactly. To round it to the nearest tick, use `StreamCopy::plan_rounded`",
            track.id, track.timebase.num, track.timebase.den
        ))
    })
}

/// `CodecDelay` from the priming an audio edit drops.
pub(super) fn set_codec_delay(track: &Track, edit: &EditPlan, spec: &mut TrackSpec) -> Result<()> {
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
pub(super) fn uniform_duration(
    track: &Track,
    timing: &[Option<PacketTiming>],
) -> Result<Option<u64>> {
    let Some(duration) = uniform_span(timing.iter().flatten().map(|block| block.duration)) else {
        return Ok(None);
    };
    let nanos = exact_nanos(track, "the duration", i128::from(duration))?;
    Ok(u64::try_from(nanos).ok())
}

/// The duration every block but the last shares, in source ticks. `None` for
/// a track of one block or none, or for a zero or a differing duration.
pub(super) fn uniform_span(durations: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    let mut durations = durations.peekable();
    let first = durations.next()?;
    durations.peek()?;
    let duration = first.filter(|&duration| duration > 0)?;
    // Every block but the last matches.
    while let Some(next) = durations.next() {
        if durations.peek().is_some() && next != Some(duration) {
            return None;
        }
    }
    Some(duration)
}

/// The largest divisor of 1 ms that holds every `pts` and duration of a track,
/// or the refusal of the first value no scale holds.
pub(super) fn exact_scale(track: &Track, timing: &[Option<PacketTiming>]) -> Result<u64> {
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
