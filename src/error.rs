//! Crate error type.
//!
//! Every message names what failed, the input, and the next action.

use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::path::PathBuf;

/// The result type of every fallible containr call.
pub type Result<T> = std::result::Result<T, Error>;

/// A demux or mux error.
///
/// `path` fields hold the box or element path inside the container, such as
/// `moov/trak[2]/mdia/minf/stbl/stsz`. A `trak[n]` index counts from 1 in
/// file order.
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
    /// A box holds bytes that do not decode.
    Malformed {
        /// The box path.
        path: String,
        /// The byte offset of the box in the input.
        offset: u64,
        /// What does not decode.
        reason: String,
    },
    /// Two sample tables disagree, or a table refers past another one.
    Inconsistent {
        /// The box path of the table that fails the check.
        path: String,
        /// The values that disagree.
        reason: String,
    },
    /// A size from the input exceeds the bound the demuxer allows.
    TooLarge {
        /// The box path.
        path: String,
        /// The size the input declares, in bytes or entries.
        declared: u64,
        /// The largest accepted value.
        limit: u64,
    },
    /// A required box is absent.
    MissingBox {
        /// The path of the absent box.
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
}

// The constructors serve the container features. A build with none of them
// compiles the type alone.
#[cfg_attr(not(feature = "mp4"), allow(dead_code))]
impl Error {
    pub(crate) fn io(what: impl Into<String>, offset: u64, source: io::Error) -> Self {
        Self::Io {
            what: what.into(),
            offset,
            source,
        }
    }

    pub(crate) fn malformed(
        path: impl Into<String>,
        offset: u64,
        reason: impl Into<String>,
    ) -> Self {
        Self::Malformed {
            path: path.into(),
            offset,
            reason: reason.into(),
        }
    }

    pub(crate) fn inconsistent(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Inconsistent {
            path: path.into(),
            reason: reason.into(),
        }
    }

    pub(crate) fn missing(path: impl Into<String>) -> Self {
        Self::MissingBox { path: path.into() }
    }

    pub(crate) fn too_large(path: impl Into<String>, declared: u64, limit: u64) -> Self {
        Self::TooLarge {
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
                path,
                offset,
                reason,
            } => write!(
                f,
                "box `{path}` at byte offset {offset} is malformed: {reason}. \
                 The input is damaged or not MP4/MOV. Remux it with \
                 `ffmpeg -i <input> -c copy <output.mp4>` and open the output"
            ),
            Self::Inconsistent { path, reason } => write!(
                f,
                "sample table `{path}` is inconsistent: {reason}. \
                 The input is damaged. Remux it with \
                 `ffmpeg -i <input> -c copy <output.mp4>` and open the output"
            ),
            Self::TooLarge {
                path,
                declared,
                limit,
            } => write!(
                f,
                "box `{path}` declares {declared}, above the limit of {limit}. \
                 The input is damaged, or it needs a larger limit than this \
                 demuxer accepts"
            ),
            Self::MissingBox { path } => write!(
                f,
                "required box `{path}` is absent. The input is truncated or \
                 not MP4/MOV. Supply a complete file"
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
