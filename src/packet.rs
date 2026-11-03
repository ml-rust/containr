//! Packets: one coded frame with its timestamps and position.

/// One coded frame. Timestamps count in the track timebase.
///
/// A packet locates its bytes in the input. Read them with the demuxer that
/// produced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Packet {
    /// The id of the track the packet belongs to.
    pub track_id: u32,
    /// The 0-based sample index within the track.
    pub index: u64,
    /// Decode timestamp. MP4: the first sample of a track decodes at 0.
    /// Matroska: `None`, because Matroska stores no decode timestamp.
    pub dts: Option<i64>,
    /// Presentation timestamp. MP4: `dts` plus the composition offset.
    /// Matroska: the Cluster `Timestamp` plus the block's relative
    /// timestamp, with no `CodecDelay` subtracted. A frame after the first
    /// in a lace adds its share of the block duration (see [`Lace`]).
    pub pts: i64,
    /// Duration in timebase ticks. MP4: the `stts` value; a zero stays zero,
    /// since some muxers write 0 for the last sample. Matroska:
    /// `BlockDuration`, else `DefaultDuration` divided by `TimestampScale`
    /// and rounded down, else `None`.
    pub duration: Option<u64>,
    /// True for a sync sample or keyframe (a random access point).
    pub sync: bool,
    /// True when a Matroska `SimpleBlock` sets its discardable flag. Always
    /// false for MP4.
    pub discardable: bool,
    /// The Matroska `DiscardPadding`, in nanoseconds: positive trims the
    /// block end, negative trims its start. It sits on the last frame of a
    /// laced block when positive, on the first when negative. The demuxer
    /// never applies it.
    pub discard_padding: Option<i64>,
    /// The frame's place in a Matroska lace. `None` for an unlaced block and
    /// for MP4.
    pub lace: Option<Lace>,
    /// The byte offset of the frame in the input.
    pub offset: u64,
    /// The frame size in bytes.
    pub size: u32,
    /// The sample description the packet uses, counted from 1. Always 1 for
    /// Matroska.
    pub description_index: u32,
}

/// A frame's place in a Matroska lace: several frames stored in one block.
///
/// The block stores one timestamp. Frame `index` presents at the block
/// timestamp plus `index / count` of the `BlockDuration`, or plus `index`
/// `DefaultDuration`s. With neither, Matroska stores no timestamp for frames
/// after the first: their `pts` repeats the block timestamp and their
/// `duration` is `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Lace {
    /// The 0-based frame index within the block.
    pub index: u16,
    /// The number of frames in the block.
    pub count: u16,
}
