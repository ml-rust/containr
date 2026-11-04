//! `first_track_timing`: `moov` timing facts, read with no sample tables.

use std::io::Cursor;

use containr::mp4::{Demuxer, TrackTiming, first_track_timing};
use containr::{EditEntry, EditList, Error, TrackKind};

use crate::common::synth;

/// The box `kind` of `bytes` renamed to `to`.
fn rename_box(bytes: &mut [u8], kind: &[u8; 4], to: &[u8; 4]) {
    let at = bytes
        .windows(4)
        .position(|window| window == kind)
        .expect("the movie holds the box");
    bytes[at..at + 4].copy_from_slice(to);
}

fn timing(bytes: Vec<u8>, kind: TrackKind) -> Option<TrackTiming> {
    first_track_timing(Cursor::new(bytes), kind).expect("the timing boxes are well formed")
}

fn video() -> TrackTiming {
    TrackTiming {
        timescale: synth::VIDEO_TIMESCALE,
        edit_list: EditList {
            movie_timescale: synth::MOVIE_TIMESCALE,
            entries: vec![EditEntry {
                segment_duration: u64::from(synth::EDIT.0),
                media_time: i64::from(synth::EDIT.1),
                media_rate: EditEntry::NORMAL_RATE,
            }],
        },
    }
}

#[test]
fn the_first_track_of_a_kind_gives_its_timescale_and_edit_list() {
    for moov_first in [true, false] {
        let (bytes, _) = synth::movie(moov_first);
        assert_eq!(timing(bytes.clone(), TrackKind::Video), Some(video()));
        let audio = timing(bytes.clone(), TrackKind::Audio).expect("the movie has audio");
        assert_eq!(audio.timescale, synth::AUDIO_TIMESCALE);
        assert!(audio.edit_list.is_empty());
        assert_eq!(timing(bytes, TrackKind::Subtitle), None);
    }
}

#[test]
fn the_timing_matches_what_the_demuxer_reads() {
    let (bytes, _) = synth::movie(true);
    let demuxer = Demuxer::open(Cursor::new(bytes)).expect("the movie is well formed");
    let track = demuxer.track(synth::VIDEO_ID).expect("track 1 exists");
    assert_eq!(video().edit_list, track.edit_list);
}

#[test]
fn a_moof_after_moov_is_read_past() {
    let (mut bytes, _) = synth::movie(true);
    bytes.extend([0, 0, 0, 8, b'm', b'o', b'o', b'f']);
    assert!(matches!(
        Demuxer::open(Cursor::new(bytes.clone())),
        Err(Error::Fragmented { .. })
    ));
    assert_eq!(timing(bytes, TrackKind::Video), Some(video()));
}

#[test]
fn an_mvex_in_moov_is_read_past() {
    let (mut bytes, _) = synth::movie(false);
    let moov = crate::common::moov_range(&bytes);
    assert_eq!(moov.end, bytes.len(), "moov ends the file");
    let size = u32::try_from(moov.len() + 8).expect("the movie is small");
    bytes[moov.start..moov.start + 4].copy_from_slice(&size.to_be_bytes());
    bytes.extend([0, 0, 0, 8, b'm', b'v', b'e', b'x']);
    assert!(matches!(
        Demuxer::open(Cursor::new(bytes.clone())),
        Err(Error::Fragmented { .. })
    ));
    assert_eq!(timing(bytes, TrackKind::Video), Some(video()));
}

#[test]
fn a_box_past_the_input_after_moov_is_not_read() {
    let (mut bytes, _) = synth::movie(true);
    bytes.extend([0, 0, 0x10, 0, b'j', b'u', b'n', b'k']);
    assert!(Demuxer::open(Cursor::new(bytes.clone())).is_err());
    assert_eq!(timing(bytes, TrackKind::Video), Some(video()));
}

#[test]
fn a_sample_table_error_gives_no_error() {
    let (mut bytes, _) = synth::movie(true);
    rename_box(&mut bytes, b"stsz", b"xxxx");
    assert!(Demuxer::open(Cursor::new(bytes.clone())).is_err());
    assert_eq!(timing(bytes, TrackKind::Video), Some(video()));
}

#[test]
fn a_missing_moov_is_named() {
    let (bytes, _) = synth::movie(false);
    let moov = crate::common::moov_range(&bytes);
    match first_track_timing(Cursor::new(bytes[..moov.start].to_vec()), TrackKind::Audio) {
        Err(Error::MissingBox { path, .. }) => assert_eq!(path, "moov"),
        other => panic!("expected MissingBox, got {other:?}"),
    }
}

#[test]
fn a_missing_mvhd_is_named() {
    let (mut bytes, _) = synth::movie(true);
    rename_box(&mut bytes, b"mvhd", b"xxxx");
    match first_track_timing(Cursor::new(bytes), TrackKind::Audio) {
        Err(Error::MissingBox { path, .. }) => assert_eq!(path, "moov/mvhd"),
        other => panic!("expected MissingBox, got {other:?}"),
    }
}

#[test]
fn an_unreadable_input_gives_an_error_and_no_panic() {
    assert!(first_track_timing(Cursor::new(Vec::new()), TrackKind::Audio).is_err());
    let text = b"not an mp4 file".to_vec();
    assert!(first_track_timing(Cursor::new(text), TrackKind::Audio).is_err());
}
