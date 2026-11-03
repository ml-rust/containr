//! Sample tables and the cursors that expand them one sample at a time.
//!
//! The cursors hold positions only. Nothing allocates per sample, so a track
//! of any declared length costs memory in proportion to its table entries.

mod build;
mod chunk;
mod cursor;
mod size;
mod sync;
mod time;

pub(crate) use build::{SampleTables, parse_stbl};
pub(crate) use cursor::SampleCursor;
