//! `ContentEncodings` on read: header stripping and zlib come out plain,
//! and every other encoding leaves the track's bytes as stored.

use std::io::Cursor;

use containr::mkv::{Demuxer, MAX_DECODED_SIZE};
use containr::{Error, Packet};

use crate::common::mkv_synth::{
    ENCODED_TRACK, EncodedFile, EncodingIn, encoded_file, stored_file, zlib_bomb,
};

type MemDemuxer = Demuxer<Cursor<Vec<u8>>>;

/// The header every plain frame starts with.
const HEADER: [u8; 3] = [0xFC, 0xFF, 0xFE];

fn open(bytes: &[u8]) -> MemDemuxer {
    Demuxer::open(Cursor::new(bytes.to_vec())).unwrap_or_else(|err| panic!("{err}"))
}

fn packets(demuxer: &MemDemuxer) -> Vec<Packet> {
    demuxer
        .track_packets(ENCODED_TRACK)
        .expect("the track exists")
        .collect::<Result<_, _>>()
        .unwrap_or_else(|err| panic!("{err}"))
}

/// Plain frames in an unlaced block, a Xiph-laced block of three, and an
/// unlaced block.
fn blocks() -> Vec<Vec<Vec<u8>>> {
    let frame = |n: u8, len: usize| {
        let mut out = HEADER.to_vec();
        out.extend((0..len).map(|i| n.wrapping_mul(31).wrapping_add(i as u8)));
        out
    };
    vec![
        vec![frame(0, 40)],
        vec![frame(1, 300), frame(2, 7), frame(3, 12)],
        vec![frame(4, 5)],
    ]
}

fn plain() -> Vec<Vec<u8>> {
    blocks().concat()
}

/// Open `file` and read every packet. Each packet's `size` is the stored
/// frame size.
fn read_all(file: &EncodedFile) -> (MemDemuxer, Vec<Packet>, Vec<Vec<u8>>) {
    let mut demuxer = open(&file.bytes);
    let packets = packets(&demuxer);
    assert_eq!(packets.len(), file.stored.len());
    assert!(packets.iter().any(|packet| packet.lace.is_some()));
    assert!(packets.iter().any(|packet| packet.lace.is_none()));
    let mut bytes = Vec::new();
    for (packet, stored) in packets.iter().zip(&file.stored) {
        assert_eq!(
            packet.size as usize,
            stored.len(),
            "{packet:?}: stored size"
        );
        bytes.push(
            demuxer
                .read_packet(packet)
                .unwrap_or_else(|err| panic!("{packet:?}: {err}")),
        );
    }
    (demuxer, packets, bytes)
}

/// Write `encodings`, read the file back, and check every frame and the
/// `CodecPrivate` come out plain.
fn check_decoded(encodings: &[EncodingIn]) -> EncodedFile {
    let file = encoded_file(encodings, &blocks());
    let (demuxer, _, bytes) = read_all(&file);
    assert_eq!(bytes, plain(), "{encodings:?}: frames");
    let mkv = demuxer
        .track(ENCODED_TRACK)
        .and_then(|track| track.matroska.as_ref())
        .expect("Matroska facts");
    assert!(mkv.content_decoded, "{encodings:?}");
    assert_eq!(
        mkv.content_encodings.len(),
        encodings.len(),
        "{encodings:?}"
    );
    let private = mkv.codec_private.as_deref().expect("a CodecPrivate");
    assert!(
        private.starts_with(b"OpusHead"),
        "{encodings:?}: {private:?}"
    );
    assert_eq!(private.len(), 19, "{encodings:?}: CodecPrivate");
    file
}

#[test]
fn header_stripping_restores_laced_and_unlaced_frames() {
    let file = check_decoded(&[EncodingIn::strip(0, 1, &HEADER)]);
    assert!(file.stored.iter().all(|frame| !frame.starts_with(&HEADER)));
}

#[test]
fn zlib_frames_inflate() {
    let file = check_decoded(&[EncodingIn::zlib(0, 1)]);
    assert_ne!(file.stored, plain(), "the frames are stored compressed");
}

#[test]
fn zlib_codec_private_inflates() {
    let file = check_decoded(&[EncodingIn::zlib(0, 2)]);
    assert!(!file.stored_private.starts_with(b"OpusHead"));
    assert_eq!(file.stored, plain(), "scope 2 leaves frames as stored");
}

#[test]
fn stacked_encodings_undo_highest_order_first() {
    // Stripped, then compressed: zlib undoes first.
    check_decoded(&[EncodingIn::strip(0, 1, &HEADER), EncodingIn::zlib(1, 3)]);
    // Compressed, then the 2-byte zlib header stripped: the header comes
    // back first.
    let zlib_header = miniz_oxide::deflate::compress_to_vec_zlib(&HEADER, 6)[..2].to_vec();
    check_decoded(&[
        EncodingIn::zlib(0, 1),
        EncodingIn::strip(1, 1, &zlib_header),
    ]);
    // File order does not matter, only `ContentEncodingOrder`.
    check_decoded(&[
        EncodingIn::strip(1, 1, &zlib_header),
        EncodingIn::zlib(0, 1),
    ]);
}

#[test]
fn a_zlib_bomb_frame_is_too_large() {
    let bomb = zlib_bomb(MAX_DECODED_SIZE + 1);
    let bytes = stored_file(
        &[EncodingIn::zlib(0, 1)],
        b"OpusHead",
        &[vec![bomb], vec![HEADER.to_vec()]],
    );
    let mut demuxer = open(&bytes);
    let packets = packets(&demuxer);
    let err = demuxer
        .read_packet(&packets[0])
        .expect_err("the frame inflates past the bound");
    assert!(
        matches!(err, Error::TooLarge { limit, .. } if limit == MAX_DECODED_SIZE),
        "{err}"
    );
    assert!(err.to_string().contains("track 1"), "{err}");
}

#[test]
fn a_zlib_bomb_codec_private_is_too_large() {
    let bomb = zlib_bomb(MAX_DECODED_SIZE + 1);
    let bytes = stored_file(&[EncodingIn::zlib(0, 2)], &bomb, &[vec![HEADER.to_vec()]]);
    let err = Demuxer::open(Cursor::new(bytes)).expect_err("CodecPrivate inflates past the bound");
    assert!(
        matches!(err, Error::TooLarge { limit, .. } if limit == MAX_DECODED_SIZE),
        "{err}"
    );
}

#[test]
fn a_corrupt_zlib_frame_is_malformed_and_names_the_track() {
    let corrupt = vec![0x78, 0x9C, 0xFF, 0xFF, 0xFF];
    let bytes = stored_file(&[EncodingIn::zlib(0, 1)], b"OpusHead", &[vec![corrupt]]);
    let mut demuxer = open(&bytes);
    let packets = packets(&demuxer);
    let err = demuxer
        .read_packet(&packets[0])
        .expect_err("the frame does not inflate");
    assert!(
        matches!(err, Error::Malformed { offset, .. } if offset == packets[0].offset),
        "{err}"
    );
    let text = err.to_string();
    assert!(text.contains("track 1") && text.contains("zlib"), "{text}");
}

#[test]
fn a_corrupt_zlib_codec_private_is_malformed() {
    let corrupt = [0x78, 0x9C, 0xFF, 0xFF, 0xFF];
    let bytes = stored_file(
        &[EncodingIn::zlib(0, 2)],
        &corrupt,
        &[vec![HEADER.to_vec()]],
    );
    let err = Demuxer::open(Cursor::new(bytes)).expect_err("CodecPrivate does not inflate");
    assert!(matches!(err, Error::Malformed { .. }), "{err}");
    let text = err.to_string();
    assert!(
        text.contains("CodecPrivate") && text.contains("zlib"),
        "{text}"
    );
}

#[test]
fn encodings_the_demuxer_cannot_undo_leave_the_track_as_stored() {
    let cases = [
        vec![EncodingIn::encryption(0)],
        vec![EncodingIn::compression(0, 1, 1)],
        vec![EncodingIn::compression(0, 1, 2)],
        vec![EncodingIn::compression(0, 1, 9)],
        vec![EncodingIn::zlib(0, 4)],
        // One encoding it can undo and one it cannot: neither is undone.
        vec![EncodingIn::strip(0, 1, &HEADER), EncodingIn::encryption(1)],
    ];
    for encodings in cases {
        let file = encoded_file(&encodings, &blocks());
        let (demuxer, _, bytes) = read_all(&file);
        assert_eq!(bytes, file.stored, "{encodings:?}: frames as stored");
        let mkv = demuxer
            .track(ENCODED_TRACK)
            .and_then(|track| track.matroska.as_ref())
            .expect("Matroska facts");
        assert!(!mkv.content_decoded, "{encodings:?}");
        assert_eq!(
            mkv.codec_private.as_deref(),
            Some(file.stored_private.as_slice()),
            "{encodings:?}: CodecPrivate as stored"
        );
    }
}
