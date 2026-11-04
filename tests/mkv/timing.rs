//! `first_track_timing`: trim facts, read with no packet index.

use std::io::Cursor;

use containr::mkv::{Demuxer, TrackTiming, first_track_timing};
use containr::{Error, TrackKind};

use crate::common::mkv_synth::{self, AUDIO, VIDEO};

/// An element of a one-byte size.
fn el(id: &[u8], body: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.push(0x80 | u8::try_from(body.len()).expect("the body is small"));
    out.extend_from_slice(body);
    out
}

const UNKNOWN: u8 = 0xFF;
const CLUSTER: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];
const SEGMENT: [u8; 4] = [0x18, 0x53, 0x80, 0x67];

fn audio_tracks(number: u8) -> Vec<u8> {
    let entry = [
        el(&[0xD7], &[number]),
        el(&[0x83], &[2]),
        el(&[0x56, 0xAA], &[0x0F, 0x42, 0x40]),
    ]
    .concat();
    el(&[0x16, 0x54, 0xAE, 0x6B], &el(&[0xAE], &entry))
}

/// A `BlockGroup` of `track` that marks `padding`.
fn group(track: u8, padding: &[u8]) -> Vec<u8> {
    let block = el(&[0xA1], &[0x80 | track, 0, 0, 0]);
    el(&[0xA0], &[block, el(&[0x75, 0xA2], padding)].concat())
}

fn timing(bytes: Vec<u8>, kind: TrackKind) -> Option<TrackTiming> {
    first_track_timing(Cursor::new(bytes), kind)
        .unwrap_or_else(|err| panic!("the read elements are well formed: {err}"))
}

/// The fixture's audio track: a positive padding and a negative one.
const AUDIO_TIMING: TrackTiming = TrackTiming {
    track_id: AUDIO,
    codec_delay: 6_500_000,
    discard_padding: 7_000_000,
};

#[test]
fn the_first_track_of_a_kind_gives_its_trims_in_both_segment_sizes() {
    for unknown_segment in [false, true] {
        let synth = mkv_synth::file(unknown_segment);
        assert_eq!(
            timing(synth.bytes.clone(), TrackKind::Audio),
            Some(AUDIO_TIMING)
        );
        assert_eq!(
            timing(synth.bytes.clone(), TrackKind::Video),
            Some(TrackTiming {
                track_id: VIDEO,
                codec_delay: 0,
                discard_padding: 0,
            })
        );
        assert_eq!(timing(synth.bytes, TrackKind::Subtitle), None);
    }
}

#[test]
fn the_sum_matches_the_packets_the_demuxer_reads() {
    let synth = mkv_synth::file(false);
    let demuxer = Demuxer::open(Cursor::new(synth.bytes)).expect("the file is well formed");
    let sum: i64 = demuxer
        .track_packets(AUDIO)
        .expect("the audio track exists")
        .map(|packet| packet.expect("the packet reads").discard_padding)
        .filter_map(|padding| padding.filter(|&value| value > 0))
        .sum();
    assert_eq!(sum, 7_000_000);
}

#[test]
fn a_block_of_an_undeclared_track_gives_no_error() {
    let mut synth = mkv_synth::file(false);
    let first = synth.packets[0].offset as usize - 4;
    synth.bytes[first] = 0x89;
    assert!(Demuxer::open(Cursor::new(synth.bytes.clone())).is_err());
    assert_eq!(timing(synth.bytes, TrackKind::Audio), Some(AUDIO_TIMING));
}

#[test]
fn text_that_is_not_utf8_gives_no_error() {
    let mut synth = mkv_synth::file(false);
    let title = synth
        .bytes
        .windows(9)
        .position(|window| window == b"synthetic")
        .expect("the Info title");
    synth.bytes[title] = 0xFF;
    assert!(Demuxer::open(Cursor::new(synth.bytes.clone())).is_err());
    assert_eq!(timing(synth.bytes, TrackKind::Audio), Some(AUDIO_TIMING));
}

#[test]
fn a_padding_in_every_unknown_size_cluster_counts() {
    let timestamp = el(&[0xE7], &[0]);
    let blocks = [timestamp, group(2, &[0x00, 0xCD, 0xFE, 0x60])].concat();
    let unknown_cluster = [CLUSTER.as_slice(), [UNKNOWN].as_slice(), blocks.as_slice()].concat();
    let segment = el(
        &SEGMENT,
        &[audio_tracks(2), unknown_cluster.clone(), unknown_cluster].concat(),
    );
    let file = [el(&[0x1A, 0x45, 0xDF, 0xA3], &[]), segment].concat();
    assert_eq!(
        timing(file, TrackKind::Audio),
        Some(TrackTiming {
            track_id: 2,
            codec_delay: 1_000_000,
            discard_padding: 27_000_000,
        })
    );
}

#[test]
fn a_cluster_before_tracks_counts() {
    let cluster = el(
        &CLUSTER,
        &[el(&[0xE7], &[0]), group(2, &[0x10]), group(1, &[0x20])].concat(),
    );
    let segment = el(&SEGMENT, &[cluster, audio_tracks(2)].concat());
    let file = [el(&[0x1A, 0x45, 0xDF, 0xA3], &[]), segment].concat();
    assert_eq!(
        timing(file, TrackKind::Audio),
        Some(TrackTiming {
            track_id: 2,
            codec_delay: 1_000_000,
            discard_padding: 16,
        })
    );
}

#[test]
fn a_void_with_an_eight_byte_size_is_read_past() {
    // FFmpeg writes this Void: its size in eight bytes.
    let void = [0xEC, 0x01, 0, 0, 0, 0, 0, 0, 0x04, 1, 2, 3, 4];
    let segment = el(
        &SEGMENT,
        &[void.as_slice(), audio_tracks(2).as_slice()].concat(),
    );
    let file = [el(&[0x1A, 0x45, 0xDF, 0xA3], &[]), segment].concat();
    assert_eq!(
        timing(file, TrackKind::Audio),
        Some(TrackTiming {
            track_id: 2,
            codec_delay: 1_000_000,
            discard_padding: 0,
        })
    );
}

#[test]
fn a_cut_keeps_the_padding_before_it() {
    let synth = mkv_synth::file(false);
    let before = timing(
        synth.bytes[..synth.clusters[1] as usize].to_vec(),
        TrackKind::Audio,
    )
    .expect("the file has audio");
    assert_eq!(before.discard_padding, 0);
    assert_eq!(before.codec_delay, 6_500_000);
}

#[test]
fn missing_tracks_is_named() {
    let segment = el(&SEGMENT, &el(&[0xEC], &[0]));
    let file = [el(&[0x1A, 0x45, 0xDF, 0xA3], &[]), segment].concat();
    match first_track_timing(Cursor::new(file), TrackKind::Audio) {
        Err(Error::MissingBox { path, .. }) => assert_eq!(path, "Segment/Tracks"),
        other => panic!("expected MissingBox, got {other:?}"),
    }
}

#[test]
fn an_unreadable_input_gives_an_error_and_no_panic() {
    assert!(first_track_timing(Cursor::new(Vec::new()), TrackKind::Audio).is_err());
    let text = b"RIFF....".to_vec();
    assert!(first_track_timing(Cursor::new(text), TrackKind::Audio).is_err());
}
