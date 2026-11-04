//! What the muxer needs to write one track, and one packet.

use crate::matroska::MatroskaTrack;
use crate::metadata::MetadataItem;
use crate::track::{Timebase, TrackKind};

/// An output track, as [`Muxer::add_track`] returns it.
///
/// [`Muxer::add_track`]: crate::mkv::Muxer::add_track
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrackId(pub(crate) u32);

impl TrackId {
    /// The `TrackNumber` of the output track.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Everything the muxer needs to write one `TrackEntry`.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackSpec {
    /// The tick of [`PacketIn`] timestamps and durations. The muxer converts
    /// them to `TimestampScale` ticks exactly, or refuses them.
    pub timebase: Timebase,
    /// The `TrackEntry` facts. A `uid` of `None` gets a UID derived from the
    /// track number and `CodecID`.
    pub matroska: MatroskaTrack,
    /// Track tags. Each is written with a target of this track's `TrackUID`.
    /// Each needs `MetadataScope::Tag` and a named key.
    pub metadata: Vec<MetadataItem>,
}

impl TrackSpec {
    /// A spec for `codec_id` with the defaults: language `und`, the track
    /// enabled and default, no lacing, and no codec configuration. A video
    /// track also needs [`MatroskaTrack::video`].
    pub fn new(kind: TrackKind, codec_id: impl Into<String>, timebase: Timebase) -> Self {
        let track_type = match kind {
            TrackKind::Video => 1,
            TrackKind::Audio => 2,
            TrackKind::Subtitle => 17,
            TrackKind::Data | TrackKind::Other => 33,
        };
        Self {
            timebase,
            matroska: MatroskaTrack {
                uid: None,
                track_type,
                codec_id: codec_id.into(),
                codec_private: None,
                codec_name: None,
                name: None,
                language: "und".to_owned(),
                language_bcp47: None,
                default_duration: None,
                codec_delay: 0,
                seek_pre_roll: 0,
                enabled: true,
                default: true,
                forced: false,
                lacing: false,
                content_encodings: Vec::new(),
                video: None,
                audio: None,
                extra: Vec::new(),
            },
            metadata: Vec::new(),
        }
    }
}

/// One packet to write. Timestamps count in [`TrackSpec::timebase`].
///
/// Send each track's packets in decode order. The muxer writes blocks in the
/// order they arrive, and `pts` is free to reorder inside a track. Matroska
/// stores no decode timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketIn<'a> {
    /// The packet bytes.
    pub data: &'a [u8],
    /// Presentation timestamp. It must be 0 or more: a Matroska block
    /// timestamp is never negative.
    pub pts: i64,
    /// Duration. `None` means the track's `DefaultDuration`. The last packet
    /// of a track needs one or the other.
    pub duration: Option<u64>,
    /// True for a keyframe (a random access point).
    pub sync: bool,
    /// True when a decoder can skip the frame.
    pub discardable: bool,
    /// The `DiscardPadding` in nanoseconds: positive trims the block end,
    /// negative trims its start.
    pub discard_padding: Option<i64>,
}
