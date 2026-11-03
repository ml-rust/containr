//! Muxer options: brands, layout, movie timescale and interleave bounds.

use crate::error::Result;
use crate::mp4::mux::bytes::invalid;
use crate::track::FourCc;

/// The default chunk duration, in milliseconds.
pub const DEFAULT_CHUNK_DURATION_MS: u32 = 500;
/// The default bound on packet bytes the muxer holds before it writes them.
pub const DEFAULT_MAX_BUFFER: usize = 64 << 20;
/// The largest chunk, in bytes. A chunk closes when it reaches this size.
pub const MAX_CHUNK_BYTES: usize = 1 << 20;
/// The most samples in one chunk.
pub const MAX_CHUNK_SAMPLES: u32 = 65_536;
/// The largest chunk duration accepted, in milliseconds.
pub const MAX_CHUNK_DURATION_MS: u32 = 60_000;

/// The `ftyp` brand set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Brand {
    /// ISO base media: major brand `isom`, compatible `isom`, `iso2`, `iso4`
    /// and `mp41`. `iso4` admits signed composition offsets.
    Isom,
    /// MP4 version 1: major brand `mp41`, compatible `mp41` and `isom`.
    /// Version 1 composition offsets are unsigned, so a negative
    /// composition offset is refused.
    Mp41,
    /// MP4 version 2: major brand `mp42`, compatible `mp42`, `isom` and
    /// `iso4`.
    Mp42,
    /// QuickTime MOV: major and compatible brand `qt  `. Handler boxes take
    /// the QuickTime form.
    QuickTime,
}

impl Brand {
    /// The major brand, the minor version and the compatible brands.
    pub(crate) fn ftyp(self) -> (FourCc, u32, Vec<FourCc>) {
        let brands =
            |list: &[&[u8; 4]]| -> Vec<FourCc> { list.iter().map(|b| FourCc(**b)).collect() };
        match self {
            Self::Isom => (
                FourCc(*b"isom"),
                512,
                brands(&[b"isom", b"iso2", b"iso4", b"mp41"]),
            ),
            Self::Mp41 => (FourCc(*b"mp41"), 0, brands(&[b"mp41", b"isom"])),
            Self::Mp42 => (FourCc(*b"mp42"), 0, brands(&[b"mp42", b"isom", b"iso4"])),
            Self::QuickTime => (FourCc(*b"qt  "), 512, brands(&[b"qt  "])),
        }
    }

    /// True when the brand set admits `ctts` version 1.
    pub(crate) fn signed_offsets(self) -> bool {
        !matches!(self, Self::Mp41)
    }

    /// True for QuickTime MOV.
    pub(crate) fn quicktime(self) -> bool {
        matches!(self, Self::QuickTime)
    }
}

/// How the muxer lays out its output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MuxOptions {
    /// The `ftyp` brand set.
    pub brand: Brand,
    /// Move `moov` before `mdat` at [`Muxer::finish`]. The muxer writes
    /// `mdat` first, then shifts it up by the `moov` size in place, on the
    /// same target, and adds the shift to every chunk offset.
    ///
    /// [`Muxer::finish`]: crate::mp4::Muxer::finish
    pub faststart: bool,
    /// Ticks per second of the movie timescale (`mvhd`). Edit list segment
    /// durations and `tkhd` durations count in it.
    pub movie_timescale: u32,
    /// The longest chunk, in milliseconds of decode time. Chunks of all
    /// tracks interleave by decode time.
    pub chunk_duration_ms: u32,
    /// The most packet bytes the muxer holds before it writes them. Past
    /// this bound it writes the earliest chunk even when another track has
    /// sent no packet up to that time.
    pub max_buffer: usize,
}

impl Default for MuxOptions {
    fn default() -> Self {
        Self {
            brand: Brand::Isom,
            faststart: false,
            movie_timescale: 1000,
            chunk_duration_ms: DEFAULT_CHUNK_DURATION_MS,
            max_buffer: DEFAULT_MAX_BUFFER,
        }
    }
}

impl MuxOptions {
    /// Check every field against its accepted range.
    pub(crate) fn check(&self) -> Result<()> {
        if self.movie_timescale == 0 {
            return Err(invalid(
                "`MuxOptions::movie_timescale` is 0. Use a positive tick rate, such as 1000"
                    .to_owned(),
            ));
        }
        if self.chunk_duration_ms == 0 || self.chunk_duration_ms > MAX_CHUNK_DURATION_MS {
            return Err(invalid(format!(
                "`MuxOptions::chunk_duration_ms` is {}. Use 1 to {MAX_CHUNK_DURATION_MS}",
                self.chunk_duration_ms
            )));
        }
        if self.max_buffer < MAX_CHUNK_BYTES {
            return Err(invalid(format!(
                "`MuxOptions::max_buffer` is {}, below one chunk of {MAX_CHUNK_BYTES} bytes",
                self.max_buffer
            )));
        }
        Ok(())
    }
}
