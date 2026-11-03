//! Raw edit lists.
//!
//! The demuxer exposes edit lists as the container stores them. The caller
//! applies them. Nothing in containr shifts or drops a packet for an edit.

/// One edit: play `segment_duration` of the movie from `media_time` in the
/// track's media, at `media_rate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EditEntry {
    /// The edit duration in movie timescale ticks.
    pub segment_duration: u64,
    /// The media start in track timescale ticks, or
    /// [`EditEntry::EMPTY_MEDIA_TIME`] for an empty edit.
    pub media_time: i64,
    /// The playback rate, 16.16 fixed point. `0x10000` is normal speed and
    /// `0` is a dwell on one frame.
    pub media_rate: i32,
}

impl EditEntry {
    /// The `media_time` of an empty edit: a gap of `segment_duration` with no
    /// media, which delays the track start.
    pub const EMPTY_MEDIA_TIME: i64 = -1;

    /// The rate of normal playback, 16.16 fixed point.
    pub const NORMAL_RATE: i32 = 0x10000;

    /// True for an empty edit.
    pub fn is_empty(&self) -> bool {
        self.media_time == Self::EMPTY_MEDIA_TIME
    }
}

/// A track's edit list and the timescale its segment durations count in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct EditList {
    /// Ticks per second of `segment_duration` (the `mvhd` timescale).
    pub movie_timescale: u32,
    /// The edits in file order. Empty when the track has no edit list.
    pub entries: Vec<EditEntry>,
}

impl EditList {
    /// True when the track has no edit list.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
