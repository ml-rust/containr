//! Matroska and WebM mux with stream copy.

mod attachments;
mod bytes;
mod chapters;
mod cluster;
mod copy;
mod cues;
mod finish;
mod from_mp4;
mod head;
mod mp4_config;
mod mp4_edit;
mod mp4_tags;
mod muxer;
mod no_seek;
mod options;
mod spec;
mod spec_check;
mod tags;
mod timing;
mod track;
mod track_entry;

pub use attachments::AttachmentIn;
pub use copy::{PacketTiming, StreamCopy};
pub use cues::MAX_CUE_POINTS;
pub use mp4_tags::tags_from_mp4;
pub use muxer::{MAX_TRACKS, Muxer};
pub use no_seek::NoSeek;
pub use options::{
    DEFAULT_CLUSTER_BYTES, DEFAULT_CLUSTER_DURATION_MS, DEFAULT_MAX_BUFFER, DocType,
    MAX_CLUSTER_BYTES, MAX_CLUSTER_DURATION_MS, MIN_CLUSTER_BYTES, MkvOptions,
};
pub use spec::{PacketIn, TrackId, TrackSpec};
