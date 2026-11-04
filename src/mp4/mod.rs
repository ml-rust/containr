//! MP4 and QuickTime MOV demux and mux.
//!
//! Fragmented MP4 (`moof`, `mvex`) is refused with [`Error::Fragmented`].
//!
//! [`Muxer`] writes `ftyp`, `mdat` and `moov`, or `moov` before `mdat` with
//! [`MuxOptions::faststart`]. [`TrackSpec::copy_from`] and [`StreamCopy`]
//! stream copy a track from [`Demuxer`] or from the Matroska demuxer.
//!
//! [`Error::Fragmented`]: crate::Error::Fragmented

mod boxes;
mod chapters;
mod colour;
mod demuxer;
mod metadata;
mod moov;
mod mux;
mod packets;
mod sample_entry;
mod tables;
mod trak;

pub use chapters::{MAX_CHAPTER_SAMPLE, MAX_CHAPTERS};
pub use demuxer::{Demuxer, MAX_FTYP_SIZE, MAX_MOOV_SIZE};
pub use mux::{
    AudioCodec, Brand, DEFAULT_CHUNK_DURATION_MS, DEFAULT_MAX_BUFFER, IDENTITY_MATRIX,
    MAX_CHPL_CHAPTERS, MAX_CHUNK_BYTES, MAX_CHUNK_DURATION_MS, MAX_CHUNK_SAMPLES, MAX_TRACKS,
    MOVE_BLOCK, MuxOptions, Muxer, PacketFormat, PacketIn, PacketTiming, SampleEntrySpec,
    StreamCopy, TrackId, TrackSpec, VisualCodec,
};
pub use packets::{Packets, TrackPackets};
