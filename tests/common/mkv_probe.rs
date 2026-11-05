//! Matroska files compared against ffprobe: track facts, codec private data,
//! packets, packet bytes and track tags. Used for FFmpeg-made fixtures and for
//! files the muxer writes.
//!
//! libavformat subtracts `CodecDelay` from every pts, rounded to the
//! nearest tick, and reports `pos` at the block body. The demuxer keeps the
//! raw block timestamp and points at the frame data, after the 4-byte
//! block header that a 1-byte track number gives.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::process::Command;

use containr::mkv::Demuxer;
use containr::{Packet, Track, TrackKind};
use serde_json::{Value, json};

use super::{Fixture, adler32, array, int, ratio, text};

/// The block header length before each frame: a 1-byte track number, the
/// 2-byte timestamp and the flags byte.
const BLOCK_HEADER: u64 = 4;

/// The Matroska `CodecID` of an ffprobe `codec_name`.
fn codec_id(codec_name: &str) -> &'static str {
    match codec_name {
        "h264" => "V_MPEG4/ISO/AVC",
        "hevc" => "V_MPEGH/ISO/HEVC",
        "vp9" => "V_VP9",
        "av1" => "V_AV1",
        "aac" => "A_AAC",
        "opus" => "A_OPUS",
        "subrip" => "S_TEXT/UTF8",
        "ass" => "S_TEXT/ASS",
        other => panic!("no CodecID mapping for ffprobe codec {other:?}"),
    }
}

/// The media streams: every ffprobe stream but attachments.
pub fn media_streams(fx: &Fixture) -> Vec<&Value> {
    array(&fx.probe, "streams")
        .iter()
        .filter(|stream| text(stream, "codec_type") != "attachment")
        .collect()
}

fn check_stream(fx: &Fixture, stream: &Value, track: &Track) {
    let what = format!("{} track {}", fx.name, track.id);
    let mkv = track
        .matroska
        .as_ref()
        .unwrap_or_else(|| panic!("{what}: no Matroska facts"));
    assert_eq!(
        mkv.codec_id,
        codec_id(text(stream, "codec_name")),
        "{what}: CodecID"
    );
    let (num, den) = ratio(text(stream, "time_base"), '/');
    assert_eq!(
        u64::from(track.timebase.num) * den,
        u64::from(track.timebase.den) * num,
        "{what}: timebase"
    );
    match (&mkv.codec_private, stream.get("extradata_hash")) {
        (Some(private), Some(hash)) => {
            assert_eq!(
                Some(adler32(private).as_str()),
                hash.as_str(),
                "{what}: CodecPrivate"
            );
        }
        (None, None) => {}
        (ours, theirs) => panic!("{what}: CodecPrivate {ours:?}, ffprobe extradata {theirs:?}"),
    }
    if let Some(language) = stream["tags"]["language"].as_str() {
        assert_eq!(track.language.iso639(), Some(language), "{what}: language");
        assert_eq!(mkv.language, language, "{what}: raw language");
    }
    if let Some(encoder) = stream["tags"]["ENCODER"].as_str() {
        let ours = track
            .metadata
            .iter()
            .find(|item| item.name() == Some("ENCODER"))
            .and_then(|item| item.text());
        assert_eq!(ours, Some(encoder), "{what}: ENCODER tag");
    }
    match text(stream, "codec_type") {
        "video" => {
            assert_eq!(track.kind, TrackKind::Video, "{what}: kind");
            let video = mkv
                .video
                .as_ref()
                .unwrap_or_else(|| panic!("{what}: no Video element"));
            assert_eq!(
                video.pixel_width as i64,
                int(stream, "width"),
                "{what}: width"
            );
            assert_eq!(
                video.pixel_height as i64,
                int(stream, "height"),
                "{what}: height"
            );
        }
        "audio" => {
            assert_eq!(track.kind, TrackKind::Audio, "{what}: kind");
            let audio = mkv
                .audio
                .as_ref()
                .unwrap_or_else(|| panic!("{what}: no Audio element"));
            assert_eq!(
                audio.sampling_frequency,
                int(stream, "sample_rate") as f64,
                "{what}: sampling frequency"
            );
            assert_eq!(
                audio.channels as i64,
                int(stream, "channels"),
                "{what}: channels"
            );
        }
        "subtitle" => assert_eq!(track.kind, TrackKind::Subtitle, "{what}: kind"),
        other => panic!("{what}: unexpected stream type {other}"),
    }
}

/// The ffprobe `discard_padding` of a packet, in samples.
fn their_padding(packet: &Value) -> Option<i64> {
    array(packet, "side_data_list")
        .iter()
        .find(|side| side["side_data_type"] == "Skip Samples")
        .map(|side| int(side, "discard_padding"))
        .filter(|&samples| samples != 0)
}

/// Compare packet `n`'s stored duration with ffprobe's.
///
/// ffprobe prints a duration for a block that stores none: its own estimate
/// from the codec. FFmpeg 8 and older write audio this way, so there the file
/// holds no value to compare. Only audio gets this allowance.
///
/// FFmpeg 8 also prints a whole frame for the last Opus packet, from its
/// parser, whatever `BlockDuration` stores. FFmpeg 9 prints the stored value.
/// A different value passes only in that one shape. `tests/mkvmux/walk.rs`
/// reads the stored last duration with no help from ffprobe.
fn check_duration(at: &str, track: &Track, our: &Packet, theirs: &[&Value], n: usize) {
    let mkv = track.matroska.as_ref().expect("checked in check_stream");
    let Some(ours) = our.duration else {
        assert_eq!(track.kind, TrackKind::Audio, "{at}: no stored duration");
        assert_eq!(mkv.default_duration, None, "{at}: no stored duration");
        return;
    };
    let ours = ours as i64;
    let their = int(theirs[n], "duration");
    if ours == their {
        return;
    }
    let frame = n
        .checked_sub(1)
        .map(|previous| int(theirs[previous], "duration"));
    assert!(
        mkv.codec_id == "A_OPUS" && n + 1 == theirs.len() && ours < their && frame == Some(their),
        "{at}: duration: ours {ours}, ffprobe {their}"
    );
}

fn check_packets(fx: &Fixture, demuxer: &mut Demuxer<BufReader<File>>, index: i64, track: &Track) {
    let what = format!("{} track {}", fx.name, track.id);
    let mkv = track.matroska.as_ref().expect("checked in check_stream");
    let scale = u64::from(track.timebase.num);
    // libavformat rounds the delay to the nearest tick, halves up.
    let delay = ((mkv.codec_delay + scale / 2) / scale) as i64;
    let theirs: Vec<&Value> = array(&fx.probe, "packets")
        .iter()
        .filter(|packet| int(packet, "stream_index") == index)
        .collect();
    let ours: Vec<Packet> = demuxer
        .track_packets(track.id)
        .expect("the track exists")
        .collect::<Result<_, _>>()
        .unwrap_or_else(|err| panic!("{what}: {err}"));
    assert_eq!(ours.len(), theirs.len(), "{what}: packet count");
    assert_eq!(
        track.sample_count,
        ours.len() as u64,
        "{what}: sample count"
    );
    for (n, (our, their)) in ours.iter().zip(&theirs).enumerate() {
        let at = format!("{what} packet {n}");
        assert_eq!(our.index, n as u64, "{at}: index");
        assert_eq!(our.dts, None, "{at}: Matroska stores no dts");
        assert_eq!(our.pts - delay, int(their, "pts"), "{at}: pts");
        check_duration(&at, track, our, &theirs, n);
        assert_eq!(i64::from(our.size), int(their, "size"), "{at}: size");
        assert_eq!(
            our.offset - BLOCK_HEADER,
            int(their, "pos") as u64,
            "{at}: offset"
        );
        assert_eq!(our.lace, None, "{at}: FFmpeg writes no lacing");
        assert_eq!(
            our.sync,
            text(their, "flags").starts_with('K'),
            "{at}: keyframe"
        );
        let rate = mkv.audio.as_ref().map(|audio| audio.sampling_frequency);
        let padding = our
            .discard_padding
            .zip(rate)
            .map(|(ns, rate)| (ns as f64 * rate / 1e9).round() as i64);
        assert_eq!(padding, their_padding(their), "{at}: DiscardPadding");
        let bytes = demuxer
            .read_packet(our)
            .unwrap_or_else(|err| panic!("{at}: {err}"));
        assert_eq!(adler32(&bytes), text(their, "data_hash"), "{at}: bytes");
    }
}

/// Compare one file against its ffprobe reference.
pub fn check_against_probe(fx: &Fixture) {
    let name = fx.name.as_str();
    let mut demuxer = Demuxer::open_path(&fx.path).unwrap_or_else(|err| panic!("{name}: {err}"));
    assert!(!demuxer.truncated(), "{name}: the file is complete");
    let streams = media_streams(fx);
    assert_eq!(streams.len(), demuxer.tracks().len(), "{name}: track count");
    let tracks = demuxer.tracks().to_vec();
    for (stream, track) in streams.iter().zip(&tracks) {
        check_stream(fx, stream, track);
        check_packets(fx, &mut demuxer, int(stream, "index"), track);
    }
    let format_tags = &fx.probe["format"]["tags"];
    // A title stored as a tag prints under its upper-case tag name.
    let tag_title = demuxer
        .metadata()
        .iter()
        .find(|item| item.name() == Some("TITLE"))
        .and_then(|item| item.text());
    assert_eq!(
        demuxer.info().title.as_deref().or(tag_title),
        format_tags["title"]
            .as_str()
            .or(format_tags["TITLE"].as_str()),
        "{name}: Title"
    );
    let encoder = demuxer
        .metadata()
        .iter()
        .find(|item| item.name() == Some("ENCODER"))
        .and_then(|item| item.text());
    assert_eq!(
        encoder,
        format_tags["ENCODER"].as_str(),
        "{name}: ENCODER tag"
    );

    let mut previous = 0;
    let mut count = 0u64;
    for packet in demuxer.packets() {
        let packet = packet.unwrap_or_else(|err| panic!("{name}: {err}"));
        assert!(
            packet.offset >= previous,
            "{name}: packets() leaves file order at {packet:?}"
        );
        previous = packet.offset;
        count += 1;
    }
    let frames: u64 = tracks.iter().map(|track| track.sample_count).sum();
    assert_eq!(count, frames, "{name}: packets() covers every frame");
}

/// The reference data of a file: ffprobe output with the Adler-32 of each
/// packet's bytes, as gen_mkv.py writes it. `None`, with a notice, when
/// ffprobe is not on PATH.
pub fn probe_file(path: &Path) -> Option<Value> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_packets",
            "-show_data_hash",
            "adler32",
            "-show_chapters",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|err| eprintln!("skipping: ffprobe does not run: {err}"))
        .ok()?;
    assert!(
        output.status.success(),
        "ffprobe {} failed: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let data: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|err| panic!("ffprobe {} gave bad JSON: {err}", path.display()));
    Some(json!({
        "streams": data["streams"],
        "packets": data["packets"],
        "chapters": data["chapters"],
        "format": data["format"],
    }))
}
