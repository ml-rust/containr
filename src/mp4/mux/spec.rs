//! What the muxer needs to write one track, and one packet.

use crate::edit::EditList;
use crate::metadata::MetadataItem;
use crate::track::{FourCc, Language, PixelAspect, TrackKind, TrackReference};

/// The identity `tkhd` matrix in file order: `a b u c d v x y w`.
pub const IDENTITY_MATRIX: [i32; 9] = [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000];

/// An output track, as [`Muxer::add_track`] returns it.
///
/// [`Muxer::add_track`]: crate::mp4::Muxer::add_track
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrackId(pub(crate) u32);

impl TrackId {
    /// The `tkhd` track id of the output track.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// A visual codec the muxer builds a sample entry for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VisualCodec {
    /// H.264: `avc1` with `avcC`.
    Avc,
    /// HEVC: `hvc1` with `hvcC`.
    Hevc,
    /// AV1: `av01` with `av1C`.
    Av1,
}

impl VisualCodec {
    /// The sample entry type and the configuration box type.
    pub(crate) const fn boxes(self) -> (&'static [u8; 4], &'static [u8; 4]) {
        match self {
            Self::Avc => (b"avc1", b"avcC"),
            Self::Hevc => (b"hvc1", b"hvcC"),
            Self::Av1 => (b"av01", b"av1C"),
        }
    }
}

/// An audio codec the muxer builds a sample entry for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    /// AAC: `mp4a` with `esds`.
    Aac,
    /// Opus: `Opus` with `dOps`.
    Opus,
}

impl AudioCodec {
    /// The sample entry type and the configuration box type.
    pub(crate) const fn boxes(self) -> (&'static [u8; 4], &'static [u8; 4]) {
        match self {
            Self::Aac => (b"mp4a", b"esds"),
            Self::Opus => (b"Opus", b"dOps"),
        }
    }
}

/// The sample description of a track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SampleEntrySpec {
    /// A whole sample entry box, header included, as
    /// [`SampleEntry::raw`](crate::SampleEntry::raw) holds it. The muxer
    /// writes it unchanged, except `data_reference_index`, which it sets to
    /// 1: the output holds one data reference, the file itself.
    Raw(Vec<u8>),
    /// A visual sample entry built from the codec configuration.
    Visual {
        /// The codec.
        codec: VisualCodec,
        /// The configuration box body: the raw `avcC`, `hvcC` or `av1C`
        /// record, such as a Matroska `CodecPrivate`.
        config: Vec<u8>,
        /// The coded width in pixels.
        width: u16,
        /// The coded height in pixels.
        height: u16,
    },
    /// An audio sample entry built from the codec configuration.
    Audio {
        /// The codec.
        codec: AudioCodec,
        /// The configuration box body: the `esds` body (version and flags
        /// first), or the `dOps` body.
        config: Vec<u8>,
        /// The channel count.
        channels: u16,
        /// The sample rate in Hz. A rate above 65 535 Hz stores as 0, as
        /// the field holds 16 integer bits.
        sample_rate: u32,
    },
    /// A `tx3g` timed text entry with one default style.
    Tx3g,
}

/// How packet bytes become sample bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PacketFormat {
    /// The sample holds the packet bytes unchanged.
    #[default]
    AsIs,
    /// The packet holds UTF-8 cue text with no framing, as a Matroska
    /// `S_TEXT/UTF8` block does. The muxer writes each cue as a `tx3g`
    /// sample: a 16-bit text length, then the text. A gap between cues
    /// becomes an empty sample, since `tx3g` samples cover the timeline.
    /// Every packet needs `pts == dts` and a duration.
    PlainText,
}

/// Everything the muxer needs to write one `trak`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackSpec {
    /// What the track carries.
    pub kind: TrackKind,
    /// The `hdlr` handler type, such as `vide`, `soun`, `sbtl` or `text`.
    pub handler: FourCc,
    /// The `hdlr` name.
    pub handler_name: String,
    /// Ticks per second of the media timescale (`mdhd`). Packet timestamps
    /// count in it.
    pub timescale: u32,
    /// The sample description.
    pub sample_entry: SampleEntrySpec,
    /// The `mdhd` language.
    pub language: Language,
    /// The `tkhd` matrix in file order: `a b u c d v x y w`.
    pub matrix: [i32; 9],
    /// The `tkhd` width, 16.16 fixed point.
    pub display_width: u32,
    /// The `tkhd` height, 16.16 fixed point.
    pub display_height: u32,
    /// The `pasp` box of a built visual entry. A [`SampleEntrySpec::Raw`]
    /// entry keeps the `pasp` it holds, and the muxer ignores this field.
    pub pixel_aspect: Option<PixelAspect>,
    /// The `tkhd` enabled flag.
    pub enabled: bool,
    /// The edit list. Empty writes no `edts`. Segment durations count in
    /// `movie_timescale`. A different timescale is converted when the
    /// conversion is exact.
    pub edit_list: EditList,
    /// The `tref` entries. Track ids name output tracks. The muxer adds a
    /// `chap` reference itself when the movie has chapters.
    pub references: Vec<TrackReference>,
    /// Track-level metadata, written to `trak/udta` and `trak/meta`.
    pub metadata: Vec<MetadataItem>,
    /// How packet bytes become sample bytes.
    pub packet_format: PacketFormat,
    /// The duration of a last packet that has none, in `timescale` ticks.
    pub default_duration: Option<u32>,
}

impl TrackSpec {
    /// A spec with the defaults for `kind`: the handler and handler name
    /// FFmpeg writes, language `und`, the identity matrix, no edits, no
    /// metadata, and the track enabled.
    pub fn new(kind: TrackKind, timescale: u32, sample_entry: SampleEntrySpec) -> Self {
        let (handler, handler_name) = match kind {
            TrackKind::Video => (b"vide", "VideoHandler"),
            TrackKind::Audio => (b"soun", "SoundHandler"),
            TrackKind::Subtitle => (b"sbtl", "SubtitleHandler"),
            TrackKind::Data | TrackKind::Other => (b"meta", "DataHandler"),
        };
        Self {
            kind,
            handler: FourCc(*handler),
            handler_name: handler_name.to_owned(),
            timescale,
            sample_entry,
            language: Language::Iso639(*b"und"),
            matrix: IDENTITY_MATRIX,
            display_width: 0,
            display_height: 0,
            pixel_aspect: None,
            enabled: true,
            edit_list: EditList::default(),
            references: Vec::new(),
            metadata: Vec::new(),
            packet_format: PacketFormat::AsIs,
            default_duration: None,
        }
    }
}

/// One packet to write. Timestamps count in the track timescale.
///
/// The `stts` table stores decode time deltas, so a packet's duration is the
/// next packet's `dts` minus its own. A packet that sets `duration` must
/// match that delta. The last packet's duration ends the track: it comes
/// from `duration`, else from [`TrackSpec::default_duration`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketIn<'a> {
    /// The packet bytes.
    pub data: &'a [u8],
    /// Presentation timestamp.
    pub pts: i64,
    /// Decode timestamp. The first packet of a track decodes at 0, and each
    /// later one above the previous.
    pub dts: i64,
    /// Duration in ticks.
    pub duration: Option<u64>,
    /// True for a sync sample (a random access point).
    pub sync: bool,
}
