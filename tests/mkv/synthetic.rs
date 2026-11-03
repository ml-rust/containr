//! Demux of the in-memory synthetic file, whose blocks are known exactly.

use std::io::Cursor;

use containr::matroska::DisplayUnit;
use containr::mkv::Demuxer;
use containr::{
    ChapterSource, Container, Error, Lace, MetadataScope, TagTarget, Timebase, TrackKind,
};

use crate::common::mkv_synth::{self, AUDIO, AUDIO_UID, Synth, VIDEO, VIDEO_UID};

fn open(synth: &Synth) -> Demuxer<Cursor<Vec<u8>>> {
    Demuxer::open(Cursor::new(synth.bytes.clone()))
        .unwrap_or_else(|err| panic!("the synthetic file is well formed: {err}"))
}

#[test]
fn synthetic_packets_cover_every_lacing_kind_in_both_segment_sizes() {
    for unknown_segment in [false, true] {
        let synth = mkv_synth::file(unknown_segment);
        let mut demuxer = open(&synth);
        assert!(!demuxer.truncated());
        let packets: Vec<_> = demuxer
            .packets()
            .collect::<Result<_, _>>()
            .expect("every packet decodes");
        assert_eq!(
            packets, synth.packets,
            "unknown_segment = {unknown_segment}"
        );
        for packet in &packets {
            let bytes = demuxer
                .read_packet(packet)
                .expect("the packet is in the input");
            assert_eq!(bytes, mkv_synth::packet_bytes(packet), "{packet:?}");
        }
        for id in [VIDEO, AUDIO] {
            let ours: Vec<_> = demuxer
                .track_packets(id)
                .expect("the track exists")
                .collect::<Result<_, _>>()
                .expect("every packet decodes");
            let want: Vec<_> = synth
                .packets
                .iter()
                .copied()
                .filter(|p| p.track_id == id)
                .collect();
            assert_eq!(ours, want, "track {id}");
            let track = demuxer.track(id).expect("the track exists");
            assert_eq!(track.sample_count, want.len() as u64);
        }
    }
}

#[test]
fn synthetic_laces_split_timestamps_and_padding() {
    let synth = mkv_synth::file(false);
    let demuxer = open(&synth);
    let audio: Vec<_> = demuxer
        .track_packets(AUDIO)
        .expect("audio track")
        .collect::<Result<_, _>>()
        .expect("every packet decodes");
    let pts: Vec<i64> = audio.iter().map(|p| p.pts).collect();
    assert_eq!(pts, vec![0, 20, 40, 60, 80, 100, 120, 140, 160, 180, 195]);
    let sizes: Vec<u32> = audio.iter().map(|p| p.size).collect();
    assert_eq!(sizes, vec![300, 4, 7, 5, 9, 3, 4, 4, 8, 3, 3]);
    assert_eq!(audio[1].lace, Some(Lace { index: 1, count: 3 }));
    assert_eq!(audio[8].lace, None);
    assert_eq!(
        audio[8].duration,
        Some(13),
        "BlockDuration wins over DefaultDuration"
    );
    assert_eq!(audio[8].discard_padding, Some(7_000_000));
    assert_eq!(
        audio[9].discard_padding,
        Some(-2_000_000),
        "negative trims the first frame"
    );
    assert_eq!(audio[10].discard_padding, None);
    assert_eq!(
        (audio[9].duration, audio[10].duration),
        (Some(15), Some(15))
    );
    let video: Vec<_> = demuxer
        .track_packets(VIDEO)
        .expect("video track")
        .collect::<Result<_, _>>()
        .expect("every packet decodes");
    let flags: Vec<_> = video
        .iter()
        .map(|p| (p.pts, p.sync, p.discardable))
        .collect();
    assert_eq!(
        flags,
        vec![(0, true, false), (40, false, false), (80, false, true)]
    );
    assert!(video.iter().chain(&audio).all(|p| p.dts.is_none()));
}

#[test]
fn synthetic_tracks_carry_their_matroska_facts() {
    let synth = mkv_synth::file(false);
    let demuxer = open(&synth);
    assert_eq!(demuxer.ebml_header().doc_type, "matroska");
    assert_eq!(demuxer.ebml_header().doc_type_version, 4);
    let info = demuxer.info();
    assert_eq!(info.timestamp_scale, 1_000_000);
    assert_eq!(info.duration, Some(200.0));
    assert_eq!(info.title.as_deref(), Some("synthetic"));
    assert_eq!(info.muxing_app.as_deref(), Some("containr-test"));

    let video = demuxer.track(VIDEO).expect("video track");
    assert_eq!(video.kind, TrackKind::Video);
    assert_eq!(video.handler, None);
    assert_eq!(video.timebase, Timebase::from_nanoseconds(1_000_000));
    assert_eq!(video.duration, None);
    assert_eq!(video.language.iso639(), Some("und"));
    let mkv = video.matroska.as_ref().expect("Matroska facts");
    assert_eq!(mkv.uid, Some(VIDEO_UID));
    assert_eq!(
        mkv.codec_private.as_deref(),
        Some(&mkv_synth::CODEC_PRIVATE[..])
    );
    assert_eq!(mkv.default_duration, Some(40_000_000));
    assert!(!mkv.lacing);
    let geometry = mkv.video.as_ref().expect("Video element");
    assert_eq!((geometry.pixel_width, geometry.pixel_height), (96, 64));
    assert_eq!(
        (geometry.display_width, geometry.display_height),
        (Some(4), Some(3))
    );
    assert_eq!(geometry.display_unit, DisplayUnit::AspectRatio);
    assert_eq!(
        geometry.projection.as_ref().map(|p| p.pose_roll),
        Some(-90.0)
    );

    let audio = demuxer.track(AUDIO).expect("audio track");
    assert_eq!(audio.codec_id(), Some("A_OPUS"));
    assert_eq!(audio.language.iso639(), Some("eng"));
    let mkv = audio.matroska.as_ref().expect("Matroska facts");
    assert_eq!(mkv.language_bcp47.as_deref(), Some("en-US"));
    assert_eq!(
        (mkv.codec_delay, mkv.seek_pre_roll),
        (6_500_000, 80_000_000)
    );
    let sound = mkv.audio.as_ref().expect("Audio element");
    assert_eq!(sound.sampling_frequency, 48_000.0, "a 4-byte float");
    assert_eq!((sound.channels, sound.bit_depth), (2, Some(16)));
    let encoding = mkv.content_encodings.first().expect("one ContentEncoding");
    assert_eq!(encoding.compression_algorithm, Some(3), "header stripping");
    assert_eq!(encoding.compression_settings.as_deref(), Some(&[0xAB][..]));
}

#[test]
fn synthetic_index_chapters_tags_and_attachment() {
    let synth = mkv_synth::file(false);
    let mut demuxer = open(&synth);
    assert_eq!(demuxer.segment_data_offset(), synth.segment_data);
    let offsets: Vec<u64> = demuxer
        .cues()
        .iter()
        .flat_map(|point| point.positions.iter().map(|p| p.cluster_offset))
        .collect();
    assert_eq!(offsets, synth.clusters.to_vec());
    let ids: Vec<u32> = demuxer.seek_head().iter().map(|entry| entry.id).collect();
    assert_eq!(ids, vec![0x1549_A966, 0x1654_AE6B, 0x1C53_BB6B]);
    for entry in demuxer.seek_head() {
        let at = entry.offset as usize;
        assert_eq!(synth.bytes[at..at + 4], entry.id.to_be_bytes(), "{entry:?}");
    }

    let chapters: Vec<_> = demuxer
        .chapters()
        .iter()
        .map(|c| (c.title.as_str(), c.start, c.duration, c.source))
        .collect();
    let source = |chapter_uid, depth| ChapterSource::Matroska {
        edition_uid: Some(7),
        chapter_uid,
        depth,
    };
    assert_eq!(
        chapters,
        vec![
            ("One", 0, Some(100_000_000), source(1, 0)),
            ("One.A", 50_000_000, None, source(2, 1)),
            ("Two", 100_000_000, None, source(3, 0)),
        ]
    );

    let segment: Vec<_> = demuxer
        .metadata()
        .iter()
        .map(|item| (item.name(), item.text(), item.value.clone()))
        .collect();
    assert_eq!(
        segment,
        vec![
            (Some("TITLE"), Some("synthetic"), b"synthetic".to_vec()),
            (Some("TITLE/SORT_WITH"), Some("synth"), b"synth".to_vec()),
            (Some("ROLE"), None, vec![1, 2]),
        ]
    );
    let role = &demuxer.metadata()[2];
    assert!(matches!(
        role.scope,
        MetadataScope::Tag(scope) if scope.binary
            && scope.target == TagTarget::Attachment(mkv_synth::ATTACHMENT_UID)
    ));
    let audio = demuxer.track(AUDIO).expect("audio track");
    let encoder = audio
        .metadata
        .first()
        .expect("the track tag moves into the track");
    assert_eq!(
        (encoder.name(), encoder.text()),
        (Some("ENCODER"), Some("test-enc"))
    );
    assert!(matches!(
        encoder.scope,
        MetadataScope::Tag(scope) if scope.target == TagTarget::Track(AUDIO_UID)
            && scope.target_type_value == 30
    ));

    let attachment = demuxer
        .attachments()
        .first()
        .cloned()
        .expect("one attachment");
    assert_eq!(
        (
            attachment.name.as_str(),
            attachment.mime.as_str(),
            attachment.uid
        ),
        ("font.ttf", "font/ttf", mkv_synth::ATTACHMENT_UID)
    );
    assert_eq!(attachment.description.as_deref(), Some("a font"));
    let bytes = demuxer
        .read_attachment(&attachment)
        .expect("the attachment reads");
    assert_eq!(bytes, mkv_synth::ATTACHMENT);
}

#[test]
fn synthetic_undeclared_track_is_inconsistent() {
    let mut synth = mkv_synth::file(false);
    let first = synth.packets[0].offset as usize - 4;
    assert_eq!(
        synth.bytes[first], 0x81,
        "the track number of the first block"
    );
    synth.bytes[first] = 0x89;
    match Demuxer::open(Cursor::new(synth.bytes)) {
        Err(Error::Inconsistent {
            container,
            path,
            reason,
        }) => {
            assert_eq!(container, Container::Matroska);
            assert_eq!(path, "Segment/Cluster[1]");
            assert!(reason.contains("track 9"), "{reason}");
        }
        other => panic!("expected Inconsistent, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn synthetic_block_before_timestamp_names_the_cluster() {
    let mut synth = mkv_synth::file(false);
    // Turn the Timestamp into a Void, so the first block has no timestamp.
    synth.bytes[synth.first_timestamp] = 0xEC;
    match Demuxer::open(Cursor::new(synth.bytes)) {
        Err(Error::Malformed { path, reason, .. }) => {
            assert_eq!(path, "Segment/Cluster[1]");
            assert!(reason.contains("Timestamp"), "{reason}");
        }
        other => panic!("expected Malformed, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn synthetic_unknown_doc_type_is_refused() {
    let mut synth = mkv_synth::file(false);
    synth.bytes[synth.doc_type] = b'X';
    match Demuxer::open(Cursor::new(synth.bytes)) {
        Err(Error::Malformed { path, .. }) => assert_eq!(path, "EBML/DocType"),
        other => panic!("expected Malformed, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn synthetic_truncation_keeps_the_whole_blocks() {
    let synth = mkv_synth::file(false);
    let cut = synth.clusters[1] as usize + 20;
    let demuxer = Demuxer::open(Cursor::new(synth.bytes[..cut].to_vec()))
        .unwrap_or_else(|err| panic!("a cut inside a Cluster keeps the packets before it: {err}"));
    assert!(demuxer.truncated());
    let packets: Vec<_> = demuxer
        .packets()
        .collect::<Result<_, _>>()
        .expect("packets");
    let whole: Vec<_> = synth
        .packets
        .iter()
        .copied()
        .filter(|p| p.offset + u64::from(p.size) <= synth.clusters[1])
        .collect();
    assert_eq!(packets, whole);
    assert!(demuxer.chapters().is_empty(), "Chapters follow the cut");
}

#[test]
fn missing_track_is_named() {
    let synth = mkv_synth::file(false);
    let demuxer = open(&synth);
    match demuxer.track_packets(5) {
        Err(Error::TrackNotFound {
            track_id,
            available,
        }) => {
            assert_eq!((track_id, available), (5, vec![VIDEO, AUDIO]));
        }
        other => panic!("expected TrackNotFound, got {:?}", other.map(|_| ())),
    }
}
