//! mkvmerge-made files with header removal and zlib compression. Each reads
//! back equal to a copy of the same source with no compression, and equal to
//! ffprobe, which undoes both encodings.

use std::fs::File;
use std::io::BufReader;

use containr::Packet;
use containr::mkv::Demuxer;

use crate::common::mkv_probe::media_streams;
use crate::common::{self, Fixture, adler32, array, int, text};

type FileDemuxer = Demuxer<BufReader<File>>;

fn fixture(name: &str) -> Option<Fixture> {
    let fx = common::fixture(name);
    if fx.is_none() {
        eprintln!(
            "skipping: {name} needs mkvmerge on PATH when `python3 tests/fixtures/gen_mkv.py` runs"
        );
    }
    fx
}

fn open(fx: &Fixture) -> FileDemuxer {
    Demuxer::open_path(&fx.path).unwrap_or_else(|err| panic!("{}: {err}", fx.name))
}

fn packets(name: &str, demuxer: &FileDemuxer, track: u32) -> Vec<Packet> {
    demuxer
        .track_packets(track)
        .expect("the track exists")
        .collect::<Result<_, _>>()
        .unwrap_or_else(|err| panic!("{name} track {track}: {err}"))
}

/// Compare `name`, whose tracks mkvmerge wrote with `ContentCompAlgo`
/// `algorithm`, with `plain_name`, the same source remuxed with
/// `--compression <track>:none`, and with ffprobe.
fn check(name: &str, plain_name: &str, algorithm: u64) {
    let (Some(plain), Some(fx)) = (fixture(plain_name), fixture(name)) else {
        return;
    };
    let mut source = open(&plain);
    let mut demuxer = open(&fx);
    let tracks = demuxer.tracks().to_vec();
    let source_tracks = source.tracks().to_vec();
    assert_eq!(tracks.len(), source_tracks.len(), "{name}: track count");
    let streams = media_streams(&fx);
    assert_eq!(streams.len(), tracks.len(), "{name}: ffprobe streams");
    let mut encoded = 0;
    for ((track, theirs), stream) in tracks.iter().zip(&source_tracks).zip(&streams) {
        let what = format!("{name} track {}", track.id);
        let mkv = track.matroska.as_ref().expect("Matroska facts");
        let source_mkv = theirs.matroska.as_ref().expect("Matroska facts");
        assert!(
            source_mkv.content_encodings.is_empty(),
            "{what}: plain source"
        );
        if mkv
            .content_encodings
            .iter()
            .any(|encoding| encoding.compression_algorithm == Some(algorithm))
        {
            encoded += 1;
        }
        assert!(mkv.content_decoded, "{what}: {:?}", mkv.content_encodings);
        assert_eq!(
            mkv.codec_private, source_mkv.codec_private,
            "{what}: CodecPrivate"
        );
        let ours = packets(name, &demuxer, track.id);
        let plain_packets = packets(plain_name, &source, theirs.id);
        assert_eq!(ours.len(), plain_packets.len(), "{what}: packet count");
        let index = int(stream, "index");
        let probed: Vec<_> = array(&fx.probe, "packets")
            .iter()
            .filter(|packet| int(packet, "stream_index") == index)
            .collect();
        assert_eq!(probed.len(), ours.len(), "{what}: ffprobe packet count");
        for (n, ((our, their), probe)) in ours.iter().zip(&plain_packets).zip(&probed).enumerate() {
            let at = format!("{what} packet {n}");
            assert_eq!(our.pts, their.pts, "{at}: pts");
            assert_eq!(our.sync, their.sync, "{at}: keyframe");
            let bytes = demuxer
                .read_packet(our)
                .unwrap_or_else(|err| panic!("{at}: {err}"));
            let plain_bytes = source
                .read_packet(their)
                .unwrap_or_else(|err| panic!("{at}: {err}"));
            assert_eq!(bytes, plain_bytes, "{at}: bytes");
            assert_eq!(bytes.len() as i64, int(probe, "size"), "{at}: ffprobe size");
            assert_eq!(
                adler32(&bytes),
                text(probe, "data_hash"),
                "{at}: ffprobe bytes"
            );
        }
    }
    assert!(
        encoded > 0,
        "{name}: no track has `ContentCompAlgo` {algorithm}. Run gen_mkv.py again"
    );
}

#[test]
fn mkvmerge_header_removal_reads_back_plain() {
    check("mkvmerge_header_removal.mkv", "mkvmerge_mpeg4_plain.mkv", 3);
}

#[test]
fn mkvmerge_zlib_reads_back_plain() {
    check("mkvmerge_zlib.mkv", "mkvmerge_plain.mkv", 0);
}
