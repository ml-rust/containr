//! Matroska and WebM demux.
//!
//! The EBML reader is containr's own: every size is checked against the
//! bytes that hold it, and against a stated limit before anything is
//! allocated from it. Errors name the element path, such as
//! `Segment/Cluster[3]/SimpleBlock[7]`.
//!
//! Timestamps count in `TimestampScale` nanoseconds, the track
//! [`Timebase`](crate::Timebase). Matroska stores no decode timestamp, so
//! [`Packet::dts`](crate::Packet::dts) is `None`. `CodecDelay` and
//! `DiscardPadding` come raw. The caller applies them.
//!
//! A single-valued element that repeats keeps its last value.

mod attachments;
mod block;
mod chapters;
mod cluster;
mod demuxer;
mod ebml;
mod frames;
mod ids;
mod index;
mod info;
mod packets;
mod segment;
mod source;
mod tags;
mod track_media;
mod tracks;

pub use attachments::{Attachment, MAX_ATTACHMENT_FIELD};
pub use chapters::MAX_NESTING;
pub use cluster::{MAX_FRAMES, MAX_LACED_BLOCK_SIZE};
pub use demuxer::Demuxer;
pub use index::{CuePoint, CuePosition, SeekEntry};
pub use info::{DEFAULT_TIMESTAMP_SCALE, EbmlHeader, SegmentInfo};
pub use packets::{Packets, TrackPackets};
pub use segment::MAX_ELEMENT_SIZE;
