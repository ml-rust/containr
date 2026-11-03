//! Media containers: boxes, EBML elements, sample tables, packets with
//! timestamps, track metadata and raw edit lists.
//!
//! Each container is a Cargo feature. `mp4` reads and writes MP4 and
//! QuickTime MOV. `mkv` reads Matroska and WebM.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod edit;
pub mod error;
pub mod matroska;
pub mod metadata;
pub mod packet;
pub mod track;

#[cfg(any(feature = "mp4", feature = "mkv"))]
mod read;

#[cfg(feature = "mkv")]
pub mod mkv;
#[cfg(feature = "mp4")]
pub mod mp4;

pub use edit::{EditEntry, EditList};
pub use error::{Container, Error, Result};
pub use metadata::{
    Chapter, ChapterSource, MetadataData, MetadataItem, MetadataKey, MetadataScope, TagScope,
    TagTarget,
};
pub use packet::{Lace, Packet};
pub use track::{
    AudioInfo, CodecConfig, FourCc, Language, PixelAspect, SampleEntry, Timebase, Track, TrackKind,
    TrackReference, VideoInfo,
};
