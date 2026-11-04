//! Cluster scans: every `SimpleBlock` and `BlockGroup`, expanded into one
//! frame per laced frame.
//!
//! The scan reads element headers and block headers only. A laced block is
//! read whole to decode its lace sizes. Frame bytes stay in the input.

use std::io::{Read, Seek};

use crate::error::{Error, Result};
use crate::mkv::block::{self, Lacing, MAX_BLOCK_HEADER_LEN, block_header};
use crate::mkv::ebml::{Element, Header, child_path, indexed_path};
use crate::mkv::ids;
use crate::mkv::source::Source;
use crate::packet::Lace;

/// The largest laced block read, in bytes. A laced block is read whole to
/// decode its lace header.
pub const MAX_LACED_BLOCK_SIZE: u64 = 16 << 20;
/// The most frames one input yields, across every track.
pub const MAX_FRAMES: usize = 1 << 24;

/// One frame as the Cluster stores it, before the track facts apply.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RawFrame {
    /// The track number the block names.
    pub track: u64,
    /// The Cluster `Timestamp` plus the block's relative timestamp.
    pub timestamp: i64,
    /// The keyframe flag, or no `ReferenceBlock` in a `BlockGroup`.
    pub keyframe: bool,
    /// The discardable bit of the block flags.
    pub discardable: bool,
    /// The `BlockDuration`, in ticks.
    pub block_duration: Option<u64>,
    /// The `DiscardPadding` in nanoseconds, on the frame it applies to.
    pub discard_padding: Option<i64>,
    /// The frame's place in its lace.
    pub lace: Option<Lace>,
    /// The byte offset of the frame data.
    pub offset: u64,
    /// The frame size in bytes.
    pub size: u32,
    /// The 1-based Cluster index, for error paths.
    pub cluster: u32,
}

/// Where a Cluster scan stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClusterEnd {
    /// The Cluster ends at this byte offset, where the next Segment child
    /// starts.
    At(u64),
    /// The input ends inside the Cluster. Frames before the cut are kept.
    Truncated,
}

/// The block-level facts a `BlockGroup` adds to its `Block`.
#[derive(Clone, Copy, Debug)]
struct BlockFacts {
    simple: bool,
    keyframe: bool,
    block_duration: Option<u64>,
    discard_padding: Option<i64>,
}

/// Scan the Cluster whose header sits at `pos`. `index` counts Clusters
/// from 1.
pub(crate) fn scan_cluster<R: Read + Seek>(
    src: &mut Source<'_, R>,
    pos: u64,
    header: Header,
    index: u32,
    frames: &mut Vec<RawFrame>,
) -> Result<ClusterEnd> {
    let path = indexed_path("Segment", ids::CLUSTER, index as usize);
    let body = pos + header.len;
    let declared_end = header.size.and_then(|size| body.checked_add(size));
    if header.size.is_some() && declared_end.is_none() {
        return Err(Error::mkv_malformed(
            &path,
            pos,
            "the Cluster size overflows 64 bits",
        ));
    }
    let end = declared_end.map_or(src.input_len, |end| end.min(src.input_len));
    let mut timestamp: Option<u64> = None;
    let mut counts = (0usize, 0usize);
    let mut at = body;
    while at < end {
        let Some(child) = src.header_at(at, &path)? else {
            return Ok(ClusterEnd::Truncated);
        };
        if header.size.is_none() && ids::ends_unknown_cluster(child.id) {
            return Ok(ClusterEnd::At(at));
        }
        let child_path = child_path(&path, child.id);
        let size = child.known_size(&child_path, at)?;
        let child_body = at + child.len;
        let child_end = child_body.saturating_add(size);
        if child_end > src.input_len {
            return Ok(ClusterEnd::Truncated);
        }
        if child_end > end {
            return Err(Error::mkv_malformed(
                &child_path,
                at,
                format!("the element declares {size} bytes, past the Cluster end at {end}"),
            ));
        }
        match child.id {
            ids::TIMESTAMP => {
                let bytes = src.read_body(child_body, size, &child_path, 8)?;
                timestamp = Some(leaf(at, child_body, &bytes).uint(&child_path)?);
            }
            ids::SIMPLE_BLOCK => {
                counts.0 += 1;
                let block_path = indexed_path(&path, ids::SIMPLE_BLOCK, counts.0);
                let cluster_ts = cluster_timestamp(timestamp, &path, at)?;
                let facts = BlockFacts {
                    simple: true,
                    keyframe: false,
                    block_duration: None,
                    discard_padding: None,
                };
                let at_block = (child_body, size, cluster_ts, index);
                read_block(src, &block_path, at_block, facts, frames)?;
            }
            ids::BLOCK_GROUP => {
                counts.1 += 1;
                let group_path = indexed_path(&path, ids::BLOCK_GROUP, counts.1);
                let cluster_ts = cluster_timestamp(timestamp, &path, at)?;
                let group = (child_body, child_end, cluster_ts, index);
                scan_block_group(src, &group_path, group, frames)?;
            }
            _ => {}
        }
        at = child_end;
    }
    Ok(match declared_end {
        Some(declared) if declared > src.input_len => ClusterEnd::Truncated,
        _ => ClusterEnd::At(end),
    })
}

/// The Cluster `Timestamp`, which must precede the Cluster's blocks.
fn cluster_timestamp(timestamp: Option<u64>, path: &str, at: u64) -> Result<u64> {
    timestamp.ok_or_else(|| {
        Error::mkv_malformed(
            path,
            at,
            "a block precedes the Cluster Timestamp, so its timestamp is unknown",
        )
    })
}

/// A leaf element over bytes already read.
fn leaf(offset: u64, body_offset: u64, body: &[u8]) -> Element<'_> {
    Element {
        id: 0,
        offset,
        body_offset,
        body,
    }
}

/// Scan one `BlockGroup`: (body offset, end offset, Cluster timestamp,
/// Cluster index).
fn scan_block_group<R: Read + Seek>(
    src: &mut Source<'_, R>,
    path: &str,
    (body, end, cluster_ts, cluster): (u64, u64, u64, u32),
    frames: &mut Vec<RawFrame>,
) -> Result<()> {
    let mut block: Option<(u64, u64)> = None;
    let mut facts = BlockFacts {
        simple: false,
        keyframe: true,
        block_duration: None,
        discard_padding: None,
    };
    let mut at = body;
    while at < end {
        let child = src.header_at(at, path)?.ok_or_else(|| {
            Error::mkv_malformed(path, at, "an element header runs past the BlockGroup end")
        })?;
        let child_path = child_path(path, child.id);
        let size = child.known_size(&child_path, at)?;
        let child_body = at + child.len;
        let child_end = child_body.saturating_add(size);
        if child_end > end {
            return Err(Error::mkv_malformed(
                &child_path,
                at,
                format!("the element declares {size} bytes, past the BlockGroup end at {end}"),
            ));
        }
        match child.id {
            ids::BLOCK if block.is_some() => {
                return Err(Error::mkv_malformed(
                    &child_path,
                    at,
                    "a second Block follows the first in one BlockGroup",
                ));
            }
            ids::BLOCK => block = Some((child_body, size)),
            ids::BLOCK_DURATION => {
                let bytes = src.read_body(child_body, size, &child_path, 8)?;
                facts.block_duration = Some(leaf(at, child_body, &bytes).uint(&child_path)?);
            }
            ids::DISCARD_PADDING => {
                let bytes = src.read_body(child_body, size, &child_path, 8)?;
                facts.discard_padding = Some(leaf(at, child_body, &bytes).int(&child_path)?);
            }
            ids::REFERENCE_BLOCK => facts.keyframe = false,
            _ => {}
        }
        at = child_end;
    }
    let (block_body, size) = block.ok_or_else(|| Error::mkv_missing(format!("{path}/Block")))?;
    let block_path = format!("{path}/Block");
    read_block(
        src,
        &block_path,
        (block_body, size, cluster_ts, cluster),
        facts,
        frames,
    )
}

/// Read one block: (body offset, body size, Cluster timestamp, Cluster
/// index). Pushes one frame per laced frame.
fn read_block<R: Read + Seek>(
    src: &mut Source<'_, R>,
    path: &str,
    (body, size, cluster_ts, cluster): (u64, u64, u64, u32),
    facts: BlockFacts,
    frames: &mut Vec<RawFrame>,
) -> Result<()> {
    let malformed = |reason: String| Error::mkv_malformed(path, body, reason);
    let mut buf = [0u8; MAX_BLOCK_HEADER_LEN];
    let take = size.min(MAX_BLOCK_HEADER_LEN as u64) as usize;
    let prefix = src.peek(
        body,
        buf.get_mut(..take).unwrap_or_default(),
        &format!("block `{path}`"),
    )?;
    let header = block_header(prefix)
        .map_err(malformed)?
        .ok_or_else(|| malformed("the block ends inside its header".to_owned()))?;
    let timestamp = i64::try_from(cluster_ts)
        .ok()
        .and_then(|ts| ts.checked_add(i64::from(header.relative)))
        .ok_or_else(|| {
            malformed(format!(
                "the Cluster Timestamp {cluster_ts} plus {} overflows 63 bits",
                header.relative
            ))
        })?;
    let frame = RawFrame {
        track: header.track,
        timestamp,
        keyframe: if facts.simple {
            header.keyframe()
        } else {
            facts.keyframe
        },
        discardable: header.discardable(),
        block_duration: facts.block_duration,
        discard_padding: facts.discard_padding,
        lace: None,
        offset: body + header.len as u64,
        size: 0,
        cluster,
    };
    let lacing = header.lacing();
    if lacing == Lacing::None {
        let data = size - header.len as u64;
        return push(
            frames,
            path,
            RawFrame {
                size: frame_size(data, path)?,
                ..frame
            },
        );
    }
    let bytes = src.read_body(body, size, path, MAX_LACED_BLOCK_SIZE)?;
    let data = bytes.get(header.len..).unwrap_or_default();
    let lace = block::lace(lacing, data).map_err(malformed)?;
    let count = lace.sizes.len();
    let mut offset = frame.offset + lace.header_len as u64;
    for (index, &data) in lace.sizes.iter().enumerate() {
        let last = index + 1 == count;
        let padding = facts
            .discard_padding
            .filter(|&padding| if padding < 0 { index == 0 } else { last });
        let laced = RawFrame {
            discard_padding: padding,
            lace: Some(Lace {
                index: index as u16,
                count: count as u16,
            }),
            offset,
            size: frame_size(data, path)?,
            ..frame
        };
        push(frames, path, laced)?;
        offset += data;
    }
    Ok(())
}

fn frame_size(size: u64, path: &str) -> Result<u32> {
    u32::try_from(size).map_err(|_| Error::mkv_too_large(path, size, u64::from(u32::MAX)))
}

fn push(frames: &mut Vec<RawFrame>, path: &str, frame: RawFrame) -> Result<()> {
    if frames.len() >= MAX_FRAMES {
        return Err(Error::mkv_too_large(
            path,
            frames.len() as u64 + 1,
            MAX_FRAMES as u64,
        ));
    }
    frames.push(frame);
    Ok(())
}
