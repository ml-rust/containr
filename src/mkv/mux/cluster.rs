//! One Cluster in memory: its `Timestamp` and blocks.
//!
//! A block stores its time as a 16-bit offset from the Cluster `Timestamp`,
//! so [`OpenCluster::accepts`] closes the Cluster before an offset passes
//! `i16`. A block that needs a `BlockDuration` or `DiscardPadding` is a
//! `BlockGroup`. Any other block is a `SimpleBlock`.

use crate::error::Result;
use crate::mkv::ebml_write::{ElementBuf, push_element, push_id, push_size};
use crate::mkv::ids;
use crate::mkv::mux::bytes::invalid;
use crate::mkv::mux::track::Block;

const KEYFRAME: u8 = 0x80;
const DISCARDABLE: u8 = 0x01;

/// The Cluster being filled.
#[derive(Debug)]
pub(crate) struct OpenCluster {
    /// The byte offset of the Cluster header in the output.
    pub start: u64,
    pub timestamp: i64,
    pub body: Vec<u8>,
    /// The track numbers with a cue point in this Cluster.
    pub cued: Vec<u32>,
}

impl OpenCluster {
    /// A Cluster at offset `start`, with timestamp `timestamp`, 0 or more.
    pub fn new(start: u64, timestamp: i64) -> Result<Self> {
        let mut head = ElementBuf::new();
        head.uint(ids::TIMESTAMP, timestamp.unsigned_abs())?;
        Ok(Self {
            start,
            timestamp,
            body: head.into_bytes(),
            cued: Vec::new(),
        })
    }

    /// The offset of a block at `ticks`, when it fits this Cluster's `i16`.
    fn offset(&self, ticks: i64) -> Option<i16> {
        ticks
            .checked_sub(self.timestamp)
            .and_then(|offset| i16::try_from(offset).ok())
    }

    /// True when a block at `ticks` belongs in this Cluster: its offset fits,
    /// and neither Cluster limit is reached.
    pub fn accepts(&self, ticks: i64, max_ticks: i64, max_bytes: usize) -> bool {
        self.offset(ticks).is_some()
            && ticks.saturating_sub(self.timestamp) < max_ticks
            && self.body.len() < max_bytes
    }

    /// Append `block` with the given `BlockDuration` and `ReferenceBlock`.
    /// Returns the block's byte offset inside the Cluster body. The caller
    /// checks [`OpenCluster::accepts`] first. A block outside the `i16` range
    /// gives an error.
    pub fn append(
        &mut self,
        number: u32,
        block: &Block,
        duration: Option<u64>,
        reference: Option<i64>,
    ) -> Result<u64> {
        let at = self.body.len() as u64;
        let Some(offset) = self.offset(block.ticks) else {
            return Err(invalid(format!(
                "a block at tick {} is too far from the Cluster at tick {}. The muxer starts a \
                 new Cluster first, so report this as a containr bug",
                block.ticks, self.timestamp
            )));
        };
        let simple = duration.is_none() && block.discard_padding.is_none();
        let mut flags = 0u8;
        if block.discardable {
            flags |= DISCARDABLE;
        }
        if simple && block.sync {
            flags |= KEYFRAME;
        }
        let head = block_head(number, offset, flags);
        if simple {
            push_element(
                &mut self.body,
                ids::SIMPLE_BLOCK,
                &[head.as_slice(), block.data.as_slice()],
            )?;
            return Ok(at);
        }
        let mut facts = ElementBuf::new();
        if let Some(duration) = duration {
            facts.uint(ids::BLOCK_DURATION, duration)?;
        }
        if let Some(reference) = reference {
            facts.int(ids::REFERENCE_BLOCK, reference)?;
        }
        if let Some(padding) = block.discard_padding {
            facts.int(ids::DISCARD_PADDING, padding)?;
        }
        let block_len = (head.len() + block.data.len()) as u64;
        let mut block_header = Vec::with_capacity(12);
        push_id(&mut block_header, ids::BLOCK);
        push_size(&mut block_header, ids::BLOCK, block_len)?;
        let group_len = block_header.len() as u64 + block_len + facts.as_bytes().len() as u64;
        let mut group_header = Vec::with_capacity(12);
        push_id(&mut group_header, ids::BLOCK_GROUP);
        push_size(&mut group_header, ids::BLOCK_GROUP, group_len)?;
        self.body.extend_from_slice(&group_header);
        self.body.extend_from_slice(&block_header);
        self.body.extend_from_slice(&head);
        self.body.extend_from_slice(&block.data);
        self.body.extend_from_slice(facts.as_bytes());
        Ok(at)
    }

    /// The Cluster header, for the body in `self.body`.
    pub fn header(&self) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(12);
        push_id(&mut out, ids::CLUSTER);
        push_size(&mut out, ids::CLUSTER, self.body.len() as u64)?;
        Ok(out)
    }
}

/// A block header: the track number, the 16-bit offset and the flags.
fn block_head(number: u32, offset: i16, flags: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(5);
    if number < 127 {
        out.push(0x80 | number as u8);
    } else {
        out.push(0x40 | ((number >> 8) & 0x3F) as u8);
        out.push((number & 0xFF) as u8);
    }
    out.extend_from_slice(&offset.to_be_bytes());
    out.push(flags);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mkv::block::block_header;

    fn block(ticks: i64) -> Block {
        Block {
            data: vec![9; 4],
            ticks,
            key: ticks,
            duration: None,
            sync: true,
            discardable: false,
            discard_padding: None,
        }
    }

    #[test]
    fn a_block_offset_never_passes_i16() {
        let cluster = OpenCluster::new(0, 40_000).expect("the timestamp fits");
        assert!(cluster.accepts(40_000 + 32_767, i64::MAX, usize::MAX));
        assert!(!cluster.accepts(40_000 + 32_768, i64::MAX, usize::MAX));
        assert!(cluster.accepts(40_000 - 32_768, i64::MAX, usize::MAX));
        assert!(!cluster.accepts(40_000 - 32_769, i64::MAX, usize::MAX));
    }

    #[test]
    fn cluster_limits_close_the_cluster() {
        let cluster = OpenCluster::new(0, 0).expect("the timestamp fits");
        assert!(!cluster.accepts(5000, 5000, usize::MAX));
        assert!(cluster.accepts(4999, 5000, usize::MAX));
        assert!(!cluster.accepts(0, 5000, cluster.body.len()));
    }

    #[test]
    fn block_heads_read_back() {
        for number in [1u32, 126, 127, 1024] {
            let head = block_head(number, -2, 0x81);
            let parsed = block_header(&head).ok().flatten().expect("a block header");
            assert_eq!(parsed.track, u64::from(number));
            assert_eq!(parsed.relative, -2);
            assert!(parsed.keyframe() && parsed.discardable());
            assert_eq!(parsed.len, head.len());
        }
    }

    #[test]
    fn a_duration_or_padding_makes_a_block_group() {
        let mut cluster = OpenCluster::new(0, 0).expect("the timestamp fits");
        let start = cluster.body.len();
        cluster
            .append(1, &block(1), None, None)
            .expect("the block fits");
        assert_eq!(cluster.body[start], ids::SIMPLE_BLOCK as u8);
        let group_at = cluster
            .append(1, &block(2), Some(5), None)
            .expect("the block fits");
        assert_eq!(cluster.body[group_at as usize], ids::BLOCK_GROUP as u8);
        let mut padded = block(3);
        padded.discard_padding = Some(-1);
        let padded_at = cluster
            .append(1, &padded, None, None)
            .expect("the block fits");
        assert_eq!(cluster.body[padded_at as usize], ids::BLOCK_GROUP as u8);
    }
}
