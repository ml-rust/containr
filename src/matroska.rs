//! Matroska track facts with no MP4 counterpart.
//!
//! [`Track::matroska`](crate::Track::matroska) holds them. Durations count
//! in nanoseconds, as Matroska stores them. Floats hold elements that
//! Matroska stores as floats: sample rates and projection angles, never
//! timestamps.

use crate::colour::ContainerColour;
#[cfg(any(feature = "mp4", feature = "mkv"))]
use crate::packet::Packet;
#[cfg(any(feature = "mp4", feature = "mkv"))]
use crate::track::{Track, TrackKind};

/// The duration a stream copy gives the last packet of a Matroska track, in
/// track ticks. `packets` holds every packet of the track.
///
/// A stored duration stays as it is. With none stored, an audio or video packet
/// lasts the gap between the track's two latest distinct timestamps. Audio
/// from FFmpeg 8 and older stores no duration at all.
///
/// A positive `DiscardPadding` comes off that gap, since a stored
/// `BlockDuration` excludes it in the files FFmpeg writes. The result rounds to
/// the nearest tick, halves up, and is at least 1 tick.
///
/// `None` keeps the muxer's [`MissingDuration`](crate::Error::MissingDuration)
/// refusal. It covers a track with one timestamp, a padding that covers the
/// gap, and a subtitle track, whose cue length has no relation to the gap
/// between cues.
#[cfg(any(feature = "mp4", feature = "mkv"))]
pub(crate) fn last_packet_duration(track: &Track, packets: &[Packet]) -> Option<u64> {
    let last = packets.last()?;
    if last.duration.is_some() {
        return last.duration;
    }
    if !matches!(track.kind, TrackKind::Audio | TrackKind::Video) {
        return None;
    }
    let latest = packets.iter().map(|packet| packet.pts).max()?;
    let previous = packets
        .iter()
        .map(|packet| packet.pts)
        .filter(|&pts| pts < latest)
        .max()?;
    let gap = u64::try_from(i128::from(latest) - i128::from(previous)).ok()?;
    let padding = i128::from(last.discard_padding.unwrap_or(0).max(0));
    if padding == 0 {
        return Some(gap);
    }
    // A Matroska tick counts `Timebase::num` nanoseconds.
    let tick = i128::from(track.timebase.num.max(1));
    let left = i128::from(gap) * tick - padding;
    if left <= 0 {
        return None;
    }
    u64::try_from(((2 * left + tick) / (2 * tick)).max(1)).ok()
}

/// One child element containr does not model. The demuxer keeps it raw, and
/// stream copy writes it back unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RawElement {
    /// The element ID, marker bits included.
    pub id: u32,
    /// The element body.
    pub body: Vec<u8>,
}

/// The facts of one Matroska `TrackEntry`.
#[derive(Clone, Debug, PartialEq)]
pub struct MatroskaTrack {
    /// The `TrackUID`. Tags name a track by it.
    pub uid: Option<u64>,
    /// The raw `TrackType`.
    pub track_type: u64,
    /// The `CodecID`, such as `V_MPEG4/ISO/AVC` or `A_OPUS`.
    pub codec_id: String,
    /// The `CodecPrivate`: the codec configuration record, such as an
    /// `avcC` body for `V_MPEG4/ISO/AVC`. The demuxer undoes the
    /// `ContentEncodings` that apply to it when `content_decoded` is true.
    pub codec_private: Option<Vec<u8>>,
    /// The `CodecName`.
    pub codec_name: Option<String>,
    /// The track `Name`.
    pub name: Option<String>,
    /// The raw `Language` element. Matroska's default is `eng`.
    pub language: String,
    /// The `LanguageBCP47` element. When present, it overrides `language`.
    pub language_bcp47: Option<String>,
    /// The `DefaultDuration`: nanoseconds per frame.
    pub default_duration: Option<u64>,
    /// The `CodecDelay` in nanoseconds: decoder delay to drop from the
    /// start. The demuxer never applies it.
    pub codec_delay: u64,
    /// The `SeekPreRoll` in nanoseconds: how far before a seek target to
    /// start decoding.
    pub seek_pre_roll: u64,
    /// `FlagEnabled`.
    pub enabled: bool,
    /// `FlagDefault`.
    pub default: bool,
    /// `FlagForced`.
    pub forced: bool,
    /// `FlagLacing`: blocks of this track can be laced.
    pub lacing: bool,
    /// The `ContentEncodings`, in file order, as the file stores them.
    ///
    /// The demuxer undoes header stripping and zlib compression on read.
    /// [`ContentEncoding::undone_on_read`] names the encodings it undoes.
    /// The muxer writes this list as it is, and it applies no encoding.
    pub content_encodings: Vec<ContentEncoding>,
    /// True when every packet and the `CodecPrivate` the demuxer returns
    /// are plain bytes: the track has no `ContentEncodings`, or the demuxer
    /// undoes all of them on read.
    ///
    /// False when one encoding is one the demuxer does not undo, such as
    /// encryption, bzlib or lzo1x. Then packets and `CodecPrivate` stay as
    /// stored, and the caller undoes every encoding.
    ///
    /// `mkv::TrackSpec::copy_from` of a track where this is true drops
    /// `content_encodings`, since the packets it writes are plain. The muxer
    /// ignores this field.
    pub content_decoded: bool,
    /// The `Video` element, for a video track.
    pub video: Option<MatroskaVideo>,
    /// The `Audio` element, for an audio track.
    pub audio: Option<MatroskaAudio>,
    /// The other `TrackEntry` children, such as `MaxBlockAdditionID` and
    /// `BlockAdditionMapping`, in file order.
    pub extra: Vec<RawElement>,
}

/// The `DisplayUnit` of `DisplayWidth` and `DisplayHeight`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DisplayUnit {
    /// 0: pixels, the default.
    Pixels,
    /// 1: centimeters.
    Centimeters,
    /// 2: inches.
    Inches,
    /// 3: a display aspect ratio. The two values carry the ratio only.
    AspectRatio,
    /// 4: unknown.
    Unknown,
    /// Any other value.
    Other(u64),
}

impl DisplayUnit {
    /// The unit a raw `DisplayUnit` value names.
    pub fn from_value(value: u64) -> Self {
        match value {
            0 => Self::Pixels,
            1 => Self::Centimeters,
            2 => Self::Inches,
            3 => Self::AspectRatio,
            4 => Self::Unknown,
            other => Self::Other(other),
        }
    }

    /// The raw `DisplayUnit` value of this unit.
    pub const fn value(self) -> u64 {
        match self {
            Self::Pixels => 0,
            Self::Centimeters => 1,
            Self::Inches => 2,
            Self::AspectRatio => 3,
            Self::Unknown => 4,
            Self::Other(value) => value,
        }
    }
}

/// The `Projection` element of a video track.
#[derive(Clone, Debug, PartialEq)]
pub struct Projection {
    /// The `ProjectionType`: 0 rectangular, 1 equirectangular, 2 cubemap,
    /// 3 mesh.
    pub projection_type: u64,
    /// The raw `ProjectionPrivate`.
    pub private: Option<Vec<u8>>,
    /// `ProjectionPoseYaw` in degrees.
    pub pose_yaw: f64,
    /// `ProjectionPosePitch` in degrees.
    pub pose_pitch: f64,
    /// `ProjectionPoseRoll` in degrees, counter-clockwise. FFmpeg writes a
    /// display rotation here.
    pub pose_roll: f64,
}

/// The `Video` element of a track.
#[derive(Clone, Debug, PartialEq)]
pub struct MatroskaVideo {
    /// `PixelWidth`: the coded width.
    pub pixel_width: u64,
    /// `PixelHeight`: the coded height.
    pub pixel_height: u64,
    /// `DisplayWidth`, in `display_unit`. Matroska's default is the pixel
    /// width after cropping.
    pub display_width: Option<u64>,
    /// `DisplayHeight`, in `display_unit`. Matroska's default is the pixel
    /// height after cropping.
    pub display_height: Option<u64>,
    /// `DisplayUnit`.
    pub display_unit: DisplayUnit,
    /// `PixelCropTop`, `PixelCropBottom`, `PixelCropLeft`, `PixelCropRight`.
    pub crop: [u64; 4],
    /// `FlagInterlaced`: 0 undetermined, 1 interlaced, 2 progressive.
    pub interlaced: u64,
    /// The `Projection` element.
    pub projection: Option<Projection>,
    /// The H.273 tags of the `Colour` element: `MatrixCoefficients`,
    /// `TransferCharacteristics`, `Primaries` and `Range`. `None` when the
    /// element is absent or holds none of them. The demuxer also keeps the
    /// whole `Colour` element in `extra`, so stream copy keeps
    /// `MasteringMetadata` and `MaxCLL`.
    ///
    /// The muxer writes the `extra` element unchanged when it parses to
    /// this value. A different value replaces the four tags in it and keeps
    /// its other children. A spec built by hand sets this field and leaves
    /// `extra` free of `Colour`.
    pub colour: Option<ContainerColour>,
    /// The other `Video` children, such as `StereoMode` and `AlphaMode`,
    /// and the raw `Colour` element, in file order.
    pub extra: Vec<RawElement>,
}

/// The `Audio` element of a track.
#[derive(Clone, Debug, PartialEq)]
pub struct MatroskaAudio {
    /// `SamplingFrequency` in Hz. Matroska's default is 8000.
    pub sampling_frequency: f64,
    /// `OutputSamplingFrequency` in Hz, for codecs such as HE-AAC.
    pub output_sampling_frequency: Option<f64>,
    /// `Channels`. Matroska's default is 1.
    pub channels: u64,
    /// `BitDepth`.
    pub bit_depth: Option<u64>,
    /// The other `Audio` children, in file order.
    pub extra: Vec<RawElement>,
}

/// One `ContentEncoding`: a compression or encryption the track's packets
/// carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentEncoding {
    /// `ContentEncodingOrder`: the order to undo encodings, highest first.
    pub order: u64,
    /// `ContentEncodingScope`: 1 frames, 2 `CodecPrivate`, 4 the next
    /// encoding's settings.
    pub scope: u64,
    /// `ContentEncodingType`: 0 compression, 1 encryption.
    pub encoding_type: u64,
    /// `ContentCompAlgo`, for compression: 0 zlib, 1 bzlib, 2 lzo1x, 3
    /// header stripping.
    pub compression_algorithm: Option<u64>,
    /// `ContentCompSettings`: for header stripping, the bytes to put back
    /// in front of each frame.
    pub compression_settings: Option<Vec<u8>>,
    /// `ContentEncAlgo`, for encryption.
    pub encryption_algorithm: Option<u64>,
    /// The raw `ContentEncryption` body, key IDs included.
    pub encryption_settings: Option<Vec<u8>>,
}

impl ContentEncoding {
    /// True when the Matroska demuxer undoes this encoding on read: header
    /// stripping (`ContentCompAlgo` 3) or zlib (`ContentCompAlgo` 0), on
    /// frames, `CodecPrivate`, or both.
    ///
    /// Encryption, bzlib, lzo1x, an unknown algorithm, and scope 4 (the
    /// next encoding's settings) stay as stored.
    pub fn undone_on_read(&self) -> bool {
        self.encoding_type == 0
            && self.scope & !3 == 0
            && matches!(self.compression_algorithm, Some(0 | 3))
    }

    /// A short name for this encoding, such as `zlib compression`, for
    /// errors and logs.
    pub fn label(&self) -> String {
        let what = match (self.encoding_type, self.compression_algorithm) {
            (0, Some(0)) => "zlib compression".to_owned(),
            (0, Some(1)) => "bzlib compression".to_owned(),
            (0, Some(2)) => "lzo1x compression".to_owned(),
            (0, Some(3)) => "header stripping".to_owned(),
            (0, Some(other)) => format!("compression with `ContentCompAlgo` {other}"),
            (0, None) => "compression with no `ContentCompression`".to_owned(),
            (1, _) => match self.encryption_algorithm {
                Some(algorithm) => format!("encryption with `ContentEncAlgo` {algorithm}"),
                None => "encryption".to_owned(),
            },
            (other, _) => format!("`ContentEncodingType` {other}"),
        };
        if self.scope & !3 != 0 {
            format!("{what} with `ContentEncodingScope` {}", self.scope)
        } else {
            what
        }
    }
}
