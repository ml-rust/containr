//! The whole file: header elements, Clusters and expected packets.

use containr::Packet;

use super::blocks::{Group, Laced, Spec, cluster};
use super::elements::{cat, el, frame_bytes, id_bytes, size_bytes, text, uint, uint8};

pub const VIDEO: u32 = 1;
pub const AUDIO: u32 = 2;
pub const VIDEO_UID: u64 = 0x1111;
pub const AUDIO_UID: u64 = 0x2222;
pub const ATTACHMENT_UID: u64 = 0x77;
pub const CODEC_PRIVATE: [u8; 4] = [1, 0x42, 0xC0, 0x1E];
pub const ATTACHMENT: &[u8] = b"not a real font, 32 bytes long..";

/// The built file, its packets in file order, and byte offsets the error
/// tests patch.
pub struct Synth {
    pub bytes: Vec<u8>,
    pub packets: Vec<Packet>,
    /// The `DocType` value.
    pub doc_type: usize,
    /// The ID byte of the first Cluster's `Timestamp`.
    pub first_timestamp: usize,
    /// The body offset of the Segment.
    pub segment_data: u64,
    /// The offsets of the two Clusters.
    pub clusters: [u64; 2],
}

fn tracks() -> Vec<u8> {
    let projection = el(0x7670, &el(0x7675, &(-90.0f64).to_bits().to_be_bytes()));
    let video = el(
        0xE0,
        &cat(&[
            uint(0xB0, 96),
            uint(0xBA, 64),
            uint(0x54B0, 4),
            uint(0x54BA, 3),
            uint(0x54B2, 3),
            projection,
        ]),
    );
    let video_entry = el(
        0xAE,
        &cat(&[
            uint(0xD7, u64::from(VIDEO)),
            uint(0x73C5, VIDEO_UID),
            uint(0x83, 1),
            uint(0x9C, 0),
            text(0x22B59C, "und"),
            text(0x86, "V_MPEG4/ISO/AVC"),
            el(0x63A2, &CODEC_PRIVATE),
            uint(0x23E383, 40_000_000),
            video,
        ]),
    );
    let audio = el(
        0xE1,
        &cat(&[
            el(0xB5, &48_000.0f32.to_bits().to_be_bytes()),
            uint(0x9F, 2),
            uint(0x6264, 16),
        ]),
    );
    let compression = el(0x5034, &cat(&[uint(0x4254, 3), el(0x4255, &[0xAB])]));
    let encodings = el(
        0x6D80,
        &el(
            0x6240,
            &cat(&[
                uint(0x5031, 0),
                uint(0x5032, 1),
                uint(0x5033, 0),
                compression,
            ]),
        ),
    );
    let audio_entry = el(
        0xAE,
        &cat(&[
            uint(0xD7, u64::from(AUDIO)),
            uint(0x73C5, AUDIO_UID),
            uint(0x83, 2),
            text(0x22B59C, "eng"),
            text(0x22B59D, "en-US"),
            text(0x86, "A_OPUS"),
            el(0x63A2, b"OpusHead"),
            uint(0x56AA, 6_500_000),
            uint(0x56BB, 80_000_000),
            uint(0x23E383, 20_000_000),
            audio,
            encodings,
        ]),
    );
    // A CRC-32 child, which the demuxer skips.
    el(
        0x1654_AE6B,
        &cat(&[el(0xBF, &[0; 4]), video_entry, audio_entry]),
    )
}

fn cues(clusters: [u64; 2]) -> Vec<u8> {
    let point = |time: u64, cluster: u64| {
        el(
            0xBB,
            &cat(&[
                uint(0xB3, time),
                el(0xB7, &cat(&[uint(0xF7, 1), uint8(0xF1, cluster)])),
            ]),
        )
    };
    el(
        0x1C53_BB6B,
        &cat(&[point(0, clusters[0]), point(120, clusters[1])]),
    )
}

fn chapters() -> Vec<u8> {
    let display = |title: &str| el(0x80, &text(0x85, title));
    let nested = el(
        0xB6,
        &cat(&[uint(0x73C4, 2), uint(0x91, 50_000_000), display("One.A")]),
    );
    let one = el(
        0xB6,
        &cat(&[
            uint(0x73C4, 1),
            uint(0x91, 0),
            uint(0x92, 100_000_000),
            display("One"),
            nested,
        ]),
    );
    let two = el(
        0xB6,
        &cat(&[uint(0x73C4, 3), uint(0x91, 100_000_000), display("Two")]),
    );
    el(0x1043_A770, &el(0x45B9, &cat(&[uint(0x45BC, 7), one, two])))
}

fn tags() -> Vec<u8> {
    let simple = |name: &str, value: &str, nested: Vec<u8>| {
        el(
            0x67C8,
            &cat(&[text(0x45A3, name), text(0x4487, value), nested]),
        )
    };
    let movie = el(
        0x7373,
        &cat(&[
            el(0x63C0, &uint(0x68CA, 50)),
            simple(
                "TITLE",
                "synthetic",
                simple("SORT_WITH", "synth", Vec::new()),
            ),
        ]),
    );
    let track = el(
        0x7373,
        &cat(&[
            el(0x63C0, &cat(&[uint(0x68CA, 30), uint(0x63C5, AUDIO_UID)])),
            simple("ENCODER", "test-enc", Vec::new()),
        ]),
    );
    let binary = el(0x67C8, &cat(&[text(0x45A3, "ROLE"), el(0x4485, &[1, 2])]));
    let attachment = el(
        0x7373,
        &cat(&[el(0x63C0, &uint(0x63C6, ATTACHMENT_UID)), binary]),
    );
    el(0x1254_C367, &cat(&[movie, track, attachment]))
}

fn attachments() -> Vec<u8> {
    let file = cat(&[
        text(0x466E, "font.ttf"),
        text(0x4660, "font/ttf"),
        text(0x467E, "a font"),
        el(0x465C, ATTACHMENT),
        uint(0x46AE, ATTACHMENT_UID),
    ]);
    el(0x1941_A469, &el(0x61A7, &file))
}

fn seek_head(positions: [u64; 3]) -> Vec<u8> {
    let seek = |id: u32, position: u64| {
        el(
            0x4DBB,
            &cat(&[el(0x53AB, &id.to_be_bytes()), uint8(0x53AC, position)]),
        )
    };
    el(
        0x114D_9B74,
        &cat(&[
            seek(0x1549_A966, positions[0]),
            seek(0x1654_AE6B, positions[1]),
            seek(0x1C53_BB6B, positions[2]),
        ]),
    )
}

/// Build the file. With `unknown_segment`, the Segment size is the
/// unknown-size marker.
pub fn file(unknown_segment: bool) -> Synth {
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
    let doc_type = header
        .windows(8)
        .position(|w| w == b"matroska")
        .expect("DocType");
    let info = el(
        0x1549_A966,
        &cat(&[
            uint(0x2AD7B1, 1_000_000),
            el(0x4489, &200.0f64.to_bits().to_be_bytes()),
            text(0x7BA9, "synthetic"),
            text(0x4D80, "containr-test"),
            text(0x5741, "containr-test"),
        ]),
    );
    let tracks = tracks();
    let mut counts = [0u64; 2];
    let first = [
        Spec {
            track: VIDEO,
            relative: 0,
            flags: 0x80,
            kind: Laced::No,
            sizes: &[10],
            group: None,
        },
        Spec {
            track: AUDIO,
            relative: 0,
            flags: 0x80,
            kind: Laced::Xiph,
            sizes: &[300, 4, 7],
            group: None,
        },
        Spec {
            track: VIDEO,
            relative: 40,
            flags: 0,
            kind: Laced::No,
            sizes: &[6],
            group: Some(Group {
                duration: Some(40),
                reference: true,
                discard_padding: None,
            }),
        },
        Spec {
            track: AUDIO,
            relative: 60,
            flags: 0x80,
            kind: Laced::Ebml,
            sizes: &[5, 9, 3],
            group: None,
        },
    ];
    let second = [
        Spec {
            track: AUDIO,
            relative: 0,
            flags: 0x80,
            kind: Laced::Fixed,
            sizes: &[4, 4],
            group: None,
        },
        Spec {
            track: VIDEO,
            relative: -40,
            flags: 0x01,
            kind: Laced::No,
            sizes: &[5],
            group: None,
        },
        Spec {
            track: AUDIO,
            relative: 40,
            flags: 0,
            kind: Laced::No,
            sizes: &[8],
            group: Some(Group {
                duration: Some(13),
                reference: false,
                discard_padding: Some(7_000_000),
            }),
        },
        Spec {
            track: AUDIO,
            relative: 60,
            flags: 0,
            kind: Laced::Xiph,
            sizes: &[3, 3],
            group: Some(Group {
                duration: Some(30),
                reference: false,
                discard_padding: Some(-2_000_000),
            }),
        },
    ];
    let (cluster1, packets1, ts1) = cluster(0, &first, true, &mut counts);
    let (cluster2, packets2, _) = cluster(120, &second, false, &mut counts);
    let void = el(0xEC, &[0; 3]);
    let seek_len = seek_head([0; 3]).len();
    let info_at = (seek_len + void.len()) as u64;
    let tracks_at = info_at + info.len() as u64;
    let cluster1_at = tracks_at + tracks.len() as u64;
    let cluster2_at = cluster1_at + cluster1.len() as u64;
    let cues_at = cluster2_at + cluster2.len() as u64;
    let body = cat(&[
        seek_head([info_at, tracks_at, cues_at]),
        void,
        info,
        tracks,
        cluster1,
        cluster2,
        cues([cluster1_at, cluster2_at]),
        chapters(),
        tags(),
        attachments(),
    ]);
    let mut bytes = header;
    bytes.extend(id_bytes(0x1853_8067));
    if unknown_segment {
        bytes.extend([0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    } else {
        bytes.extend(size_bytes(body.len()));
    }
    let segment_data = bytes.len() as u64;
    bytes.extend(body);
    let mut packets = Vec::new();
    for (base, list) in [(cluster1_at, packets1), (cluster2_at, packets2)] {
        for mut packet in list {
            packet.offset += segment_data + base;
            packets.push(packet);
        }
    }
    Synth {
        bytes,
        packets,
        doc_type,
        first_timestamp: (segment_data + cluster1_at) as usize + ts1,
        segment_data,
        clusters: [segment_data + cluster1_at, segment_data + cluster2_at],
    }
}

/// The bytes the synthetic file stores for `packet`.
pub fn packet_bytes(packet: &Packet) -> Vec<u8> {
    frame_bytes(packet.track_id, packet.index, packet.size)
}
