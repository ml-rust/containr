//! Raw frames to packets: track ids, sample indices, lace timestamps and
//! durations.

use crate::error::{Error, Result};
use crate::mkv::cluster::RawFrame;
use crate::packet::Packet;
use crate::track::Track;

/// Turn the frames of every Cluster into packets, in file order, and set
/// each track's `sample_count`.
///
/// A laced frame `k` of `n` presents `k / n` of the block duration after
/// the block timestamp. Durations come from `BlockDuration`, else from
/// `DefaultDuration` in nanoseconds over `timestamp_scale`, rounded down.
pub(crate) fn build_packets(
    frames: &[RawFrame],
    tracks: &mut [Track],
    timestamp_scale: u64,
) -> Result<Vec<Packet>> {
    let mut counts = vec![0u64; tracks.len()];
    let mut out = Vec::with_capacity(frames.len());
    for frame in frames {
        let slot = tracks
            .iter()
            .position(|track| u64::from(track.id) == frame.track)
            .ok_or_else(|| {
                Error::mkv_inconsistent(
                    format!("Segment/Cluster[{}]", frame.cluster),
                    format!(
                        "a block names track {}, which Tracks does not declare; it declares {:?}",
                        frame.track,
                        tracks.iter().map(|track| track.id).collect::<Vec<_>>()
                    ),
                )
            })?;
        let default_duration = tracks
            .get(slot)
            .and_then(|track| track.matroska.as_ref())
            .and_then(|mkv| mkv.default_duration);
        let (shift, duration) = lace_timing(frame, default_duration, timestamp_scale);
        let pts = frame.timestamp.checked_add(shift).ok_or_else(|| {
            Error::mkv_inconsistent(
                format!("Segment/Cluster[{}]", frame.cluster),
                format!(
                    "the block timestamp {} plus the lace offset {shift} overflows 64 bits",
                    frame.timestamp
                ),
            )
        })?;
        let index = counts.get_mut(slot).map_or(0, |count| {
            let index = *count;
            *count += 1;
            index
        });
        out.push(Packet {
            track_id: frame.track as u32,
            index,
            dts: None,
            pts,
            duration,
            sync: frame.keyframe,
            discardable: frame.discardable,
            discard_padding: frame.discard_padding,
            lace: frame.lace,
            offset: frame.offset,
            size: frame.size,
            description_index: 1,
        });
    }
    for (track, count) in tracks.iter_mut().zip(counts) {
        track.sample_count = count;
    }
    Ok(out)
}

/// The pts offset from the block timestamp and the duration of one frame,
/// in ticks.
fn lace_timing(frame: &RawFrame, default_duration: Option<u64>, scale: u64) -> (i64, Option<u64>) {
    let (index, count) = frame.lace.map_or((0u128, 1u128), |lace| {
        (u128::from(lace.index), u128::from(lace.count))
    });
    // The block span in ticks is `numerator / denominator`.
    let (numerator, denominator) = match (frame.block_duration, default_duration) {
        (Some(ticks), _) => (u128::from(ticks), count),
        (None, Some(nanoseconds)) => (u128::from(nanoseconds), u128::from(scale)),
        (None, None) => return (0, None),
    };
    let start = index * numerator / denominator;
    let end = (index + 1) * numerator / denominator;
    let shift = i64::try_from(start).unwrap_or(i64::MAX);
    (shift, u64::try_from(end - start).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::Lace;

    fn frame(lace: Option<Lace>, block_duration: Option<u64>) -> RawFrame {
        RawFrame {
            track: 1,
            timestamp: 100,
            keyframe: true,
            discardable: false,
            block_duration,
            discard_padding: None,
            lace,
            offset: 0,
            size: 1,
            cluster: 1,
        }
    }

    #[test]
    fn block_duration_splits_across_a_lace() {
        let laced = |index| frame(Some(Lace { index, count: 3 }), Some(10));
        let timing: Vec<_> = (0..3)
            .map(|index| lace_timing(&laced(index), None, 1_000_000))
            .collect();
        assert_eq!(timing, vec![(0, Some(3)), (3, Some(3)), (6, Some(4))]);
    }

    #[test]
    fn default_duration_counts_in_nanoseconds() {
        let laced = frame(Some(Lace { index: 2, count: 4 }), None);
        assert_eq!(
            lace_timing(&laced, Some(21_333_333), 1_000_000),
            (42, Some(21))
        );
        assert_eq!(lace_timing(&frame(None, None), None, 1_000_000), (0, None));
    }
}
