//! Walk one track's samples in decode order.

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::mp4::tables::build::SampleTables;
use crate::mp4::tables::chunk::ChunkCursor;
use crate::mp4::tables::sync::SyncCursor;
use crate::mp4::tables::time::RunCursor;
use crate::packet::Packet;

/// The next sample of one track. After an error it yields nothing more.
#[derive(Clone, Debug)]
pub(crate) struct SampleCursor {
    tables: Arc<SampleTables>,
    index: u64,
    dts: i64,
    durations: RunCursor,
    offsets: RunCursor,
    chunks: ChunkCursor,
    sync: SyncCursor,
    failed: bool,
}

impl SampleCursor {
    pub fn new(tables: Arc<SampleTables>) -> Self {
        Self {
            tables,
            index: 0,
            dts: 0,
            durations: RunCursor::default(),
            offsets: RunCursor::default(),
            chunks: ChunkCursor::default(),
            sync: SyncCursor::default(),
            failed: false,
        }
    }

    /// The next packet, or `None` past the last sample.
    pub fn next_packet(&mut self) -> Result<Option<Packet>> {
        if self.failed || self.index >= self.tables.sample_count {
            return Ok(None);
        }
        let result = self.step();
        if result.is_err() {
            self.failed = true;
        }
        result.map(Some)
    }

    fn step(&mut self) -> Result<Packet> {
        let tables: &SampleTables = &self.tables;
        let path = tables.path.as_str();
        let index = self.index;
        let ended = |table: &str| {
            Error::inconsistent(
                format!("{path}/{table}"),
                format!("the table ends before sample {index}"),
            )
        };
        let size = tables.sizes.get(index).ok_or_else(|| ended("stsz"))?;
        let duration = self
            .durations
            .next_value(&tables.durations)
            .ok_or_else(|| ended("stts"))?;
        let composition = match &tables.offsets {
            Some(runs) => self.offsets.next_value(runs).ok_or_else(|| ended("ctts"))?,
            None => 0,
        };
        let (offset, description_index) = self
            .chunks
            .next_sample(&tables.chunks, size, &tables.stsc_path)?
            .ok_or_else(|| ended("stsc"))?;
        let dts = self.dts;
        let pts = dts.checked_add(composition).ok_or_else(|| {
            Error::inconsistent(
                format!("{path}/ctts"),
                format!("sample {index}: dts {dts} plus offset {composition} overflows 64 bits"),
            )
        })?;
        self.dts = dts.checked_add(i64::from(duration)).ok_or_else(|| {
            Error::inconsistent(
                format!("{path}/stts"),
                format!("sample {index}: dts {dts} plus duration {duration} overflows 64 bits"),
            )
        })?;
        let sync = self.sync.is_sync(&tables.sync, index + 1);
        self.index += 1;
        Ok(Packet {
            track_id: tables.track_id,
            index,
            dts,
            pts,
            duration,
            sync,
            offset,
            size,
            description_index,
        })
    }
}
