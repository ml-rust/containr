//! What the muxer refuses, by name.

use std::io::Cursor;

use containr::matroska::{DisplayUnit, MatroskaVideo};
use containr::mkv::{AttachmentIn, DocType, MkvOptions, Muxer, PacketIn, StreamCopy, TrackSpec};
use containr::{
    Chapter, ChapterSource, Container, Error, FourCc, MetadataItem, MetadataKey, MetadataScope,
    TagScope, TagTarget, Timebase, TrackKind,
};

use crate::common;
use crate::mkvmux::support::{collect, ok, open_mkv};

pub(super) type MemoryMuxer = Muxer<Cursor<Vec<u8>>>;

pub(super) const MS: Timebase = Timebase::from_nanoseconds(1_000_000);

pub(super) fn muxer(options: MkvOptions) -> MemoryMuxer {
    Muxer::new(Cursor::new(Vec::new()), options).unwrap_or_else(|err| panic!("{err}"))
}

fn webm() -> MemoryMuxer {
    muxer(MkvOptions {
        doc_type: DocType::WebM,
        ..MkvOptions::default()
    })
}

pub(super) fn audio(codec_id: &str) -> TrackSpec {
    TrackSpec::new(TrackKind::Audio, codec_id, MS)
}

pub(super) fn video(codec_id: &str) -> TrackSpec {
    let mut spec = TrackSpec::new(TrackKind::Video, codec_id, MS);
    spec.matroska.video = Some(MatroskaVideo {
        pixel_width: 16,
        pixel_height: 16,
        display_width: None,
        display_height: None,
        display_unit: DisplayUnit::Pixels,
        crop: [0; 4],
        interlaced: 0,
        projection: None,
        colour: None,
        extra: Vec::new(),
    });
    spec
}

pub(super) fn packet(pts: i64, duration: Option<u64>) -> PacketIn<'static> {
    PacketIn {
        data: &[1, 2],
        pts,
        duration,
        sync: true,
        discardable: false,
        discard_padding: None,
    }
}

fn tag(name: &str, target: TagTarget) -> MetadataItem {
    MetadataItem {
        scope: MetadataScope::Tag(TagScope {
            target_type_value: 50,
            target,
            binary: false,
        }),
        key: MetadataKey::Named(name.as_bytes().to_vec()),
        value: b"v".to_vec(),
        data: Vec::new(),
    }
}

fn assert_names_matroska(err: &Error, needle: &str) {
    assert!(
        matches!(
            err,
            Error::Unsupported {
                container: Container::WebM,
                use_instead: Container::Matroska,
                ..
            }
        ),
        "{err}"
    );
    let text = err.to_string();
    assert!(text.contains(needle), "{text}");
    assert!(text.contains("WebM output cannot hold"), "{text}");
    assert!(text.contains("Mux it into Matroska"), "{text}");
}

#[test]
fn webm_refuses_each_other_codec_and_names_matroska() {
    for codec_id in [
        "V_MPEG4/ISO/AVC",
        "V_MPEGH/ISO/HEVC",
        "A_AAC",
        "A_FLAC",
        "S_TEXT/UTF8",
        "S_TEXT/ASS",
    ] {
        let spec = if codec_id.starts_with('V') {
            video(codec_id)
        } else {
            audio(codec_id)
        };
        let err = webm().add_track(spec).expect_err(codec_id);
        assert_names_matroska(&err, codec_id);
    }
}

#[test]
fn webm_holds_its_own_codecs() {
    for spec in [
        video("V_VP8"),
        video("V_VP9"),
        video("V_AV1"),
        audio("A_OPUS"),
        audio("A_VORBIS"),
        TrackSpec::new(TrackKind::Subtitle, "D_WEBVTT/SUBTITLES", MS),
        TrackSpec::new(TrackKind::Subtitle, "S_TEXT/WEBVTT", MS),
    ] {
        let codec_id = spec.matroska.codec_id.clone();
        webm()
            .add_track(spec)
            .unwrap_or_else(|err| panic!("{codec_id}: {err}"));
    }
}

#[test]
fn webm_refuses_a_fixture_track_that_matroska_holds() {
    let Some(fixture) = common::fixture("h264_aac.mkv") else {
        return;
    };
    let input = open_mkv(&fixture.bytes);
    for track in input.tracks() {
        let packets = collect("h264_aac", ok("h264_aac", input.track_packets(track.id)));
        let plan = ok("h264_aac", StreamCopy::plan(track, &packets));
        let err = webm().add_track(plan.spec).expect_err("a WebM refusal");
        assert_names_matroska(&err, track.codec_id().expect("a CodecID"));
    }
}

#[test]
fn webm_refuses_attachments_and_names_matroska() {
    let err = webm()
        .add_attachment("font.ttf", "font/ttf", b"font")
        .expect_err("an attachment");
    assert_names_matroska(&err, "font.ttf");
}

#[test]
fn matroska_holds_attachments_with_derived_uids() {
    let mut muxer = muxer(MkvOptions::default());
    let first = muxer
        .add_attachment("font.ttf", "font/ttf", b"font")
        .expect("Matroska holds it");
    let second = muxer
        .add_attachment("font.ttf", "font/ttf", b"font")
        .expect("Matroska holds it");
    assert_ne!(first, 0);
    assert_ne!(first, second);
    let file = |uid| AttachmentIn {
        name: "a",
        mime: "b",
        description: None,
        uid: Some(uid),
        data: b"",
    };
    assert!(muxer.attach(&file(first)).is_err(), "a repeated UID");
    assert!(muxer.attach(&file(0)).is_err(), "UID 0");
    let long = "n".repeat(70_000);
    assert!(muxer.add_attachment(&long, "b", b"").is_err());
}

#[test]
fn a_negative_timestamp_is_refused_and_names_mp4() {
    let mut muxer = muxer(MkvOptions::default());
    let track = muxer.add_track(audio("A_OPUS")).expect("a track");
    let err = muxer
        .write_packet(track, &packet(-1, Some(1)))
        .expect_err("a negative pts");
    assert!(matches!(
        err,
        Error::Unsupported {
            use_instead: Container::Mp4,
            ..
        }
    ));
    assert!(err.to_string().contains("never negative"), "{err}");
}

#[test]
fn a_value_no_scale_holds_is_refused_and_names_mp4() {
    let mut muxer = muxer(MkvOptions::default());
    let spec = TrackSpec::new(TrackKind::Audio, "A_AAC", Timebase::from_timescale(48_000));
    let track = muxer.add_track(spec).expect("a track");
    let err = muxer
        .write_packet(track, &packet(1024, Some(1024)))
        .expect_err("1024/48000 s");
    assert!(
        matches!(
            err,
            Error::Unsupported {
                container: Container::Matroska,
                use_instead: Container::Mp4,
                ..
            }
        ),
        "{err}"
    );
    assert!(err.to_string().contains("48000"), "{err}");
    muxer
        .write_packet(track, &packet(960, Some(960)))
        .expect("a refused packet leaves the muxer usable");
}

#[test]
fn a_value_a_finer_scale_holds_names_that_scale() {
    let mut muxer = muxer(MkvOptions {
        timestamp_scale: 2_000_000,
        ..MkvOptions::default()
    });
    let track = muxer.add_track(audio("A_OPUS")).expect("a track");
    let err = muxer
        .write_packet(track, &packet(21, Some(2)))
        .expect_err("21 ms at 2 ms per tick");
    assert!(
        matches!(
            err,
            Error::InvalidPacket {
                container: Container::Matroska,
                ..
            }
        ),
        "{err}"
    );
    assert!(err.to_string().contains("1000000"), "{err}");
    let err = muxer
        .write_packet(track, &packet(20, Some(3)))
        .expect_err("a 3 ms duration at 2 ms per tick");
    assert!(err.to_string().contains("duration"), "{err}");
}

#[test]
fn a_last_packet_with_no_duration_is_refused() {
    let mut no_default = muxer(MkvOptions::default());
    let track = no_default.add_track(audio("A_OPUS")).expect("a track");
    no_default
        .write_packet(track, &packet(0, None))
        .expect("the middle of a track needs no duration");
    no_default
        .write_packet(track, &packet(20, None))
        .expect("the middle of a track needs no duration");
    assert!(matches!(
        no_default.finish(),
        Err(Error::MissingDuration {
            track_id: 1,
            index: 1
        })
    ));
}

#[test]
fn metadata_must_be_named_tags_that_name_held_uids() {
    let mut mux = muxer(MkvOptions::default());
    let mp4_item = MetadataItem {
        scope: MetadataScope::Ilst,
        key: MetadataKey::FourCc(FourCc(*b"\xA9too")),
        value: Vec::new(),
        data: Vec::new(),
    };
    let err = mux.set_metadata(vec![mp4_item]).expect_err("an ilst item");
    assert!(err.to_string().contains("tags_from_mp4"), "{err}");
    mux.set_metadata(vec![tag("A", TagTarget::Track(99))])
        .expect("the target is checked at the finish");
    let err = mux.finish().expect_err("no such track");
    assert!(err.to_string().contains("Track(99)"), "{err}");

    let mut spec = audio("A_OPUS");
    spec.metadata = vec![tag("A", TagTarget::Chapter(1))];
    let err = muxer(MkvOptions::default())
        .add_track(spec)
        .expect_err("a chapter tag on a track");
    assert!(err.to_string().contains("set_metadata"), "{err}");
}

#[test]
fn a_repeated_chapter_uid_is_refused_by_name() {
    let chapter = |start, uid| Chapter {
        start,
        duration: None,
        timescale: 1000,
        title: "c".to_owned(),
        source: ChapterSource::Matroska {
            edition_uid: None,
            chapter_uid: uid,
            depth: 0,
        },
    };
    let err = muxer(MkvOptions::default())
        .set_chapters(vec![chapter(0, 9), chapter(5, 8), chapter(10, 9)])
        .expect_err("UID 9 repeats");
    let text = err.to_string();
    assert!(matches!(err, Error::InvalidMux { .. }), "{text}");
    assert!(text.contains("`ChapterUID` 9"), "{text}");
    assert!(text.contains("chapter 2"), "{text}");
    assert!(text.contains("chapter 0"), "{text}");
}

#[test]
fn chapters_with_no_exact_time_are_refused() {
    let chapter = Chapter {
        start: 1,
        duration: None,
        timescale: 48_000,
        title: "c".to_owned(),
        source: ChapterSource::Chpl,
    };
    assert!(
        muxer(MkvOptions::default())
            .set_chapters(vec![chapter])
            .is_err()
    );
}
