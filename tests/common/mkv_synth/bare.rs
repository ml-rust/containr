//! A one-track Matroska file whose blocks store no duration: no
//! `DefaultDuration`, no `BlockDuration`. FFmpeg 8 and older write audio this way.

use super::elements::{cat, el, text, uint};

/// The `TrackNumber` of the one track.
pub const BARE_TRACK: u32 = 1;

/// The track a [`bare_file`] holds.
#[derive(Clone, Copy)]
pub enum BareKind {
    Opus,
    Video,
    Text,
}

fn track_entry(kind: BareKind) -> Vec<u8> {
    let (track_type, codec, body) = match kind {
        BareKind::Opus => {
            let mut head = b"OpusHead".to_vec();
            head.extend([1, 2]);
            head.extend(312u16.to_le_bytes());
            head.extend(48_000u32.to_le_bytes());
            head.extend([0, 0, 0]);
            let audio = el(
                0xE1,
                &cat(&[
                    el(0xB5, &48_000.0f32.to_bits().to_be_bytes()),
                    uint(0x9F, 2),
                ]),
            );
            let body = cat(&[el(0x63A2, &head), uint(0x56AA, 6_500_000), audio]);
            (2, "A_OPUS", body)
        }
        BareKind::Video => {
            let video = el(0xE0, &cat(&[uint(0xB0, 96), uint(0xBA, 64)]));
            let body = cat(&[el(0x63A2, &[1, 0x42, 0xC0, 0x1E]), video]);
            (1, "V_MPEG4/ISO/AVC", body)
        }
        BareKind::Text => (17, "S_TEXT/UTF8", Vec::new()),
    };
    el(
        0xAE,
        &cat(&[
            uint(0xD7, u64::from(BARE_TRACK)),
            uint(0x73C5, 0x3333),
            uint(0x83, track_type),
            text(0x86, codec),
            body,
        ]),
    )
}

/// A file of one track and one Cluster at timestamp 0. Each block is its
/// timestamp in 1 ms ticks and its `DiscardPadding` in nanoseconds. A block with
/// padding is a `BlockGroup`, and each other block is a `SimpleBlock`.
pub fn bare_file(kind: BareKind, blocks: &[(i16, Option<i64>)]) -> Vec<u8> {
    let header = el(
        0x1A45_DFA3,
        &cat(&[
            uint(0x4286, 1),
            uint(0x42F2, 4),
            uint(0x42F3, 8),
            text(0x4282, "matroska"),
            uint(0x4287, 4),
            uint(0x4285, 2),
        ]),
    );
    let info = el(
        0x1549_A966,
        &cat(&[
            uint(0x2AD7B1, 1_000_000),
            text(0x4D80, "containr-test"),
            text(0x5741, "containr-test"),
        ]),
    );
    let tracks = el(0x1654_AE6B, &track_entry(kind));
    let mut cluster = uint(0xE7, 0);
    for &(timestamp, padding) in blocks {
        let mut body = vec![0x80 | BARE_TRACK as u8];
        body.extend(timestamp.to_be_bytes());
        match padding {
            None => {
                body.extend([0x80, 1, 2, 3]);
                cluster.extend(el(0xA3, &body));
            }
            Some(padding) => {
                body.extend([0, 1, 2, 3]);
                let group = cat(&[el(0xA1, &body), el(0x75A2, &padding.to_be_bytes())]);
                cluster.extend(el(0xA0, &group));
            }
        }
    }
    let segment = cat(&[info, tracks, el(0x1F43_B675, &cluster)]);
    cat(&[header, el(0x1853_8067, &segment)])
}
