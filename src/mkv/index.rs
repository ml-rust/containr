//! The index elements: `SeekHead` entries and `Cues`.

use crate::error::{Error, Result};
use crate::mkv::ebml::{Element, child_path, indexed_path};
use crate::mkv::ids;

/// One `Seek` entry of a `SeekHead`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SeekEntry {
    /// The `SeekID`: the ID of the element it locates, marker bits
    /// included.
    pub id: u32,
    /// The byte offset of that element in the input: the `SeekPosition`
    /// plus the Segment data start.
    pub offset: u64,
}

/// One `CueTrackPositions` entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CuePosition {
    /// `CueTrack`: the track number.
    pub track: u64,
    /// The byte offset of the Cluster in the input: the
    /// `CueClusterPosition` plus the Segment data start.
    pub cluster_offset: u64,
    /// `CueRelativePosition`: the block offset inside the Cluster body.
    pub relative_position: Option<u64>,
    /// `CueDuration`, in ticks.
    pub duration: Option<u64>,
    /// `CueBlockNumber`, counted from 1.
    pub block_number: Option<u64>,
}

/// One `CuePoint`: a seek target.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CuePoint {
    /// `CueTime`, in ticks.
    pub time: u64,
    /// Where each track's block for this time sits.
    pub positions: Vec<CuePosition>,
}

/// The absolute offset of a Segment-relative position.
fn absolute(segment_data: u64, position: u64, path: &str, offset: u64) -> Result<u64> {
    segment_data.checked_add(position).ok_or_else(|| {
        Error::mkv_malformed(
            path,
            offset,
            format!("the position {position} overflows 64 bits"),
        )
    })
}

/// Parse one `SeekHead` body. `segment_data` is the Segment body offset.
pub(crate) fn parse_seek_head(
    seek_head: &Element<'_>,
    index: usize,
    segment_data: u64,
) -> Result<Vec<SeekEntry>> {
    let path = indexed_path("Segment", ids::SEEK_HEAD, index);
    let mut out = Vec::new();
    for seek in seek_head.children(&path) {
        let seek = seek?;
        if seek.id != ids::SEEK {
            continue;
        }
        let seek_path = indexed_path(&path, ids::SEEK, out.len() + 1);
        let mut id = None;
        let mut position = None;
        for child in seek.children(&seek_path) {
            let child = child?;
            let at = child_path(&seek_path, child.id);
            match child.id {
                ids::SEEK_ID => {
                    let raw = child.uint(&at)?;
                    id = Some(u32::try_from(raw).map_err(|_| {
                        Error::mkv_malformed(
                            &at,
                            child.offset,
                            format!("SeekID {raw:#X} is longer than 4 bytes"),
                        )
                    })?);
                }
                ids::SEEK_POSITION => position = Some(child.uint(&at)?),
                _ => {}
            }
        }
        let id = id.ok_or_else(|| Error::mkv_missing(format!("{seek_path}/SeekID")))?;
        let position =
            position.ok_or_else(|| Error::mkv_missing(format!("{seek_path}/SeekPosition")))?;
        out.push(SeekEntry {
            id,
            offset: absolute(segment_data, position, &seek_path, seek.offset)?,
        });
    }
    Ok(out)
}

/// Parse the `Cues` body. `segment_data` is the Segment body offset.
pub(crate) fn parse_cues(cues: &Element<'_>, segment_data: u64) -> Result<Vec<CuePoint>> {
    let path = "Segment/Cues";
    let mut out = Vec::new();
    for point in cues.children(path) {
        let point = point?;
        if point.id != ids::CUE_POINT {
            continue;
        }
        let point_path = indexed_path(path, ids::CUE_POINT, out.len() + 1);
        let mut time = None;
        let mut positions = Vec::new();
        for child in point.children(&point_path) {
            let child = child?;
            let at = child_path(&point_path, child.id);
            match child.id {
                ids::CUE_TIME => time = Some(child.uint(&at)?),
                ids::CUE_TRACK_POSITIONS => {
                    positions.push(parse_position(&child, &at, segment_data)?);
                }
                _ => {}
            }
        }
        let time = time.ok_or_else(|| Error::mkv_missing(format!("{point_path}/CueTime")))?;
        out.push(CuePoint { time, positions });
    }
    Ok(out)
}

fn parse_position(positions: &Element<'_>, path: &str, segment_data: u64) -> Result<CuePosition> {
    let mut track = None;
    let mut cluster = None;
    let mut out = CuePosition {
        track: 0,
        cluster_offset: 0,
        relative_position: None,
        duration: None,
        block_number: None,
    };
    for child in positions.children(path) {
        let child = child?;
        let at = child_path(path, child.id);
        match child.id {
            ids::CUE_TRACK => track = Some(child.uint(&at)?),
            ids::CUE_CLUSTER_POSITION => cluster = Some(child.uint(&at)?),
            ids::CUE_RELATIVE_POSITION => out.relative_position = Some(child.uint(&at)?),
            ids::CUE_DURATION => out.duration = Some(child.uint(&at)?),
            ids::CUE_BLOCK_NUMBER => out.block_number = Some(child.uint(&at)?),
            _ => {}
        }
    }
    out.track = track.ok_or_else(|| Error::mkv_missing(format!("{path}/CueTrack")))?;
    let cluster =
        cluster.ok_or_else(|| Error::mkv_missing(format!("{path}/CueClusterPosition")))?;
    out.cluster_offset = absolute(segment_data, cluster, path, positions.offset)?;
    Ok(out)
}
