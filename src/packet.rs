//! Packets: one coded sample with its timestamps and position.

/// One coded sample. Timestamps count in the track timescale.
///
/// A packet locates its bytes in the input. Read them with the demuxer that
/// produced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Packet {
    /// The id of the track the packet belongs to.
    pub track_id: u32,
    /// The 0-based sample index within the track.
    pub index: u64,
    /// Decode timestamp. The first sample of a track decodes at 0.
    pub dts: i64,
    /// Presentation timestamp: `dts` plus the composition offset.
    pub pts: i64,
    /// Duration in track timescale ticks, as `stts` stores it. A zero stays
    /// zero: some muxers write 0 for the last sample.
    pub duration: u32,
    /// True for a sync sample (a random access point).
    pub sync: bool,
    /// The byte offset of the sample in the input.
    pub offset: u64,
    /// The sample size in bytes.
    pub size: u32,
    /// The sample description the packet uses, counted from 1.
    pub description_index: u32,
}
