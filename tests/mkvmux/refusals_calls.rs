//! Refused calls: the call order, track specs and options, each by name.

use std::io::Cursor;

use containr::mkv::{MkvOptions, Muxer, NoSeek, TrackSpec};
use containr::{Error, Timebase, TrackKind};

use crate::mkvmux::refusals::{MS, audio, muxer, packet};

#[test]
fn the_call_order_and_track_ids_are_checked() {
    let mut first = muxer(MkvOptions::default());
    let track = first.add_track(audio("A_OPUS")).expect("a track");
    first
        .write_packet(track, &packet(0, Some(1)))
        .expect("a packet");
    let err = first
        .add_track(audio("A_OPUS"))
        .expect_err("a track after a packet");
    assert!(err.to_string().contains("add_track"), "{err}");
    let mut other = muxer(MkvOptions::default());
    let second = other.add_track(audio("A_OPUS")).expect("a track");
    let extra = other.add_track(audio("A_OPUS")).expect("a track");
    assert_eq!((second.get(), extra.get()), (1, 2));
    let err = first
        .write_packet(extra, &packet(0, Some(1)))
        .expect_err("no such track");
    assert!(matches!(err, Error::TrackNotFound { track_id: 2, .. }));
}

#[test]
fn bad_track_specs_name_the_field() {
    let no_size = TrackSpec::new(TrackKind::Video, "V_VP9", MS);
    let empty_codec = audio("");
    let spaced_codec = audio("A OPUS");
    let mut zero_type = audio("A_OPUS");
    zero_type.matroska.track_type = 0;
    let mut zero_default = audio("A_OPUS");
    zero_default.matroska.default_duration = Some(0);
    let zero_base = TrackSpec::new(TrackKind::Audio, "A_OPUS", Timebase { num: 1, den: 0 });
    let mut zero_uid = audio("A_OPUS");
    zero_uid.matroska.uid = Some(0);
    for (spec, needle) in [
        (empty_codec, "CodecID"),
        (spaced_codec, "CodecID"),
        (zero_type, "TrackType"),
        (zero_default, "DefaultDuration"),
        (zero_base, "timebase"),
        (zero_uid, "TrackUID"),
    ] {
        let err = muxer(MkvOptions::default())
            .add_track(spec)
            .expect_err(needle);
        assert!(err.to_string().contains(needle), "{err}");
    }
    let err = muxer(MkvOptions::default())
        .add_track(no_size)
        .expect_err("no size");
    assert!(err.to_string().contains("PixelWidth"), "{err}");
    let mut with_uid = audio("A_OPUS");
    with_uid.matroska.uid = Some(5);
    let mut mux = muxer(MkvOptions::default());
    mux.add_track(with_uid.clone()).expect("a track");
    let err = mux.add_track(with_uid).expect_err("a repeated UID");
    assert!(err.to_string().contains("repeats"), "{err}");
}

#[test]
fn every_option_error_names_its_field() {
    for (options, field) in [
        (
            MkvOptions {
                timestamp_scale: 0,
                ..MkvOptions::default()
            },
            "timestamp_scale",
        ),
        (
            MkvOptions {
                timestamp_scale: 1 << 32,
                ..MkvOptions::default()
            },
            "timestamp_scale",
        ),
        (
            MkvOptions {
                cluster_duration_ms: 0,
                ..MkvOptions::default()
            },
            "cluster_duration_ms",
        ),
        (
            MkvOptions {
                cluster_bytes: 1,
                ..MkvOptions::default()
            },
            "cluster_bytes",
        ),
        (
            MkvOptions {
                max_buffer: 1,
                ..MkvOptions::default()
            },
            "max_buffer",
        ),
        (
            MkvOptions {
                title: Some("t".repeat(70_000)),
                ..MkvOptions::default()
            },
            "title",
        ),
    ] {
        match Muxer::new(Cursor::new(Vec::new()), options) {
            Err(err) => assert!(err.to_string().contains(field), "{err}"),
            Ok(_) => panic!("{field}: the options are not refused"),
        }
    }
}

#[test]
fn a_seekable_finish_on_a_target_with_no_seek_reports_the_seek() {
    let mux = Muxer::new(NoSeek(Vec::new()), MkvOptions::default()).expect("options");
    let err = mux.finish().expect_err("no seek");
    assert!(err.to_string().contains("seekable"), "{err}");
}
