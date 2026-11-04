//! Each refusal: a stream or packet MP4 cannot hold gives a named error.

use containr::matroska::{MatroskaTrack, MatroskaVideo};
use containr::mp4::{IDENTITY_MATRIX, StreamCopy, TrackSpec};
use containr::{
    Container, EditList, Error, Language, Packet, SampleEntry, Timebase, Track, TrackKind,
};

fn entry(kind: &[u8; 4]) -> SampleEntry {
    let mut raw = vec![0, 0, 0, 16];
    raw.extend_from_slice(kind);
    raw.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
    SampleEntry {
        fourcc: containr::FourCc(*kind),
        raw,
        data_reference_index: 1,
        codec_config: None,
        video: None,
        audio: None,
    }
}

fn mp4_track(entries: Vec<SampleEntry>) -> Track {
    Track {
        id: 1,
        kind: TrackKind::Video,
        handler: Some(containr::FourCc(*b"vide")),
        timebase: Timebase::from_timescale(1000),
        duration: Some(0),
        language: Language::Iso639(*b"und"),
        matrix: IDENTITY_MATRIX,
        display_width: 0,
        display_height: 0,
        sample_entries: entries,
        edit_list: EditList::default(),
        references: Vec::new(),
        chapter_track: false,
        metadata: Vec::new(),
        sample_count: 0,
        matroska: None,
    }
}

fn mkv_track(codec_id: &str, kind: TrackKind) -> Track {
    let matroska = MatroskaTrack {
        uid: None,
        track_type: 1,
        codec_id: codec_id.to_owned(),
        codec_private: None,
        codec_name: None,
        name: None,
        language: "eng".to_owned(),
        language_bcp47: None,
        default_duration: None,
        codec_delay: 0,
        seek_pre_roll: 0,
        enabled: true,
        default: true,
        forced: false,
        lacing: false,
        content_encodings: Vec::new(),
        video: None,
        audio: None,
        extra: Vec::new(),
    };
    Track {
        kind,
        handler: None,
        timebase: Timebase::from_nanoseconds(1_000_000),
        matroska: Some(matroska),
        ..mp4_track(Vec::new())
    }
}

fn assert_names_matroska(err: &Error, what: &str) {
    assert!(
        matches!(
            err,
            Error::Unsupported {
                container: Container::Mp4,
                use_instead: Container::Matroska,
                ..
            }
        ),
        "{what}: {err}"
    );
    let text = err.to_string();
    assert!(text.contains("Matroska/WebM"), "{what}: {text}");
    assert!(text.contains(what), "{what}: {text}");
}

fn refusal(track: &Track) -> Error {
    match TrackSpec::copy_from(track) {
        Err(err) => err,
        Ok(spec) => panic!("{spec:?} is not refused"),
    }
}

#[test]
fn matroska_codecs_mp4_cannot_hold_name_matroska() {
    for codec in [
        "V_VP9",
        "V_VP8",
        "A_VORBIS",
        "A_FLAC",
        "S_TEXT/ASS",
        "S_TEXT/SSA",
        "S_TEXT/WEBVTT",
        "S_HDMV/PGS",
        "S_VOBSUB",
    ] {
        let err = refusal(&mkv_track(codec, TrackKind::Other));
        assert_names_matroska(&err, codec);
    }
}

#[test]
fn subtitle_refusals_say_what_kind_they_are() {
    let ass = refusal(&mkv_track("S_TEXT/ASS", TrackKind::Subtitle));
    assert!(ass.to_string().contains("ASS/SSA"), "{ass}");
    let pgs = refusal(&mkv_track("S_HDMV/PGS", TrackKind::Subtitle));
    assert!(pgs.to_string().contains("image subtitles"), "{pgs}");
}

#[test]
fn a_mapped_codec_with_no_codec_private_names_matroska() {
    let err = refusal(&mkv_track("A_AAC", TrackKind::Audio));
    assert_names_matroska(&err, "A_AAC");
}

#[test]
fn matroska_crop_and_content_encodings_name_matroska() {
    let mut track = mkv_track("V_MPEG4/ISO/AVC", TrackKind::Video);
    let mkv = track.matroska.as_mut().expect("a Matroska track");
    mkv.codec_private = Some(vec![1]);
    mkv.video = Some(MatroskaVideo {
        pixel_width: 96,
        pixel_height: 64,
        display_width: None,
        display_height: None,
        display_unit: containr::matroska::DisplayUnit::Pixels,
        crop: [0, 2, 0, 0],
        interlaced: 0,
        projection: None,
        extra: Vec::new(),
    });
    assert_names_matroska(&refusal(&track), "PixelCrop");
    let mkv = track.matroska.as_mut().expect("a Matroska track");
    mkv.content_encodings
        .push(containr::matroska::ContentEncoding {
            order: 0,
            scope: 1,
            encoding_type: 0,
            compression_algorithm: Some(3),
            compression_settings: None,
            encryption_algorithm: None,
            encryption_settings: None,
        });
    assert_names_matroska(&refusal(&track), "ContentEncodings");
}

#[test]
fn two_sample_descriptions_are_refused() {
    let track = mp4_track(vec![entry(b"avc1"), entry(b"avc1")]);
    let err = refusal(&track);
    assert!(
        matches!(
            err,
            Error::SampleDescriptionChange {
                track_id: 1,
                descriptions: 2
            }
        ),
        "{err}"
    );
}

fn packet(index: u64, description_index: u32) -> Packet {
    Packet {
        track_id: 1,
        index,
        dts: Some(index as i64),
        pts: index as i64,
        duration: Some(1),
        sync: true,
        discardable: false,
        discard_padding: None,
        lace: None,
        offset: 0,
        size: 1,
        description_index,
    }
}

#[test]
fn a_packet_on_a_second_description_is_refused() {
    let track = mp4_track(vec![entry(b"avc1")]);
    let err = StreamCopy::plan(&track, &[packet(0, 1), packet(1, 2)], 1000);
    assert!(
        matches!(err, Err(Error::SampleDescriptionChange { .. })),
        "{err:?}"
    );
}

#[test]
fn mid_stream_discard_padding_names_matroska() {
    let track = mkv_track("S_TEXT/UTF8", TrackKind::Subtitle);
    let mut first = packet(0, 1);
    first.dts = None;
    first.discard_padding = Some(5);
    let mut second = packet(1, 1);
    second.dts = None;
    let err = StreamCopy::plan(&track, &[first, second], 1000).expect_err("the plan is refused");
    assert_names_matroska(&err, "DiscardPadding");
}
