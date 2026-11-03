//! Media containers: boxes, sample tables, packets with timestamps, track
//! metadata and raw edit lists.
//!
//! Each container is a Cargo feature. `mp4` reads MP4 and QuickTime MOV.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod edit;
pub mod error;
pub mod metadata;
pub mod packet;
pub mod track;

#[cfg(feature = "mp4")]
pub mod mp4;

pub use edit::{EditEntry, EditList};
pub use error::{Error, Result};
pub use metadata::{
    Chapter, ChapterSource, MetadataData, MetadataItem, MetadataKey, MetadataScope,
};
pub use packet::Packet;
pub use track::{
    AudioInfo, CodecConfig, FourCc, Language, PixelAspect, SampleEntry, Track, TrackKind,
    TrackReference, VideoInfo,
};
