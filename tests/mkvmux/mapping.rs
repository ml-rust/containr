//! MP4 tracks as Matroska specs, built by hand, so each mapping and refusal is
//! known exactly.

use containr::mkv::{StreamCopy, TrackSpec};
use containr::mp4::IDENTITY_MATRIX;
use containr::{
    AudioInfo, CodecConfig, Container, EditEntry, EditList, Error, FourCc, Language, Packet,
    PixelAspect, SampleEntry, Timebase, Track, TrackKind, VideoInfo,
};

pub(super) fn track(kind: TrackKind, timescale: u32, entries: Vec<SampleEntry>) -> Track {
    Track {
        id: 3,
        kind,
        handler: None,
        timebase: Timebase::from_timescale(timescale),
        duration: None,
        language: Language::Iso639(*b"fre"),
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

pub(super) fn entry(fourcc: &[u8; 4], config: Option<CodecConfig>) -> SampleEntry {
    SampleEntry {
        fourcc: FourCc(*fourcc),
        raw: Vec::new(),
        data_reference_index: 1,
        codec_config: config,
        video: Some(VideoInfo {
            width: 96,
            height: 64,
            pixel_aspect: None,
            colour: None,
        }),
        audio: Some(AudioInfo {
            channels: 2,
            sample_size: 16,
            sample_rate: 48_000,
        }),
    }
}

/// An `esds` body with `config` as its AudioSpecificConfig.
fn esds(object_type: u8, config: &[u8]) -> Vec<u8> {
    let mut decoder = vec![object_type];
    decoder.extend_from_slice(&[0; 12]);
    decoder.extend_from_slice(&[0x05, config.len() as u8]);
    decoder.extend_from_slice(config);
    let mut es = vec![0, 1, 0, 0x04, decoder.len() as u8];
    es.extend(decoder);
    es.extend_from_slice(&[0x06, 1, 2]);
    let mut body = vec![0, 0, 0, 0, 0x03, es.len() as u8];
    body.extend(es);
    body
}

fn mapped(track: &Track) -> TrackSpec {
    TrackSpec::copy_from(track).unwrap_or_else(|err| panic!("{err}"))
}

pub(super) fn assert_names_mp4(err: &Error, needle: &str) {
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
    assert!(err.to_string().contains(needle), "{err}");
    assert!(err.to_string().contains("MP4/MOV"), "{err}");
}

pub(super) fn packet(index: u64, pts: i64, duration: u64, size: u32) -> Packet {
    Packet {
        track_id: 3,
        index,
        dts: Some(pts),
        pts,
        duration: Some(duration),
        sync: true,
        discardable: false,
        discard_padding: None,
        lace: None,
        offset: 0,
        size,
        description_index: 1,
    }
}

#[test]
fn video_entries_map_with_their_config_as_codec_private() {
    for (fourcc, config, codec_id) in [
        (b"avc1", CodecConfig::Avc(vec![1, 2]), "V_MPEG4/ISO/AVC"),
        (b"avc3", CodecConfig::Avc(vec![1, 2]), "V_MPEG4/ISO/AVC"),
        (b"hvc1", CodecConfig::Hevc(vec![1, 2]), "V_MPEGH/ISO/HEVC"),
        (b"hev1", CodecConfig::Hevc(vec![1, 2]), "V_MPEGH/ISO/HEVC"),
        (b"av01", CodecConfig::Av1(vec![1, 2]), "V_AV1"),
    ] {
        let spec = mapped(&track(
            TrackKind::Video,
            12_800,
            vec![entry(fourcc, Some(config))],
        ));
        assert_eq!(spec.matroska.codec_id, codec_id);
        assert_eq!(spec.matroska.codec_private, Some(vec![1, 2]));
        assert_eq!(spec.matroska.track_type, 1);
        assert_eq!(spec.matroska.language, "fre");
        let video = spec.matroska.video.as_ref().expect("a Video element");
        assert_eq!((video.pixel_width, video.pixel_height), (96, 64));
    }
}

#[test]
fn vp09_maps_with_no_codec_private() {
    let config = CodecConfig::Vp(vec![1, 0, 0, 0, 0, 0, 0x80]);
    let spec = mapped(&track(
        TrackKind::Video,
        12_800,
        vec![entry(b"vp09", Some(config))],
    ));
    assert_eq!(spec.matroska.codec_id, "V_VP9");
    assert_eq!(spec.matroska.codec_private, None);
}

#[test]
fn colr_colour_maps_to_the_video_colour() {
    let colour = containr::ContainerColour {
        primaries: Some(9),
        transfer: Some(16),
        matrix: Some(9),
        full_range: Some(false),
    };
    let mut tagged = entry(b"vp09", None);
    tagged.video = Some(VideoInfo {
        width: 96,
        height: 64,
        pixel_aspect: None,
        colour: Some(colour),
    });
    let spec = mapped(&track(TrackKind::Video, 12_800, vec![tagged]));
    let video = spec.matroska.video.as_ref().expect("a Video element");
    assert_eq!(video.colour, Some(colour));
    let untagged = mapped(&track(TrackKind::Video, 12_800, vec![entry(b"vp09", None)]));
    let video = untagged.matroska.video.as_ref().expect("a Video element");
    assert_eq!(video.colour, None);
}

#[test]
fn pixel_aspect_and_rotation_map() {
    let mut with_pasp = entry(b"vp09", None);
    with_pasp.video = Some(VideoInfo {
        width: 96,
        height: 64,
        pixel_aspect: Some(PixelAspect {
            h_spacing: 4,
            v_spacing: 3,
        }),
        colour: None,
    });
    let mut source = track(TrackKind::Video, 12_800, vec![with_pasp]);
    source.matrix = [0, -0x10000, 0, 0x10000, 0, 0, 0, 0, 0x4000_0000];
    let spec = mapped(&source);
    let video = spec.matroska.video.as_ref().expect("a Video element");
    assert_eq!(video.display_width, Some(128));
    assert_eq!(video.display_height, Some(64));
    let projection = video.projection.as_ref().expect("a Projection element");
    assert_eq!(projection.pose_roll, 90.0);
    source.matrix = [0x8000, 0, 0, 0, 0x8000, 0, 0, 0, 0x4000_0000];
    assert_names_mp4(
        &TrackSpec::copy_from(&source).expect_err("a scale has no form"),
        "tkhd",
    );
}

#[test]
fn mp4a_maps_with_the_audio_specific_config() {
    let config = CodecConfig::Esds(esds(0x40, &[0x11, 0x90]));
    let spec = mapped(&track(
        TrackKind::Audio,
        48_000,
        vec![entry(b"mp4a", Some(config))],
    ));
    assert_eq!(spec.matroska.codec_id, "A_AAC");
    assert_eq!(spec.matroska.codec_private, Some(vec![0x11, 0x90]));
    assert_eq!(spec.matroska.track_type, 2);
    let audio = spec.matroska.audio.as_ref().expect("an Audio element");
    assert_eq!(audio.channels, 2);
    assert_eq!(audio.sampling_frequency, 48_000.0);
}

#[test]
fn mp4a_with_another_object_type_is_refused() {
    let config = CodecConfig::Esds(esds(0x6B, &[1]));
    let source = track(TrackKind::Audio, 48_000, vec![entry(b"mp4a", Some(config))]);
    assert_names_mp4(
        &TrackSpec::copy_from(&source).expect_err("MP3 in mp4a"),
        "0x6b",
    );
}

#[test]
fn opus_maps_with_an_opus_head_built_from_dops() {
    let dops = vec![0, 2, 0x01, 0x38, 0, 0, 0xBB, 0x80, 0, 0, 0];
    let spec = mapped(&track(
        TrackKind::Audio,
        48_000,
        vec![entry(b"Opus", Some(CodecConfig::Opus(dops)))],
    ));
    assert_eq!(spec.matroska.codec_id, "A_OPUS");
    let mut head = b"OpusHead".to_vec();
    head.extend_from_slice(&[1, 2, 0x38, 0x01, 0x80, 0xBB, 0, 0, 0, 0, 0]);
    assert_eq!(spec.matroska.codec_private, Some(head));
    assert_eq!(spec.matroska.seek_pre_roll, 80_000_000);
}

#[test]
fn tx3g_maps_to_subrip() {
    let spec = mapped(&track(
        TrackKind::Subtitle,
        1000,
        vec![entry(b"tx3g", None)],
    ));
    assert_eq!(spec.matroska.codec_id, "S_TEXT/UTF8");
    assert_eq!(spec.matroska.track_type, 17);
    assert_eq!(spec.matroska.codec_private, None);
}

#[test]
fn entries_that_do_not_map_are_refused_by_name() {
    let source = track(TrackKind::Video, 12_800, vec![entry(b"mp4v", None)]);
    assert_names_mp4(&TrackSpec::copy_from(&source).expect_err("mp4v"), "mp4v");
    let no_config = track(TrackKind::Video, 12_800, vec![entry(b"avc1", None)]);
    let err = TrackSpec::copy_from(&no_config).expect_err("no avcC");
    assert!(err.to_string().contains("avcC"), "{err}");
    let two = track(
        TrackKind::Video,
        12_800,
        vec![entry(b"vp09", None), entry(b"vp09", None)],
    );
    assert!(matches!(
        TrackSpec::copy_from(&two),
        Err(Error::SampleDescriptionChange { .. })
    ));
}

#[test]
fn aac_at_48_khz_maps_but_its_frames_are_refused_by_name() {
    let config = CodecConfig::Esds(esds(0x40, &[0x11, 0x90]));
    let source = track(TrackKind::Audio, 48_000, vec![entry(b"mp4a", Some(config))]);
    let packets = [packet(0, 0, 1024, 8), packet(1, 1024, 1024, 8)];
    let err = StreamCopy::plan(&source, &packets).expect_err("1024/48000 s");
    assert_names_mp4(&err, "nanoseconds");
    assert!(err.to_string().contains("1/48000"), "{err}");
    assert!(err.to_string().contains("plan_rounded"), "{err}");
}

#[test]
fn aac_at_16_khz_has_exact_timestamps() {
    let config = CodecConfig::Esds(esds(0x40, &[0x14, 0x08]));
    let source = track(TrackKind::Audio, 16_000, vec![entry(b"mp4a", Some(config))]);
    let packets = [
        packet(0, 0, 1024, 8),
        packet(1, 1024, 1024, 8),
        packet(2, 2048, 512, 8),
    ];
    let plan = StreamCopy::plan(&source, &packets).expect("64 ms is exact");
    assert_eq!(plan.spec.matroska.default_duration, Some(64_000_000));
    assert_eq!(
        StreamCopy::timestamp_scale(std::slice::from_ref(&plan)),
        1_000_000
    );
}

#[test]
fn the_scale_divides_every_plan() {
    let video = track(TrackKind::Video, 12_800, vec![entry(b"vp09", None)]);
    let video = StreamCopy::plan(&video, &[packet(0, 0, 512, 1)]).expect("40 ms is exact");
    let opus = track(TrackKind::Video, 48_000, vec![entry(b"vp09", None)]);
    let opus = StreamCopy::plan(&opus, &[packet(0, 0, 312, 1)]).expect("6.5 ms is exact");
    assert_eq!(StreamCopy::timestamp_scale(&[video, opus]), 500_000);
    assert_eq!(StreamCopy::timestamp_scale(&[]), 1_000_000);
}

pub(super) fn edit(segment_duration: u64, media_time: i64) -> EditEntry {
    EditEntry {
        segment_duration,
        media_time,
        media_rate: EditEntry::NORMAL_RATE,
    }
}

#[test]
fn a_video_edit_shifts_and_a_cut_frame_is_refused() {
    let mut source = track(TrackKind::Video, 1000, vec![entry(b"vp09", None)]);
    source.edit_list = EditList {
        movie_timescale: 1000,
        entries: vec![edit(500, -1), edit(1000, 40)],
    };
    let packets = [packet(0, 40, 40, 1), packet(1, 80, 40, 1)];
    let plan = StreamCopy::plan(&source, &packets).expect("the edit maps");
    let pts: Vec<i64> = plan
        .timing()
        .iter()
        .map(|timing| timing.expect("a block").pts)
        .collect();
    assert_eq!(pts, [500, 540]);

    source.edit_list.entries = vec![edit(1000, 80)];
    let err = StreamCopy::plan(&source, &packets).expect_err("the first frame is cut");
    assert_names_mp4(&err, "never negative");
}

#[test]
fn an_audio_edit_that_ends_early_becomes_discard_padding() {
    let mut source = track(TrackKind::Audio, 1000, vec![entry(b"Opus", None)]);
    source.sample_entries[0].codec_config = Some(CodecConfig::Opus(vec![0; 11]));
    source.edit_list = EditList {
        movie_timescale: 1000,
        entries: vec![edit(35, 5)],
    };
    let packets = [packet(0, 0, 20, 1), packet(1, 20, 20, 1)];
    let plan = StreamCopy::plan(&source, &packets).expect("the edit maps");
    assert_eq!(plan.spec.matroska.codec_delay, 5_000_000);
    let last = plan.timing()[1].expect("a block");
    assert_eq!(last.discard_padding, None, "the edit ends at the media end");

    source.edit_list.entries = vec![edit(25, 5)];
    let plan = StreamCopy::plan(&source, &packets).expect("the edit maps");
    let last = plan.timing()[1].expect("a block");
    assert_eq!(last.discard_padding, Some(10_000_000));

    source.edit_list.entries = vec![edit(1, 5)];
    assert_names_mp4(
        &StreamCopy::plan(&source, &packets).expect_err("more than one block"),
        "last packet",
    );
}

#[test]
fn a_video_edit_that_ends_early_is_refused() {
    let mut source = track(TrackKind::Video, 1000, vec![entry(b"vp09", None)]);
    source.edit_list = EditList {
        movie_timescale: 1000,
        entries: vec![edit(20, 0)],
    };
    let packets = [packet(0, 0, 20, 1), packet(1, 20, 20, 1)];
    assert_names_mp4(
        &StreamCopy::plan(&source, &packets).expect_err("a video end trim"),
        "ends before",
    );
}

#[test]
fn tx3g_samples_lose_their_length_and_gaps_write_nothing() {
    let source = track(TrackKind::Subtitle, 1000, vec![entry(b"tx3g", None)]);
    let packets = [packet(0, 0, 200, 2), packet(1, 200, 700, 4)];
    let plan = StreamCopy::plan(&source, &packets).expect("the track maps");
    assert_eq!(plan.spec.matroska.default_duration, None);
    assert_eq!(
        plan.packet(&packets[0], &[0, 0]).expect("a gap"),
        None,
        "an empty sample writes no block"
    );
    let cue = plan
        .packet(&packets[1], &[0, 2, b'h', b'i'])
        .expect("a cue")
        .expect("a block");
    assert_eq!(cue.data, b"hi");
    assert_eq!(cue.pts, 200);
    assert_eq!(cue.duration, Some(700));
    let styled = plan
        .packet(&packets[1], &[0, 1, b'h', 0, 0, 0, 8])
        .expect_err("style boxes");
    assert_names_mp4(&styled, "style");
    assert!(plan.packet(&packets[1], &[0]).is_err());
    let past = packet(9, 0, 1, 1);
    assert!(plan.packet(&past, &[]).is_err());
}

#[test]
fn a_plan_needs_the_tracks_packets_in_order() {
    let source = track(TrackKind::Video, 1000, vec![entry(b"vp09", None)]);
    let err = StreamCopy::plan(&source, &[packet(1, 0, 1, 1)]).expect_err("index 1 first");
    assert!(err.to_string().contains("decode order"), "{err}");
    let mut other = packet(0, 0, 1, 1);
    other.track_id = 4;
    assert!(StreamCopy::plan(&source, &[other]).is_err());
    let mut second = packet(0, 0, 1, 1);
    second.description_index = 2;
    assert!(matches!(
        StreamCopy::plan(&source, &[second]),
        Err(Error::SampleDescriptionChange { .. })
    ));
}
