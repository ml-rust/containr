//! One `TrackEntry` from a track spec: every `MatroskaTrack` fact.
//!
//! A value that equals the Matroska default is written anyway when it is a
//! flag or a language, since a reader's default can differ. Other optional
//! values stay absent, so they read back absent.

use crate::error::Result;
use crate::matroska::{
    ContentEncoding, DisplayUnit, MatroskaAudio, MatroskaTrack, MatroskaVideo, Projection,
    RawElement,
};
use crate::mkv::colour::write_colour;
use crate::mkv::ebml_write::ElementBuf;
use crate::mkv::ids;

/// Write `flag` as 0 or 1.
fn flag(out: &mut ElementBuf, id: u32, value: bool) -> Result<()> {
    out.uint(id, u64::from(value))
}

fn raw(out: &mut ElementBuf, extra: &[RawElement]) -> Result<()> {
    for element in extra {
        out.binary(element.id, &element.body)?;
    }
    Ok(())
}

/// The `TrackEntry` body of track `number`, with UID `uid`.
pub(crate) fn track_entry(number: u32, uid: u64, mkv: &MatroskaTrack) -> Result<ElementBuf> {
    let mut out = ElementBuf::new();
    out.uint(ids::TRACK_NUMBER, u64::from(number))?;
    out.uint(ids::TRACK_UID, uid)?;
    out.uint(ids::TRACK_TYPE, mkv.track_type)?;
    flag(&mut out, ids::FLAG_ENABLED, mkv.enabled)?;
    flag(&mut out, ids::FLAG_DEFAULT, mkv.default)?;
    flag(&mut out, ids::FLAG_FORCED, mkv.forced)?;
    flag(&mut out, ids::FLAG_LACING, mkv.lacing)?;
    if let Some(duration) = mkv.default_duration {
        out.uint(ids::DEFAULT_DURATION, duration)?;
    }
    if let Some(name) = &mkv.name {
        out.text(ids::NAME, name)?;
    }
    out.text(ids::LANGUAGE, &mkv.language)?;
    if let Some(language) = &mkv.language_bcp47 {
        out.text(ids::LANGUAGE_BCP47, language)?;
    }
    out.text(ids::CODEC_ID, &mkv.codec_id)?;
    if let Some(private) = &mkv.codec_private {
        out.binary(ids::CODEC_PRIVATE, private)?;
    }
    if let Some(name) = &mkv.codec_name {
        out.text(ids::CODEC_NAME, name)?;
    }
    if mkv.codec_delay != 0 {
        out.uint(ids::CODEC_DELAY, mkv.codec_delay)?;
    }
    if mkv.seek_pre_roll != 0 {
        out.uint(ids::SEEK_PRE_ROLL, mkv.seek_pre_roll)?;
    }
    if let Some(video) = &mkv.video {
        out.master(ids::VIDEO, &video_body(video)?)?;
    }
    if let Some(audio) = &mkv.audio {
        out.master(ids::AUDIO, &audio_body(audio)?)?;
    }
    if !mkv.content_encodings.is_empty() {
        let mut encodings = ElementBuf::new();
        for encoding in &mkv.content_encodings {
            encodings.master(ids::CONTENT_ENCODING, &encoding_body(encoding)?)?;
        }
        out.master(ids::CONTENT_ENCODINGS, &encodings)?;
    }
    raw(&mut out, &mkv.extra)?;
    Ok(out)
}

fn video_body(video: &MatroskaVideo) -> Result<ElementBuf> {
    let mut out = ElementBuf::new();
    out.uint(ids::PIXEL_WIDTH, video.pixel_width)?;
    out.uint(ids::PIXEL_HEIGHT, video.pixel_height)?;
    if let Some(width) = video.display_width {
        out.uint(ids::DISPLAY_WIDTH, width)?;
    }
    if let Some(height) = video.display_height {
        out.uint(ids::DISPLAY_HEIGHT, height)?;
    }
    if video.display_unit != DisplayUnit::Pixels {
        out.uint(ids::DISPLAY_UNIT, video.display_unit.value())?;
    }
    let [top, bottom, left, right] = video.crop;
    for (id, value) in [
        (ids::PIXEL_CROP_TOP, top),
        (ids::PIXEL_CROP_BOTTOM, bottom),
        (ids::PIXEL_CROP_LEFT, left),
        (ids::PIXEL_CROP_RIGHT, right),
    ] {
        if value != 0 {
            out.uint(id, value)?;
        }
    }
    if video.interlaced != 0 {
        out.uint(ids::FLAG_INTERLACED, video.interlaced)?;
    }
    if let Some(projection) = &video.projection {
        out.master(ids::PROJECTION, &projection_body(projection)?)?;
    }
    let last_colour = video
        .extra
        .iter()
        .rposition(|element| element.id == ids::COLOUR);
    for (index, element) in video.extra.iter().enumerate() {
        if Some(index) == last_colour {
            write_colour(&mut out, video.colour, Some(element.body.as_slice()))?;
        } else {
            out.binary(element.id, &element.body)?;
        }
    }
    if last_colour.is_none() {
        write_colour(&mut out, video.colour, None)?;
    }
    Ok(out)
}

fn projection_body(projection: &Projection) -> Result<ElementBuf> {
    let mut out = ElementBuf::new();
    out.uint(ids::PROJECTION_TYPE, projection.projection_type)?;
    if let Some(private) = &projection.private {
        out.binary(ids::PROJECTION_PRIVATE, private)?;
    }
    out.float(ids::PROJECTION_POSE_YAW, projection.pose_yaw)?;
    out.float(ids::PROJECTION_POSE_PITCH, projection.pose_pitch)?;
    out.float(ids::PROJECTION_POSE_ROLL, projection.pose_roll)?;
    Ok(out)
}

fn audio_body(audio: &MatroskaAudio) -> Result<ElementBuf> {
    let mut out = ElementBuf::new();
    out.float(ids::SAMPLING_FREQUENCY, audio.sampling_frequency)?;
    if let Some(frequency) = audio.output_sampling_frequency {
        out.float(ids::OUTPUT_SAMPLING_FREQUENCY, frequency)?;
    }
    out.uint(ids::CHANNELS, audio.channels)?;
    if let Some(depth) = audio.bit_depth {
        out.uint(ids::BIT_DEPTH, depth)?;
    }
    raw(&mut out, &audio.extra)?;
    Ok(out)
}

fn encoding_body(encoding: &ContentEncoding) -> Result<ElementBuf> {
    let mut out = ElementBuf::new();
    out.uint(ids::CONTENT_ENCODING_ORDER, encoding.order)?;
    out.uint(ids::CONTENT_ENCODING_SCOPE, encoding.scope)?;
    out.uint(ids::CONTENT_ENCODING_TYPE, encoding.encoding_type)?;
    if let Some(algorithm) = encoding.compression_algorithm {
        let mut compression = ElementBuf::new();
        compression.uint(ids::CONTENT_COMP_ALGO, algorithm)?;
        if let Some(settings) = &encoding.compression_settings {
            compression.binary(ids::CONTENT_COMP_SETTINGS, settings)?;
        }
        out.master(ids::CONTENT_COMPRESSION, &compression)?;
    }
    if let Some(settings) = &encoding.encryption_settings {
        out.binary(ids::CONTENT_ENCRYPTION, settings)?;
    } else if let Some(algorithm) = encoding.encryption_algorithm {
        let mut encryption = ElementBuf::new();
        encryption.uint(ids::CONTENT_ENC_ALGO, algorithm)?;
        out.master(ids::CONTENT_ENCRYPTION, &encryption)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colour::ContainerColour;
    use crate::mkv::ebml::Element;
    use crate::mkv::mux::spec::TrackSpec;
    use crate::mkv::tracks::parse_tracks;
    use crate::track::{Timebase, TrackKind};

    #[test]
    fn every_fact_reads_back() {
        let timebase = Timebase::from_nanoseconds(1_000_000);
        let mut spec = TrackSpec::new(TrackKind::Video, "V_VP9", timebase);
        let mkv = &mut spec.matroska;
        mkv.uid = Some(9);
        mkv.codec_private = Some(vec![1, 2]);
        mkv.codec_name = Some("codec".to_owned());
        mkv.name = Some("name".to_owned());
        mkv.language = "fre".to_owned();
        mkv.language_bcp47 = Some("fr-CA".to_owned());
        mkv.default_duration = Some(40_000_000);
        mkv.codec_delay = 7;
        mkv.seek_pre_roll = 8;
        mkv.enabled = false;
        mkv.default = false;
        mkv.forced = true;
        mkv.lacing = true;
        mkv.extra = vec![RawElement {
            id: 0x55EE,
            body: vec![0],
        }];
        mkv.content_encodings = vec![ContentEncoding {
            order: 1,
            scope: 2,
            encoding_type: 1,
            compression_algorithm: Some(3),
            compression_settings: Some(vec![0xAB]),
            encryption_algorithm: Some(5),
            encryption_settings: Some(vec![0x47, 0xE1, 0x81, 5]),
        }];
        mkv.video = Some(MatroskaVideo {
            pixel_width: 96,
            pixel_height: 64,
            display_width: Some(128),
            display_height: Some(64),
            display_unit: DisplayUnit::AspectRatio,
            crop: [1, 2, 3, 4],
            interlaced: 2,
            projection: Some(Projection {
                projection_type: 1,
                private: Some(vec![3]),
                pose_yaw: 1.0,
                pose_pitch: 2.0,
                pose_roll: 90.0,
            }),
            colour: Some(ContainerColour {
                primaries: None,
                transfer: None,
                matrix: None,
                full_range: Some(false),
            }),
            extra: vec![RawElement {
                id: 0x55B0,
                body: vec![0x55, 0xB9, 0x81, 1],
            }],
        });
        mkv.audio = Some(MatroskaAudio {
            sampling_frequency: 44_100.0,
            output_sampling_frequency: Some(88_200.0),
            channels: 6,
            bit_depth: Some(24),
            extra: Vec::new(),
        });
        let entry = track_entry(3, 9, &spec.matroska).expect("the entry fits");
        let mut tracks = ElementBuf::new();
        tracks
            .master(ids::TRACK_ENTRY, &entry)
            .expect("the entry fits");
        let bytes = tracks.into_bytes();
        let element = Element {
            id: ids::TRACKS,
            offset: 0,
            body_offset: 0,
            body: &bytes,
        };
        let parsed = parse_tracks(&element, timebase).expect("the entry reads back");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, 3);
        assert_eq!(parsed[0].matroska.as_ref(), Some(&spec.matroska));
    }
}
