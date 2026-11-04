//! FFmpeg-made fixtures compared against ffprobe.

use crate::common::{self, mkv_probe};

/// Compare one fixture against its ffprobe reference. Skips when the
/// generator has not run.
pub fn check_fixture(name: &str) {
    if let Some(fx) = common::fixture(name) {
        mkv_probe::check_against_probe(&fx);
    }
}

#[test]
fn ffprobe_h264_aac() {
    check_fixture("h264_aac.mkv");
}

#[test]
fn ffprobe_hevc_opus() {
    check_fixture("hevc_opus.mkv");
}

#[test]
fn ffprobe_vp9_opus_webm() {
    check_fixture("vp9_opus.webm");
}

#[test]
fn ffprobe_av1_webm() {
    check_fixture("av1.webm");
}

#[test]
fn ffprobe_subtitles() {
    check_fixture("subtitles.mkv");
}

#[test]
fn ffprobe_chapters() {
    check_fixture("chapters.mkv");
}

#[test]
fn ffprobe_attachment() {
    check_fixture("attachment.mkv");
}

#[test]
fn ffprobe_rotate90() {
    check_fixture("rotate90.mkv");
}

#[test]
fn ffprobe_opus_trim() {
    check_fixture("opus_trim.webm");
}

#[test]
fn ffprobe_live_webm() {
    check_fixture("live.webm");
}
