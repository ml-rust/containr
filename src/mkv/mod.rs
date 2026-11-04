//! Matroska and WebM demux, mux and stream copy.
//!
//! The EBML reader and writer are containr's own: every size is checked
//! against the bytes that hold it, and against a stated limit before anything
//! is allocated from it. Errors name the element path, such as
//! `Segment/Cluster[3]/SimpleBlock[7]`.
//!
//! Timestamps count in `TimestampScale` nanoseconds, the track
//! [`Timebase`](crate::Timebase). Matroska stores no decode timestamp, so
//! [`Packet::dts`](crate::Packet::dts) is `None`. `CodecDelay` and
//! `DiscardPadding` come raw. The caller applies them.
//!
//! A single-valued element that repeats keeps its last value.
//!
//! The muxer stores no timestamp it cannot state exactly. A value converts to
//! `TimestampScale` ticks when it is a whole number of nanoseconds and a whole
//! multiple of the scale. [`StreamCopy::timestamp_scale`] picks a scale that
//! holds a stream copy. A value with no whole number of nanoseconds, such as
//! an AAC frame at 48 kHz in an MP4 timescale, fits no scale: the mux refuses
//! it, and the error names MP4. [`StreamCopy::plan_rounded`] is the opt-in that
//! rounds an MP4 track to a scale the caller names.

mod attachments;
mod block;
mod chapters;
mod cluster;
mod colour;
mod demuxer;
mod ebml;
mod ebml_write;
mod frames;
mod ids;
mod index;
mod info;
mod mux;
mod packets;
mod segment;
mod source;
mod tags;
mod timing;
mod track_media;
mod tracks;

pub use attachments::{Attachment, MAX_ATTACHMENT_FIELD};
pub use chapters::MAX_NESTING;
pub use cluster::{MAX_FRAMES, MAX_LACED_BLOCK_SIZE};
pub use demuxer::Demuxer;
pub use index::{CuePoint, CuePosition, SeekEntry};
pub use info::{DEFAULT_TIMESTAMP_SCALE, EbmlHeader, SegmentInfo};
pub use mux::{
    AttachmentIn, DEFAULT_CLUSTER_BYTES, DEFAULT_CLUSTER_DURATION_MS, DEFAULT_MAX_BUFFER, DocType,
    MAX_CLUSTER_BYTES, MAX_CLUSTER_DURATION_MS, MAX_CUE_POINTS, MAX_TRACKS, MIN_CLUSTER_BYTES,
    MkvOptions, Muxer, NoSeek, PacketIn, PacketTiming, StreamCopy, TimestampRounding, TrackId,
    TrackSpec, tags_from_mp4,
};
pub use packets::{Packets, TrackPackets};
pub use segment::MAX_ELEMENT_SIZE;
pub use timing::{TrackTiming, first_track_timing};
