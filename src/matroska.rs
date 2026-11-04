//! Matroska track facts with no MP4 counterpart.
//!
//! [`Track::matroska`](crate::Track::matroska) holds them. Durations count
//! in nanoseconds, as Matroska stores them. Floats hold elements that
//! Matroska stores as floats: sample rates and projection angles, never
//! timestamps.

use crate::colour::ContainerColour;

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
    /// The raw `CodecPrivate`: the codec configuration record, such as an
    /// `avcC` body for `V_MPEG4/ISO/AVC`.
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
    /// The `ContentEncodings`, in file order. Packet bytes stay as stored:
    /// the caller undoes the encodings.
    pub content_encodings: Vec<ContentEncoding>,
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
