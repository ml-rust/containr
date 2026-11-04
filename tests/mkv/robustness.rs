//! Truncation and corruption sweeps. Every outcome must be `Ok` or `Err`;
//! a panic at any offset fails the test and names the offset.

use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};

use containr::Error;
use containr::mkv::Demuxer;

use crate::common::{self, mkv_synth};

/// Packets read back per opened input. Every packet is still iterated.
const READS: usize = 8;

/// The bytes the corruption sweep of an FFmpeg fixture covers: the EBML
/// header, the Segment header elements and the first Cluster's blocks.
const FIXTURE_SWEEP: usize = 2048;

/// Open `bytes` and walk everything the demuxer exposes.
fn exercise(bytes: Vec<u8>) {
    let Ok(mut demuxer) = Demuxer::open(Cursor::new(bytes)) else {
        return;
    };
    let ids: Vec<u32> = demuxer.tracks().iter().map(|track| track.id).collect();
    for id in ids {
        if let Ok(packets) = demuxer.track_packets(id) {
            for packet in packets {
                if packet.is_err() {
                    break;
                }
            }
        }
    }
    let packets: Vec<_> = demuxer.packets().collect();
    for packet in packets.into_iter().flatten().take(READS) {
        let _ = demuxer.read_packet(&packet);
    }
    let attachments = demuxer.attachments().to_vec();
    for attachment in &attachments {
        let _ = demuxer.read_attachment(attachment);
    }
    let _ = demuxer.chapters().len()
        + demuxer.metadata().len()
        + demuxer.cues().len()
        + demuxer.seek_head().len();
}

fn check(label: &str, what: &str, bytes: Vec<u8>) {
    if catch_unwind(AssertUnwindSafe(|| exercise(bytes))).is_err() {
        panic!("{label}: the demuxer panicked on {what}");
    }
}

fn corruptions(label: &str, bytes: &[u8], range: std::ops::Range<usize>) {
    for at in range {
        let original = bytes[at];
        for value in [0x00, 0xFF, 0x01, original ^ 0x80] {
            if value == original {
                continue;
            }
            let mut copy = bytes.to_vec();
            copy[at] = value;
            check(label, &format!("byte {at} set to {value:#04x}"), copy);
        }
        if at + 8 <= bytes.len() {
            // An 8-byte size of all ones is the unknown-size marker.
            let mut copy = bytes.to_vec();
            copy[at..at + 8].copy_from_slice(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
            check(label, &format!("an unknown size at {at}"), copy);
        }
    }
}

#[test]
fn synthetic_file_survives_truncation_at_every_offset() {
    for unknown_segment in [false, true] {
        let synth = mkv_synth::file(unknown_segment);
        for cut in 0..synth.bytes.len() {
            check(
                "synthetic",
                &format!("a cut at {cut}"),
                synth.bytes[..cut].to_vec(),
            );
        }
    }
}

#[test]
fn synthetic_file_survives_corruption_at_every_offset() {
    for unknown_segment in [false, true] {
        let synth = mkv_synth::file(unknown_segment);
        corruptions("synthetic", &synth.bytes, 0..synth.bytes.len());
    }
}

#[test]
fn ffmpeg_fixtures_survive_truncation() {
    for name in ["h264_aac.mkv", "vp9_opus.webm", "live.webm"] {
        if let Some(fx) = common::fixture(name) {
            let len = fx.bytes.len();
            let cuts = (0..len)
                .step_by(61)
                .chain(0..FIXTURE_SWEEP.min(len))
                .chain([len - 1]);
            for cut in cuts {
                check(name, &format!("a cut at {cut}"), fx.bytes[..cut].to_vec());
            }
        }
    }
}

#[test]
fn ffmpeg_fixtures_survive_corruption() {
    for name in ["h264_aac.mkv", "subtitles.mkv", "live.webm"] {
        if let Some(fx) = common::fixture(name) {
            let end = FIXTURE_SWEEP.min(fx.bytes.len());
            corruptions(name, &fx.bytes, 0..end);
        }
    }
}

#[test]
fn colour_file_survives_truncation_and_corruption() {
    // MatrixCoefficients 9, TransferCharacteristics 16, Primaries 9, Range 1.
    let body = [
        0x55, 0xB1, 0x81, 9, 0x55, 0xBA, 0x81, 16, 0x55, 0xBB, 0x81, 9, 0x55, 0xB9, 0x81, 1,
    ];
    let synth = mkv_synth::file_with_colour(false, Some(&body));
    for cut in 0..synth.bytes.len() {
        check(
            "colour",
            &format!("a cut at {cut}"),
            synth.bytes[..cut].to_vec(),
        );
    }
    corruptions("colour", &synth.bytes, 0..synth.bytes.len());
}

#[test]
fn malformed_colour_elements_give_a_named_error() {
    let cases: [(&str, &[u8]); 4] = [
        (
            "a child that runs past its parent",
            &[0x55, 0xB1, 0x85, 0x01],
        ),
        ("a header cut short", &[0x55]),
        (
            "a Range of nine bytes",
            &[0x55, 0xB9, 0x89, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        ),
        (
            "a Primaries value above 255",
            &[0x55, 0xBB, 0x82, 0x01, 0x00],
        ),
    ];
    for (what, body) in cases {
        let synth = mkv_synth::file_with_colour(false, Some(body));
        let err = match Demuxer::open(Cursor::new(synth.bytes)) {
            Ok(_) => panic!("{what}: the demuxer accepted the element"),
            Err(err) => err,
        };
        assert!(matches!(err, Error::Malformed { .. }), "{what}: {err}");
        assert!(err.to_string().contains("Colour"), "{what}: {err}");
    }
}
