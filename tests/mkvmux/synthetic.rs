//! Streams built by hand, with timestamps known exactly.

use std::io::Cursor;

use containr::matroska::{DisplayUnit, MatroskaVideo, RawElement};
use containr::mkv::{MkvOptions, Muxer, PacketIn, TrackSpec, tags_from_mp4};
use containr::{Chapter, ChapterSource, MetadataData, MetadataItem, MetadataKey, MetadataScope};
use containr::{FourCc, Timebase, TrackKind};

use crate::mkvmux::support::open_mkv;

const MS: Timebase = Timebase::from_nanoseconds(1_000_000);

fn video() -> TrackSpec {
    let mut spec = TrackSpec::new(TrackKind::Video, "V_MPEG4/ISO/AVC", MS);
    spec.matroska.codec_private = Some(vec![1, 2, 3]);
    spec.matroska.default_duration = Some(40_000_000);
    spec.matroska.video = Some(MatroskaVideo {
        pixel_width: 96,
        pixel_height: 64,
        display_width: None,
        display_height: None,
        display_unit: DisplayUnit::Pixels,
        crop: [0, 2, 0, 4],
        interlaced: 2,
        projection: None,
        colour: Some(containr::ContainerColour {
            primaries: None,
            transfer: None,
            matrix: None,
            full_range: Some(false),
        }),
        extra: vec![RawElement {
            id: 0x55B0,
            body: vec![0x55, 0xB9, 0x81, 1],
        }],
    });
    spec
}

fn frame(data: &[u8], pts: i64, sync: bool) -> PacketIn<'_> {
    PacketIn {
        data,
        pts,
        duration: None,
        sync,
        discardable: false,
        discard_padding: None,
    }
}

#[test]
fn pts_reorders_in_decode_order_and_flags_survive() {
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), MkvOptions::default()).expect("options");
    let track = muxer.add_track(video()).expect("a track");
    let order = [
        (0, true, false),
        (120, false, false),
        (40, false, true),
        (80, false, true),
    ];
    for &(pts, sync, discardable) in &order {
        let data = [pts as u8];
        let mut packet = frame(&data, pts, sync);
        packet.discardable = discardable;
        muxer.write_packet(track, &packet).expect("a packet");
    }
    let bytes = muxer.finish().expect("the file ends").into_inner();
    let mut demuxer = open_mkv(&bytes);
    let packets: Vec<_> = demuxer
        .track_packets(1)
        .expect("the track")
        .map(|packet| packet.expect("a packet"))
        .collect();
    let read: Vec<(i64, bool, bool)> = packets
        .iter()
        .map(|p| (p.pts, p.sync, p.discardable))
        .collect();
    assert_eq!(read, order);
    for packet in &packets {
        assert_eq!(packet.duration, Some(40));
        let data = demuxer.read_packet(packet).expect("the bytes");
        assert_eq!(data, [packet.pts as u8]);
    }
}

#[test]
fn track_facts_read_back_with_a_rescale_that_stays_exact() {
    let options = MkvOptions {
        timestamp_scale: 500_000,
        title: Some("a title".to_owned()),
        writing_app: "a test".to_owned(),
        ..MkvOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("options");
    let track = muxer.add_track(video()).expect("a track");
    muxer
        .write_packet(track, &frame(&[1], 40, true))
        .expect("a packet");
    let bytes = muxer.finish().expect("the file ends").into_inner();
    let demuxer = open_mkv(&bytes);
    assert_eq!(demuxer.info().timestamp_scale, 500_000);
    assert_eq!(demuxer.info().title.as_deref(), Some("a title"));
    assert_eq!(demuxer.info().writing_app.as_deref(), Some("a test"));
    assert!(
        demuxer
            .info()
            .muxing_app
            .as_deref()
            .is_some_and(|app| app.starts_with("containr"))
    );
    let read = demuxer.tracks()[0].clone();
    assert_eq!(read.timebase, Timebase::from_nanoseconds(500_000));
    let mkv = read.matroska.as_ref().expect("Matroska facts");
    let mut want = video().matroska;
    want.uid = mkv.uid;
    assert_ne!(mkv.uid, None);
    assert_eq!(mkv, &want);
    let packet = demuxer
        .track_packets(1)
        .expect("the track")
        .next()
        .expect("a packet")
        .expect("a packet");
    assert_eq!(packet.pts, 80, "40 ms at 0.5 ms per tick");
    assert_eq!(packet.duration, Some(80));
}

#[test]
fn tags_chapters_and_track_tags_read_back() {
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), MkvOptions::default()).expect("options");
    let mut spec = video();
    let item = |key: &[u8; 4], text: &str| MetadataItem {
        scope: MetadataScope::Ilst,
        key: MetadataKey::FourCc(FourCc(*key)),
        value: Vec::new(),
        data: vec![MetadataData {
            type_indicator: 1,
            locale: 0,
            payload: text.as_bytes().to_vec(),
        }],
    };
    spec.metadata = tags_from_mp4(&[item(b"\xA9cmt", "a track note")]).expect("the item maps");
    let track = muxer.add_track(spec).expect("a track");
    muxer
        .set_metadata(tags_from_mp4(&[item(b"\xA9nam", "a movie")]).expect("the item maps"))
        .expect("the tags fit");
    muxer
        .set_chapters(vec![Chapter {
            start: 40,
            duration: Some(40),
            timescale: 1000,
            title: "one".to_owned(),
            source: ChapterSource::Chpl,
        }])
        .expect("the chapter fits");
    muxer
        .write_packet(track, &frame(&[1], 0, true))
        .expect("a packet");
    let bytes = muxer.finish().expect("the file ends").into_inner();
    let demuxer = open_mkv(&bytes);
    assert_eq!(demuxer.metadata().len(), 1);
    assert_eq!(demuxer.metadata()[0].name(), Some("TITLE"));
    assert_eq!(demuxer.metadata()[0].text(), Some("a movie"));
    let track = &demuxer.tracks()[0];
    assert_eq!(track.metadata[0].name(), Some("COMMENT"));
    assert_eq!(track.metadata[0].text(), Some("a track note"));
    let chapter = &demuxer.chapters()[0];
    assert_eq!(chapter.start, 40_000_000);
    assert_eq!(chapter.duration, Some(40_000_000));
    assert!(matches!(
        chapter.source,
        ChapterSource::Matroska { chapter_uid, depth: 0, edition_uid: None } if chapter_uid != 0
    ));
}

#[test]
fn an_empty_file_reads_back() {
    let muxer = Muxer::new(Cursor::new(Vec::new()), MkvOptions::default()).expect("options");
    let bytes = muxer.finish().expect("the file ends").into_inner();
    let demuxer = open_mkv(&bytes);
    assert!(demuxer.tracks().is_empty());
    assert_eq!(demuxer.info().duration, Some(0.0));
}

#[test]
fn tracks_interleave_by_timestamp() {
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), MkvOptions::default()).expect("options");
    let first = muxer.add_track(video()).expect("a track");
    let second = muxer.add_track(video()).expect("a track");
    for n in 0..4 {
        muxer
            .write_packet(first, &frame(&[1], n * 40, true))
            .expect("a packet");
    }
    for n in 0..4 {
        muxer
            .write_packet(second, &frame(&[2], n * 40, true))
            .expect("a packet");
    }
    let bytes = muxer.finish().expect("the file ends").into_inner();
    let times: Vec<i64> = open_mkv(&bytes)
        .packets()
        .map(|packet| packet.expect("a packet").pts)
        .collect();
    assert_eq!(times, [0, 0, 40, 40, 80, 80, 120, 120]);
}

#[test]
fn buffered_bytes_stay_within_the_queue_and_cluster_bounds() {
    let cluster_bytes = containr::mkv::MIN_CLUSTER_BYTES * 4;
    let options = MkvOptions {
        cluster_bytes,
        max_buffer: cluster_bytes,
        ..MkvOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("options");
    let track = muxer.add_track(video()).expect("a track");
    // This track never sends a packet, so only the bound lets video out.
    let mut idle = video();
    idle.matroska.uid = Some(7);
    let _idle = muxer.add_track(idle).expect("a track");
    let data = vec![1u8; 512];
    for n in 0..200 {
        muxer
            .write_packet(track, &frame(&data, n * 40, true))
            .expect("a packet");
        let bound = cluster_bytes * 2 + data.len() + 64;
        assert!(muxer.buffered_bytes() <= bound, "packet {n}");
    }
}
