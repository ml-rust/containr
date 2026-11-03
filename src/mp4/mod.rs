//! MP4 and QuickTime MOV demux.
//!
//! Fragmented MP4 (`moof`, `mvex`) is refused with [`Error::Fragmented`].
//!
//! [`Error::Fragmented`]: crate::Error::Fragmented

mod boxes;
mod chapters;
mod demuxer;
mod metadata;
mod moov;
mod packets;
mod sample_entry;
mod tables;
mod trak;

pub use chapters::{MAX_CHAPTER_SAMPLE, MAX_CHAPTERS};
pub use demuxer::{Demuxer, MAX_FTYP_SIZE, MAX_MOOV_SIZE};
pub use packets::{Packets, TrackPackets};
