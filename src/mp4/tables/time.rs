//! Run-length time tables: `stts` (sample durations) and `ctts`
//! (composition offsets).

use mp4_atom::{Ctts, Stts};

/// `count` consecutive samples that share `value`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Run<V> {
    pub count: u32,
    pub value: V,
}

/// The number of samples a run table covers.
pub(crate) fn run_total<V>(runs: &[Run<V>]) -> u64 {
    runs.iter().map(|run| u64::from(run.count)).sum()
}

/// `stts` runs: sample count and duration in track ticks.
pub(crate) fn duration_runs(stts: Stts) -> Vec<Run<u32>> {
    stts.entries
        .into_iter()
        .map(|entry| Run {
            count: entry.sample_count,
            value: entry.sample_delta,
        })
        .collect()
}

/// `ctts` runs: sample count and signed composition offset in track ticks.
///
/// Version 1 offsets are signed on the wire. Version 0 offsets are unsigned,
/// but libavformat and other readers treat them as signed 32-bit values, and
/// some writers store negative offsets in version 0. Both versions read as
/// signed 32-bit here.
pub(crate) fn offset_runs(ctts: Ctts) -> Vec<Run<i64>> {
    ctts.entries
        .into_iter()
        .map(|entry| Run {
            count: entry.sample_count,
            value: i64::from(entry.sample_offset as u32 as i32),
        })
        .collect()
}

/// A position in a run table. Runs with a zero count hold no sample.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RunCursor {
    entry: usize,
    used: u32,
}

impl RunCursor {
    /// The value of the next sample, or `None` past the last run.
    pub fn next_value<V: Copy>(&mut self, runs: &[Run<V>]) -> Option<V> {
        while let Some(run) = runs.get(self.entry) {
            if self.used < run.count {
                self.used += 1;
                return Some(run.value);
            }
            self.entry += 1;
            self.used = 0;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use mp4_atom::{CttsEntry, SttsEntry};

    use super::*;

    fn expand<V: Copy>(runs: &[Run<V>]) -> Vec<V> {
        let mut cursor = RunCursor::default();
        std::iter::from_fn(|| cursor.next_value(runs)).collect()
    }

    #[test]
    fn stts_expands_runs_in_order() {
        let runs = duration_runs(Stts {
            entries: vec![
                SttsEntry {
                    sample_count: 2,
                    sample_delta: 512,
                },
                SttsEntry {
                    sample_count: 0,
                    sample_delta: 7,
                },
                SttsEntry {
                    sample_count: 1,
                    sample_delta: 256,
                },
            ],
        });
        assert_eq!(run_total(&runs), 3);
        assert_eq!(expand(&runs), vec![512, 512, 256]);
    }

    #[test]
    fn ctts_reads_version_1_offsets_as_signed() {
        let runs = offset_runs(Ctts {
            entries: vec![
                CttsEntry {
                    sample_count: 1,
                    sample_offset: 1024,
                },
                CttsEntry {
                    sample_count: 2,
                    sample_offset: -512,
                },
            ],
        });
        assert_eq!(expand(&runs), vec![1024, -512, -512]);
    }

    #[test]
    fn ctts_reads_large_version_0_offsets_as_signed() {
        let runs = offset_runs(Ctts {
            entries: vec![CttsEntry {
                sample_count: 1,
                sample_offset: i64::from(u32::MAX),
            }],
        });
        assert_eq!(expand(&runs), vec![-1]);
    }

    #[test]
    fn empty_tables_expand_to_nothing() {
        let runs: Vec<Run<u32>> = Vec::new();
        assert_eq!(run_total(&runs), 0);
        assert!(expand(&runs).is_empty());
    }
}
