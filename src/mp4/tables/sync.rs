//! The sync sample table, `stss`.

use mp4_atom::Stss;

use crate::error::{Error, Result};

/// Which samples are sync samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SyncSamples {
    /// No `stss`: every sample is a sync sample.
    All,
    /// The sync sample numbers, counted from 1, strictly increasing. An empty
    /// list means no sample is a sync sample.
    Listed(Vec<u32>),
}

impl SyncSamples {
    /// Check `stss` against the sample count.
    pub fn new(stss: Option<Stss>, sample_count: u64, path: &str) -> Result<Self> {
        let Some(stss) = stss else {
            return Ok(Self::All);
        };
        let mut previous = 0u32;
        for (i, &number) in stss.entries.iter().enumerate() {
            if number <= previous {
                return Err(Error::mp4_inconsistent(
                    path,
                    format!("entry {i} names sample {number} after sample {previous}"),
                ));
            }
            if u64::from(number) > sample_count {
                return Err(Error::mp4_inconsistent(
                    path,
                    format!(
                        "entry {i} names sample {number}, but the track holds {sample_count} samples"
                    ),
                ));
            }
            previous = number;
        }
        Ok(Self::Listed(stss.entries))
    }
}

/// A position in the sync sample list.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SyncCursor {
    next: usize,
}

impl SyncCursor {
    /// True when sample `number` (counted from 1) is a sync sample. Calls
    /// must pass increasing numbers.
    pub fn is_sync(&mut self, sync: &SyncSamples, number: u64) -> bool {
        let SyncSamples::Listed(list) = sync else {
            return true;
        };
        while let Some(&listed) = list.get(self.next) {
            let listed = u64::from(listed);
            if listed > number {
                return false;
            }
            self.next += 1;
            if listed == number {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(sync: &SyncSamples, count: u64) -> Vec<bool> {
        let mut cursor = SyncCursor::default();
        (1..=count).map(|n| cursor.is_sync(sync, n)).collect()
    }

    #[test]
    fn stss_absent_marks_every_sample_sync() {
        let sync = SyncSamples::new(None, 3, "stss").expect("no table is consistent");
        assert_eq!(flags(&sync, 3), vec![true, true, true]);
    }

    #[test]
    fn stss_marks_the_listed_samples() {
        let stss = Stss {
            entries: vec![1, 4],
        };
        let sync = SyncSamples::new(Some(stss), 5, "stss").expect("the table is consistent");
        assert_eq!(flags(&sync, 5), vec![true, false, false, true, false]);
    }

    #[test]
    fn stss_empty_marks_no_sample_sync() {
        let sync = SyncSamples::new(Some(Stss { entries: vec![] }), 2, "stss")
            .expect("the table is consistent");
        assert_eq!(flags(&sync, 2), vec![false, false]);
    }

    #[test]
    fn stss_rejects_unordered_and_out_of_range_entries() {
        let unordered = Stss {
            entries: vec![3, 2],
        };
        assert!(SyncSamples::new(Some(unordered), 5, "stss").is_err());
        let past_end = Stss { entries: vec![6] };
        assert!(SyncSamples::new(Some(past_end), 5, "stss").is_err());
        let zero = Stss { entries: vec![0] };
        assert!(SyncSamples::new(Some(zero), 5, "stss").is_err());
    }
}
