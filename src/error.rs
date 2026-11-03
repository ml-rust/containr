//! Crate error type.
//!
//! Every message names what failed, the input, and the next action.

use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::path::PathBuf;

/// The result type of every fallible containr call.
pub type Result<T> = std::result::Result<T, Error>;

/// The container format an error comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Container {
    /// MP4 or QuickTime MOV.
    Mp4,
    /// Matroska or WebM.
    Matroska,
}

impl Container {
    /// The format name, as error messages print it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4/MOV",
            Self::Matroska => "Matroska/WebM",
        }
    }

    /// What the format calls one structural unit: `box` or `element`.
    pub const fn unit(self) -> &'static str {
        match self {
            Self::Mp4 => "box",
            Self::Matroska => "element",
        }
    }

    /// The output name the remux hint uses.
    const fn remux_output(self) -> &'static str {
        match self {
            Self::Mp4 => "output.mp4",
            Self::Matroska => "output.mkv",
        }
    }
}

/// A demux or mux error.
///
/// `path` fields hold the box or element path inside the container, such as
/// `moov/trak[2]/mdia/minf/stbl/stsz` or `Segment/Tracks/TrackEntry[2]/CodecID`.
/// A `trak[n]` or `TrackEntry[n]` index counts from 1 in file order.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A read or seek on the input failed.
    Io {
        /// What the demuxer was reading.
        what: String,
        /// The byte offset of the read.
        offset: u64,
        /// The underlying error.
        source: io::Error,
    },
    /// A box or element holds bytes that do not decode.
    Malformed {
        /// The container format.
        container: Container,
        /// The box or element path.
        path: String,
        /// The byte offset of the box or element in the input.
        offset: u64,
        /// What does not decode.
        reason: String,
    },
    /// Two tables or elements disagree, or one refers past another one.
    Inconsistent {
        /// The container format.
        container: Container,
        /// The box or element path that fails the check.
        path: String,
        /// The values that disagree.
        reason: String,
    },
    /// A size from the input exceeds the bound the demuxer allows.
    TooLarge {
        /// The container format.
        container: Container,
        /// The box or element path.
        path: String,
        /// The size the input declares, in bytes or entries.
        declared: u64,
        /// The largest accepted value.
        limit: u64,
    },
    /// A required box or element is absent.
    MissingBox {
        /// The container format.
        container: Container,
        /// The path of the absent box or element.
        path: String,
    },
    /// The input is fragmented MP4 (`moof` or `mvex`), which is out of scope.
    Fragmented {
        /// The box path that marks the fragmentation.
        path: String,
        /// The byte offset of that box.
        offset: u64,
    },
    /// No track has the requested id.
    TrackNotFound {
        /// The requested track id.
        track_id: u32,
        /// The track ids the input holds.
        available: Vec<u32>,
    },
    /// A packet refers to bytes outside the input.
    PacketOutOfBounds {
        /// The track id of the packet.
        track_id: u32,
        /// The 0-based sample index of the packet.
        index: u64,
        /// The byte offset of the packet.
        offset: u64,
        /// The packet size in bytes.
        size: u32,
        /// The input length in bytes.
        input_len: u64,
    },
    /// An error while reading a named file.
    InFile {
        /// The file.
        file: PathBuf,
        /// The error.
        source: Box<Error>,
    },
    /// A muxer refuses a stream, codec or item the output container cannot
    /// hold.
    Unsupported {
        /// The output container.
        container: Container,
        /// What the output cannot hold, such as ``codec `S_TEXT/ASS` ``.
        what: String,
        /// The container that holds it.
        use_instead: Container,
    },
    /// A packet's decode timestamp is not above the previous packet's.
    DtsOutOfOrder {
        /// The output track id.
        track_id: u32,
        /// The 0-based index of the packet in the track.
        index: u64,
        /// The packet's decode timestamp.
        dts: i64,
        /// The previous packet's decode timestamp.
        previous: i64,
    },
    /// `pts - dts` fits no composition offset the output can store. Readers
    /// treat `ctts` offsets as signed, so the range is -2^31 to 2^31 - 1.
    CompositionOffset {
        /// The output track id.
        track_id: u32,
        /// The 0-based index of the packet in the track.
        index: u64,
        /// The packet's presentation timestamp.
        pts: i64,
        /// The packet's decode timestamp.
        dts: i64,
        /// Why the offset does not fit.
        reason: String,
    },
    /// A track needs more than one sample description, as after a
    /// parameter-set change. The muxer writes one per track.
    SampleDescriptionChange {
        /// The track id.
        track_id: u32,
        /// The number of sample descriptions the track needs.
        descriptions: u64,
    },
    /// The last packet of a track has no duration, and the track has no
    /// default duration.
    MissingDuration {
        /// The output track id.
        track_id: u32,
        /// The 0-based index of the packet in the track.
        index: u64,
    },
    /// A packet breaks a muxer rule other than the named ones.
    InvalidPacket {
        /// The output container.
        container: Container,
        /// The output track id.
        track_id: u32,
        /// The 0-based index of the packet in the track.
        index: u64,
        /// The rule the packet breaks.
        reason: String,
    },
    /// A muxer call is invalid: an option, a track spec, a metadata item, a
    /// chapter, or the call order.
    InvalidMux {
        /// The output container.
        container: Container,
        /// What is invalid, and the accepted values.
        reason: String,
    },
}

// The constructors serve the container features. A build with none of them
// compiles the type alone. The `mp4_` constructors build MP4/MOV errors.
// The `mkv_` constructors build Matroska/WebM errors.
#[cfg_attr(not(any(feature = "mp4", feature = "mkv")), allow(dead_code))]
impl Error {
    pub(crate) fn io(what: impl Into<String>, offset: u64, source: io::Error) -> Self {
        Self::Io {
            what: what.into(),
            offset,
            source,
        }
    }

    #[cfg_attr(not(feature = "mp4"), allow(dead_code))]
    pub(crate) fn mp4_malformed(
        path: impl Into<String>,
        offset: u64,
        reason: impl Into<String>,
    ) -> Self {
        Self::malformed_in(Container::Mp4, path, offset, reason)
    }

    #[cfg_attr(not(feature = "mp4"), allow(dead_code))]
    pub(crate) fn mp4_inconsistent(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::inconsistent_in(Container::Mp4, path, reason)
    }

    #[cfg_attr(not(feature = "mp4"), allow(dead_code))]
    pub(crate) fn mp4_missing(path: impl Into<String>) -> Self {
        Self::missing_in(Container::Mp4, path)
    }

    #[cfg_attr(not(feature = "mp4"), allow(dead_code))]
    pub(crate) fn mp4_too_large(path: impl Into<String>, declared: u64, limit: u64) -> Self {
        Self::too_large_in(Container::Mp4, path, declared, limit)
    }

    #[cfg_attr(not(feature = "mkv"), allow(dead_code))]
    pub(crate) fn mkv_malformed(
        path: impl Into<String>,
        offset: u64,
        reason: impl Into<String>,
    ) -> Self {
        Self::malformed_in(Container::Matroska, path, offset, reason)
    }

    #[cfg_attr(not(feature = "mkv"), allow(dead_code))]
    pub(crate) fn mkv_inconsistent(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::inconsistent_in(Container::Matroska, path, reason)
    }

    #[cfg_attr(not(feature = "mkv"), allow(dead_code))]
    pub(crate) fn mkv_missing(path: impl Into<String>) -> Self {
        Self::missing_in(Container::Matroska, path)
    }

    #[cfg_attr(not(feature = "mkv"), allow(dead_code))]
    pub(crate) fn mkv_too_large(path: impl Into<String>, declared: u64, limit: u64) -> Self {
        Self::too_large_in(Container::Matroska, path, declared, limit)
    }

    fn malformed_in(
        container: Container,
        path: impl Into<String>,
        offset: u64,
        reason: impl Into<String>,
    ) -> Self {
        Self::Malformed {
            container,
            path: path.into(),
            offset,
            reason: reason.into(),
        }
    }

    fn inconsistent_in(
        container: Container,
        path: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::Inconsistent {
            container,
            path: path.into(),
            reason: reason.into(),
        }
    }

    fn missing_in(container: Container, path: impl Into<String>) -> Self {
        Self::MissingBox {
            container,
            path: path.into(),
        }
    }

    fn too_large_in(
        container: Container,
        path: impl Into<String>,
        declared: u64,
        limit: u64,
    ) -> Self {
        Self::TooLarge {
            container,
            path: path.into(),
            declared,
            limit,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                what,
                offset,
                source,
            } => write!(
                f,
                "reading {what} at byte offset {offset} failed: {source}. \
                 Check that the input is complete and readable"
            ),
            Self::Malformed {
                container,
                path,
                offset,
                reason,
            } => write!(
                f,
                "{} `{path}` at byte offset {offset} is malformed: {reason}. \
                 The input is damaged or not {}. Remux it with \
                 `ffmpeg -i <input> -c copy <{}>` and open the output",
                container.unit(),
                container.name(),
                container.remux_output()
            ),
            Self::Inconsistent {
                container,
                path,
                reason,
            } => write!(
                f,
                "{} `{path}` is inconsistent: {reason}. The input is damaged. \
                 Remux it with `ffmpeg -i <input> -c copy <{}>` and open the output",
                container.unit(),
                container.remux_output()
            ),
            Self::TooLarge {
                container,
                path,
                declared,
                limit,
            } => write!(
                f,
                "{} `{path}` declares {declared}, above the limit of {limit}. \
                 The input is damaged, or it needs a larger limit than this \
                 demuxer accepts",
                container.unit()
            ),
            Self::MissingBox { container, path } => write!(
                f,
                "required {} `{path}` is absent. The input is truncated or \
                 not {}. Supply a complete file",
                container.unit(),
                container.name()
            ),
            Self::Fragmented { path, offset } => write!(
                f,
                "box `{path}` at byte offset {offset} marks fragmented MP4, \
                 which this demuxer does not read. Defragment it with \
                 `ffmpeg -i <input> -c copy <output.mp4>` and open the output"
            ),
            Self::TrackNotFound {
                track_id,
                available,
            } => write!(
                f,
                "no track has id {track_id}. The input holds track ids \
                 {available:?}. Pass one of those ids"
            ),
            Self::PacketOutOfBounds {
                track_id,
                index,
                offset,
                size,
                input_len,
            } => write!(
                f,
                "packet {index} of track {track_id} spans {size} bytes at \
                 offset {offset}, past the input end at {input_len}. The input \
                 is truncated. Supply the complete file"
            ),
            Self::InFile { file, source } => write!(f, "{}: {source}", file.display()),
            Self::Unsupported {
                container,
                what,
                use_instead,
            } => write!(
                f,
                "{} output cannot hold {what}. Mux it into {} instead",
                container.name(),
                use_instead.name()
            ),
            Self::DtsOutOfOrder {
                track_id,
                index,
                dts,
                previous,
            } => write!(
                f,
                "packet {index} of track {track_id} has dts {dts}, not above the \
                 previous dts {previous}. Send the packets of each track in decode \
                 order, with increasing dts"
            ),
            Self::CompositionOffset {
                track_id,
                index,
                pts,
                dts,
                reason,
            } => write!(
                f,
                "packet {index} of track {track_id} has pts {pts} and dts {dts}: \
                 {reason}"
            ),
            Self::SampleDescriptionChange {
                track_id,
                descriptions,
            } => write!(
                f,
                "track {track_id} needs {descriptions} sample descriptions, as after \
                 a parameter-set change. The muxer writes one per track. Split the \
                 stream at the change, or re-encode it with one configuration"
            ),
            Self::MissingDuration { track_id, index } => write!(
                f,
                "packet {index} of track {track_id} is the last of its track and has \
                 no duration, and the track spec has no default duration. Set \
                 `PacketIn::duration` on the last packet, or \
                 `TrackSpec::default_duration`"
            ),
            Self::InvalidPacket {
                container,
                track_id,
                index,
                reason,
            } => write!(
                f,
                "packet {index} of track {track_id} is invalid for {} output: {reason}",
                container.name()
            ),
            Self::InvalidMux { container, reason } => {
                write!(
                    f,
                    "the {} muxer refuses the call: {reason}",
                    container.name()
                )
            }
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InFile { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}
