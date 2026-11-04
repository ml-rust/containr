//! Rounded MP4 timing: the opt-in for times Matroska cannot hold exactly.
//!
//! An exact time here is a rational: a count over `timescale * movie timescale`
//! per second, so an edit's start delay stays exact. Each time rounds once,
//! from that value, so no error accumulates. A block's duration is the distance
//! between two rounded times, so durations sum to the rounded end.

use crate::error::{Error, Result};
use crate::mkv::mux::bytes::{invalid, refuse};
use crate::mkv::mux::copy::PacketTiming;
use crate::mkv::mux::copy_timing::{end_gap, negative, uniform_span};
use crate::mkv::mux::mp4_edit::{EditPlan, rounded_edit_plan};
use crate::mkv::mux::timing::round_time;
use crate::packet::Packet;
use crate::track::Track;

/// What [`StreamCopy::plan_rounded`](crate::mkv::StreamCopy::plan_rounded) did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TimestampRounding {
    /// The `TimestampScale` the plan rounds to, in nanoseconds per tick.
    pub scale: u64,
    /// The largest absolute error of any stored time, in nanoseconds, rounded
    /// up. It covers `pts`, the track's end, `CodecDelay`, `DiscardPadding` and
    /// `DefaultDuration`. 0 means every value is exact.
    pub max_error_ns: u64,
}

/// The rounded facts of one track.
pub(crate) struct RoundedPlan {
    pub timing: Vec<Option<PacketTiming>>,
    pub codec_delay: u64,
    pub default_duration: Option<u64>,
    pub rounding: TimestampRounding,
}

/// One source sample on the exact time line.
struct Sample {
    /// Its position in decode order.
    slot: usize,
    /// Its start and end, over the clock's `per_second`.
    start: i128,
    end: i128,
    /// False for an empty `tx3g` sample, which writes no block.
    block: bool,
}

/// Rounds times for one track, and keeps the largest error.
struct Clock<'a> {
    track: &'a Track,
    /// Counts per second of [`Sample`] times.
    per_second: i128,
    scale: i128,
    max_error_ns: u64,
}

impl Clock<'_> {
    fn overflow(&self, what: &str, value: i128) -> Error {
        invalid(format!(
            "track {}: the {what} {value} over {} per second overflows while it rounds. Use a \
             coarser `scale`, or a shorter track",
            self.track.id, self.per_second
        ))
    }

    /// `value / per_second` seconds in nanoseconds per `unit`.
    fn round(&mut self, what: &str, value: i128, per_second: i128, unit: i128) -> Result<i128> {
        let rounded =
            round_time(value, per_second, unit).ok_or_else(|| self.overflow(what, value))?;
        self.max_error_ns = self.max_error_ns.max(rounded.error_ns);
        Ok(rounded.count)
    }

    /// A [`Sample`] time in output ticks.
    fn tick(&mut self, what: &str, value: i128) -> Result<i64> {
        let count = self.round(what, value, self.per_second, self.scale)?;
        i64::try_from(count).map_err(|_| self.overflow(what, value))
    }

    /// A span of track ticks as a whole number of nanoseconds, 0 or more.
    fn span_nanos(&mut self, what: &str, value: i128, per_second: i128) -> Result<u64> {
        let count = self.round(what, value, per_second, 1)?;
        u64::try_from(count).map_err(|_| self.overflow(what, value))
    }
}

/// Lay the samples out on the exact time line, sorted by presentation time.
/// Also returns the decode slot of the last block, and the media end in track
/// ticks.
fn samples_of(
    track: &Track,
    framed_text: bool,
    packets: &[Packet],
    edit: &EditPlan,
    movie: i128,
) -> Result<(Vec<Sample>, Option<usize>, i128)> {
    let mut samples = Vec::with_capacity(packets.len());
    let mut last = None;
    let mut media_end = 0i128;
    for (slot, packet) in packets.iter().enumerate() {
        if packet.description_index != 1 {
            return Err(Error::SampleDescriptionChange {
                track_id: track.id,
                descriptions: u64::from(packet.description_index),
            });
        }
        let start = i128::from(packet.pts) * movie + edit.shift_scaled;
        let span = i128::from(packet.duration.unwrap_or(0));
        let block = !(framed_text && packet.size == 2);
        if block {
            if start < 0 {
                return Err(negative(track, packet.index, packet.pts));
            }
            media_end = media_end.max(i128::from(packet.pts) + span);
            last = Some(slot);
        }
        samples.push(Sample {
            slot,
            start,
            end: start + span * movie,
            block,
        });
    }
    samples.sort_by_key(|sample| (sample.start, sample.slot));
    Ok((samples, last, media_end))
}

/// The refusal of a block that rounds onto the tick of the next sample.
fn collapse(clock: &Clock<'_>, block: &Sample, next: &Sample, tick: i64) -> Error {
    let track = clock.track.id;
    if block.start == next.start {
        return refuse(format!(
            "packets {} and {} of track {track} at one source `pts`: \
             `StreamCopy::plan_rounded` takes a duration from the next `pts`, so the first \
             lasts nothing",
            block.slot, next.slot
        ));
    }
    let gap = (next.start - block.start)
        .saturating_mul(1_000_000_000)
        .div_euclid(clock.per_second);
    let finer = gap.min(clock.scale - 1).max(1);
    refuse(format!(
        "packets {} and {} of track {track} on one tick: both round to tick {tick} at scale {}. \
         Pass `StreamCopy::plan_rounded` a scale of {finer} or less",
        block.slot, next.slot, clock.scale
    ))
}

/// `DiscardPadding` in nanoseconds for what the edit cuts from the last block.
fn padding_of(
    clock: &mut Clock<'_>,
    edit: &EditPlan,
    packet: &Packet,
    media_end: i128,
    movie: i128,
) -> Result<Option<i64>> {
    let track = clock.track;
    let Some(gap) = end_gap(track, edit, media_end)? else {
        return Ok(None);
    };
    if gap > i128::from(packet.duration.unwrap_or(0)) * movie {
        return Err(refuse(format!(
            "the edit list of track {} cuts {gap} over {movie} ticks from the end, more than its \
             last packet lasts: `DiscardPadding` trims one block",
            track.id
        )));
    }
    let nanos = clock.span_nanos("end trim", gap, clock.per_second)?;
    let nanos = i64::try_from(nanos).map_err(|_| clock.overflow("end trim", gap))?;
    Ok(Some(nanos))
}

/// Round every time of an MP4 track to `scale` nanoseconds per tick.
pub(crate) fn round_mp4(
    track: &Track,
    framed_text: bool,
    packets: &[Packet],
    scale: u32,
) -> Result<RoundedPlan> {
    if track.timebase.den == 0 {
        return Err(invalid(format!("track {}: the timescale is 0", track.id)));
    }
    let edit = rounded_edit_plan(track)?;
    let movie = edit.movie_timescale.max(1);
    let timescale = i128::from(track.timebase.den);
    let mut clock = Clock {
        track,
        per_second: timescale * movie,
        scale: i128::from(scale),
        max_error_ns: 0,
    };
    let (samples, last, media_end) = samples_of(track, framed_text, packets, &edit, movie)?;
    let ticks = samples
        .iter()
        .map(|sample| clock.tick("pts", sample.start))
        .collect::<Result<Vec<i64>>>()?;

    let mut timing: Vec<Option<PacketTiming>> = vec![None; packets.len()];
    for (n, (sample, &tick)) in samples.iter().zip(&ticks).enumerate() {
        if !sample.block {
            continue;
        }
        let end = match (samples.get(n + 1), ticks.get(n + 1)) {
            (Some(next), Some(&next_tick)) => {
                if next_tick == tick {
                    return Err(collapse(&clock, sample, next, tick));
                }
                next_tick
            }
            _ => clock.tick("end", sample.end)?,
        };
        if let Some(slot) = timing.get_mut(sample.slot) {
            *slot = Some(PacketTiming {
                pts: tick,
                duration: Some(end.saturating_sub(tick).unsigned_abs()),
                discard_padding: None,
            });
        }
    }

    if let Some(last) = last
        && let Some(packet) = packets.get(last)
        && let Some(padding) = padding_of(&mut clock, &edit, packet, media_end, movie)?
        && let Some(Some(block)) = timing.get_mut(last)
    {
        block.discard_padding = Some(padding);
    }
    let codec_delay = if edit.delay > 0 {
        clock.span_nanos("`CodecDelay`", i128::from(edit.delay), timescale)?
    } else {
        0
    };
    let durations = packets.iter().map(|packet| packet.duration);
    let default_duration = match uniform_span(durations).filter(|_| !framed_text) {
        Some(span) => {
            let nanos = clock.span_nanos("`DefaultDuration`", i128::from(span), timescale)?;
            Some(nanos).filter(|&nanos| nanos > 0)
        }
        None => None,
    };
    Ok(RoundedPlan {
        timing,
        codec_delay,
        default_duration,
        rounding: TimestampRounding {
            scale: u64::from(scale),
            max_error_ns: clock.max_error_ns,
        },
    })
}
