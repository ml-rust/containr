//! FFmpeg-made fixtures compared against ffprobe: track facts, codec
//! private data, packets, packet bytes and track tags.
//!
//! libavformat subtracts `CodecDelay` from every pts, rounded to the
//! nearest tick, and reports `pos` at the block body. The demuxer keeps the
//! raw block timestamp and points at the frame data, after the 4-byte
//! block header that a 1-byte track number gives.

use std::fs::File;
use std::io::BufReader;

use containr::mkv::Demuxer;
use containr::{Packet, Track, TrackKind};
use serde_json::Value;

use crate::common::{self, Fixture, adler32, array, int, ratio, text};

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
        assert_eq!(
            our.duration.map(|d| d as i64),
            their.get("duration").map(|_| int(their, "duration")),
            "{at}: duration"
        );
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

/// Compare one fixture against its ffprobe reference. Skips when the
/// generator has not run.
pub fn check_fixture(name: &str) {
    let Some(fx) = common::fixture(name) else {
        return;
    };
    let mut demuxer = Demuxer::open_path(&fx.path).unwrap_or_else(|err| panic!("{name}: {err}"));
    assert!(!demuxer.truncated(), "{name}: the file is complete");
    let streams = media_streams(&fx);
    assert_eq!(streams.len(), demuxer.tracks().len(), "{name}: track count");
    let tracks = demuxer.tracks().to_vec();
    for (stream, track) in streams.iter().zip(&tracks) {
        check_stream(&fx, stream, track);
        check_packets(&fx, &mut demuxer, int(stream, "index"), track);
    }
    let format_tags = &fx.probe["format"]["tags"];
    assert_eq!(
        demuxer.info().title.as_deref(),
        format_tags["title"].as_str(),
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
