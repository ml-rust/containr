//! Stream copy of tracks with `ContentEncodings`. A track the demuxer
//! decodes copies as plain frames with no encodings. A track it leaves
//! encoded copies its stored bytes and its encodings.

use crate::common::mkv_synth::{EncodingIn, encoded_file};
use crate::mkvmux::support::{MkvDemuxer, collect, copy_mkv, ok, open_mkv};

const PREFIX: [u8; 2] = [0xFC, 0xFF];

fn blocks() -> Vec<Vec<Vec<u8>>> {
    let frame = |text: &[u8]| [&PREFIX[..], text].concat();
    vec![
        vec![frame(b"frame one")],
        vec![frame(b"two"), frame(b"three"), frame(b"four")],
        vec![frame(b"five")],
    ]
}

/// Every packet of the one track of `demuxer`, read.
fn read_all(name: &str, demuxer: &mut MkvDemuxer) -> Vec<Vec<u8>> {
    let id = demuxer.tracks()[0].id;
    let packets = collect(name, ok(name, demuxer.track_packets(id)));
    packets
        .iter()
        .map(|packet| {
            let bytes = ok(name, demuxer.read_packet(packet));
            assert_eq!(packet.size as usize, bytes.len(), "{name}: {packet:?}");
            bytes
        })
        .collect()
}

#[test]
fn a_decoded_track_copies_plain_frames_and_no_content_encodings() {
    let encodings = [EncodingIn::strip(0, 1, &PREFIX), EncodingIn::zlib(1, 3)];
    let file = encoded_file(&encodings, &blocks());
    let copy = copy_mkv("encoded", &file.bytes, |_| {});
    let mut input = open_mkv(&file.bytes);
    let mut output = open_mkv(&copy);
    let source = input.tracks()[0].matroska.clone().expect("Matroska facts");
    let copied = output.tracks()[0].matroska.clone().expect("Matroska facts");
    assert!(source.content_decoded);
    assert_eq!(source.content_encodings.len(), 2);
    assert!(copied.content_decoded);
    assert!(copied.content_encodings.is_empty(), "{copied:?}");
    assert_eq!(copied.codec_private, source.codec_private);
    let plain: Vec<Vec<u8>> = blocks().concat();
    let theirs: Vec<Vec<u8>> = {
        let id = input.tracks()[0].id;
        let packets = collect("encoded", ok("encoded", input.track_packets(id)));
        packets
            .iter()
            .map(|packet| ok("encoded", input.read_packet(packet)))
            .collect()
    };
    assert_eq!(theirs, plain, "the source reads back plain");
    assert_eq!(
        read_all("copy", &mut output),
        plain,
        "the copy stores plain"
    );
}

#[test]
fn an_encrypted_track_copies_its_stored_bytes_and_encodings() {
    let file = encoded_file(&[EncodingIn::encryption(0)], &blocks());
    let copy = copy_mkv("encrypted", &file.bytes, |_| {});
    let input = open_mkv(&file.bytes);
    let mut output = open_mkv(&copy);
    let source = input.tracks()[0].matroska.clone().expect("Matroska facts");
    let copied = output.tracks()[0].matroska.clone().expect("Matroska facts");
    assert!(!source.content_decoded);
    assert!(!copied.content_decoded);
    assert_eq!(copied.content_encodings, source.content_encodings);
    assert_eq!(read_all("copy", &mut output), file.stored);
}
