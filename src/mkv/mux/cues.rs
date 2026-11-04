//! The `Cues` element: one point per seek target.
//!
//! Each keyframe of a video track gets a point. A file with no video track
//! gets one point per track per Cluster, at the track's first keyframe there.

use crate::error::Result;
use crate::mkv::ebml_write::ElementBuf;
use crate::mkv::ids;

/// The most cue points written. Past it the output stays valid and its index
/// stops growing, so the `Cues` element stays below what the demuxer reads
/// whole.
pub const MAX_CUE_POINTS: usize = 1 << 22;

/// One seek target.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cue {
    pub time: u64,
    pub track: u32,
    /// The Cluster offset from the Segment body.
    pub cluster: u64,
    /// The block offset inside the Cluster body.
    pub relative: u64,
}

/// The `Cues` body. Points come in time order, with blocks of one time in
/// one point.
pub(crate) fn cues_body(cues: &mut [Cue]) -> Result<ElementBuf> {
    cues.sort_by_key(|cue| cue.time);
    let mut body = ElementBuf::new();
    for group in cues.chunk_by(|a, b| a.time == b.time) {
        let Some(first) = group.first() else {
            continue;
        };
        let mut point = ElementBuf::new();
        point.uint(ids::CUE_TIME, first.time)?;
        for cue in group {
            let mut position = ElementBuf::new();
            position.uint(ids::CUE_TRACK, u64::from(cue.track))?;
            position.uint(ids::CUE_CLUSTER_POSITION, cue.cluster)?;
            position.uint(ids::CUE_RELATIVE_POSITION, cue.relative)?;
            point.master(ids::CUE_TRACK_POSITIONS, &position)?;
        }
        body.master(ids::CUE_POINT, &point)?;
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mkv::ebml::Element;
    use crate::mkv::index::parse_cues;

    #[test]
    fn points_sort_by_time_and_share_one_time() {
        let cue = |time, track| Cue {
            time,
            track,
            cluster: 5,
            relative: 7,
        };
        let mut cues = [cue(20, 1), cue(0, 1), cue(20, 2)];
        let body = cues_body(&mut cues).expect("the cues fit").into_bytes();
        let element = Element {
            id: ids::CUES,
            offset: 0,
            body_offset: 0,
            body: &body,
        };
        let points = parse_cues(&element, 100).expect("the cues read back");
        assert_eq!(points.len(), 2);
        assert_eq!(points[0].time, 0);
        assert_eq!(points[1].time, 20);
        assert_eq!(points[1].positions.len(), 2);
        assert_eq!(points[1].positions[1].cluster_offset, 105);
        assert_eq!(points[1].positions[1].relative_position, Some(7));
    }
}
