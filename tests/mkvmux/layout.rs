//! The bytes of a file the muxer writes, walked apart from the demuxer.

use std::io::Cursor;

use containr::mkv::{DocType, MkvOptions, Muxer, NoSeek, PacketIn, TrackSpec};
use containr::{Timebase, TrackKind};

use crate::common::mkv_synth;
use crate::mkvmux::support::{copy_mkv, open_mkv};
use crate::mkvmux::walk::{
    ATTACHMENTS, CHAPTERS, CLUSTER, CUES, INFO, SEEK_HEAD, TAGS, TRACKS, walk,
};

const MS: Timebase = Timebase::from_nanoseconds(1_000_000);

fn audio_spec() -> TrackSpec {
    let mut spec = TrackSpec::new(TrackKind::Audio, "A_OPUS", MS);
    spec.matroska.default_duration = Some(20_000_000);
    spec
}

fn packet(data: &[u8], pts: i64, duration: Option<u64>) -> PacketIn<'_> {
    PacketIn {
        data,
        pts,
        duration,
        sync: true,
        discardable: false,
        discard_padding: None,
    }
}

#[test]
fn segment_children_come_in_the_documented_order_and_the_seek_head_finds_them() {
    let copy = copy_mkv("synthetic", &mkv_synth::file(false).bytes, |_| {});
    let layout = walk(&copy);
    assert!(layout.segment_size_known);
    assert_eq!(layout.doc_type, "matroska");
    assert_eq!(
        layout.order(),
        [
            SEEK_HEAD,
            INFO,
            TRACKS,
            CLUSTER,
            CUES,
            CHAPTERS,
            TAGS,
            ATTACHMENTS
        ]
    );
    let demuxer = open_mkv(&copy);
    let entries = demuxer.seek_head();
    assert_eq!(entries.len(), 6);
    for entry in entries {
        assert!(
            layout
                .children
                .iter()
                .any(|&(id, offset)| id == entry.id && offset == entry.offset),
            "a SeekHead entry misses: {entry:?}"
        );
    }
    let (duration, end) = (demuxer.info().duration, demuxer.tracks().len());
    assert_eq!(end, 2);
    assert!(duration.is_some_and(|duration| duration > 0.0));
}

#[test]
fn blocks_are_simple_until_they_need_a_duration() {
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), MkvOptions::default()).expect("options");
    let track = muxer.add_track(audio_spec()).expect("a track");
    let data = [1u8; 8];
    for (pts, duration) in [(0, None), (20, Some(20)), (40, Some(7)), (47, None)] {
        muxer
            .write_packet(track, &packet(&data, pts, duration))
            .expect("the packet fits");
    }
    let bytes = muxer.finish().expect("the file ends").into_inner();
    let blocks = walk(&bytes).blocks;
    let simple: Vec<bool> = blocks.iter().map(|block| block.simple).collect();
    assert_eq!(simple, [true, true, false, false]);
    assert_eq!(blocks[2].duration, Some(7));
    assert_eq!(blocks[3].duration, Some(20), "the last block is explicit");
    let mut demuxer = open_mkv(&bytes);
    let durations: Vec<Option<u64>> = demuxer
        .track_packets(1)
        .expect("the track exists")
        .map(|packet| packet.expect("a packet").duration)
        .collect();
    assert_eq!(durations, [Some(20), Some(20), Some(7), Some(20)]);
    let last = demuxer
        .packets()
        .last()
        .expect("a packet")
        .expect("a packet");
    assert_eq!(
        demuxer.read_packet(&last).expect("the bytes read"),
        data.to_vec()
    );
}

#[test]
fn block_groups_hold_a_reference_and_padding_and_keep_the_discardable_bit() {
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), MkvOptions::default()).expect("options");
    let track = muxer.add_track(audio_spec()).expect("a track");
    let mut trimmed = packet(&[1], 0, None);
    trimmed.discard_padding = Some(-3);
    muxer
        .write_packet(track, &trimmed)
        .expect("the packet fits");
    let mut dependent = packet(&[2], 20, None);
    dependent.sync = false;
    dependent.discardable = true;
    muxer
        .write_packet(track, &dependent)
        .expect("the packet fits");
    let bytes = muxer.finish().expect("the file ends").into_inner();
    let blocks = walk(&bytes).blocks;
    assert!(!blocks[0].simple && blocks[0].padding && !blocks[0].has_reference);
    assert_eq!(blocks[0].duration, None, "a duration the default gives");
    assert!(!blocks[1].simple && !blocks[1].padding && blocks[1].has_reference);
    assert_eq!(blocks[1].flags, 0x01, "no keyframe bit in a Block");
    let demuxer = open_mkv(&bytes);
    let packets: Vec<_> = demuxer
        .track_packets(1)
        .expect("the track exists")
        .map(|packet| packet.expect("a packet"))
        .collect();
    assert!(packets[0].sync && !packets[0].discardable);
    assert_eq!(packets[0].discard_padding, Some(-3));
    assert!(!packets[1].sync && packets[1].discardable);
}

#[test]
fn a_new_cluster_starts_before_an_offset_passes_i16() {
    let options = MkvOptions {
        cluster_duration_ms: 3_600_000,
        ..MkvOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("options");
    let track = muxer.add_track(audio_spec()).expect("a track");
    let pts = [0, 32_767, 32_768, 100_000, 67_233, 67_232];
    for &at in &pts {
        muxer
            .write_packet(track, &packet(&[0], at, Some(1)))
            .expect("the packet fits");
    }
    let bytes = muxer.finish().expect("the file ends").into_inner();
    let layout = walk(&bytes);
    let stored: Vec<i64> = layout
        .blocks
        .iter()
        .map(|block| block.cluster_timestamp as i64 + i64::from(block.relative))
        .collect();
    assert_eq!(stored, pts);
    assert_eq!(layout.blocks[1].relative, i16::MAX);
    assert_eq!(layout.blocks[4].relative, i16::MIN + 1);
    assert_eq!(layout.clusters.len(), 3, "{:?}", layout.clusters);
}

#[test]
fn cluster_limits_close_a_cluster() {
    let options = MkvOptions {
        cluster_duration_ms: 100,
        ..MkvOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("options");
    let track = muxer.add_track(audio_spec()).expect("a track");
    for n in 0..10 {
        muxer
            .write_packet(track, &packet(&[0], n * 20, None))
            .expect("the packet fits");
    }
    let bytes = muxer.finish().expect("the file ends").into_inner();
    assert_eq!(walk(&bytes).clusters.len(), 2);

    let options = MkvOptions {
        cluster_bytes: containr::mkv::MIN_CLUSTER_BYTES,
        ..MkvOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("options");
    let track = muxer.add_track(audio_spec()).expect("a track");
    let data = vec![0u8; containr::mkv::MIN_CLUSTER_BYTES];
    for n in 0..4 {
        muxer
            .write_packet(track, &packet(&data, n * 20, None))
            .expect("the packet fits");
    }
    let bytes = muxer.finish().expect("the file ends").into_inner();
    assert_eq!(walk(&bytes).clusters.len(), 4);
}

#[test]
fn cues_point_at_keyframes_of_the_video_track() {
    let mut video = TrackSpec::new(TrackKind::Video, "V_VP9", MS);
    video.matroska.video = Some(containr::matroska::MatroskaVideo {
        pixel_width: 16,
        pixel_height: 16,
        display_width: None,
        display_height: None,
        display_unit: containr::matroska::DisplayUnit::Pixels,
        crop: [0; 4],
        interlaced: 0,
        projection: None,
        extra: Vec::new(),
    });
    video.matroska.default_duration = Some(40_000_000);
    let options = MkvOptions {
        cluster_duration_ms: 100,
        ..MkvOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("options");
    let video = muxer.add_track(video).expect("a track");
    let audio = muxer.add_track(audio_spec()).expect("a track");
    for n in 0..10 {
        let mut frame = packet(&[1], n * 40, None);
        frame.sync = n % 4 == 0;
        muxer.write_packet(video, &frame).expect("the packet fits");
        muxer
            .write_packet(audio, &packet(&[2], n * 40, None))
            .expect("the packet fits");
    }
    let bytes = muxer.finish().expect("the file ends").into_inner();
    let layout = walk(&bytes);
    let demuxer = open_mkv(&bytes);
    let times: Vec<u64> = demuxer.cues().iter().map(|point| point.time).collect();
    assert_eq!(times, [0, 160, 320]);
    for point in demuxer.cues() {
        assert_eq!(point.positions.len(), 1);
        let position = point.positions[0];
        assert_eq!(position.track, u64::from(video.get()));
        let block = layout
            .blocks
            .iter()
            .find(|block| {
                layout.clusters.iter().any(|&(offset, timestamp)| {
                    offset == position.cluster_offset && timestamp == block.cluster_timestamp
                }) && Some(block.at) == position.relative_position
            })
            .expect("the cue finds a block");
        assert_eq!(block.track, position.track);
        assert_eq!(
            block.cluster_timestamp as i64 + i64::from(block.relative),
            point.time as i64
        );
        assert_eq!(block.flags & 0x80, 0x80, "a keyframe");
    }
}

#[test]
fn audio_with_no_video_gets_one_cue_per_cluster() {
    let options = MkvOptions {
        cluster_duration_ms: 100,
        ..MkvOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("options");
    let track = muxer.add_track(audio_spec()).expect("a track");
    for n in 0..10 {
        muxer
            .write_packet(track, &packet(&[0], n * 20, None))
            .expect("the packet fits");
    }
    let bytes = muxer.finish().expect("the file ends").into_inner();
    let times: Vec<u64> = open_mkv(&bytes).cues().iter().map(|p| p.time).collect();
    assert_eq!(times, [0, 100]);
}

#[test]
fn cues_off_writes_no_cues() {
    let options = MkvOptions {
        cues: false,
        ..MkvOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("options");
    let track = muxer.add_track(audio_spec()).expect("a track");
    muxer
        .write_packet(track, &packet(&[0], 0, Some(1)))
        .expect("the packet fits");
    let bytes = muxer.finish().expect("the file ends").into_inner();
    assert!(!walk(&bytes).order().contains(&CUES));
}

#[test]
fn a_target_with_no_seek_gets_an_unknown_size_segment() {
    let options = MkvOptions {
        seekable: false,
        doc_type: DocType::WebM,
        ..MkvOptions::default()
    };
    let mut muxer = Muxer::new(NoSeek(Vec::new()), options).expect("options");
    let track = muxer.add_track(audio_spec()).expect("a track");
    for n in 0..3 {
        muxer
            .write_packet(track, &packet(&[n as u8], n * 20, None))
            .expect("the packet fits");
    }
    let bytes = muxer.finish().expect("the file ends").0;
    let layout = walk(&bytes);
    assert!(!layout.segment_size_known);
    assert_eq!(layout.doc_type, "webm");
    assert_eq!(layout.order(), [INFO, TRACKS, CLUSTER, CUES]);
    let demuxer = open_mkv(&bytes);
    assert_eq!(demuxer.info().duration, None);
    assert!(!demuxer.truncated());
    assert_eq!(demuxer.tracks()[0].sample_count, 3);
}

#[test]
fn output_bytes_repeat() {
    let synth = mkv_synth::file(false);
    assert_eq!(
        copy_mkv("synthetic", &synth.bytes, |_| {}),
        copy_mkv("synthetic", &synth.bytes, |_| {})
    );
}

#[test]
fn a_full_buffer_writes_early_blocks_with_their_durations_stated() {
    let options = MkvOptions {
        max_buffer: containr::mkv::MIN_CLUSTER_BYTES,
        cluster_bytes: containr::mkv::MIN_CLUSTER_BYTES,
        ..MkvOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("options");
    let quiet = muxer.add_track(audio_spec()).expect("a track");
    let busy = muxer.add_track(audio_spec()).expect("a track");
    let data = vec![0u8; 700];
    for n in 0..3 {
        muxer
            .write_packet(busy, &packet(&data, n * 20, None))
            .expect("the packet fits");
    }
    muxer
        .write_packet(quiet, &packet(&data, 40, None))
        .expect("a late packet fits");
    let bytes = muxer.finish().expect("the file ends").into_inner();
    let layout = walk(&bytes);
    let last = layout
        .last_block(u64::from(busy.get()))
        .expect("the busy track has blocks");
    assert_eq!(last.duration, Some(20));
    let mut demuxer = open_mkv(&bytes);
    let count = demuxer
        .track_packets(busy.get())
        .expect("the track exists")
        .count();
    assert_eq!(count, 3);
    let packets: Vec<_> = demuxer
        .packets()
        .map(|packet| packet.expect("a packet"))
        .collect();
    for packet in &packets {
        assert_eq!(demuxer.read_packet(packet).expect("the bytes read"), data);
    }
}
