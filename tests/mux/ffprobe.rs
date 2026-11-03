//! A second reader: ffprobe and ffmpeg read each file the muxer writes.
//! Packets and stream facts must match what the demuxer reports, as the
//! FFmpeg-fixture demux tests check, and a decode must log no error.

use std::process::Command;

use crate::common::{Fixture, mp4_probe};
use crate::mux::support::{Remux, remux_mkv, remux_mp4, write_output};

/// The lines `ffmpeg -v error -i path -f null -` logs. `None`, with a
/// notice, when ffmpeg is not on PATH.
fn decode_errors(path: &std::path::Path) -> Option<String> {
    let output = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .output()
        .map_err(|err| eprintln!("skipping: ffmpeg does not run: {err}"))
        .ok()?;
    assert!(
        output.status.success(),
        "ffmpeg failed on {}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8_lossy(&output.stderr).into_owned())
}

fn check(output_name: &str, remux: Remux) {
    let path = write_output(output_name, &remux.bytes);
    let Some(probe) = mp4_probe::probe_file(&path) else {
        return;
    };
    let fixture = Fixture {
        name: output_name.to_owned(),
        path: path.clone(),
        bytes: remux.bytes,
        probe,
    };
    mp4_probe::check_against_probe(&fixture);
    // A fixture that logs errors by itself, such as a cut that starts on
    // undecodable frames, proves nothing about the copy.
    let Some(source) = decode_errors(&remux.fixture.path) else {
        return;
    };
    let Some(ours) = decode_errors(&path) else {
        return;
    };
    assert!(
        ours.is_empty() || !source.is_empty(),
        "{output_name}: ffmpeg logs errors on the copy and none on the source: {ours}"
    );
}

fn check_mp4(name: &str) {
    for faststart in [false, true] {
        let Some(remux) = remux_mp4(name, faststart) else {
            return;
        };
        let suffix = if faststart { "fast" } else { "end" };
        check(&format!("{suffix}_{name}"), remux);
    }
}

fn check_mkv(name: &str) {
    let Some(remux) = remux_mkv(name, false) else {
        return;
    };
    check(&format!("mkv_{name}.mp4"), remux);
}

#[test]
fn ffprobe_reads_h264_aac() {
    check_mp4("h264_aac_moov_end.mp4");
}

#[test]
fn ffprobe_reads_h264_negative_cts() {
    check_mp4("h264_negative_cts.mp4");
}

#[test]
fn ffprobe_reads_hevc_opus() {
    check_mp4("hevc_opus.mp4");
}

#[test]
fn ffprobe_reads_mov_text() {
    check_mp4("mov_text.mp4");
}

#[test]
fn ffprobe_reads_rotate90() {
    check_mp4("rotate90.mp4");
}

#[test]
fn ffprobe_reads_itsoffset() {
    check_mp4("itsoffset.mp4");
}

#[test]
fn ffprobe_reads_ss_copy() {
    check_mp4("ss_copy.mp4");
}

#[test]
fn ffprobe_reads_chapters_mp4() {
    check_mp4("chapters_mp4.mp4");
}

#[test]
fn ffprobe_reads_chapters_mov() {
    check_mp4("chapters_mov.mov");
}

#[test]
fn ffprobe_reads_mkv_h264_aac() {
    check_mkv("h264_aac.mkv");
}

#[test]
fn ffprobe_reads_mkv_hevc_opus() {
    check_mkv("hevc_opus.mkv");
}

#[test]
fn ffprobe_reads_mkv_av1() {
    check_mkv("av1.webm");
}

#[test]
fn ffprobe_reads_mkv_subtitles() {
    check_mkv("subtitles.mkv");
}

#[test]
fn ffprobe_reads_mkv_chapters() {
    check_mkv("chapters.mkv");
}

#[test]
fn ffprobe_reads_mkv_rotate90() {
    check_mkv("rotate90.mkv");
}

#[test]
fn ffprobe_reads_mkv_opus_trim() {
    check_mkv("opus_trim.webm");
}
