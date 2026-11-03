//! The QuickTime chapter track: one text sample per chapter, named by a
//! `tref` `chap` reference from the other tracks.
//!
//! Each sample holds a 16-bit title length, the UTF-8 title, and an `encd`
//! box that marks the text UTF-8, as FFmpeg writes it. The track counts in
//! the first chapter's timescale. A timescale above
//! [`MAX_TRACK_TIMESCALE`], such as Matroska's nanoseconds, gives
//! milliseconds, rounded down, since a 32-bit `stts` delta at 1 GHz spans
//! 4.3 seconds. `chpl` keeps its own precision. A sample lasts until the
//! next chapter starts. The last one lasts its own duration, else until the
//! movie ends.
//! When the first chapter starts after 0, every sample carries that start
//! as its composition offset, so each sample presents at its chapter start.

use crate::error::Result;
use crate::metadata::Chapter;
use crate::mp4::mux::bytes::invalid;
use crate::mp4::mux::entry::{CHAPTER_TEXT_ENTRY, build_entry};
use crate::mp4::mux::metadata::rescale;
use crate::mp4::mux::spec::{PacketIn, SampleEntrySpec, TrackSpec};
use crate::mp4::mux::track::TrackState;
use crate::track::{FourCc, TrackKind};

/// The largest timescale a chapter track keeps. 100 kHz spans 11.9 hours in
/// one sample.
const MAX_TRACK_TIMESCALE: u32 = 100_000;
/// The timescale of a chapter track whose chapters count finer.
const FALLBACK_TIMESCALE: u32 = 1000;

/// The `encd` box of a UTF-8 text sample.
const ENCD: [u8; 12] = [0, 0, 0, 12, b'e', b'n', b'c', b'd', 0, 0, 1, 0];

/// One chapter text sample.
fn sample(title: &str) -> Result<Vec<u8>> {
    let len = u16::try_from(title.len()).map_err(|_| {
        invalid(format!(
            "a chapter title holds {} bytes, above the 65 535 a text sample holds. Shorten the title",
            title.len()
        ))
    })?;
    let mut out = Vec::with_capacity(2 + title.len() + ENCD.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(title.as_bytes());
    out.extend_from_slice(&ENCD);
    Ok(out)
}

/// Build the chapter track `id`, its samples queued as chunks.
/// `movie_duration` counts in `movie_timescale` ticks.
pub(crate) fn chapter_track(
    id: u32,
    chapters: &[Chapter],
    movie_timescale: u32,
    movie_duration: u64,
    chunk_ms: u32,
) -> Result<TrackState> {
    let Some(first) = chapters.first() else {
        return Err(invalid(
            "a chapter track needs one chapter or more".to_owned(),
        ));
    };
    let timescale = if first.timescale > MAX_TRACK_TIMESCALE {
        FALLBACK_TIMESCALE
    } else {
        first.timescale
    };
    let mut spec = TrackSpec::new(
        TrackKind::Subtitle,
        timescale,
        SampleEntrySpec::Raw(CHAPTER_TEXT_ENTRY.to_vec()),
    );
    spec.handler = FourCc(*b"text");
    spec.enabled = false;
    let (_, entry) = build_entry(&spec.sample_entry, None)?;
    let mut track = TrackState::new(id, spec, entry, chunk_ms);

    let starts: Vec<u64> = chapters
        .iter()
        .map(|chapter| rescale(chapter.start, chapter.timescale, timescale))
        .collect();
    let origin = starts.first().copied().unwrap_or(0);
    let movie_end = rescale(movie_duration, movie_timescale, timescale);
    let too_late = |value: u64| {
        i64::try_from(value)
            .map_err(|_| invalid(format!("a chapter start of {value} ticks exceeds 2^63")))
    };
    for (n, chapter) in chapters.iter().enumerate() {
        let start = starts.get(n).copied().unwrap_or(0);
        let duration = match starts.get(n + 1) {
            Some(&next) => next.saturating_sub(start),
            None => chapter
                .duration
                .map(|d| rescale(d, chapter.timescale, timescale))
                .unwrap_or_else(|| movie_end.saturating_sub(start)),
        };
        let bytes = sample(&chapter.title)?;
        track.push(
            &PacketIn {
                data: &bytes,
                pts: too_late(start)?,
                dts: too_late(start.saturating_sub(origin))?,
                duration: Some(duration),
                sync: true,
            },
            true,
        )?;
    }
    track.finish()?;
    Ok(track)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::ChapterSource;

    fn chapter(start: u64, title: &str) -> Chapter {
        Chapter {
            start,
            duration: None,
            timescale: 1000,
            title: title.to_owned(),
            source: ChapterSource::Chpl,
        }
    }

    #[test]
    fn samples_last_until_the_next_chapter_and_the_movie_end() {
        let chapters = [chapter(0, "Opening"), chapter(800, "Closing")];
        let track = chapter_track(3, &chapters, 1000, 2000, 500).expect("the track builds");
        assert_eq!(track.table.count, 2);
        assert_eq!(track.table.media_duration, 2000);
        assert_eq!(track.closed.len(), 2);
        assert_eq!(&track.closed[0].data[..9], b"\0\x07Opening");
    }

    #[test]
    fn a_late_first_chapter_presents_at_its_start() {
        let chapters = [chapter(300, "Late")];
        let track = chapter_track(2, &chapters, 1000, 1000, 500).expect("the track builds");
        assert_eq!(track.table.offset_range, Some((300, 300)));
        assert_eq!(track.table.media_duration, 700);
    }
}
