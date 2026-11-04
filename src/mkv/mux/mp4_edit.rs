//! An MP4 edit list as Matroska timing.

use crate::edit::{EditEntry, EditList};
use crate::error::{Error, Result};
use crate::mkv::mux::bytes::refuse;
use crate::track::{Track, TrackKind};

/// What an edit list asks of the copy, in track ticks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EditPlan {
    /// The amount to add to every `pts`.
    pub shift: i64,
    /// The media time the audio decoder drops: the `CodecDelay` in ticks.
    pub delay: i64,
    /// The end of the edit in media ticks, as `end_scaled / movie_timescale`.
    /// `None` when the edit runs to the end of the media.
    pub end_scaled: Option<i128>,
    /// Ticks per second of the movie timescale.
    pub movie_timescale: i128,
}

fn no_form(track: &Track, why: String) -> Error {
    refuse(format!(
        "the edit list of track {} ({:?}, codec `{}`): {why}",
        track.id,
        track.kind,
        track
            .codec()
            .map(|fourcc| fourcc.to_string())
            .unwrap_or_default()
    ))
}

/// Read an edit list the Matroska way: an optional empty edit, then one edit
/// at normal rate. Anything else has no Matroska form.
///
/// The empty edit delays the track. For audio, `media_time` is the priming
/// the decoder drops: Matroska stores it as `CodecDelay`, and block timestamps
/// keep their values. For any other track, the frame at `media_time` presents
/// first, so every `pts` moves down by it.
pub(crate) fn edit_plan(track: &Track) -> Result<EditPlan> {
    let EditList {
        movie_timescale,
        entries,
    } = &track.edit_list;
    let (empty, media): (Option<&EditEntry>, Option<&EditEntry>) = match entries.as_slice() {
        [] => (None, None),
        [media] if !media.is_empty() => (None, Some(media)),
        [empty, media] if empty.is_empty() && !media.is_empty() => (Some(empty), Some(media)),
        other => {
            return Err(no_form(
                track,
                format!(
                    "{} edits. Matroska holds one start delay and one start trim",
                    other.len()
                ),
            ));
        }
    };
    let Some(media) = media else {
        return Ok(EditPlan::default());
    };
    if media.media_rate != EditEntry::NORMAL_RATE {
        return Err(no_form(
            track,
            format!(
                "the rate {:#x} is not normal speed. Matroska has no playback rate",
                media.media_rate
            ),
        ));
    }
    let timescale = i128::from(track.timebase.den);
    let movie = i128::from(*movie_timescale);
    if movie == 0 {
        return Err(no_form(track, "the movie timescale is 0".to_owned()));
    }
    let delay_scaled = empty.map_or(0, |edit| i128::from(edit.segment_duration)) * timescale;
    if delay_scaled % movie != 0 {
        return Err(no_form(
            track,
            format!(
                "the empty edit of {delay_scaled} ticks over {movie} is no whole number of \
                 track ticks"
            ),
        ));
    }
    let empty_ticks = i64::try_from(delay_scaled / movie).map_err(|_| {
        no_form(
            track,
            format!("the empty edit of {delay_scaled} ticks over {movie} overflows"),
        )
    })?;
    let audio = track.kind == TrackKind::Audio;
    let start = media.media_time;
    let shift = if audio {
        empty_ticks
    } else {
        empty_ticks
            .checked_sub(start)
            .ok_or_else(|| no_form(track, format!("the media time {start} overflows 64 bits")))?
    };
    let end_scaled = (media.segment_duration != 0)
        .then(|| i128::from(start) * movie + i128::from(media.segment_duration) * timescale);
    Ok(EditPlan {
        shift,
        delay: if audio { start } else { 0 },
        end_scaled,
        movie_timescale: movie,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{FourCc, Language, SampleEntry, Timebase};

    fn track(kind: TrackKind, timescale: u32) -> Track {
        Track {
            id: 1,
            kind,
            handler: None,
            timebase: Timebase::from_timescale(timescale),
            duration: None,
            language: Language::Iso639(*b"und"),
            matrix: [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000],
            display_width: 0,
            display_height: 0,
            sample_entries: vec![SampleEntry {
                fourcc: FourCc(*b"vp09"),
                raw: Vec::new(),
                data_reference_index: 1,
                codec_config: None,
                video: None,
                audio: None,
            }],
            edit_list: EditList::default(),
            references: Vec::new(),
            chapter_track: false,
            metadata: Vec::new(),
            sample_count: 0,
            matroska: None,
        }
    }

    fn edits(movie_timescale: u32, entries: Vec<EditEntry>) -> EditList {
        EditList {
            movie_timescale,
            entries,
        }
    }

    fn edit(segment_duration: u64, media_time: i64) -> EditEntry {
        EditEntry {
            segment_duration,
            media_time,
            media_rate: EditEntry::NORMAL_RATE,
        }
    }

    #[test]
    fn video_edits_shift_and_audio_edits_become_a_delay() {
        let mut video = track(TrackKind::Video, 12_800);
        video.edit_list = edits(1000, vec![edit(500, -1), edit(2000, 1024)]);
        let plan = edit_plan(&video).expect("the edit maps");
        assert_eq!((plan.shift, plan.delay), (6400 - 1024, 0));
        let mut audio = track(TrackKind::Audio, 48_000);
        audio.edit_list = edits(1000, vec![edit(2000, 312)]);
        let plan = edit_plan(&audio).expect("the edit maps");
        assert_eq!((plan.shift, plan.delay), (0, 312));
    }

    #[test]
    fn edits_with_no_matroska_form_name_mp4() {
        let mut video = track(TrackKind::Video, 12_800);
        video.edit_list = edits(1000, vec![edit(100, 0), edit(100, 512)]);
        let err = edit_plan(&video).expect_err("two media edits");
        assert!(err.to_string().contains("MP4/MOV"), "{err}");
        video.edit_list = edits(
            1000,
            vec![EditEntry {
                media_rate: 0,
                ..edit(100, 0)
            }],
        );
        assert!(edit_plan(&video).is_err());
        video.edit_list = edits(7, vec![edit(1, -1), edit(1, 0)]);
        assert!(edit_plan(&video).is_err());
    }
}
