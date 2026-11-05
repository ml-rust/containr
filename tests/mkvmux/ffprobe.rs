//! A second reader: ffprobe and ffmpeg read each file the muxer writes, and
//! mkvalidator when it is on PATH. Packets and stream facts must match what
//! the demuxer reports, as the FFmpeg-fixture demux tests check, and a decode
//! must log no error.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::common::{self, Fixture, array, mkv_probe, stream_track_id, text};
use crate::mkvmux::support::{Remux, copy_mkv, remux_mp4, remux_mp4_rounded, write_output};

/// The lines `ffmpeg -v error -i path -f null -` logs. `None`, with a notice,
/// when ffmpeg is not on PATH.
fn decode_errors(path: &Path) -> Option<String> {
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

/// Run mkvalidator on `path`. A notice when it is not on PATH.
fn validate(path: &Path) {
    match Command::new("mkvalidator").arg(path).output() {
        Ok(output) => assert!(
            output.status.success(),
            "mkvalidator refuses {}: {}{}",
            path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(err) => eprintln!("skipping mkvalidator: it does not run: {err}"),
    }
}

/// Read `bytes` with ffprobe and ffmpeg. Returns the probe.
fn read_back(output_name: &str, bytes: &[u8], source: &Path) -> Option<Value> {
    let path = write_output(output_name, bytes);
    let probe = mkv_probe::probe_file(&path)?;
    let fixture = Fixture {
        name: output_name.to_owned(),
        path: path.clone(),
        bytes: bytes.to_vec(),
        probe: probe.clone(),
    };
    mkv_probe::check_against_probe(&fixture);
    validate(&path);
    // A source that logs errors by itself, such as a cut that starts on
    // undecodable frames, proves nothing about the copy.
    let theirs = decode_errors(source)?;
    let ours = decode_errors(&path)?;
    if ours.is_empty() || !theirs.is_empty() {
        return Some(probe);
    }
    // FFmpeg 8's Opus parser logs an error at the end of any Matroska input, with
    // every packet decoded. An MP4 source has no such parser. So FFmpeg's own
    // Matroska of the source is the reference: it must log the same messages.
    let own = ffmpeg_copy(source, &format!("ffcopy_{output_name}"))?;
    let reference = decode_errors(&own)?;
    assert_eq!(
        messages(&ours),
        messages(&reference),
        "{output_name}: ffmpeg logs errors on the copy that it does not log on its own copy"
    );
    Some(probe)
}

/// `ffmpeg -c copy` of `source` to `name` in the output directory. `None`, with a
/// notice, when ffmpeg is not on PATH.
fn ffmpeg_copy(source: &Path, name: &str) -> Option<PathBuf> {
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

/// Each line of an ffmpeg log, with no `[name @ address]` prefix.
fn messages(log: &str) -> Vec<&str> {
    log.lines()
        .map(|line| line.split_once("] ").map_or(line, |(_, message)| message))
        .collect()
}

/// The ffprobe rotation of a stream.
fn rotation(stream: &Value) -> Option<i64> {
    array(stream, "side_data_list")
        .iter()
        .find(|side| side["side_data_type"] == "Display Matrix")
        .and_then(|side| side["rotation"].as_i64())
}

/// The stream facts a copy keeps, compared with the source's.
fn check_facts(name: &str, ours: &Value, theirs: &Value) {
    assert_eq!(rotation(ours), rotation(theirs), "{name}: rotation");
    for field in ["codec_name", "width", "height", "channels", "sample_rate"] {
        assert_eq!(ours[field], theirs[field], "{name}: {field}");
    }
    if text(ours, "codec_type") == "video" {
        assert_eq!(
            ours["sample_aspect_ratio"], theirs["sample_aspect_ratio"],
            "{name}: sample aspect ratio"
        );
    }
}

fn check_mkv(name: &str) {
    let Some(fixture) = common::fixture(name) else {
        return;
    };
    let copy = copy_mkv(name, &fixture.bytes, |_| {});
    let Some(probe) = read_back(&format!("copy_{name}"), &copy, &fixture.path) else {
        return;
    };
    let ours = mkv_probe::media_streams(&Fixture {
        name: name.to_owned(),
        path: fixture.path.clone(),
        bytes: Vec::new(),
        probe,
    })
    .into_iter()
    .cloned()
    .collect::<Vec<Value>>();
    let theirs = mkv_probe::media_streams(&fixture);
    assert_eq!(ours.len(), theirs.len(), "{name}: stream count");
    for (our, their) in ours.iter().zip(theirs) {
        check_facts(name, our, their);
    }
}

fn check_mp4(name: &str) {
    let Some(remux) = remux_mp4(name) else {
        return;
    };
    check_remux(name, &remux);
}

/// Read the remux back with ffprobe and ffmpeg, and compare stream facts.
fn check_remux(name: &str, remux: &Remux) {
    let Remux {
        bytes,
        copied,
        fixture,
        ..
    } = remux;
    let output = format!("mp4_{name}.mkv");
    let Some(probe) = read_back(&output, bytes, &fixture.path) else {
        return;
    };
    let streams = array(&probe, "streams");
    for (stream, track) in streams.iter().zip(copied) {
        let theirs = array(&fixture.probe, "streams")
            .iter()
            .find(|theirs| stream_track_id(theirs) == track.source.id)
            .unwrap_or_else(|| panic!("{name}: no stream for track {}", track.source.id));
        if text(stream, "codec_type") == "subtitle" {
            // `mov_text` maps to SubRip, on purpose.
            assert_eq!(text(stream, "codec_name"), "subrip", "{name}");
            continue;
        }
        check_facts(name, stream, theirs);
    }
}

#[test]
fn ffprobe_reads_copies_of_h264_aac() {
    check_mkv("h264_aac.mkv");
}

#[test]
fn ffprobe_reads_copies_of_hevc_opus() {
    check_mkv("hevc_opus.mkv");
}

#[test]
fn ffprobe_reads_copies_of_vp9_opus() {
    check_mkv("vp9_opus.webm");
}

#[test]
fn ffprobe_reads_copies_of_av1() {
    check_mkv("av1.webm");
}

#[test]
fn ffprobe_reads_copies_of_subtitles() {
    check_mkv("subtitles.mkv");
}

#[test]
fn ffprobe_reads_copies_of_chapters() {
    check_mkv("chapters.mkv");
}

#[test]
fn ffprobe_reads_copies_of_attachments() {
    check_mkv("attachment.mkv");
}

#[test]
fn ffprobe_reads_copies_of_rotate90() {
    check_mkv("rotate90.mkv");
}

#[test]
fn ffprobe_reads_copies_of_opus_trim() {
    check_mkv("opus_trim.webm");
}

#[test]
fn ffprobe_reads_copies_of_live() {
    check_mkv("live.webm");
}

#[test]
fn ffprobe_reads_mp4_h264_aac() {
    check_mp4("h264_aac_moov_end.mp4");
}

#[test]
fn ffprobe_reads_mp4_h264_negative_cts() {
    check_mp4("h264_negative_cts.mp4");
}

#[test]
fn ffprobe_reads_mp4_hevc_opus() {
    check_mp4("hevc_opus.mp4");
}

#[test]
fn ffprobe_reads_mp4_vp9_opus() {
    check_mp4("vp9_opus.mp4");
}

#[test]
fn ffprobe_reads_mp4_av1() {
    check_mp4("av1.mp4");
}

#[test]
fn ffprobe_reads_mp4_aac_32k() {
    check_mp4("aac_32k.mp4");
}

#[test]
fn ffprobe_reads_mp4_mov_text() {
    check_mp4("mov_text.mp4");
}

#[test]
fn ffprobe_reads_mp4_rotate90() {
    check_mp4("rotate90.mp4");
}

#[test]
fn ffprobe_reads_mp4_itsoffset() {
    check_mp4("itsoffset.mp4");
}

#[test]
fn ffprobe_reads_mp4_chapters() {
    check_mp4("chapters_mp4.mp4");
}

#[test]
fn ffprobe_reads_mp4_chapters_mov() {
    check_mp4("chapters_mov.mov");
}

#[test]
fn ffprobe_reads_mp4_ntsc_h264_aac_rounded() {
    let Some(remux) = remux_mp4_rounded("ntsc_h264_aac.mp4", 1_000_000) else {
        return;
    };
    assert!(remux.refused.is_empty(), "{:?}", remux.refused);
    check_remux("ntsc_h264_aac.mp4", &remux);
}

/// A second reader agrees on the language: ffprobe on the MOV and on the copy.
#[test]
fn ffprobe_reads_the_language_of_a_mov_as_the_source_has_it() {
    let name = "languages.mov";
    let Some(remux) = remux_mp4(name) else {
        return;
    };
    let Some(probe) = read_back("mp4_languages.mkv", &remux.bytes, &remux.fixture.path) else {
        return;
    };
    let ours = array(&probe, "streams");
    let theirs = array(&remux.fixture.probe, "streams");
    assert_eq!(ours.len(), theirs.len(), "{name}: stream count");
    assert_eq!(ours.len(), 5, "{name}: stream count");
    let mut tagged = 0;
    for (our, their) in ours.iter().zip(theirs) {
        // `languages.mov` holds one code with two forms: MP4 `fra`, Matroska `fre`.
        let language = match their["tags"]["language"].as_str() {
            Some("fra") => Some("fre"),
            code => code,
        };
        assert_eq!(
            our["tags"]["language"].as_str(),
            language,
            "{name}: stream {}",
            their["index"]
        );
        tagged += usize::from(language.is_some());
    }
    assert_eq!(tagged, 4, "{name}: tagged streams");
}
