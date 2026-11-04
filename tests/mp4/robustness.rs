//! Truncation and corruption sweeps. Every outcome must be `Ok` or `Err`;
//! a panic at any offset fails the test and names the offset.

use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};

use containr::Error;
use containr::mp4::Demuxer;

use crate::common::{self, moov_range, synth};

/// Packets read back per opened input. Every packet is still iterated.
const READS: usize = 8;

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
    let _ = demuxer.chapters().len() + demuxer.metadata().len();
}

fn check(label: &str, what: &str, bytes: Vec<u8>) {
    if catch_unwind(AssertUnwindSafe(|| exercise(bytes))).is_err() {
        panic!("{label}: the demuxer panicked on {what}");
    }
}

fn truncations(label: &str, bytes: &[u8]) {
    let moov = moov_range(bytes);
    let cuts = (0..bytes.len())
        .step_by(61)
        .chain(moov.clone())
        .chain([bytes.len() - 1]);
    for cut in cuts {
        check(label, &format!("a cut at {cut}"), bytes[..cut].to_vec());
    }
}

fn corruptions(label: &str, bytes: &[u8]) {
    for at in moov_range(bytes) {
        let original = bytes[at];
        for value in [0x00, 0xFF, original ^ 0x80] {
            if value == original {
                continue;
            }
            let mut copy = bytes.to_vec();
            copy[at] = value;
            check(label, &format!("byte {at} set to {value:#04x}"), copy);
        }
        if at + 4 <= bytes.len() {
            for value in [0u32, 0x7FFF_FFFF, 0xFFFF_FFFF] {
                let mut copy = bytes.to_vec();
                copy[at..at + 4].copy_from_slice(&value.to_be_bytes());
                check(label, &format!("u32 at {at} set to {value:#010x}"), copy);
            }
        }
    }
}

#[test]
fn synthetic_movie_survives_truncation_at_every_offset() {
    for moov_first in [true, false] {
        let (bytes, _) = synth::movie(moov_first);
        for cut in 0..bytes.len() {
            check(
                "synthetic",
                &format!("a cut at {cut}"),
                bytes[..cut].to_vec(),
            );
        }
    }
}

#[test]
fn synthetic_movie_survives_corruption_at_every_moov_offset() {
    for moov_first in [true, false] {
        let (bytes, _) = synth::movie(moov_first);
        corruptions("synthetic", &bytes);
    }
}

#[test]
fn ffmpeg_fixtures_survive_truncation() {
    for name in ["h264_aac_moov_end.mp4", "h264_aac_faststart.mp4"] {
        if let Some(fx) = common::fixture(name) {
            truncations(name, &fx.bytes);
        }
    }
}

#[test]
fn ffmpeg_fixtures_survive_corruption() {
    for name in ["h264_aac_faststart.mp4", "chapters_mov.mov"] {
        if let Some(fx) = common::fixture(name) {
            corruptions(name, &fx.bytes);
        }
    }
}

#[test]
fn colr_movie_survives_truncation_and_corruption() {
    let nclx: &[u8] = &[b'n', b'c', b'l', b'x', 0, 9, 0, 16, 0, 9, 0x80];
    let nclc: &[u8] = &[b'n', b'c', b'l', b'c', 0, 1, 0, 1, 0, 1];
    let (bytes, _) = synth::movie_with_colr(true, &[b"prof\x01", nclx, nclc]);
    for cut in 0..bytes.len() {
        check("colr", &format!("a cut at {cut}"), bytes[..cut].to_vec());
    }
    corruptions("colr", &bytes);
}

#[test]
fn malformed_colr_boxes_give_a_named_error() {
    let cases: [(&str, &[u8]); 6] = [
        ("an empty body", b""),
        ("a colour type cut short", b"ncl"),
        ("an nclx box with no values", b"nclx"),
        (
            "an nclx box without its flag byte",
            &[b'n', b'c', b'l', b'x', 0, 9, 0, 16, 0, 9],
        ),
        (
            "an nclc box with one value",
            &[b'n', b'c', b'l', b'c', 0, 1],
        ),
        (
            "a primaries value above 255",
            &[b'n', b'c', b'l', b'x', 1, 0, 0, 16, 0, 9, 0],
        ),
    ];
    for (what, body) in cases {
        let (bytes, _) = synth::movie_with_colr(true, &[body]);
        let err = match Demuxer::open(Cursor::new(bytes)) {
            Ok(_) => panic!("{what}: the demuxer accepted the box"),
            Err(err) => err,
        };
        assert!(matches!(err, Error::Malformed { .. }), "{what}: {err}");
        assert!(err.to_string().contains("colr"), "{what}: {err}");
    }
}
