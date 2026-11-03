//! Per-fixture behaviour: what each FFmpeg fixture exists to cover.

use std::fs::File;
use std::io::BufReader;

use containr::mkv::Demuxer;
use containr::{ChapterSource, MetadataScope, Packet, TagTarget, TrackKind};

use crate::common::{self, Fixture, adler32, array, int, ratio, text};
use crate::mkv::ffprobe::media_streams;

type FileDemuxer = Demuxer<BufReader<File>>;

fn open(name: &str) -> Option<(Fixture, FileDemuxer)> {
    let fx = common::fixture(name)?;
    let demuxer = Demuxer::open_path(&fx.path).unwrap_or_else(|err| panic!("{name}: {err}"));
    Some((fx, demuxer))
}

fn packets(demuxer: &FileDemuxer, track_id: u32) -> Vec<Packet> {
    demuxer
        .track_packets(track_id)
        .expect("the track exists")
        .collect::<Result<_, _>>()
        .unwrap_or_else(|err| panic!("{err}"))
}

fn first_of(demuxer: &FileDemuxer, kind: TrackKind) -> u32 {
    demuxer
        .tracks()
        .iter()
        .find(|track| track.kind == kind)
        .map(|track| track.id)
        .unwrap_or_else(|| panic!("no {kind:?} track"))
}

/// The element ID at `offset` in `bytes`, marker bits included.
fn id_at(bytes: &[u8], offset: u64) -> u32 {
    let at = offset as usize;
    let first = bytes[at];
    let len = first.leading_zeros() as usize + 1;
    bytes[at..at + len]
        .iter()
        .fold(0, |acc, &byte| (acc << 8) | u32::from(byte))
}

#[test]
fn h264_b_frames_keep_raw_pts_and_no_dts() {
    let Some((fx, demuxer)) = open("h264_aac.mkv") else {
        return;
    };
    assert_eq!(demuxer.ebml_header().doc_type, "matroska");
    assert_eq!(demuxer.info().timestamp_scale, 1_000_000);
    let muxer = demuxer.info().muxing_app.as_deref().unwrap_or_default();
    assert!(muxer.starts_with("Lavf"), "MuxingApp {muxer:?}");
    let video = packets(&demuxer, first_of(&demuxer, TrackKind::Video));
    assert!(video.iter().all(|p| p.dts.is_none()));
    assert!(
        video.windows(2).any(|pair| pair[1].pts < pair[0].pts),
        "B-frames present out of storage order"
    );
    let audio = first_of(&demuxer, TrackKind::Audio);
    let mkv = demuxer
        .track(audio)
        .and_then(|track| track.matroska.as_ref())
        .expect("Matroska facts");
    assert!(
        mkv.codec_delay > 0,
        "FFmpeg writes the AAC priming as CodecDelay"
    );
    // The Segment Duration is the float FFmpeg writes, 2.021 s of audio.
    let seconds = fx.probe["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .expect("format duration");
    let duration = demuxer.info().duration.expect("FFmpeg writes Duration");
    assert!(
        (duration / 1000.0 - seconds).abs() < 0.001,
        "{duration} vs {seconds}"
    );
}

#[test]
fn seek_head_and_cues_point_at_their_elements() {
    let Some((fx, demuxer)) = open("h264_aac.mkv") else {
        return;
    };
    assert!(!demuxer.seek_head().is_empty());
    for entry in demuxer.seek_head() {
        assert_eq!(id_at(&fx.bytes, entry.offset), entry.id, "{entry:?}");
    }
    assert!(!demuxer.cues().is_empty(), "FFmpeg writes Cues");
    for point in demuxer.cues() {
        for position in &point.positions {
            assert_eq!(
                id_at(&fx.bytes, position.cluster_offset),
                0x1F43_B675,
                "{point:?}"
            );
        }
    }
    assert!(demuxer.segment_data_offset() > 0);
}

#[test]
fn webm_opus_carries_codec_delay_and_seek_pre_roll() {
    let Some((_, demuxer)) = open("vp9_opus.webm") else {
        return;
    };
    assert_eq!(demuxer.ebml_header().doc_type, "webm");
    let audio = demuxer
        .track(first_of(&demuxer, TrackKind::Audio))
        .expect("audio track");
    let mkv = audio.matroska.as_ref().expect("Matroska facts");
    assert_eq!(audio.codec_id(), Some("A_OPUS"));
    assert_eq!(mkv.codec_delay, 6_500_000, "libopus: 312 samples at 48 kHz");
    assert_eq!(mkv.seek_pre_roll, 80_000_000);
    assert_eq!(mkv.default_duration, Some(20_000_000));
}

#[test]
fn av1_webm_has_its_configuration_record() {
    let Some((_, demuxer)) = open("av1.webm") else {
        return;
    };
    let video = demuxer
        .track(first_of(&demuxer, TrackKind::Video))
        .expect("video track");
    let mkv = video.matroska.as_ref().expect("Matroska facts");
    assert_eq!(mkv.codec_id, "V_AV1");
    // av1C starts with marker 1 and version 1: 0x81.
    assert_eq!(
        mkv.codec_private.as_deref().and_then(|p| p.first()),
        Some(&0x81)
    );
}

#[test]
fn srt_and_ass_are_subtitle_tracks_with_block_durations() {
    let Some((_, demuxer)) = open("subtitles.mkv") else {
        return;
    };
    let subtitles: Vec<_> = demuxer
        .tracks()
        .iter()
        .filter(|track| track.kind == TrackKind::Subtitle)
        .collect();
    let ids: Vec<_> = subtitles.iter().map(|track| track.codec_id()).collect();
    assert_eq!(ids, vec![Some("S_TEXT/UTF8"), Some("S_TEXT/ASS")]);
    let languages: Vec<_> = subtitles.iter().map(|t| t.language.iso639()).collect();
    assert_eq!(languages, vec![Some("eng"), Some("fre")]);
    let ass = subtitles[1].matroska.as_ref().expect("Matroska facts");
    let header = ass.codec_private.as_deref().unwrap_or_default();
    assert!(
        header.starts_with(b"[Script Info]"),
        "the ASS header is CodecPrivate"
    );
    for track in subtitles {
        for packet in packets(&demuxer, track.id) {
            assert_eq!(packet.duration, Some(700), "SRT cues last 0.7 s");
            assert!(packet.sync);
        }
    }
}

#[test]
fn chapters_match_ffprobe_in_nanoseconds() {
    let Some((fx, demuxer)) = open("chapters.mkv") else {
        return;
    };
    let theirs = array(&fx.probe, "chapters");
    let ours = demuxer.chapters();
    assert_eq!(ours.len(), theirs.len(), "chapter count");
    assert_eq!(demuxer.info().title.as_deref(), Some("containr fixture"));
    for (our, their) in ours.iter().zip(theirs) {
        let (num, den) = ratio(text(their, "time_base"), '/');
        assert_eq!((num, den), (1, u64::from(our.timescale)), "{our:?}");
        assert_eq!(our.start as i64, int(their, "start"), "{our:?}");
        assert_eq!(
            our.duration.map(|d| our.start + d).map(|end| end as i64),
            Some(int(their, "end")),
            "{our:?}"
        );
        assert_eq!(our.title, text(&their["tags"], "title"));
        assert!(matches!(
            our.source,
            ChapterSource::Matroska { depth: 0, .. }
        ));
    }
}

#[test]
fn attachment_is_located_and_read() {
    let Some((fx, mut demuxer)) = open("attachment.mkv") else {
        return;
    };
    let stream = array(&fx.probe, "streams")
        .iter()
        .find(|stream| text(stream, "codec_type") == "attachment")
        .expect("ffprobe lists the attachment");
    assert_eq!(media_streams(&fx).len(), demuxer.tracks().len());
    let attachment = demuxer
        .attachments()
        .first()
        .cloned()
        .expect("one attachment");
    assert_eq!(attachment.name, text(&stream["tags"], "filename"));
    assert_eq!(attachment.mime, text(&stream["tags"], "mimetype"));
    assert_eq!(attachment.size as i64, int(stream, "extradata_size"));
    assert_ne!(attachment.uid, 0);
    let bytes = demuxer
        .read_attachment(&attachment)
        .unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(adler32(&bytes), text(stream, "extradata_hash"));
    assert_eq!(
        &bytes[..4],
        &[0, 1, 0, 0],
        "a TrueType font starts with 0x00010000"
    );
}

#[test]
fn display_rotation_is_projection_pose_roll() {
    let Some((fx, demuxer)) = open("rotate90.mkv") else {
        return;
    };
    let stream = media_streams(&fx)
        .into_iter()
        .find(|stream| text(stream, "codec_type") == "video")
        .expect("video stream");
    let rotation = array(stream, "side_data_list")
        .iter()
        .find(|side| side["side_data_type"] == "Display Matrix")
        .map(|side| int(side, "rotation"))
        .expect("ffprobe reports the rotation");
    let video = demuxer
        .track(first_of(&demuxer, TrackKind::Video))
        .expect("video track");
    let projection = video
        .matroska
        .as_ref()
        .and_then(|mkv| mkv.video.as_ref())
        .and_then(|video| video.projection.as_ref())
        .expect("FFmpeg writes the rotation into Projection");
    // FFmpeg writes `-display_rotation 90` as ProjectionPoseRoll 90.0 and no
    // ProjectionType, which defaults to 0 (rectangular).
    assert_eq!(projection.pose_roll, rotation as f64);
    assert_eq!(projection.pose_roll, 90.0);
    assert_eq!(projection.projection_type, 0);
    assert_eq!(
        video.matrix,
        [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x4000_0000]
    );
}

#[test]
fn opus_end_trim_is_discard_padding_on_the_last_block() {
    let Some((_, demuxer)) = open("opus_trim.webm") else {
        return;
    };
    let audio = packets(&demuxer, first_of(&demuxer, TrackKind::Audio));
    let (last, rest) = audio.split_last().expect("audio packets");
    // FFmpeg trims 0.5 ms from the last frame to end the track at 1.013 s.
    assert_eq!(last.discard_padding, Some(500_000));
    assert!(rest.iter().all(|p| p.discard_padding.is_none()));
}

#[test]
fn live_webm_has_unknown_size_segment_and_clusters() {
    let Some((fx, demuxer)) = open("live.webm") else {
        return;
    };
    let cluster = [0x1F, 0x43, 0xB6, 0x75];
    let starts: Vec<usize> = fx
        .bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == cluster)
        .map(|(at, _)| at)
        .collect();
    assert!(starts.len() > 1, "the fixture has several Clusters");
    for at in starts {
        // The unknown-size marker: every bit after the length marker is 1.
        let first = fx.bytes[at + 4];
        let len = first.leading_zeros() as usize + 1;
        let mask = (0xFFu16 >> len) as u8;
        let unknown =
            first & mask == mask && fx.bytes[at + 5..at + 4 + len].iter().all(|&b| b == 0xFF);
        assert!(unknown, "the Cluster at {at} has a known size");
    }
    assert_eq!(demuxer.info().duration, None, "a live WebM has no Duration");
    assert!(demuxer.cues().is_empty(), "a live WebM has no Cues");
    let tracks: Vec<_> = demuxer
        .metadata()
        .iter()
        .filter(|item| matches!(item.scope, MetadataScope::Tag(scope) if scope.target != TagTarget::Segment))
        .collect();
    assert!(tracks.is_empty(), "every track tag moves into its track");
}
