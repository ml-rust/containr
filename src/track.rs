//! Track facts: kind, codec, timing, geometry and audio layout.

use std::fmt;

use crate::colour::ContainerColour;
use crate::edit::EditList;
#[cfg(feature = "mkv")]
use crate::mac_language::bibliographic;
use crate::mac_language::macintosh_iso639;
#[cfg(feature = "mp4")]
use crate::mac_language::terminology;
use crate::matroska::MatroskaTrack;
use crate::metadata::MetadataItem;

/// A four-character code, as the container stores it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FourCc(pub [u8; 4]);

impl FourCc {
    /// The code as bytes.
    pub const fn bytes(self) -> [u8; 4] {
        self.0
    }
}

impl fmt::Display for FourCc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &byte in &self.0 {
            if byte.is_ascii_graphic() || byte == b' ' {
                write!(f, "{}", byte as char)?;
            } else {
                write!(f, "\\x{byte:02x}")?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for FourCc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FourCc({self})")
    }
}

impl From<&[u8; 4]> for FourCc {
    fn from(bytes: &[u8; 4]) -> Self {
        Self(*bytes)
    }
}

/// What a track carries: from the MP4 handler type, or the Matroska
/// `TrackType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrackKind {
    /// Handler `vide`. `TrackType` 1.
    Video,
    /// Handler `soun`. `TrackType` 2.
    Audio,
    /// Handler `sbtl`, `subt`, `text` or `clcp`. `TrackType` 17.
    Subtitle,
    /// Handler `meta`, `tmcd` or `hint`. `TrackType` 33 (metadata) or 32
    /// (control).
    Data,
    /// Any other handler or `TrackType`.
    Other,
}

impl TrackKind {
    /// The kind a handler type names.
    pub fn from_handler(handler: FourCc) -> Self {
        match &handler.0 {
            b"vide" => Self::Video,
            b"soun" => Self::Audio,
            b"sbtl" | b"subt" | b"text" | b"clcp" => Self::Subtitle,
            b"meta" | b"tmcd" | b"hint" => Self::Data,
            _ => Self::Other,
        }
    }

    /// The kind a Matroska `TrackType` names.
    pub fn from_matroska_type(track_type: u64) -> Self {
        match track_type {
            1 => Self::Video,
            2 => Self::Audio,
            17 => Self::Subtitle,
            32 | 33 => Self::Data,
            _ => Self::Other,
        }
    }
}

/// The duration of one timestamp tick: `num / den` seconds.
///
/// MP4 counts in `1 / timescale` seconds. Matroska counts in
/// `TimestampScale / 1 000 000 000` seconds. Both stay integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Timebase {
    /// Numerator, in seconds.
    pub num: u32,
    /// Denominator.
    pub den: u32,
}

impl Timebase {
    /// The timebase of an MP4 `timescale`: `1 / timescale` seconds.
    pub const fn from_timescale(timescale: u32) -> Self {
        Self {
            num: 1,
            den: timescale,
        }
    }

    /// The timebase of a Matroska `TimestampScale`, in nanoseconds per tick.
    pub const fn from_nanoseconds(nanoseconds: u32) -> Self {
        Self {
            num: nanoseconds,
            den: 1_000_000_000,
        }
    }
}

/// A track language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    /// A packed ISO 639-2/T code, three lowercase letters, such as `und`.
    Iso639([u8; 3]),
    /// A code that is not a packed ISO 639-2/T code: a QuickTime Macintosh
    /// language code (below `0x400`, or `0x7FFF` for unspecified), or a
    /// packed code with letters outside `a`-`z`.
    Other(u16),
}

impl Language {
    /// Decode the 16-bit `mdhd` language field.
    pub fn from_code(code: u16) -> Self {
        if code < 0x400 || code == 0x7FFF {
            return Self::Other(code);
        }
        let letters = [
            (((code >> 10) & 0x1F) as u8) + 0x60,
            (((code >> 5) & 0x1F) as u8) + 0x60,
            ((code & 0x1F) as u8) + 0x60,
        ];
        if letters.iter().all(u8::is_ascii_lowercase) {
            Self::Iso639(letters)
        } else {
            Self::Other(code)
        }
    }

    /// The packed ISO 639-2/T code as a string, when the track has one. A
    /// Macintosh code gives `None`. [`Language::iso639_mapped`] maps it.
    pub fn iso639(&self) -> Option<&str> {
        match self {
            Self::Iso639(letters) => std::str::from_utf8(letters).ok(),
            Self::Other(_) => None,
        }
    }

    /// The ISO 639-2/T code, with a Macintosh code mapped through the
    /// QuickTime table.
    ///
    /// `None` when the table has no entry: `0x7FFF` (unspecified), a code
    /// from 95 to 127, a code above 138, or a packed code with letters
    /// outside `a`-`z`.
    pub fn iso639_mapped(&self) -> Option<&str> {
        match self {
            Self::Iso639(_) => self.iso639(),
            Self::Other(code) => macintosh_iso639(*code),
        }
    }

    /// The code a Matroska `Language` element stores: the bibliographic form,
    /// such as `fre`, or `und` when [`Language::iso639_mapped`] gives none.
    #[cfg(feature = "mkv")]
    pub fn matroska_code(&self) -> &str {
        bibliographic(self.iso639_mapped().unwrap_or("und"))
    }

    /// The language the MP4 `mdhd` box stores: a packed code in terminology
    /// form, such as `fra`. Any other value stays as it is.
    #[cfg(feature = "mp4")]
    pub fn mdhd_form(self) -> Self {
        self.iso639()
            .map(terminology)
            .and_then(|code| <[u8; 3]>::try_from(code.as_bytes()).ok())
            .map_or(self, Self::Iso639)
    }
}

/// Codec configuration from a sample entry, as the raw box body.
///
/// Each variant holds the bytes after the 8-byte box header. For the full
/// boxes (`esds`, `vpcC`) the body starts with the version and flags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecConfig {
    /// `avcC`: AVCDecoderConfigurationRecord.
    Avc(Vec<u8>),
    /// `hvcC`: HEVCDecoderConfigurationRecord.
    Hevc(Vec<u8>),
    /// `av1C`: AV1CodecConfigurationRecord.
    Av1(Vec<u8>),
    /// `vpcC`: VPCodecConfigurationRecord.
    Vp(Vec<u8>),
    /// `esds`: MPEG-4 elementary stream descriptor.
    Esds(Vec<u8>),
    /// `dOps`: Opus specific box.
    Opus(Vec<u8>),
}

impl CodecConfig {
    /// The box type the configuration came from.
    pub fn box_type(&self) -> FourCc {
        FourCc(match self {
            Self::Avc(_) => *b"avcC",
            Self::Hevc(_) => *b"hvcC",
            Self::Av1(_) => *b"av1C",
            Self::Vp(_) => *b"vpcC",
            Self::Esds(_) => *b"esds",
            Self::Opus(_) => *b"dOps",
        })
    }

    /// The raw box body.
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Avc(bytes)
            | Self::Hevc(bytes)
            | Self::Av1(bytes)
            | Self::Vp(bytes)
            | Self::Esds(bytes)
            | Self::Opus(bytes) => bytes,
        }
    }
}

/// The `pasp` pixel aspect ratio: a pixel is `h_spacing` wide for every
/// `v_spacing` of height.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PixelAspect {
    /// Relative pixel width.
    pub h_spacing: u32,
    /// Relative pixel height.
    pub v_spacing: u32,
}

/// Facts from a visual sample entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VideoInfo {
    /// Coded width in pixels.
    pub width: u16,
    /// Coded height in pixels.
    pub height: u16,
    /// The `pasp` box, when present.
    pub pixel_aspect: Option<PixelAspect>,
    /// The first `colr` box that gives H.273 values: `nclx` or QuickTime
    /// `nclc`. `None` when the entry has no such box. A `prof` or `rICC`
    /// box gives no H.273 values.
    pub colour: Option<ContainerColour>,
}

/// Facts from an audio sample entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AudioInfo {
    /// Channel count.
    pub channels: u32,
    /// Bits per sample, as the entry declares it.
    pub sample_size: u32,
    /// Sample rate in Hz. Version 0 and 1 entries store its integer part.
    pub sample_rate: u32,
}

/// One sample description: what `stsd` holds for one codec configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleEntry {
    /// The sample entry type, such as `avc1`, `hvc1` or `mp4a`.
    pub fourcc: FourCc,
    /// The whole sample entry box, header included, for stream copy.
    pub raw: Vec<u8>,
    /// The `data_reference_index` field.
    pub data_reference_index: u16,
    /// The codec configuration box, when the entry holds one.
    pub codec_config: Option<CodecConfig>,
    /// Visual facts, for a video track.
    pub video: Option<VideoInfo>,
    /// Audio facts, for an audio track.
    pub audio: Option<AudioInfo>,
}

/// One `tref` entry: this track refers to `track_ids` for `kind`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackReference {
    /// The reference type, such as `chap` or `tmcd`.
    pub kind: FourCc,
    /// The referenced track ids.
    pub track_ids: Vec<u32>,
}

/// A track and its facts. Timestamps count in `timebase` ticks.
///
/// The `handler`, `matrix`, `display_width`, `display_height`,
/// `sample_entries`, `edit_list`, `references` and `chapter_track` fields
/// hold MP4/MOV facts. A Matroska track leaves them empty, and holds its
/// own facts in `matroska`.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// The track id: the `tkhd` track id, or the Matroska `TrackNumber`.
    pub id: u32,
    /// What the track carries.
    pub kind: TrackKind,
    /// The `hdlr` handler type. `None` for Matroska, which has no handler.
    pub handler: Option<FourCc>,
    /// The tick of the track's timestamps: `1 / mdhd timescale` seconds, or
    /// `TimestampScale` nanoseconds.
    pub timebase: Timebase,
    /// Media duration in `timebase` ticks (`mdhd`). `None` for Matroska,
    /// which stores no per-track duration.
    pub duration: Option<u64>,
    /// The `mdhd` language, or the Matroska `Language` element. A Matroska
    /// value that is not three lowercase letters gives `und`. The raw
    /// strings, `LanguageBCP47` included, are in
    /// [`MatroskaTrack::language`] and [`MatroskaTrack::language_bcp47`].
    pub language: Language,
    /// The `tkhd` matrix in file order: `a b u c d v x y w`. `a`, `b`, `c`,
    /// `d`, `x` and `y` are 16.16 fixed point, `u`, `v` and `w` are 2.30.
    /// Identity for Matroska, which stores rotation as
    /// [`Projection::pose_roll`](crate::matroska::Projection::pose_roll).
    pub matrix: [i32; 9],
    /// The `tkhd` width, 16.16 fixed point. 0 for Matroska.
    pub display_width: u32,
    /// The `tkhd` height, 16.16 fixed point. 0 for Matroska.
    pub display_height: u32,
    /// The sample descriptions. A packet's `description_index` counts into
    /// this list from 1. Empty for Matroska.
    pub sample_entries: Vec<SampleEntry>,
    /// The raw edit list. The demuxer never applies it. Empty for Matroska.
    pub edit_list: EditList,
    /// The `tref` entries. Empty for Matroska.
    pub references: Vec<TrackReference>,
    /// True when another track names this one in a `chap` reference.
    pub chapter_track: bool,
    /// Track-level metadata: `udta` and `meta` items, or Matroska tags
    /// whose target is this track's `TrackUID`.
    pub metadata: Vec<MetadataItem>,
    /// The number of samples: MP4 samples, or Matroska frames after lacing
    /// is expanded.
    pub sample_count: u64,
    /// Matroska facts. `None` for an MP4/MOV track.
    pub matroska: Option<MatroskaTrack>,
}

impl Track {
    /// The Matroska `CodecID`, such as `V_MPEG4/ISO/AVC`.
    pub fn codec_id(&self) -> Option<&str> {
        self.matroska.as_ref().map(|mkv| mkv.codec_id.as_str())
    }

    /// The first sample entry, which holds the codec of most tracks.
    pub fn sample_entry(&self) -> Option<&SampleEntry> {
        self.sample_entries.first()
    }

    /// The sample entry type of the first sample entry.
    pub fn codec(&self) -> Option<FourCc> {
        self.sample_entry().map(|entry| entry.fourcc)
    }

    /// The codec configuration of the first sample entry.
    pub fn codec_config(&self) -> Option<&CodecConfig> {
        self.sample_entry()
            .and_then(|entry| entry.codec_config.as_ref())
    }

    /// Visual facts of the first sample entry.
    pub fn video(&self) -> Option<&VideoInfo> {
        self.sample_entry().and_then(|entry| entry.video.as_ref())
    }

    /// The container colour tags of a video track: the `colr` box of the
    /// first sample entry, or the Matroska `Video/Colour` element.
    pub fn colour(&self) -> Option<ContainerColour> {
        match &self.matroska {
            Some(mkv) => mkv.video.as_ref().and_then(|video| video.colour),
            None => self.video().and_then(|video| video.colour),
        }
    }

    /// Audio facts of the first sample entry.
    pub fn audio(&self) -> Option<&AudioInfo> {
        self.sample_entry().and_then(|entry| entry.audio.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_decodes_packed_iso_codes() {
        assert_eq!(Language::from_code(0x55C4).iso639(), Some("und"));
        assert_eq!(Language::from_code(0x15C7).iso639(), Some("eng"));
    }

    #[test]
    fn language_keeps_macintosh_codes() {
        assert_eq!(Language::from_code(0), Language::Other(0));
        assert_eq!(Language::from_code(0x7FFF), Language::Other(0x7FFF));
    }

    #[test]
    fn iso639_keeps_its_contract_and_iso639_mapped_adds_the_table() {
        let english = Language::from_code(0);
        assert_eq!(english.iso639(), None);
        assert_eq!(english.iso639_mapped(), Some("eng"));
        assert_eq!(Language::from_code(0x15C7).iso639_mapped(), Some("eng"));
        assert_eq!(Language::from_code(0x7FFF).iso639_mapped(), None);
        assert_eq!(Language::Other(0x0400).iso639_mapped(), None);
    }

    #[cfg(feature = "mkv")]
    #[test]
    fn matroska_code_is_bibliographic_with_und_for_no_code() {
        assert_eq!(Language::Iso639(*b"fra").matroska_code(), "fre");
        assert_eq!(Language::Iso639(*b"fre").matroska_code(), "fre");
        assert_eq!(Language::Other(1).matroska_code(), "fre");
        assert_eq!(Language::Other(0x7FFF).matroska_code(), "und");
    }

    #[cfg(feature = "mp4")]
    #[test]
    fn mdhd_form_is_terminology_and_keeps_other_values() {
        assert_eq!(
            Language::Iso639(*b"fre").mdhd_form(),
            Language::Iso639(*b"fra")
        );
        assert_eq!(
            Language::Iso639(*b"eng").mdhd_form(),
            Language::Iso639(*b"eng")
        );
        assert_eq!(Language::Other(1).mdhd_form(), Language::Other(1));
    }

    #[test]
    fn fourcc_displays_unprintable_bytes_escaped() {
        assert_eq!(FourCc(*b"avc1").to_string(), "avc1");
        assert_eq!(FourCc([0xA9, b'n', b'a', b'm']).to_string(), "\\xa9nam");
    }
}
