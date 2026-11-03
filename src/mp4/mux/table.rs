//! The sample tables of one output track, kept run-length coded where the
//! format allows, and their `stbl` encoding.
//!
//! `stts`, `ctts`, `cslg`, `stsc` and `stss` use mp4-atom encoders. `stsz`
//! and `stco`/`co64` are written here, straight from the tables, so the
//! largest tables are never copied.

use mp4_atom::{Cslg, Ctts, CttsEntry, Stsc, StscEntry, Stss, Stts, SttsEntry};

use crate::error::Result;
use crate::mp4::mux::bytes::BoxBuf;

/// The sample tables of one track.
#[derive(Debug, Default)]
pub(crate) struct SampleTable {
    /// The number of samples.
    pub count: u32,
    /// `stts` runs: (sample count, delta).
    durations: Vec<(u32, u32)>,
    /// `ctts` runs: (sample count, offset).
    offsets: Vec<(u32, i64)>,
    /// The least and greatest composition offset.
    pub offset_range: Option<(i64, i64)>,
    sizes: Vec<u32>,
    /// 1-based numbers of the sync samples.
    sync: Vec<u32>,
    /// Chunks: (byte offset before any faststart shift, sample count).
    chunks: Vec<(u64, u32)>,
    /// The sum of the sample durations: the `mdhd` duration.
    pub media_duration: u64,
    /// The earliest presentation time and the latest presentation end.
    pub presentation: Option<(i64, i64)>,
}

impl SampleTable {
    /// Add one sample. Its duration follows with [`SampleTable::push_duration`].
    pub fn push_sample(&mut self, size: u32, offset: i64, sync: bool) {
        self.count += 1;
        self.sizes.push(size);
        if sync {
            self.sync.push(self.count);
        }
        match self.offsets.last_mut() {
            Some((count, value)) if *value == offset => *count += 1,
            _ => self.offsets.push((1, offset)),
        }
        self.offset_range = Some(match self.offset_range {
            Some((low, high)) => (low.min(offset), high.max(offset)),
            None => (offset, offset),
        });
    }

    /// Set the duration of the oldest sample that has none, and note its
    /// presentation span.
    pub fn push_duration(&mut self, delta: u32, pts: i64) {
        match self.durations.last_mut() {
            Some((count, value)) if *value == delta => *count += 1,
            _ => self.durations.push((1, delta)),
        }
        self.media_duration += u64::from(delta);
        let end = pts.saturating_add(i64::from(delta));
        self.presentation = Some(match self.presentation {
            Some((start, last)) => (start.min(pts), last.max(end)),
            None => (pts, end),
        });
    }

    /// Record a written chunk.
    pub fn push_chunk(&mut self, offset: u64, samples: u32) {
        self.chunks.push((offset, samples));
    }

    /// True when some composition offset is not 0, so the track needs
    /// `ctts`.
    pub fn has_offsets(&self) -> bool {
        self.offset_range
            .is_some_and(|(low, high)| low != 0 || high != 0)
    }

    /// True when a chunk offset, after `shift`, needs 64 bits.
    fn needs_co64(&self, shift: u64) -> bool {
        self.chunks
            .last()
            .is_some_and(|&(offset, _)| offset.saturating_add(shift) > u64::from(u32::MAX))
    }

    /// Write `stbl` with the sample entry `entry` and every chunk offset
    /// moved up by `shift`.
    pub fn encode(&self, buf: &mut BoxBuf, entry: &[u8], shift: u64, path: &str) -> Result<()> {
        let stbl = buf.open(b"stbl");
        let stsd = buf.open_full(b"stsd", 0, 0);
        buf.u32(1);
        buf.bytes(entry);
        buf.close(stsd, &format!("{path}/stsd"))?;

        let stts = Stts {
            entries: self
                .durations
                .iter()
                .map(|&(sample_count, sample_delta)| SttsEntry {
                    sample_count,
                    sample_delta,
                })
                .collect(),
        };
        buf.encode(&stts, &format!("{path}/stts"))?;

        if self.has_offsets() {
            let ctts = Ctts {
                entries: self
                    .offsets
                    .iter()
                    .map(|&(sample_count, sample_offset)| CttsEntry {
                        sample_count,
                        sample_offset,
                    })
                    .collect(),
            };
            buf.encode(&ctts, &format!("{path}/ctts"))?;
            if let (Some((low, high)), Some((start, end))) = (self.offset_range, self.presentation)
                && low < 0
            {
                let cslg = Cslg {
                    composition_to_dts_shift: low.saturating_neg(),
                    least_decode_to_display_shift: low,
                    greatest_decode_to_display_delta: high,
                    composition_start_time: start,
                    composition_end_time: end,
                };
                buf.encode(&cslg, &format!("{path}/cslg"))?;
            }
        }

        let mut stsc = Stsc::default();
        for (n, &(_, samples)) in self.chunks.iter().enumerate() {
            if stsc
                .entries
                .last()
                .is_some_and(|entry| entry.samples_per_chunk == samples)
            {
                continue;
            }
            stsc.entries.push(StscEntry {
                first_chunk: n as u32 + 1,
                samples_per_chunk: samples,
                sample_description_index: 1,
            });
        }
        buf.encode(&stsc, &format!("{path}/stsc"))?;

        let stsz = buf.open_full(b"stsz", 0, 0);
        // A `sample_size` of 0 means a table follows, so empty samples always
        // take the table form.
        let uniform = self
            .sizes
            .first()
            .filter(|&&first| first != 0 && self.sizes.iter().all(|&size| size == first));
        match uniform {
            Some(&size) => {
                buf.u32(size);
                buf.u32(self.count);
            }
            None => {
                buf.u32(0);
                buf.u32(self.count);
                for &size in &self.sizes {
                    buf.u32(size);
                }
            }
        }
        buf.close(stsz, &format!("{path}/stsz"))?;

        let chunk_count = self.chunks.len() as u32;
        if self.needs_co64(shift) {
            let co64 = buf.open_full(b"co64", 0, 0);
            buf.u32(chunk_count);
            for &(offset, _) in &self.chunks {
                buf.u64(offset.saturating_add(shift));
            }
            buf.close(co64, &format!("{path}/co64"))?;
        } else {
            let stco = buf.open_full(b"stco", 0, 0);
            buf.u32(chunk_count);
            for &(offset, _) in &self.chunks {
                // needs_co64 is false, so every shifted offset fits.
                buf.u32(offset.saturating_add(shift) as u32);
            }
            buf.close(stco, &format!("{path}/stco"))?;
        }

        if self.sync.len() != self.count as usize {
            let stss = Stss {
                entries: self.sync.clone(),
            };
            buf.encode(&stss, &format!("{path}/stss"))?;
        }
        buf.close(stbl, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(offsets: &[u64]) -> SampleTable {
        let mut table = SampleTable::default();
        for (n, &offset) in offsets.iter().enumerate() {
            table.push_sample(10, if n == 1 { -5 } else { 0 }, n == 0);
            table.push_duration(20, 20 * n as i64);
            table.push_chunk(offset, 1);
        }
        table
    }

    fn kinds(bytes: &[u8]) -> Vec<[u8; 4]> {
        let mut out = Vec::new();
        let mut pos = 8;
        while pos + 8 <= bytes.len() {
            let size =
                u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
            out.push([
                bytes[pos + 4],
                bytes[pos + 5],
                bytes[pos + 6],
                bytes[pos + 7],
            ]);
            pos += size as usize;
        }
        out
    }

    #[test]
    fn offsets_past_4_gib_switch_to_co64() {
        let small = table(&[100, 4_000_000_000]);
        assert!(!small.needs_co64(0));
        assert!(small.needs_co64(u64::from(u32::MAX)));
        let mut buf = BoxBuf::new();
        let big = table(&[100, 5 << 30, 6 << 30]);
        big.encode(&mut buf, &[], 16, "stbl")
            .expect("the tables encode");
        let bytes = buf.into_bytes();
        let found = kinds(&bytes);
        assert!(found.contains(b"co64"), "{found:?}");
        assert!(!found.contains(b"stco"), "{found:?}");
        let at = bytes
            .windows(4)
            .position(|w| w == b"co64")
            .expect("co64 is present");
        let entry = |n: usize| {
            let start = at + 12 + 8 * n;
            u64::from_be_bytes(bytes[start..start + 8].try_into().expect("8 bytes"))
        };
        assert_eq!(
            [entry(0), entry(1), entry(2)],
            [116, (5 << 30) + 16, (6 << 30) + 16]
        );
    }

    #[test]
    fn negative_offsets_write_ctts_version_1_and_cslg() {
        let mut buf = BoxBuf::new();
        table(&[0, 10, 20])
            .encode(&mut buf, &[], 0, "stbl")
            .expect("the tables encode");
        let bytes = buf.into_bytes();
        let found = kinds(&bytes);
        assert!(found.contains(b"cslg"), "{found:?}");
        assert!(found.contains(b"stss"), "{found:?}");
        let at = bytes
            .windows(4)
            .position(|w| w == b"ctts")
            .expect("ctts is present");
        assert_eq!(bytes[at + 4], 1, "ctts version");
    }

    #[test]
    fn stsc_merges_chunks_of_equal_size() {
        let mut table = SampleTable::default();
        for (offset, samples) in [(0, 2), (10, 2), (20, 1), (30, 2)] {
            table.push_chunk(offset, samples);
        }
        let mut buf = BoxBuf::new();
        table
            .encode(&mut buf, &[], 0, "stbl")
            .expect("the tables encode");
        let bytes = buf.into_bytes();
        let at = bytes
            .windows(4)
            .position(|w| w == b"stsc")
            .expect("stsc is present");
        let count = u32::from_be_bytes(bytes[at + 8..at + 12].try_into().expect("4 bytes"));
        assert_eq!(count, 3);
    }
}
