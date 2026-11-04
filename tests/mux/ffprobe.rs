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

/// ffprobe's first-audio-stream `start_time` string and `start_pts` in
/// nanoseconds. `None`, with a notice, when ffprobe is not on PATH.
fn audio_start(path: &std::path::Path) -> Option<(String, i128)> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=start_time,start_pts,time_base",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(path)
        .output()
        .map_err(|err| eprintln!("skipping: ffprobe does not run: {err}"))
        .ok()?;
    assert!(
        output.status.success(),
        "ffprobe failed on {}",
        path.display()
    );
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
            .unwrap_or_else(|| panic!("{}: ffprobe prints no {key}: {text}", path.display()))
            .to_owned()
    };
    let (num, den) = field("time_base")
        .split_once('/')
        .map(|(n, d)| (n.parse::<i128>(), d.parse::<i128>()))
        .map(|(n, d)| (n.unwrap_or(1), d.unwrap_or(1)))
        .unwrap_or((1, 1));
    let pts: i128 = field("start_pts").parse().unwrap_or(0);
    Some((field("start_time"), pts * num * 1_000_000_000 / den))
}

/// The samples `ffmpeg -f null` style decoding yields for the first audio
/// stream, counted from the raw 16-bit output.
fn decoded_samples(path: &std::path::Path) -> Option<usize> {
    let output = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-f", "s16le", "-"])
        .output()
        .map_err(|err| eprintln!("skipping: ffmpeg does not run: {err}"))
        .ok()?;
    assert!(
        output.status.success(),
        "ffmpeg failed on {}",
        path.display()
    );
    Some(output.stdout.len() / 2)
}

/// `ffmpeg -c copy` of `source` to MP4, as the second reference.
fn ffmpeg_copy(source: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let path = write_output(name, &[]);
    let status = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-y", "-i"])
        .arg(source)
        .args(["-c", "copy"])
        .arg(&path)
        .status()
        .map_err(|err| eprintln!("skipping: ffmpeg does not run: {err}"))
        .ok()?;
    assert!(
        status.success(),
        "ffmpeg -c copy failed on {}",
        source.display()
    );
    Some(path)
}

/// The audio start and decoded length of the copy match the source and
/// FFmpeg's own copy. A decoder drops whole frames at the end, so the sample
/// counts agree within one frame of `frame` samples.
fn check_audio_sync(name: &str, frame: usize) {
    let Some(remux) = remux_mkv(name, false) else {
        return;
    };
    let ours = write_output(&format!("sync_{name}.mp4"), &remux.bytes);
    let Some(theirs) = ffmpeg_copy(&remux.fixture.path, &format!("ffcopy_{name}.mp4")) else {
        return;
    };
    let source = &remux.fixture.path;
    let (Some(src_start), Some(our_start), Some(ff_start)) = (
        audio_start(source),
        audio_start(&ours),
        audio_start(&theirs),
    ) else {
        return;
    };
    assert_eq!(
        our_start, src_start,
        "{name}: start_time and start_pts against the source"
    );
    assert_eq!(
        our_start, ff_start,
        "{name}: start_time and start_pts against ffmpeg -c copy"
    );
    let (Some(src_count), Some(our_count), Some(ff_count)) = (
        decoded_samples(source),
        decoded_samples(&ours),
        decoded_samples(&theirs),
    ) else {
        return;
    };
    assert!(
        src_count.abs_diff(our_count) <= frame,
        "{name}: {our_count} decoded samples against {src_count} in the source"
    );
    assert!(
        ff_count.abs_diff(our_count) <= frame,
        "{name}: {our_count} decoded samples against {ff_count} in ffmpeg -c copy"
    );
}

#[test]
fn audio_sync_matches_the_source_for_aac() {
    check_audio_sync("h264_aac.mkv", 1024);
}

#[test]
fn audio_sync_matches_the_source_for_opus() {
    check_audio_sync("hevc_opus.mkv", 960);
}

#[test]
fn audio_sync_matches_the_source_for_trimmed_opus() {
    check_audio_sync("opus_trim.webm", 960);
}
