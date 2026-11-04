//! Muxer options: doc type, `TimestampScale`, Cluster limits and layout.

use crate::error::Result;
use crate::mkv::attachments::MAX_ATTACHMENT_FIELD;
use crate::mkv::info::DEFAULT_TIMESTAMP_SCALE;
use crate::mkv::mux::bytes::invalid;

/// The `MuxingApp` value, and the default `WritingApp`.
pub(crate) const APP_NAME: &str = concat!("containr ", env!("CARGO_PKG_VERSION"));
/// The default longest Cluster, in milliseconds.
pub const DEFAULT_CLUSTER_DURATION_MS: u32 = 5000;
/// The largest Cluster duration accepted, in milliseconds.
pub const MAX_CLUSTER_DURATION_MS: u32 = 3_600_000;
/// The default Cluster body size at which the Cluster closes, in bytes.
pub const DEFAULT_CLUSTER_BYTES: usize = 5 << 20;
/// The smallest `cluster_bytes` accepted.
pub const MIN_CLUSTER_BYTES: usize = 1 << 10;
/// The largest `cluster_bytes` accepted. The muxer holds one Cluster in memory.
pub const MAX_CLUSTER_BYTES: usize = 1 << 30;
/// The default bound on packet bytes held before they reach a Cluster.
pub const DEFAULT_MAX_BUFFER: usize = 64 << 20;

/// The EBML `DocType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DocType {
    /// `matroska`: every codec, and attachments.
    Matroska,
    /// `webm`: VP8, VP9, AV1, Opus, Vorbis and WebVTT only, with no
    /// attachments. The muxer refuses anything else, and the error names
    /// Matroska.
    WebM,
}

impl DocType {
    /// The `DocType` value.
    pub(crate) const fn value(self) -> &'static str {
        match self {
            Self::Matroska => "matroska",
            Self::WebM => "webm",
        }
    }
}

/// True for a `CodecID` WebM can hold.
pub(crate) fn webm_allows(codec_id: &str) -> bool {
    matches!(
        codec_id,
        "V_VP8"
            | "V_VP9"
            | "V_AV1"
            | "A_OPUS"
            | "A_VORBIS"
            | "S_TEXT/WEBVTT"
            | "D_WEBVTT/SUBTITLES"
            | "D_WEBVTT/CAPTIONS"
            | "D_WEBVTT/DESCRIPTIONS"
            | "D_WEBVTT/METADATA"
    )
}

/// How the muxer lays out its output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MkvOptions {
    /// The EBML `DocType`.
    pub doc_type: DocType,
    /// `TimestampScale`: nanoseconds per output tick, 1 to 2^32 - 1. Block
    /// timestamps and durations count in it, and a value that does not land
    /// on a whole tick is refused.
    /// [`StreamCopy::timestamp_scale`](crate::mkv::StreamCopy::timestamp_scale)
    /// picks a scale that holds a stream copy.
    pub timestamp_scale: u64,
    /// The longest Cluster, in milliseconds of block time.
    pub cluster_duration_ms: u32,
    /// The Cluster body size in bytes at which the Cluster closes.
    pub cluster_bytes: usize,
    /// The most packet bytes held before they reach a Cluster. Past this
    /// bound the earliest block leaves even when another track has sent no
    /// packet up to that time.
    pub max_buffer: usize,
    /// Write `Cues`.
    pub cues: bool,
    /// Patch the output at [`Muxer::finish`](crate::mkv::Muxer::finish): the
    /// `SeekHead`, the Segment size and `Duration`. With false, the muxer
    /// never seeks: the Segment keeps an unknown size, and the output has no
    /// `SeekHead` and no `Duration`.
    pub seekable: bool,
    /// The `Title` element.
    pub title: Option<String>,
    /// The `WritingApp` element.
    pub writing_app: String,
}

impl Default for MkvOptions {
    fn default() -> Self {
        Self {
            doc_type: DocType::Matroska,
            timestamp_scale: DEFAULT_TIMESTAMP_SCALE,
            cluster_duration_ms: DEFAULT_CLUSTER_DURATION_MS,
            cluster_bytes: DEFAULT_CLUSTER_BYTES,
            max_buffer: DEFAULT_MAX_BUFFER,
            cues: true,
            seekable: true,
            title: None,
            writing_app: APP_NAME.to_owned(),
        }
    }
}

impl MkvOptions {
    /// Check every field against its accepted range.
    pub(crate) fn check(&self) -> Result<()> {
        if self.timestamp_scale == 0 || self.timestamp_scale > u64::from(u32::MAX) {
            return Err(invalid(format!(
                "`MkvOptions::timestamp_scale` is {}. Use 1 to {} nanoseconds per tick",
                self.timestamp_scale,
                u32::MAX
            )));
        }
        if self.cluster_duration_ms == 0 || self.cluster_duration_ms > MAX_CLUSTER_DURATION_MS {
            return Err(invalid(format!(
                "`MkvOptions::cluster_duration_ms` is {}. Use 1 to {MAX_CLUSTER_DURATION_MS}",
                self.cluster_duration_ms
            )));
        }
        if !(MIN_CLUSTER_BYTES..=MAX_CLUSTER_BYTES).contains(&self.cluster_bytes) {
            return Err(invalid(format!(
                "`MkvOptions::cluster_bytes` is {}. Use {MIN_CLUSTER_BYTES} to {MAX_CLUSTER_BYTES}",
                self.cluster_bytes
            )));
        }
        if self.max_buffer < self.cluster_bytes {
            return Err(invalid(format!(
                "`MkvOptions::max_buffer` is {}, below `cluster_bytes` {}. Use at least \
                 `cluster_bytes`",
                self.max_buffer, self.cluster_bytes
            )));
        }
        let title = self.title.as_deref().unwrap_or_default();
        for (field, value) in [("title", title), ("writing_app", self.writing_app.as_str())] {
            if value.len() as u64 > MAX_ATTACHMENT_FIELD {
                return Err(invalid(format!(
                    "`MkvOptions::{field}` holds {} bytes, above the limit of \
                     {MAX_ATTACHMENT_FIELD}. Use a shorter text",
                    value.len()
                )));
            }
        }
        Ok(())
    }

    /// The longest Cluster in output ticks, 1 or more.
    pub(crate) fn cluster_ticks(&self) -> i64 {
        let ticks = u128::from(self.cluster_duration_ms) * 1_000_000
            / u128::from(self.timestamp_scale.max(1));
        i64::try_from(ticks).unwrap_or(i64::MAX).max(1)
    }
}
