//! Track facts: kind, codec, timing, geometry and audio layout.

use std::fmt;

use crate::edit::EditList;
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

/// What a track carries, from its handler type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrackKind {
    /// Handler `vide`.
    Video,
    /// Handler `soun`.
    Audio,
    /// Handler `sbtl`, `subt`, `text` or `clcp`.
    Subtitle,
    /// Handler `meta`, `tmcd` or `hint`.
    Data,
    /// Any other handler.
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

    /// The ISO 639-2/T code as a string, when the track has one.
    pub fn iso639(&self) -> Option<&str> {
        match self {
            Self::Iso639(letters) => std::str::from_utf8(letters).ok(),
            Self::Other(_) => None,
        }
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

/// A track and its facts. Timestamps count in `timescale` ticks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    /// The track id from `tkhd`.
    pub id: u32,
    /// What the track carries.
    pub kind: TrackKind,
    /// The `hdlr` handler type.
    pub handler: FourCc,
    /// Ticks per second of the track's timestamps (`mdhd`).
    pub timescale: u32,
    /// Media duration in `timescale` ticks (`mdhd`).
    pub duration: u64,
    /// The `mdhd` language.
    pub language: Language,
    /// The `tkhd` matrix in file order: `a b u c d v x y w`. `a`, `b`, `c`,
    /// `d`, `x` and `y` are 16.16 fixed point, `u`, `v` and `w` are 2.30.
    pub matrix: [i32; 9],
    /// The `tkhd` width, 16.16 fixed point.
    pub display_width: u32,
    /// The `tkhd` height, 16.16 fixed point.
    pub display_height: u32,
    /// The sample descriptions. A packet's `description_index` counts into
    /// this list from 1.
    pub sample_entries: Vec<SampleEntry>,
    /// The raw edit list. The demuxer never applies it.
    pub edit_list: EditList,
    /// The `tref` entries.
    pub references: Vec<TrackReference>,
    /// True when another track names this one in a `chap` reference.
    pub chapter_track: bool,
    /// Track-level metadata from `udta` and `meta`.
    pub metadata: Vec<MetadataItem>,
    /// The number of samples.
    pub sample_count: u64,
}

impl Track {
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
    fn fourcc_displays_unprintable_bytes_escaped() {
        assert_eq!(FourCc(*b"avc1").to_string(), "avc1");
        assert_eq!(FourCc([0xA9, b'n', b'a', b'm']).to_string(), "\\xa9nam");
    }
}
