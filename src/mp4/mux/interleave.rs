//! The chunk write order: chunks of all tracks interleave by decode time.
//!
//! A track's chunk is written once no other track can still produce a chunk
//! that decodes before it: every other track has a queued chunk, or a last
//! packet, at or after its start. A track that has sent no packet yet
//! holds the others back, until the buffered bytes pass
//! [`MuxOptions::max_buffer`](crate::mp4::MuxOptions::max_buffer). Past that
//! bound the earliest chunk is written anyway.

use std::cmp::Ordering;

use crate::mp4::mux::track::TrackState;

/// Compare two decode times in different timescales, exactly.
fn compare(a: (i64, u32), b: (i64, u32)) -> Ordering {
    (i128::from(a.0) * i128::from(b.1)).cmp(&(i128::from(b.0) * i128::from(a.1)))
}

/// The slot of the track whose first queued chunk decodes first.
fn earliest_closed(tracks: &[TrackState]) -> Option<(usize, (i64, u32))> {
    let mut best: Option<(usize, (i64, u32))> = None;
    for (slot, track) in tracks.iter().enumerate() {
        let Some(chunk) = track.closed.front() else {
            continue;
        };
        let time = (chunk.first_dts, track.spec.timescale);
        if best.is_none_or(|(_, best_time)| compare(time, best_time) == Ordering::Less) {
            best = Some((slot, time));
        }
    }
    best
}

/// The slot of the next chunk to write. With `force`, the earliest queued
/// chunk, whatever the other tracks hold.
pub(crate) fn next_chunk(tracks: &[TrackState], force: bool) -> Option<usize> {
    let (slot, time) = earliest_closed(tracks)?;
    if force {
        return Some(slot);
    }
    for (other, track) in tracks.iter().enumerate() {
        if other == slot {
            continue;
        }
        let bound = track.lower_bound()?;
        if compare((bound, track.spec.timescale), time) == Ordering::Less {
            return None;
        }
    }
    Some(slot)
}

/// The slot of the track whose open chunk decodes first.
pub(crate) fn earliest_open(tracks: &[TrackState]) -> Option<usize> {
    let mut best: Option<(usize, (i64, u32))> = None;
    for (slot, track) in tracks.iter().enumerate() {
        let Some(chunk) = &track.open else {
            continue;
        };
        let time = (chunk.first_dts, track.spec.timescale);
        if best.is_none_or(|(_, best_time)| compare(time, best_time) == Ordering::Less) {
            best = Some((slot, time));
        }
    }
    best.map(|(slot, _)| slot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_is_exact_across_timescales() {
        assert_eq!(compare((1, 3), (333_333, 1_000_000)), Ordering::Greater);
        assert_eq!(compare((1, 2), (500, 1000)), Ordering::Equal);
        assert_eq!(compare((-1, 2), (0, 1000)), Ordering::Less);
    }
}
