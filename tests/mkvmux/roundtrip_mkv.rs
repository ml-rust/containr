//! Matroska and WebM fixtures: demux, stream copy, demux again. Packets, track
//! facts, tags, chapters and attachments come back equal.

use containr::Packet;

use crate::common::{self, mkv_synth};
use crate::mkvmux::support::{MkvDemuxer, collect, copy_mkv, ok, open_mkv};
use crate::mkvmux::walk::walk;

/// What a reader takes as equal: every packet field but the byte offset, and
/// the lace, since the copy writes one frame per block.
fn check_packet(at: &str, ours: &Packet, theirs: &Packet) {
    assert_eq!(ours.track_id, theirs.track_id, "{at}: track");
    assert_eq!(ours.index, theirs.index, "{at}: index");
    assert_eq!(ours.dts, None, "{at}: Matroska stores no dts");
    assert_eq!(ours.pts, theirs.pts, "{at}: pts");
    assert_eq!(ours.duration, theirs.duration, "{at}: duration");
    assert_eq!(ours.sync, theirs.sync, "{at}: keyframe");
    assert_eq!(ours.discardable, theirs.discardable, "{at}: discardable");
    assert_eq!(
        ours.discard_padding, theirs.discard_padding,
        "{at}: DiscardPadding"
    );
    assert_eq!(ours.size, theirs.size, "{at}: size");
    assert_eq!(ours.lace, None, "{at}: the copy writes no lacing");
}

fn check_packets(name: &str, input: &mut MkvDemuxer, output: &mut MkvDemuxer, track: u32) {
    let theirs = collect(name, ok(name, input.track_packets(track)));
    let ours = collect(name, ok(name, output.track_packets(track)));
    assert_eq!(ours.len(), theirs.len(), "{name} track {track}: packets");
    for (n, (our, their)) in ours.iter().zip(&theirs).enumerate() {
        let at = format!("{name} track {track} packet {n}");
        check_packet(&at, our, their);
        let our_bytes = ok(name, output.read_packet(our));
        let their_bytes = ok(name, input.read_packet(their));
        assert_eq!(our_bytes, their_bytes, "{at}: bytes");
    }
}

/// Copy `bytes`, read the copy back, and compare it with the source.
fn check_bytes(name: &str, bytes: &[u8]) {
    let copy = copy_mkv(name, bytes, |_| {});
    let mut input = open_mkv(bytes);
    let mut output = open_mkv(&copy);
    assert!(!output.truncated(), "{name}: the copy is whole");
    assert_eq!(
        output.ebml_header().doc_type,
        input.ebml_header().doc_type,
        "{name}: DocType"
    );
    assert_eq!(
        output.info().timestamp_scale,
        input.info().timestamp_scale,
        "{name}: TimestampScale"
    );
    assert_eq!(output.info().title, input.info().title, "{name}: Title");
    assert_eq!(output.tracks(), input.tracks(), "{name}: track facts");
    assert_eq!(output.metadata(), input.metadata(), "{name}: tags");
    assert_eq!(output.chapters(), input.chapters(), "{name}: chapters");
    let tracks: Vec<u32> = input.tracks().iter().map(|track| track.id).collect();
    for &track in &tracks {
        check_packets(name, &mut input, &mut output, track);
    }
    let theirs = input.attachments().to_vec();
    let ours = output.attachments().to_vec();
    assert_eq!(ours.len(), theirs.len(), "{name}: attachments");
    for (our, their) in ours.iter().zip(&theirs) {
        assert_eq!(our.uid, their.uid, "{name}: FileUID");
        assert_eq!(our.name, their.name, "{name}: FileName");
        assert_eq!(our.mime, their.mime, "{name}: FileMediaType");
        assert_eq!(our.description, their.description, "{name}: description");
        assert_eq!(
            ok(name, output.read_attachment(our)),
            ok(name, input.read_attachment(their)),
            "{name}: FileData"
        );
    }
    // The last block of each track states its duration, whatever the source
    // stored, so the track's end reads back from the block itself.
    let layout = walk(&copy);
    for &track in &tracks {
        let theirs = collect(name, ok(name, input.track_packets(track)));
        let Some(last) = theirs.last() else {
            continue;
        };
        let stored = layout
            .last_block(u64::from(track))
            .unwrap_or_else(|| panic!("{name} track {track}: no block"));
        assert_eq!(stored.duration, last.duration, "{name} track {track}: last");
        assert!(stored.duration.is_some(), "{name} track {track}: explicit");
    }
}

fn check(name: &str) {
    if let Some(fixture) = common::fixture(name) {
        check_bytes(name, &fixture.bytes);
    }
}

#[test]
fn round_trip_synthetic_every_lacing_kind() {
    for unknown_segment in [false, true] {
        check_bytes("synthetic", &mkv_synth::file(unknown_segment).bytes);
    }
}

#[test]
fn round_trip_h264_aac() {
    check("h264_aac.mkv");
}

#[test]
fn round_trip_hevc_opus() {
    check("hevc_opus.mkv");
}

#[test]
fn round_trip_vp9_opus_webm() {
    check("vp9_opus.webm");
}

#[test]
fn round_trip_av1_webm() {
    check("av1.webm");
}

#[test]
fn round_trip_subtitles() {
    check("subtitles.mkv");
}

#[test]
fn round_trip_chapters() {
    check("chapters.mkv");
}

#[test]
fn round_trip_attachment() {
    check("attachment.mkv");
}

#[test]
fn round_trip_rotate90() {
    check("rotate90.mkv");
}

#[test]
fn round_trip_opus_trim() {
    check("opus_trim.webm");
}

#[test]
fn round_trip_live_webm() {
    check("live.webm");
}
