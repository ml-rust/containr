//! Per-fixture behaviour: what each FFmpeg fixture exists to cover.

use containr::mp4::Demuxer;
use containr::{
    ChapterSource, CodecConfig, Error, FourCc, MetadataKey, MetadataScope, Packet, TrackKind,
};

use crate::common::{self, Fixture, array, int, ratio, text};

const NAM: [u8; 4] = [0xA9, b'n', b'a', b'm'];

fn open(name: &str) -> Option<(Fixture, Demuxer<std::io::BufReader<std::fs::File>>)> {
    let fx = common::fixture(name)?;
    let demuxer = Demuxer::open_path(&fx.path).unwrap_or_else(|err| panic!("{err}"));
    Some((fx, demuxer))
}

fn packets(demuxer: &Demuxer<std::io::BufReader<std::fs::File>>, track_id: u32) -> Vec<Packet> {
    demuxer
        .track_packets(track_id)
        .expect("the track exists")
        .collect::<Result<_, _>>()
        .unwrap_or_else(|err| panic!("{err}"))
}

fn first_of(demuxer: &Demuxer<std::io::BufReader<std::fs::File>>, kind: TrackKind) -> u32 {
    demuxer
        .tracks()
        .iter()
        .find(|track| track.kind == kind)
        .map(|track| track.id)
        .unwrap_or_else(|| panic!("no {kind:?} track"))
}

fn kinds(fx: &Fixture) -> Vec<String> {
    array(&fx.probe, "top_level")
        .iter()
        .map(|kind| kind.as_str().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn moov_after_mdat_and_faststart_hold_the_same_samples() {
    let (Some((end_fx, end)), Some((fast_fx, fast))) = (
        open("h264_aac_moov_end.mp4"),
        open("h264_aac_faststart.mp4"),
    ) else {
        return;
    };
    let end_kinds = kinds(&end_fx);
    let fast_kinds = kinds(&fast_fx);
    let position =
        |kinds: &[String], kind: &str| kinds.iter().position(|k| k == kind).expect("box present");
    assert!(
        position(&end_kinds, "moov") > position(&end_kinds, "mdat"),
        "{end_kinds:?}"
    );
    assert!(
        position(&fast_kinds, "moov") < position(&fast_kinds, "mdat"),
        "{fast_kinds:?}"
    );
    let timing = |packets: Vec<Packet>| -> Vec<(i64, i64, u32, u32, bool)> {
        packets
            .iter()
            .map(|p| (p.pts, p.dts, p.duration, p.size, p.sync))
            .collect()
    };
    for track in end.tracks() {
        assert_eq!(
            timing(packets(&end, track.id)),
            timing(packets(&fast, track.id)),
            "track {}",
            track.id
        );
    }
}

#[test]
fn h264_b_frames_give_pts_after_dts() {
    let Some((_, demuxer)) = open("h264_aac_faststart.mp4") else {
        return;
    };
    let video = first_of(&demuxer, TrackKind::Video);
    assert!(matches!(
        demuxer.track(video).and_then(|t| t.codec_config()),
        Some(CodecConfig::Avc(_))
    ));
    assert!(packets(&demuxer, video).iter().any(|p| p.pts != p.dts));
    let audio = first_of(&demuxer, TrackKind::Audio);
    assert!(matches!(
        demuxer.track(audio).and_then(|t| t.codec_config()),
        Some(CodecConfig::Esds(_))
    ));
    assert!(
        packets(&demuxer, audio)
            .iter()
            .all(|p| p.sync && p.pts == p.dts)
    );
}

#[test]
fn negative_cts_offsets_give_pts_before_dts() {
    let Some((_, demuxer)) = open("h264_negative_cts.mp4") else {
        return;
    };
    let video = first_of(&demuxer, TrackKind::Video);
    assert!(packets(&demuxer, video).iter().any(|p| p.pts < p.dts));
}

#[test]
fn hevc_and_opus_configs_are_found() {
    let Some((_, demuxer)) = open("hevc_opus.mp4") else {
        return;
    };
    let video = demuxer
        .track(first_of(&demuxer, TrackKind::Video))
        .expect("video track");
    assert_eq!(video.codec(), Some(FourCc(*b"hvc1")));
    assert!(matches!(video.codec_config(), Some(CodecConfig::Hevc(bytes)) if !bytes.is_empty()));
    let pasp = video
        .video()
        .and_then(|v| v.pixel_aspect)
        .expect("setsar writes pasp");
    assert_eq!(pasp.h_spacing * 3, pasp.v_spacing * 4);
    let audio = demuxer
        .track(first_of(&demuxer, TrackKind::Audio))
        .expect("audio track");
    assert_eq!(audio.codec(), Some(FourCc(*b"Opus")));
    assert!(matches!(audio.codec_config(), Some(CodecConfig::Opus(bytes)) if !bytes.is_empty()));
}

#[test]
fn mov_text_is_a_subtitle_track() {
    let Some((_, demuxer)) = open("mov_text.mp4") else {
        return;
    };
    let subtitle = demuxer
        .track(first_of(&demuxer, TrackKind::Subtitle))
        .expect("subtitle track");
    assert_eq!(subtitle.codec(), Some(FourCc(*b"tx3g")));
    assert_eq!(subtitle.language.iso639(), Some("eng"));
    // ffprobe does not report the subtitle `mdhd` duration, so check it
    // against the track's own sample durations.
    let samples: u64 = demuxer
        .track_packets(subtitle.id)
        .expect("subtitle packets")
        .map(|p| u64::from(p.expect("subtitle packet").duration))
        .sum();
    assert_eq!(subtitle.duration, samples, "subtitle mdhd duration");
}

#[test]
fn rotation_90_is_in_the_raw_matrix() {
    let Some((_, demuxer)) = open("rotate90.mp4") else {
        return;
    };
    let video = demuxer
        .track(first_of(&demuxer, TrackKind::Video))
        .expect("video track");
    assert_eq!(
        video.matrix,
        [0, -0x10000, 0, 0x10000, 0, 0, 0, 0, 0x4000_0000]
    );
}

#[test]
fn itsoffset_writes_an_empty_edit() {
    let Some((_, demuxer)) = open("itsoffset.mp4") else {
        return;
    };
    let video = demuxer
        .track(first_of(&demuxer, TrackKind::Video))
        .expect("video track");
    let first = video
        .edit_list
        .entries
        .first()
        .expect("the track has an edit list");
    assert!(first.is_empty());
    // -itsoffset 0.5: half a second of movie time.
    assert_eq!(
        first.segment_duration * 2,
        u64::from(demuxer.movie_timescale())
    );
    assert_eq!(
        packets(&demuxer, video.id)[0].dts,
        0,
        "the demuxer must not apply the edit list"
    );
}

#[test]
fn stream_copy_cut_writes_a_media_time_start() {
    let Some((_, demuxer)) = open("ss_copy.mp4") else {
        return;
    };
    for track in demuxer.tracks() {
        let first = track
            .edit_list
            .entries
            .first()
            .expect("the track has an edit list");
        assert!(
            !first.is_empty() && first.media_time > 0,
            "track {}: {first:?}",
            track.id
        );
    }
}

fn check_chapters(name: &str) {
    let Some((fx, demuxer)) = open(name) else {
        return;
    };
    let theirs = array(&fx.probe, "chapters");
    assert!(!theirs.is_empty(), "{name}: ffprobe lists chapters");
    let chapter_track = demuxer
        .tracks()
        .iter()
        .find(|t| t.chapter_track)
        .expect("a chap reference names a track");
    for source in [ChapterSource::Chpl, ChapterSource::Track(chapter_track.id)] {
        let ours: Vec<_> = demuxer
            .chapters()
            .iter()
            .filter(|c| c.source == source)
            .collect();
        assert_eq!(ours.len(), theirs.len(), "{name} {source:?}: chapter count");
        for (our, their) in ours.iter().zip(theirs) {
            assert_eq!(
                our.title,
                text(&their["tags"], "title"),
                "{name} {source:?}: title"
            );
            let (num, den) = ratio(text(their, "time_base"), '/');
            let start = int(their, "start") as u128;
            assert_eq!(
                u128::from(our.start) * u128::from(den),
                start * u128::from(num) * u128::from(our.timescale),
                "{name} {source:?}: start of {}",
                our.title
            );
        }
    }
}

#[test]
fn chapters_mp4_has_chpl_and_a_chapter_track() {
    check_chapters("chapters_mp4.mp4");
}

#[test]
fn chapters_mov_has_chpl_and_a_chapter_track() {
    check_chapters("chapters_mov.mov");
}

#[test]
fn mp4_title_is_an_ilst_item() {
    let Some((fx, demuxer)) = open("chapters_mp4.mp4") else {
        return;
    };
    let title = demuxer
        .metadata()
        .iter()
        .find(|item| {
            item.scope == MetadataScope::Ilst && item.key == MetadataKey::FourCc(FourCc(NAM))
        })
        .expect("ffmpeg writes the title into ilst");
    assert_eq!(
        title.text(),
        Some(text(&fx.probe["format"]["tags"], "title"))
    );
}

#[test]
fn mov_title_is_a_raw_udta_item() {
    let Some((fx, demuxer)) = open("chapters_mov.mov") else {
        return;
    };
    let title = demuxer
        .metadata()
        .iter()
        .find(|item| {
            item.scope == MetadataScope::Udta && item.key == MetadataKey::FourCc(FourCc(NAM))
        })
        .expect("ffmpeg writes the title into udta");
    // QuickTime text: 16-bit length, 16-bit language, then the text.
    let want = text(&fx.probe["format"]["tags"], "title").as_bytes();
    assert_eq!(title.value.get(4..), Some(want));
    let audio = demuxer
        .track(first_of(&demuxer, TrackKind::Audio))
        .expect("audio track");
    assert!(
        matches!(audio.codec_config(), Some(CodecConfig::Esds(_))),
        "mov keeps esds inside wave"
    );
}

#[test]
fn fragmented_mp4_is_refused_by_name() {
    let Some(fx) = common::fixture("fragmented.mp4") else {
        return;
    };
    match Demuxer::open_path(&fx.path) {
        Err(Error::InFile { source, .. }) => {
            assert!(matches!(*source, Error::Fragmented { .. }), "{source}");
        }
        other => panic!("expected a Fragmented error, got {:?}", other.map(|_| ())),
    }
}
