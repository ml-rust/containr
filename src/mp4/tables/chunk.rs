//! Chunk tables: `stsc` (samples per chunk) with `stco`/`co64` (chunk
//! offsets) place each sample in the input.

use mp4_atom::Stsc;

use crate::error::{Error, Result};

/// One `stsc` entry: from `first_chunk` (counted from 1) on, each chunk holds
/// `samples_per_chunk` samples that use `description_index`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChunkRun {
    pub first_chunk: u32,
    pub samples_per_chunk: u32,
    pub description_index: u32,
}

/// The checked `stsc` runs and the chunk offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChunkMap {
    runs: Vec<ChunkRun>,
    offsets: Vec<u64>,
}

impl ChunkMap {
    /// Check `stsc` against the chunk offsets and the sample description
    /// count.
    pub fn new(stsc: Stsc, offsets: Vec<u64>, descriptions: usize, path: &str) -> Result<Self> {
        let runs: Vec<ChunkRun> = stsc
            .entries
            .into_iter()
            .map(|entry| ChunkRun {
                first_chunk: entry.first_chunk,
                samples_per_chunk: entry.samples_per_chunk,
                description_index: entry.sample_description_index,
            })
            .collect();
        let chunk_count = offsets.len() as u64;
        let mut previous = 0u32;
        for (i, run) in runs.iter().enumerate() {
            if i == 0 && run.first_chunk != 1 {
                return Err(Error::mp4_inconsistent(
                    path,
                    format!(
                        "the first entry starts at chunk {}, not chunk 1",
                        run.first_chunk
                    ),
                ));
            }
            if run.first_chunk <= previous {
                return Err(Error::mp4_inconsistent(
                    path,
                    format!(
                        "entry {i} starts at chunk {} after an entry that starts at chunk {previous}",
                        run.first_chunk
                    ),
                ));
            }
            if u64::from(run.first_chunk) > chunk_count {
                return Err(Error::mp4_inconsistent(
                    path,
                    format!(
                        "entry {i} starts at chunk {}, but the chunk offset table holds {chunk_count} chunks",
                        run.first_chunk
                    ),
                ));
            }
            if run.description_index == 0 || run.description_index as usize > descriptions {
                return Err(Error::mp4_inconsistent(
                    path,
                    format!(
                        "entry {i} uses sample description {}, but stsd holds {descriptions}",
                        run.description_index
                    ),
                ));
            }
            previous = run.first_chunk;
        }
        if runs.is_empty() && !offsets.is_empty() {
            return Err(Error::mp4_inconsistent(
                path,
                format!(
                    "the table is empty, but the chunk offset table holds {chunk_count} chunks"
                ),
            ));
        }
        Ok(Self { runs, offsets })
    }

    /// The number of samples the chunks hold.
    pub fn sample_count(&self, path: &str) -> Result<u64> {
        let chunk_count = self.offsets.len() as u64;
        let mut total = 0u64;
        for (i, run) in self.runs.iter().enumerate() {
            let end = self
                .runs
                .get(i + 1)
                .map_or(chunk_count, |next| u64::from(next.first_chunk) - 1);
            let chunks = end - (u64::from(run.first_chunk) - 1);
            total = chunks
                .checked_mul(u64::from(run.samples_per_chunk))
                .and_then(|samples| total.checked_add(samples))
                .ok_or_else(|| {
                    Error::mp4_inconsistent(path, "the sample count overflows 64 bits")
                })?;
        }
        Ok(total)
    }
}

/// A position in the chunk map.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ChunkCursor {
    /// The next chunk to open, counted from 0.
    chunk: usize,
    /// The current run.
    run: usize,
    /// Samples left in the open chunk.
    left: u32,
    /// The offset of the next sample in the open chunk.
    next_offset: u64,
    description_index: u32,
}

impl ChunkCursor {
    /// The offset and description index of the next sample, which spans
    /// `size` bytes. `None` past the last chunk.
    pub fn next_sample(
        &mut self,
        map: &ChunkMap,
        size: u32,
        path: &str,
    ) -> Result<Option<(u64, u32)>> {
        while self.left == 0 {
            let Some(&offset) = map.offsets.get(self.chunk) else {
                return Ok(None);
            };
            while let Some(next) = map.runs.get(self.run + 1) {
                if (next.first_chunk as usize) - 1 <= self.chunk {
                    self.run += 1;
                } else {
                    break;
                }
            }
            let Some(run) = map.runs.get(self.run) else {
                return Ok(None);
            };
            self.left = run.samples_per_chunk;
            self.description_index = run.description_index;
            self.next_offset = offset;
            self.chunk += 1;
        }
        let offset = self.next_offset;
        self.next_offset = offset.checked_add(u64::from(size)).ok_or_else(|| {
            Error::mp4_inconsistent(
                path,
                format!("a sample of {size} bytes at offset {offset} ends past 2^64"),
            )
        })?;
        self.left -= 1;
        Ok(Some((offset, self.description_index)))
    }
}

#[cfg(test)]
mod tests {
    use mp4_atom::StscEntry;

    use super::*;

    fn stsc(entries: &[(u32, u32, u32)]) -> Stsc {
        Stsc {
            entries: entries
                .iter()
                .map(
                    |&(first_chunk, samples_per_chunk, sample_description_index)| StscEntry {
                        first_chunk,
                        samples_per_chunk,
                        sample_description_index,
                    },
                )
                .collect(),
        }
    }

    fn expand(map: &ChunkMap, sizes: &[u32]) -> Vec<(u64, u32)> {
        let mut cursor = ChunkCursor::default();
        sizes
            .iter()
            .map_while(|&size| cursor.next_sample(map, size, "stsc").ok().flatten())
            .collect()
    }

    #[test]
    fn stsc_places_samples_in_chunks() {
        let map = ChunkMap::new(
            stsc(&[(1, 2, 1), (3, 1, 2)]),
            vec![100, 200, 300, 400],
            2,
            "stsc",
        )
        .expect("the table is consistent");
        assert_eq!(map.sample_count("stsc").ok(), Some(6));
        let placed = expand(&map, &[10, 20, 30, 40, 50, 60]);
        assert_eq!(
            placed,
            vec![(100, 1), (110, 1), (200, 1), (230, 1), (300, 2), (400, 2)]
        );
    }

    #[test]
    fn stsc_skips_chunks_with_no_samples() {
        let map = ChunkMap::new(
            stsc(&[(1, 1, 1), (2, 0, 1), (3, 1, 1)]),
            vec![10, 20, 30],
            1,
            "stsc",
        )
        .expect("the table is consistent");
        assert_eq!(map.sample_count("stsc").ok(), Some(2));
        assert_eq!(expand(&map, &[5, 5]), vec![(10, 1), (30, 1)]);
    }

    #[test]
    fn stsc_rejects_a_first_entry_after_chunk_1() {
        let err = ChunkMap::new(stsc(&[(2, 1, 1)]), vec![10, 20], 1, "stsc");
        assert!(matches!(err, Err(Error::Inconsistent { .. })));
    }

    #[test]
    fn stsc_rejects_entries_out_of_order() {
        let err = ChunkMap::new(
            stsc(&[(1, 1, 1), (3, 1, 1), (2, 1, 1)]),
            vec![1, 2, 3],
            1,
            "stsc",
        );
        assert!(matches!(err, Err(Error::Inconsistent { .. })));
    }

    #[test]
    fn stsc_rejects_a_chunk_past_the_offset_table() {
        let err = ChunkMap::new(stsc(&[(1, 1, 1), (5, 1, 1)]), vec![1, 2], 1, "stsc");
        assert!(matches!(err, Err(Error::Inconsistent { .. })));
    }

    #[test]
    fn stsc_rejects_an_unknown_sample_description() {
        let err = ChunkMap::new(stsc(&[(1, 1, 2)]), vec![1], 1, "stsc");
        assert!(matches!(err, Err(Error::Inconsistent { .. })));
    }

    #[test]
    fn chunk_cursor_reports_offset_overflow() {
        let map = ChunkMap::new(stsc(&[(1, 2, 1)]), vec![u64::MAX - 1], 1, "stsc")
            .expect("the table is consistent");
        let mut cursor = ChunkCursor::default();
        assert_eq!(
            cursor.next_sample(&map, 1, "stsc").ok().flatten(),
            Some((u64::MAX - 1, 1))
        );
        assert!(cursor.next_sample(&map, 4, "stsc").is_err());
    }
}
