//! Sample size tables: `stsz` and the compact `stz2`.

use mp4_atom::{Stsz, StszSamples};

use crate::error::{Error, Result};
use crate::mp4::boxes::RawBox;

/// The size of every sample in a track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SampleSizes {
    /// `count` samples of `size` bytes each. Nothing is allocated per sample.
    Uniform { size: u32, count: u32 },
    /// One size per sample.
    Table(Vec<u32>),
    /// `stz2` entries of `field_size` bits (4, 8 or 16), packed as stored.
    Packed {
        field_size: u8,
        count: u32,
        data: Vec<u8>,
    },
}

impl SampleSizes {
    /// Sizes from a decoded `stsz`.
    pub fn from_stsz(stsz: Stsz) -> Self {
        match stsz.samples {
            StszSamples::Identical { count, size } => Self::Uniform { size, count },
            StszSamples::Different { sizes } => Self::Table(sizes),
        }
    }

    /// Parse a `stz2` box.
    pub fn parse_stz2(raw: &RawBox<'_>, path: &str) -> Result<Self> {
        let mut reader = raw.reader(path);
        reader.skip(4, "version and flags")?;
        reader.skip(3, "reserved")?;
        let field_size = reader.u8("field_size")?;
        let count = reader.u32("sample_count")?;
        if !matches!(field_size, 4 | 8 | 16) {
            return Err(Error::mp4_malformed(
                path,
                raw.offset,
                format!("field_size is {field_size}. It must be 4, 8 or 16"),
            ));
        }
        let needed = (u64::from(count) * u64::from(field_size)).div_ceil(8);
        let available = reader.rest().len() as u64;
        if needed > available {
            return Err(Error::mp4_malformed(
                path,
                raw.offset,
                format!(
                    "{count} entries of {field_size} bits need {needed} bytes, but the box holds {available}"
                ),
            ));
        }
        let data = reader.bytes(needed as usize, "entry_size")?.to_vec();
        Ok(Self::Packed {
            field_size,
            count,
            data,
        })
    }

    /// The number of samples.
    pub fn count(&self) -> u64 {
        match self {
            Self::Uniform { count, .. } | Self::Packed { count, .. } => u64::from(*count),
            Self::Table(sizes) => sizes.len() as u64,
        }
    }

    /// The size of sample `index`, counted from 0.
    pub fn get(&self, index: u64) -> Option<u32> {
        if index >= self.count() {
            return None;
        }
        let index = usize::try_from(index).ok()?;
        match self {
            Self::Uniform { size, .. } => Some(*size),
            Self::Table(sizes) => sizes.get(index).copied(),
            Self::Packed {
                field_size, data, ..
            } => match field_size {
                4 => data.get(index / 2).map(|&byte| {
                    u32::from(if index.is_multiple_of(2) {
                        byte >> 4
                    } else {
                        byte & 0x0F
                    })
                }),
                8 => data.get(index).map(|&byte| u32::from(byte)),
                _ => {
                    let at = index.checked_mul(2)?;
                    let pair = data.get(at..at.checked_add(2)?)?;
                    Some(u32::from(u16::from_be_bytes([pair[0], pair[1]])))
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stz2(field_size: u8, count: u32, entries: &[u8]) -> Vec<u8> {
        let mut body = vec![0, 0, 0, 0, 0, 0, 0, field_size];
        body.extend_from_slice(&count.to_be_bytes());
        body.extend_from_slice(entries);
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(b"stz2");
        out.extend(body);
        out
    }

    fn parse(bytes: &[u8]) -> Result<SampleSizes> {
        let raw = RawBox::whole(bytes, 0, "stz2")?;
        SampleSizes::parse_stz2(&raw, "stz2")
    }

    fn all(sizes: &SampleSizes) -> Vec<u32> {
        (0..sizes.count()).filter_map(|i| sizes.get(i)).collect()
    }

    #[test]
    fn stsz_uniform_sizes_cover_every_sample() {
        let sizes = SampleSizes::from_stsz(Stsz {
            samples: StszSamples::Identical { count: 3, size: 9 },
        });
        assert_eq!(all(&sizes), vec![9, 9, 9]);
        assert_eq!(sizes.get(3), None);
    }

    #[test]
    fn stsz_table_sizes_follow_the_table() {
        let sizes = SampleSizes::from_stsz(Stsz {
            samples: StszSamples::Different {
                sizes: vec![4, 5, 6],
            },
        });
        assert_eq!(all(&sizes), vec![4, 5, 6]);
    }

    #[test]
    fn stz2_unpacks_4_bit_entries_high_nibble_first() {
        let sizes = parse(&stz2(4, 3, &[0x12, 0x30])).expect("the box is well formed");
        assert_eq!(all(&sizes), vec![1, 2, 3]);
    }

    #[test]
    fn stz2_unpacks_8_and_16_bit_entries() {
        let sizes = parse(&stz2(8, 2, &[7, 200])).expect("the box is well formed");
        assert_eq!(all(&sizes), vec![7, 200]);
        let sizes = parse(&stz2(16, 2, &[1, 0, 0, 9])).expect("the box is well formed");
        assert_eq!(all(&sizes), vec![256, 9]);
    }

    #[test]
    fn stz2_rejects_a_short_table_and_a_bad_field_size() {
        assert!(matches!(
            parse(&stz2(8, 3, &[1, 2])),
            Err(Error::Malformed { .. })
        ));
        assert!(matches!(
            parse(&stz2(12, 1, &[1, 2])),
            Err(Error::Malformed { .. })
        ));
    }
}
