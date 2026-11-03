//! Container metadata and chapters.

use crate::track::FourCc;

/// Where a metadata item sits in the container.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetadataScope {
    /// A direct child of `udta`, such as a QuickTime `©nam` text box.
    Udta,
    /// An item of `meta/ilst`.
    Ilst,
}

/// The key of a metadata item.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MetadataKey {
    /// The box type, such as `©nam` or `----`.
    FourCc(FourCc),
    /// The key name a QuickTime `keys` box gives the item, namespace
    /// excluded.
    Named(Vec<u8>),
}

/// One typed value inside an `ilst` item (a `data` box).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MetadataData {
    /// The type indicator. `1` is UTF-8 text, `21` a signed big-endian
    /// integer, `13` JPEG and `14` PNG.
    pub type_indicator: u32,
    /// The locale field.
    pub locale: u32,
    /// The value bytes.
    pub payload: Vec<u8>,
}

/// One metadata item, kept raw.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MetadataItem {
    /// Where the item sits.
    pub scope: MetadataScope,
    /// The item key.
    pub key: MetadataKey,
    /// The item box body, after the 8-byte header.
    pub value: Vec<u8>,
    /// The `data` boxes of an `ilst` item. Empty for a `udta` item.
    pub data: Vec<MetadataData>,
}

impl MetadataItem {
    /// The first UTF-8 text value of an `ilst` item.
    pub fn text(&self) -> Option<&str> {
        self.data
            .iter()
            .find(|data| data.type_indicator == 1)
            .and_then(|data| std::str::from_utf8(&data.payload).ok())
    }
}

/// Where a chapter came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChapterSource {
    /// The Nero `chpl` box in `moov/udta`.
    Chpl,
    /// A QuickTime chapter track, named by a `tref` `chap` reference.
    Track(u32),
}

/// One chapter.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Chapter {
    /// The chapter start in `timescale` ticks.
    pub start: u64,
    /// The chapter duration in `timescale` ticks, when the source stores one.
    pub duration: Option<u64>,
    /// Ticks per second of `start` and `duration`. `chpl` counts in
    /// 100-nanosecond ticks, so its timescale is 10 000 000.
    pub timescale: u32,
    /// The chapter title.
    pub title: String,
    /// Where the chapter came from.
    pub source: ChapterSource,
}
