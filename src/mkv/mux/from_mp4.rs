//! An MP4 track as a Matroska track spec.
//!
//! `avc1` and `avc3` map to `V_MPEG4/ISO/AVC`, `hvc1` and `hev1` to
//! `V_MPEGH/ISO/HEVC`, with the `avcC` or `hvcC` body as `CodecPrivate`.
//! `av01` maps to `V_AV1` with `av1C`, `vp09` to `V_VP9` with no
//! `CodecPrivate`, `mp4a` to `A_AAC` with the AudioSpecificConfig, `Opus` to
//! `A_OPUS` with `OpusHead`, and `tx3g` to `S_TEXT/UTF8`. Every other entry
//! is refused, and the error names MP4. A `colr` `nclx` or `nclc` box maps to
//! `Video/Colour`.

use crate::error::{Error, Result};
use crate::matroska::{DisplayUnit, MatroskaAudio, MatroskaVideo, Projection};
use crate::mkv::mux::bytes::{invalid, refuse};
use crate::mkv::mux::mp4_config::{audio_specific_config, opus_head};
use crate::mkv::mux::mp4_tags::tags_from_mp4;
use crate::mkv::mux::spec::TrackSpec;
use crate::track::{CodecConfig, Language, PixelAspect, SampleEntry, Track, TrackKind};

/// The `SeekPreRoll` the Opus mapping asks for: 80 ms.
const OPUS_PRE_ROLL_NS: u64 = 80_000_000;
/// 1.0 in the 16.16 fixed point of a `tkhd` matrix.
const UNIT: i32 = 0x10000;

/// The spec for an MP4 track, or the refusal that names MP4. Timing facts,
/// `DefaultDuration` and `CodecDelay` included, come from
/// [`StreamCopy::plan`](crate::mkv::StreamCopy::plan).
pub(crate) fn spec_from_mp4(track: &Track) -> Result<TrackSpec> {
    let entry = match track.sample_entries.as_slice() {
        [entry] => entry,
        [] => {
            return Err(invalid(format!(
                "track {} has no sample description and no Matroska facts. Pass a demuxed track",
                track.id
            )));
        }
        entries => {
            return Err(Error::SampleDescriptionChange {
                track_id: track.id,
                descriptions: entries.len() as u64,
            });
        }
    };
    if track.timebase.num != 1 || track.timebase.den == 0 {
        return Err(invalid(format!(
            "track {} has timebase {}/{}. An MP4 track counts in 1/timescale seconds",
            track.id, track.timebase.num, track.timebase.den
        )));
    }
    let config = entry.codec_config.as_ref();
    let missing = |name: &str| {
        invalid(format!(
            "track {}: sample entry `{}` has no `{name}` box. Remux the source with \
             `ffmpeg -c copy`",
            track.id, entry.fourcc
        ))
    };
    let mut spec = match (&entry.fourcc.0, config) {
        (b"avc1" | b"avc3", Some(CodecConfig::Avc(bytes))) => {
            video_spec(track, entry, "V_MPEG4/ISO/AVC", Some(bytes.clone()))?
        }
        (b"avc1" | b"avc3", _) => return Err(missing("avcC")),
        (b"hvc1" | b"hev1", Some(CodecConfig::Hevc(bytes))) => {
            video_spec(track, entry, "V_MPEGH/ISO/HEVC", Some(bytes.clone()))?
        }
        (b"hvc1" | b"hev1", _) => return Err(missing("hvcC")),
        (b"av01", Some(CodecConfig::Av1(bytes))) => {
            video_spec(track, entry, "V_AV1", Some(bytes.clone()))?
        }
        (b"av01", _) => return Err(missing("av1C")),
        (b"vp09", _) => video_spec(track, entry, "V_VP9", None)?,
        (b"mp4a", Some(CodecConfig::Esds(bytes))) => {
            let private = audio_specific_config(bytes, track.id)?.to_vec();
            audio_spec(track, entry, "A_AAC", private)?
        }
        (b"mp4a", _) => return Err(missing("esds")),
        (b"Opus", Some(CodecConfig::Opus(bytes))) => {
            let private = opus_head(bytes, track.id)?;
            let mut spec = audio_spec(track, entry, "A_OPUS", private)?;
            spec.matroska.seek_pre_roll = OPUS_PRE_ROLL_NS;
            spec
        }
        (b"Opus", _) => return Err(missing("dOps")),
        (b"tx3g", _) => TrackSpec::new(TrackKind::Subtitle, "S_TEXT/UTF8", track.timebase),
        _ => {
            return Err(refuse(format!(
                "track {} with sample entry `{}`: this muxer maps avc1, avc3, hvc1, hev1, av01, \
                 vp09, mp4a, Opus and tx3g",
                track.id, entry.fourcc
            )));
        }
    };
    spec.matroska.language = language_of(track.language);
    spec.metadata = tags_from_mp4(&track.metadata)?;
    Ok(spec)
}

/// True for a timed text track, whose samples need their framing removed.
pub(crate) fn is_tx3g(track: &Track) -> bool {
    track.codec().is_some_and(|fourcc| &fourcc.0 == b"tx3g")
}

/// The `Language` value: the track's code in bibliographic form, or `und` when
/// it has none. The muxer writes `DocTypeReadVersion` 2, so a version 2 reader
/// sees this element.
fn language_of(language: Language) -> String {
    language.matroska_code().to_owned()
}

fn video_spec(
    track: &Track,
    entry: &SampleEntry,
    codec_id: &str,
    private: Option<Vec<u8>>,
) -> Result<TrackSpec> {
    let Some(info) = entry.video else {
        return Err(invalid(format!(
            "track {}: sample entry `{}` has no visual facts, so its size is unknown. Pass a \
             demuxed video track",
            track.id, entry.fourcc
        )));
    };
    let width = u64::from(info.width);
    let height = u64::from(info.height);
    let (display_width, display_height, display_unit) =
        display_of(width, height, info.pixel_aspect);
    let projection = roll_of(track)?.map(|pose_roll| Projection {
        projection_type: 0,
        private: None,
        pose_yaw: 0.0,
        pose_pitch: 0.0,
        pose_roll,
    });
    let mut spec = TrackSpec::new(TrackKind::Video, codec_id, track.timebase);
    spec.matroska.codec_private = private;
    spec.matroska.video = Some(MatroskaVideo {
        pixel_width: width,
        pixel_height: height,
        display_width,
        display_height,
        display_unit,
        crop: [0; 4],
        interlaced: 0,
        projection,
        colour: info.colour,
        extra: Vec::new(),
    });
    Ok(spec)
}

/// The display size for a `pasp`. A width that lands on a whole pixel stays
/// in pixels. Any other stores the exact ratio with `DisplayUnit` 3.
fn display_of(
    width: u64,
    height: u64,
    aspect: Option<PixelAspect>,
) -> (Option<u64>, Option<u64>, DisplayUnit) {
    let Some(aspect) = aspect else {
        return (None, None, DisplayUnit::Pixels);
    };
    let (h, v) = (u64::from(aspect.h_spacing), u64::from(aspect.v_spacing));
    if h == v || h == 0 || v == 0 {
        return (None, None, DisplayUnit::Pixels);
    }
    let scaled = width * h;
    if scaled.is_multiple_of(v) {
        (Some(scaled / v), Some(height), DisplayUnit::Pixels)
    } else {
        (Some(scaled), Some(height * v), DisplayUnit::AspectRatio)
    }
}

/// `ProjectionPoseRoll` for a `tkhd` matrix: `None` with no rotation. A
/// translation changes nothing in Matroska. A scale, a shear or an angle that
/// is not a right angle is refused.
fn roll_of(track: &Track) -> Result<Option<f64>> {
    let [a, b, _, c, d, ..] = track.matrix;
    match (a, b, c, d) {
        (UNIT, 0, 0, UNIT) => Ok(None),
        (0, b, c, 0) if b == -UNIT && c == UNIT => Ok(Some(90.0)),
        (a, 0, 0, d) if a == -UNIT && d == -UNIT => Ok(Some(180.0)),
        (0, UNIT, c, 0) if c == -UNIT => Ok(Some(-90.0)),
        _ => Err(refuse(format!(
            "track {} with `tkhd` matrix {:?}: `ProjectionPoseRoll` holds right-angle rotations \
             in this muxer",
            track.id, track.matrix
        ))),
    }
}

fn audio_spec(
    track: &Track,
    entry: &SampleEntry,
    codec_id: &str,
    private: Vec<u8>,
) -> Result<TrackSpec> {
    let Some(info) = entry.audio else {
        return Err(invalid(format!(
            "track {}: sample entry `{}` has no audio facts. Pass a demuxed audio track",
            track.id, entry.fourcc
        )));
    };
    let mut spec = TrackSpec::new(TrackKind::Audio, codec_id, track.timebase);
    spec.matroska.codec_private = Some(private);
    spec.matroska.audio = Some(MatroskaAudio {
        sampling_frequency: f64::from(info.sample_rate),
        output_sampling_frequency: None,
        channels: u64::from(info.channels),
        bit_depth: None,
        extra: Vec::new(),
    });
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::EditList;
    use crate::track::{Timebase, VideoInfo};

    fn track(fourcc: &[u8; 4], kind: TrackKind, timescale: u32) -> Track {
        Track {
            id: 1,
            kind,
            handler: None,
            timebase: Timebase::from_timescale(timescale),
            duration: None,
            language: Language::Iso639(*b"und"),
            matrix: [UNIT, 0, 0, 0, UNIT, 0, 0, 0, 0x4000_0000],
            display_width: 0,
            display_height: 0,
            sample_entries: vec![SampleEntry {
                fourcc: crate::track::FourCc(*fourcc),
                raw: Vec::new(),
                data_reference_index: 1,
                codec_config: None,
                video: Some(VideoInfo {
                    width: 96,
                    height: 64,
                    pixel_aspect: None,
                    colour: None,
                }),
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

    #[test]
    fn languages_map_through_the_macintosh_table() {
        assert_eq!(language_of(Language::Other(0)), "eng");
        assert_eq!(language_of(Language::Other(2)), "ger");
        assert_eq!(language_of(Language::Iso639(*b"fra")), "fre");
        assert_eq!(language_of(Language::Iso639(*b"fre")), "fre");
        assert_eq!(language_of(Language::Other(95)), "und");
        assert_eq!(language_of(Language::Other(0x7FFF)), "und");
    }

    #[test]
    fn rotation_maps_right_angles_only() {
        let mut video = track(b"vp09", TrackKind::Video, 12_800);
        assert_eq!(roll_of(&video).ok(), Some(None));
        video.matrix = [0, -UNIT, 0, UNIT, 0, 0, 640 * UNIT, 0, 0x4000_0000];
        assert_eq!(roll_of(&video).ok(), Some(Some(90.0)));
        video.matrix = [-UNIT, 0, 0, 0, -UNIT, 0, 0, 0, 0x4000_0000];
        assert_eq!(roll_of(&video).ok(), Some(Some(180.0)));
        video.matrix = [0, UNIT, 0, -UNIT, 0, 0, 0, 0, 0x4000_0000];
        assert_eq!(roll_of(&video).ok(), Some(Some(-90.0)));
        video.matrix = [UNIT, UNIT, 0, 0, UNIT, 0, 0, 0, 0x4000_0000];
        assert!(roll_of(&video).is_err());
    }

    #[test]
    fn pixel_aspect_keeps_an_exact_display_size() {
        let pasp = |h_spacing, v_spacing| {
            Some(PixelAspect {
                h_spacing,
                v_spacing,
            })
        };
        assert_eq!(
            display_of(96, 64, pasp(4, 3)),
            (Some(128), Some(64), DisplayUnit::Pixels)
        );
        assert_eq!(
            display_of(100, 64, pasp(4, 3)),
            (Some(400), Some(192), DisplayUnit::AspectRatio)
        );
        assert_eq!(
            display_of(96, 64, pasp(1, 1)),
            (None, None, DisplayUnit::Pixels)
        );
    }
}
