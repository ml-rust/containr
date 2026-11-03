//! FFmpeg-made fixtures compared against ffprobe: track facts, packets,
//! packet bytes and edit lists.

use crate::common::{self, mp4_probe};

/// Compare one fixture against its ffprobe reference. Skips when the
/// generator has not run.
pub fn check_fixture(name: &str) {
    let Some(fx) = common::fixture(name) else {
        return;
    };
    mp4_probe::check_against_probe(&fx);
}

#[test]
fn ffprobe_h264_aac_moov_at_end() {
    check_fixture("h264_aac_moov_end.mp4");
}

#[test]
fn ffprobe_h264_aac_faststart() {
    check_fixture("h264_aac_faststart.mp4");
}

#[test]
fn ffprobe_h264_negative_cts() {
    check_fixture("h264_negative_cts.mp4");
}

#[test]
fn ffprobe_hevc_opus() {
    check_fixture("hevc_opus.mp4");
}

#[test]
fn ffprobe_mov_text() {
    check_fixture("mov_text.mp4");
}

#[test]
fn ffprobe_rotate90() {
    check_fixture("rotate90.mp4");
}

#[test]
fn ffprobe_itsoffset() {
    check_fixture("itsoffset.mp4");
}

#[test]
fn ffprobe_ss_copy() {
    check_fixture("ss_copy.mp4");
}

#[test]
fn ffprobe_chapters_mp4() {
    check_fixture("chapters_mp4.mp4");
}

#[test]
fn ffprobe_chapters_mov() {
    check_fixture("chapters_mov.mov");
}
