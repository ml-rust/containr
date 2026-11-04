//! Block headers and lacing: the track number, relative timestamp and
//! flags of a `SimpleBlock` or `Block`, and the frame sizes of a lace.

use crate::mkv::ebml::{Vint, vint};

/// The longest block header this reader needs to see: an 8-byte track
/// number, a 2-byte timestamp and the flags byte.
pub(crate) const MAX_BLOCK_HEADER_LEN: usize = 11;

/// The lacing a block's flags select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lacing {
    /// One frame, no lace header.
    None,
    /// Xiph lacing: sizes as runs of 255.
    Xiph,
    /// Fixed-size lacing: equal frames.
    Fixed,
    /// EBML lacing: a first size, then signed differences.
    Ebml,
}

/// The fixed part of a block: what precedes the frame data or lace header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BlockHeader {
    /// The track number.
    pub track: u64,
    /// The timestamp relative to the Cluster `Timestamp`, in ticks.
    pub relative: i16,
    /// The flags byte.
    pub flags: u8,
    /// The header length in bytes.
    pub len: usize,
}

impl BlockHeader {
    /// The lacing bits 1 and 2 select.
    pub fn lacing(&self) -> Lacing {
        match (self.flags >> 1) & 0b11 {
            0 => Lacing::None,
            1 => Lacing::Xiph,
            2 => Lacing::Fixed,
            _ => Lacing::Ebml,
        }
    }

    /// The `SimpleBlock` keyframe flag.
    pub fn keyframe(&self) -> bool {
        self.flags & 0x80 != 0
    }

    /// The discardable bit. `SimpleBlock` defines it. The muxer also sets it
    /// on a `Block` in a `BlockGroup`, where other readers ignore it.
    pub fn discardable(&self) -> bool {
        self.flags & 0x01 != 0
    }
}

/// Decode the block header at the start of `bytes`. `Ok(None)` when the
/// bytes end first.
pub(crate) fn block_header(bytes: &[u8]) -> Result<Option<BlockHeader>, String> {
    let (track, track_len) = match vint(bytes) {
        Vint::Value(track, len) => (track, len),
        Vint::Short => return Ok(None),
        Vint::Invalid => return Err("the track number starts with byte 0x00".to_owned()),
    };
    let Some(&[high, low, flags]) = bytes.get(track_len..track_len + 3) else {
        return Ok(None);
    };
    Ok(Some(BlockHeader {
        track,
        relative: i16::from_be_bytes([high, low]),
        flags,
        len: track_len + 3,
    }))
}

/// The frames of a laced block: their sizes, and the lace header length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LaceFrames {
    /// The frame sizes in file order.
    pub sizes: Vec<u64>,
    /// The bytes before the first frame: the frame count and the size
    /// fields.
    pub header_len: usize,
}

/// Decode the lace header in `data`: every block byte after the block
/// header.
pub(crate) fn lace(lacing: Lacing, data: &[u8]) -> Result<LaceFrames, String> {
    let Some(&count_byte) = data.first() else {
        return Err("the lace frame count is missing".to_owned());
    };
    let count = usize::from(count_byte) + 1;
    let mut pos = 1usize;
    let mut sizes: Vec<u64> = Vec::with_capacity(count);
    match lacing {
        Lacing::None => return Err("an unlaced block has no lace header".to_owned()),
        Lacing::Fixed => {
            let total = data.len() as u64 - 1;
            if !total.is_multiple_of(count as u64) {
                return Err(format!(
                    "fixed-size lacing splits {total} bytes into {count} frames unevenly"
                ));
            }
            sizes.resize(count, total / count as u64);
            return Ok(LaceFrames {
                sizes,
                header_len: 1,
            });
        }
        Lacing::Xiph => {
            for frame in 0..count - 1 {
                let mut size = 0u64;
                loop {
                    let Some(&byte) = data.get(pos) else {
                        return Err(format!(
                            "the Xiph size of frame {frame} runs past the block"
                        ));
                    };
                    pos += 1;
                    size += u64::from(byte);
                    if byte != 0xFF {
                        break;
                    }
                }
                sizes.push(size);
            }
        }
        Lacing::Ebml => {
            let mut size = 0i64;
            for frame in 0..count - 1 {
                let rest = data.get(pos..).unwrap_or_default();
                let (raw, len) = match vint(rest) {
                    Vint::Value(raw, len) => (raw, len),
                    Vint::Short => {
                        return Err(format!(
                            "the EBML size of frame {frame} runs past the block"
                        ));
                    }
                    Vint::Invalid => {
                        return Err(format!("the EBML size of frame {frame} starts with 0x00"));
                    }
                };
                pos += len;
                size = if frame == 0 {
                    raw as i64
                } else {
                    // A signed difference: subtract half the range.
                    let bias = (1i64 << (7 * len - 1)) - 1;
                    size.checked_add(raw as i64 - bias).ok_or_else(|| {
                        format!("the EBML size of frame {frame} overflows 64 bits")
                    })?
                };
                if size < 0 {
                    return Err(format!(
                        "the EBML size of frame {frame} is negative: {size}"
                    ));
                }
                sizes.push(size as u64);
            }
        }
    }
    let remaining = (data.len() - pos) as u64;
    let laced = sizes
        .iter()
        .try_fold(0u64, |acc, &size| acc.checked_add(size))
        .unwrap_or(u64::MAX);
    let last = remaining.checked_sub(laced).ok_or_else(|| {
        format!("the lace sizes add up to {laced} bytes, but {remaining} follow the lace header")
    })?;
    sizes.push(last);
    Ok(LaceFrames {
        sizes,
        header_len: pos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_header_reads_track_timestamp_and_flags() {
        let header = block_header(&[0x81, 0xFF, 0xFE, 0x86]);
        assert_eq!(
            header,
            Ok(Some(BlockHeader {
                track: 1,
                relative: -2,
                flags: 0x86,
                len: 4
            }))
        );
        assert_eq!(
            header.ok().flatten().map(|h| h.lacing()),
            Some(Lacing::Ebml)
        );
        assert_eq!(block_header(&[0x81, 0x00]), Ok(None));
    }

    #[test]
    fn xiph_lacing_sums_runs_of_255() {
        let mut data = vec![2, 0xFF, 0x01, 3];
        data.extend(vec![0; 256 + 3 + 5]);
        let lace = lace(Lacing::Xiph, &data);
        assert_eq!(
            lace,
            Ok(LaceFrames {
                sizes: vec![256, 3, 5],
                header_len: 4
            })
        );
    }

    #[test]
    fn ebml_lacing_applies_signed_differences() {
        // Sizes 10, then +2 (0xC1 = 65 - 63), then the remainder 7.
        let mut data = vec![2, 0x8A, 0xC1];
        data.extend(vec![0; 10 + 12 + 7]);
        assert_eq!(
            lace(Lacing::Ebml, &data).map(|l| l.sizes),
            Ok(vec![10, 12, 7])
        );
    }

    #[test]
    fn fixed_lacing_refuses_uneven_splits() {
        assert_eq!(
            lace(Lacing::Fixed, &[1, 0, 0]).map(|l| l.sizes),
            Ok(vec![1, 1])
        );
        assert!(lace(Lacing::Fixed, &[1, 0, 0, 0]).is_err());
    }

    #[test]
    fn lace_sizes_past_the_block_are_refused() {
        assert!(lace(Lacing::Xiph, &[1, 9, 0]).is_err());
    }
}
