//! MP4 and QuickTime MOV mux with stream copy.

mod bytes;
mod chapters;
mod codec_config;
mod copy;
mod entry;
mod interleave;
mod layout;
mod matroska;
mod metadata;
mod moov;
mod muxer;
mod options;
mod spec;
mod table;
mod track;

pub use copy::{PacketTiming, StreamCopy};
pub use layout::MOVE_BLOCK;
pub use metadata::MAX_CHPL_CHAPTERS;
pub use muxer::{MAX_TRACKS, Muxer};
pub use options::{
    Brand, DEFAULT_CHUNK_DURATION_MS, DEFAULT_MAX_BUFFER, MAX_CHUNK_BYTES, MAX_CHUNK_DURATION_MS,
    MAX_CHUNK_SAMPLES, MuxOptions,
};
pub use spec::{
    AudioCodec, IDENTITY_MATRIX, PacketFormat, PacketIn, SampleEntrySpec, TrackId, TrackSpec,
    VisualCodec,
};
