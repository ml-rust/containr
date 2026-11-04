//! The `Video` and `Audio` elements of a `TrackEntry`.

use crate::error::Result;
use crate::matroska::{DisplayUnit, MatroskaAudio, MatroskaVideo, Projection, RawElement};
use crate::mkv::ebml::{Element, child_path};
use crate::mkv::ids;

/// Parse a `Video` element.
pub(crate) fn parse_video(video: &Element<'_>, path: &str) -> Result<MatroskaVideo> {
    let mut out = MatroskaVideo {
        pixel_width: 0,
        pixel_height: 0,
        display_width: None,
        display_height: None,
        display_unit: DisplayUnit::Pixels,
        crop: [0; 4],
        interlaced: 0,
        projection: None,
        extra: Vec::new(),
    };
    for child in video.children(path) {
        let child = child?;
        let at = child_path(path, child.id);
        match child.id {
            ids::PIXEL_WIDTH => out.pixel_width = child.uint(&at)?,
            ids::PIXEL_HEIGHT => out.pixel_height = child.uint(&at)?,
            ids::DISPLAY_WIDTH => out.display_width = Some(child.uint(&at)?),
            ids::DISPLAY_HEIGHT => out.display_height = Some(child.uint(&at)?),
            ids::DISPLAY_UNIT => out.display_unit = DisplayUnit::from_value(child.uint(&at)?),
            ids::PIXEL_CROP_TOP => out.crop[0] = child.uint(&at)?,
            ids::PIXEL_CROP_BOTTOM => out.crop[1] = child.uint(&at)?,
            ids::PIXEL_CROP_LEFT => out.crop[2] = child.uint(&at)?,
            ids::PIXEL_CROP_RIGHT => out.crop[3] = child.uint(&at)?,
            ids::FLAG_INTERLACED => out.interlaced = child.uint(&at)?,
            ids::PROJECTION => out.projection = Some(parse_projection(&child, &at)?),
            other => out.extra.push(RawElement {
                id: other,
                body: child.binary(),
            }),
        }
    }
    Ok(out)
}

fn parse_projection(projection: &Element<'_>, path: &str) -> Result<Projection> {
    let mut out = Projection {
        projection_type: 0,
        private: None,
        pose_yaw: 0.0,
        pose_pitch: 0.0,
        pose_roll: 0.0,
    };
    for child in projection.children(path) {
        let child = child?;
        let at = child_path(path, child.id);
        match child.id {
            ids::PROJECTION_TYPE => out.projection_type = child.uint(&at)?,
            ids::PROJECTION_PRIVATE => out.private = Some(child.binary()),
            ids::PROJECTION_POSE_YAW => out.pose_yaw = child.float(&at)?,
            ids::PROJECTION_POSE_PITCH => out.pose_pitch = child.float(&at)?,
            ids::PROJECTION_POSE_ROLL => out.pose_roll = child.float(&at)?,
            _ => {}
        }
    }
    Ok(out)
}

/// Parse an `Audio` element.
pub(crate) fn parse_audio(audio: &Element<'_>, path: &str) -> Result<MatroskaAudio> {
    let mut out = MatroskaAudio {
        sampling_frequency: 8000.0,
        output_sampling_frequency: None,
        channels: 1,
        bit_depth: None,
        extra: Vec::new(),
    };
    for child in audio.children(path) {
        let child = child?;
        let at = child_path(path, child.id);
        match child.id {
            ids::SAMPLING_FREQUENCY => out.sampling_frequency = child.float(&at)?,
            ids::OUTPUT_SAMPLING_FREQUENCY => {
                out.output_sampling_frequency = Some(child.float(&at)?);
            }
            ids::CHANNELS => out.channels = child.uint(&at)?,
            ids::BIT_DEPTH => out.bit_depth = Some(child.uint(&at)?),
            other => out.extra.push(RawElement {
                id: other,
                body: child.binary(),
            }),
        }
    }
    Ok(out)
}
