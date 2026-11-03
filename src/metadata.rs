//! Container metadata and chapters.

use crate::track::FourCc;

/// Where a metadata item sits in the container.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetadataScope {
    /// A direct child of `udta`, such as a QuickTime `©nam` text box.
    Udta,
    /// An item of `meta/ilst`.
    Ilst,
    /// A Matroska `SimpleTag`.
    Tag(TagScope),
}

/// What a Matroska `Tag` applies to, from its `Targets`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TagTarget {
    /// The whole segment: the `Targets` names no UID.
    Segment,
    /// The track with this `TrackUID`.
    Track(u64),
    /// The edition with this `EditionUID`.
    Edition(u64),
    /// The chapter with this `ChapterUID`.
    Chapter(u64),
    /// The attachment with this `FileUID`.
    Attachment(u64),
}

/// Where a Matroska `SimpleTag` sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TagScope {
    /// The `TargetTypeValue`: 50 is an album, movie or episode, 30 a track
    /// or chapter.
    pub target_type_value: u64,
    /// The UID the tag applies to. A `Targets` that names several UIDs
    /// gives one item per UID.
    pub target: TagTarget,
    /// True for a `TagBinary` value, false for a `TagString` value.
    pub binary: bool,
}

/// The key of a metadata item.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MetadataKey {
    /// The box type, such as `©nam` or `----`.
    FourCc(FourCc),
    /// The key name a QuickTime `keys` box gives the item, namespace
    /// excluded, or the Matroska `TagName`. A nested `SimpleTag` joins its
    /// parents' names with `/`, such as `ARTIST/URL`.
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
    /// The item box body, after the 8-byte header, or the Matroska
    /// `TagString` or `TagBinary` bytes.
    pub value: Vec<u8>,
    /// The `data` boxes of an `ilst` item. Empty for a `udta` item and for
    /// a Matroska tag.
    pub data: Vec<MetadataData>,
}

impl MetadataItem {
    /// The text value: the first UTF-8 `data` box of an `ilst` item, or the
    /// `TagString` of a Matroska tag.
    pub fn text(&self) -> Option<&str> {
        match self.scope {
            MetadataScope::Tag(scope) if !scope.binary => std::str::from_utf8(&self.value).ok(),
            MetadataScope::Tag(_) => None,
            MetadataScope::Udta | MetadataScope::Ilst => self
                .data
                .iter()
                .find(|data| data.type_indicator == 1)
                .and_then(|data| std::str::from_utf8(&data.payload).ok()),
        }
    }

    /// The key as text, for a [`MetadataKey::Named`] key.
    pub fn name(&self) -> Option<&str> {
        match &self.key {
            MetadataKey::Named(name) => std::str::from_utf8(name).ok(),
            MetadataKey::FourCc(_) => None,
        }
    }
}

/// Where a chapter came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChapterSource {
    /// The Nero `chpl` box in `moov/udta`.
    Chpl,
    /// A QuickTime chapter track, named by a `tref` `chap` reference.
    Track(u32),
    /// A Matroska `ChapterAtom`. Nested atoms follow their parent in
    /// depth-first order.
    Matroska {
        /// The `EditionUID` of the edition that holds the atom, when set.
        edition_uid: Option<u64>,
        /// The `ChapterUID`.
        chapter_uid: u64,
        /// The nesting depth: 0 for an atom directly in its edition.
        depth: u32,
    },
}

/// One chapter.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Chapter {
    /// The chapter start in `timescale` ticks.
    pub start: u64,
    /// The chapter duration in `timescale` ticks, when the source stores one.
    pub duration: Option<u64>,
    /// Ticks per second of `start` and `duration`. `chpl` counts in
    /// 100-nanosecond ticks, so its timescale is 10 000 000. Matroska
    /// counts in nanoseconds, so its timescale is 1 000 000 000.
    pub timescale: u32,
    /// The chapter title.
    pub title: String,
    /// Where the chapter came from.
    pub source: ChapterSource,
}
