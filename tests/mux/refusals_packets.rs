//! Each packet refusal: a packet that breaks a rule gives a named error and
//! leaves the muxer usable.

use std::io::Cursor;

use containr::mp4::{
    Brand, MuxOptions, Muxer, PacketIn, SampleEntrySpec, TrackId, TrackSpec, VisualCodec,
};
use containr::{Error, TrackKind};

fn muxer(brand: Brand) -> (Muxer<Cursor<Vec<u8>>>, TrackId) {
    let options = MuxOptions {
        brand,
        ..MuxOptions::default()
    };
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("the options are valid");
    let spec = TrackSpec::new(
        TrackKind::Video,
        1000,
        SampleEntrySpec::Visual {
            codec: VisualCodec::Avc,
            config: vec![1],
            width: 16,
            height: 16,
        },
    );
    let id = muxer.add_track(spec).expect("the spec is valid");
    (muxer, id)
}

fn input(pts: i64, dts: i64, duration: Option<u64>) -> PacketIn<'static> {
    PacketIn {
        data: &[1, 2, 3],
        pts,
        dts,
        duration,
        sync: true,
    }
}

#[test]
fn dts_out_of_order_is_refused_and_the_muxer_stays_usable() {
    let (mut muxer, id) = muxer(Brand::Isom);
    muxer.write_packet(id, &input(0, 0, None)).expect("first");
    muxer
        .write_packet(id, &input(10, 10, None))
        .expect("second");
    for dts in [5, 10] {
        let err = muxer.write_packet(id, &input(dts, dts, None));
        assert!(
            matches!(
                err,
                Err(Error::DtsOutOfOrder {
                    index: 2,
                    previous: 10,
                    ..
                })
            ),
            "{err:?}"
        );
    }
    muxer
        .write_packet(id, &input(20, 20, Some(10)))
        .expect("third");
    let bytes = muxer.finish().expect("the movie finishes").into_inner();
    let demuxer = containr::mp4::Demuxer::open(Cursor::new(bytes)).expect("the output reads");
    assert_eq!(demuxer.tracks()[0].sample_count, 3);
}

#[test]
fn a_first_dts_other_than_0_is_refused() {
    let (mut muxer, id) = muxer(Brand::Isom);
    let err = muxer.write_packet(id, &input(5, 5, None));
    assert!(
        matches!(err, Err(Error::InvalidPacket { index: 0, .. })),
        "{err:?}"
    );
}

#[test]
fn pts_before_dts_under_mp41_is_refused() {
    let (mut muxer, id) = muxer(Brand::Mp41);
    muxer.write_packet(id, &input(0, 0, None)).expect("first");
    let err = muxer.write_packet(id, &input(5, 10, None));
    match err {
        Err(Error::CompositionOffset {
            pts: 5, dts: 10, ..
        }) => {}
        other => panic!("{other:?}"),
    }
    let (mut muxer, id) = self::muxer(Brand::Isom);
    muxer.write_packet(id, &input(0, 0, None)).expect("first");
    muxer
        .write_packet(id, &input(5, 10, Some(1)))
        .expect("signed offset");
}

#[test]
fn composition_offsets_outside_ctts_are_refused() {
    for (pts, what) in [
        (i64::from(i32::MAX) + 1, "just above the signed range"),
        (
            i64::from(u32::MAX),
            "the version 0 maximum, which readers misread",
        ),
        (i64::from(i32::MIN) - 1, "below"),
        (i64::MAX, "max"),
    ] {
        let (mut muxer, id) = muxer(Brand::Isom);
        let err = muxer.write_packet(id, &input(pts, 0, None));
        assert!(
            matches!(err, Err(Error::CompositionOffset { .. })),
            "{what}: {err:?}"
        );
    }
    let (mut muxer, id) = muxer(Brand::Isom);
    muxer
        .write_packet(id, &input(i64::from(i32::MAX), 0, None))
        .expect("the top of the signed range fits");
}

#[test]
fn a_missing_last_duration_is_refused_at_finish() {
    let (mut muxer, id) = muxer(Brand::Isom);
    muxer.write_packet(id, &input(0, 0, None)).expect("first");
    muxer.write_packet(id, &input(1, 1, None)).expect("second");
    let err = muxer.finish();
    assert!(
        matches!(
            err,
            Err(Error::MissingDuration {
                track_id: 1,
                index: 1
            })
        ),
        "{:?}",
        err.map(|_| ())
    );
}

#[test]
fn a_default_duration_ends_the_track() {
    let options = MuxOptions::default();
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("the options are valid");
    let mut spec = TrackSpec::new(
        TrackKind::Video,
        1000,
        SampleEntrySpec::Visual {
            codec: VisualCodec::Avc,
            config: vec![1],
            width: 16,
            height: 16,
        },
    );
    spec.default_duration = Some(7);
    let id = muxer.add_track(spec).expect("the spec is valid");
    muxer.write_packet(id, &input(0, 0, None)).expect("first");
    let bytes = muxer.finish().expect("the default applies").into_inner();
    let demuxer = containr::mp4::Demuxer::open(Cursor::new(bytes)).expect("the output reads");
    assert_eq!(demuxer.tracks()[0].duration, Some(7));
}

#[test]
fn a_duration_that_disagrees_with_the_next_dts_is_refused() {
    let (mut muxer, id) = muxer(Brand::Isom);
    muxer
        .write_packet(id, &input(0, 0, Some(5)))
        .expect("first");
    let err = muxer.write_packet(id, &input(9, 9, None));
    assert!(
        matches!(err, Err(Error::InvalidPacket { index: 0, .. })),
        "{err:?}"
    );
}

#[test]
fn call_order_and_ids_are_checked() {
    let (mut muxer, id) = muxer(Brand::Isom);
    muxer
        .write_packet(id, &input(0, 0, Some(1)))
        .expect("first");
    let spec = TrackSpec::new(TrackKind::Audio, 48_000, SampleEntrySpec::Tx3g);
    assert!(matches!(
        muxer.add_track(spec),
        Err(Error::InvalidMux { .. })
    ));
    let (mut other, _) = self::muxer(Brand::Isom);
    let spec = TrackSpec::new(TrackKind::Video, 0, SampleEntrySpec::Tx3g);
    assert!(matches!(
        other.add_track(spec),
        Err(Error::InvalidMux { .. })
    ));
}

#[test]
fn a_reference_to_an_absent_track_is_refused_at_finish() {
    let options = MuxOptions::default();
    let mut muxer = Muxer::new(Cursor::new(Vec::new()), options).expect("the options are valid");
    let mut spec = TrackSpec::new(TrackKind::Video, 1000, SampleEntrySpec::Tx3g);
    spec.references.push(containr::TrackReference {
        kind: containr::FourCc(*b"tmcd"),
        track_ids: vec![9],
    });
    muxer.add_track(spec).expect("the spec is valid");
    assert!(matches!(muxer.finish(), Err(Error::InvalidMux { .. })));
}
