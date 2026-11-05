//! A small Matroska file built in memory, so demux and corruption tests run
//! without FFmpeg. It holds every lacing kind, `SimpleBlock` and
//! `BlockGroup` blocks, a Cluster of unknown size, `Cues`, `Chapters`,
//! `Tags` and an attachment. The expected packets are known exactly.

mod bare;
mod blocks;
mod elements;
mod file;

// Only the mux test binaries use these.
#[allow(unused_imports)]
pub use bare::{BARE_TRACK, BareKind, bare_file};
// Only the `mkv_demux` test binary uses these. The `mp4_demux` binary
// compiles this module too, so the re-exports are unused there.
#[allow(unused_imports)]
pub use file::{
    ATTACHMENT, ATTACHMENT_UID, AUDIO, AUDIO_UID, CODEC_PRIVATE, Synth, VIDEO, VIDEO_UID, file,
    file_with_colour, packet_bytes,
};
