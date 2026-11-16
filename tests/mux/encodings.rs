//! Matroska tracks with `ContentEncodings` copied into MP4. A track the
//! demuxer decodes copies as plain samples. A track it leaves encoded is
//! refused, and the error names Matroska.

use std::io::Cursor;

use containr::mp4::{MuxOptions, Muxer, StreamCopy};
use containr::{Container, Error, Result};

use crate::common::mkv_synth::{EncodingIn, encoded_file};
use crate::mux::support::{collect, ok, open_mkv, open_mp4};

const PREFIX: [u8; 2] = [0xFC, 0xFF];

fn blocks() -> Vec<Vec<Vec<u8>>> {
    let frame = |text: &[u8]| [&PREFIX[..], text].concat();
    vec![
        vec![frame(b"frame one")],
        vec![frame(b"two"), frame(b"three")],
        vec![frame(b"four")],
    ]
}

/// Copy the one track of the Matroska file `bytes` into an MP4 file.
fn copy_to_mp4(name: &str, bytes: &[u8]) -> Result<Vec<u8>> {
    let mut input = open_mkv(bytes);
    let options = MuxOptions::default();
    let movie_timescale = options.movie_timescale;
    let mut muxer = ok(name, Muxer::new(Cursor::new(Vec::new()), options));
    let track = input.tracks()[0].clone();
    let packets = collect(name, ok(name, input.track_packets(track.id)));
    let plan = StreamCopy::plan(&track, &packets, movie_timescale)?;
    let id = ok(name, muxer.add_track(plan.spec.clone()));
    for packet in &packets {
        let data = ok(name, input.read_packet(packet));
        let packet_in = ok(name, plan.packet(packet, &data));
        ok(name, muxer.write_packet(id, &packet_in));
    }
    Ok(ok(name, muxer.finish()).into_inner())
}

#[test]
fn a_header_stripped_track_copies_plain_samples() {
    let file = encoded_file(&[EncodingIn::strip(0, 1, &PREFIX)], &blocks());
    let bytes = ok("stripped", copy_to_mp4("stripped", &file.bytes));
    let mut output = open_mp4(&bytes);
    let id = output.tracks()[0].id;
    let packets = collect("stripped", ok("stripped", output.track_packets(id)));
    let samples: Vec<Vec<u8>> = packets
        .iter()
        .map(|packet| ok("stripped", output.read_packet(packet)))
        .collect();
    assert_eq!(samples, blocks().concat());
}

#[test]
fn an_encrypted_track_is_refused_and_names_matroska() {
    let file = encoded_file(&[EncodingIn::encryption(0)], &blocks());
    let err = copy_to_mp4("encrypted", &file.bytes).expect_err("MP4 cannot undo encryption");
    assert!(
        matches!(
            err,
            Error::Unsupported {
                container: Container::Mp4,
                use_instead: Container::Matroska,
                ..
            }
        ),
        "{err}"
    );
    let text = err.to_string();
    assert!(
        text.contains("ContentEncodings") && text.contains("encryption"),
        "{text}"
    );
}
